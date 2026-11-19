use ascii::AsciiString;

use std::io::Error as IoError;
use std::io::Result as IoResult;
use std::io::{BufReader, BufWriter, ErrorKind, Read};

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::Limits;
use crate::Request;
use crate::common::{HTTPVersion, Method};
use crate::util::RefinedTcpStream;
use crate::util::{SequentialReader, SequentialReaderBuilder, SequentialWriterBuilder};

/// Landscaper patch: the most of a request's body left unread that is thrown away to keep the connection for the next
/// request; more closes it. The Landscaper's largest body is an upload of 150 MB.
const MAX_DISCARD_BYTES: u64 = 256 << 20;

/// A ClientConnection is an object that will store a socket to a client
/// and return Request objects.
pub struct ClientConnection {
    // address of the client
    remote_addr: IoResult<Option<SocketAddr>>,

    // sequence of Readers to the stream, so that the data is not read in
    //  the wrong order
    source: SequentialReaderBuilder<BufReader<RefinedTcpStream>>,

    // sequence of Writers to the stream, to avoid writing response #2 before
    //  response #1
    sink: SequentialWriterBuilder<BufWriter<RefinedTcpStream>>,

    // Reader to read the next header from
    next_header_source: SequentialReader<BufReader<RefinedTcpStream>>,

    // set to true if we know that the previous request is the last one
    no_more_requests: bool,

    // true if the connection goes through SSL
    secure: bool,

    // Landscaper patch: how big a request's head may be and how long reading it may take
    limits: Limits,

    // Landscaper patch: how much of the last request's body was left when it was dropped before its end, to throw away here,
    // on the connection's own thread, before the next request (`EqualReader::abandon_into`)
    abandoned: Arc<AtomicU64>,
}

/// Error that can happen when reading a request.
#[derive(Debug)]
enum ReadError {
    WrongRequestLine,
    WrongHeader(HTTPVersion),
    /// Landscaper patch: the request line was over `Limits::max_head_bytes` (answered 414)
    RequestLineTooLong,
    /// Landscaper patch: the request line and headers were over `Limits::max_head_bytes` (answered 431)
    HeadTooLarge(HTTPVersion),
    /// the client sent an unrecognized `Expect` header
    ExpectationFailed(HTTPVersion),
    /// the client sent a `Transfer-Encoding` header tiny-http cannot safely
    /// interpret, or both `Transfer-Encoding` and `Content-Length`
    InvalidTransferEncoding(HTTPVersion),
    ReadIoError(IoError),
}

impl ClientConnection {
    /// Creates a new `ClientConnection` that takes ownership of the `TcpStream`.
    pub fn new(
        write_socket: RefinedTcpStream,
        mut read_socket: RefinedTcpStream,
        limits: Limits,
    ) -> ClientConnection {
        let remote_addr = read_socket.peer_addr();
        let secure = read_socket.secure();

        let mut source = SequentialReaderBuilder::new(BufReader::with_capacity(1024, read_socket));
        let first_header = source.next().unwrap();

        ClientConnection {
            source,
            sink: SequentialWriterBuilder::new(BufWriter::with_capacity(1024, write_socket)),
            remote_addr,
            next_header_source: first_header,
            no_more_requests: false,
            secure,
            limits,
            abandoned: Arc::new(AtomicU64::new(0)),
        }
    }

    /// true if the connection is HTTPS
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Reads the next line from self.next_header_source.
    ///
    /// Reads until `CRLF` is reached. The next read will start
    ///  at the first byte of the new line.
    ///
    /// Landscaper patch: `left` is how many more bytes the request's head may hold (a longer line fails with
    /// `ErrorKind::InvalidData`, before more of it is kept), and `began` when the head's first byte arrived (set here when
    /// it does): reading the head fails with `ErrorKind::TimedOut` once `Limits::head_timeout` has passed since.
    fn read_next_line(
        &mut self,
        left: &mut usize,
        began: &mut Option<Instant>,
    ) -> IoResult<AsciiString> {
        let mut buf = Vec::with_capacity(64);
        let mut prev_byte_was_cr = false;

        loop {
            // `next_header_source` wraps a `BufReader`, so this doesn't issue a
            // syscall per byte despite the lint; `SequentialReader` doesn't
            // implement `BufRead` to let us use a buffered line reader directly.
            #[allow(clippy::unbuffered_bytes)]
            let byte = self.next_header_source.by_ref().bytes().next();

            let byte = match byte {
                Some(Ok(b)) => b,
                // A socket timeout during a head that has begun is a slow client: answered 408. (On Unix a read timeout is
                // `WouldBlock`.) Before the head's first byte it is an idle keep-alive connection, closed without a word.
                Some(Err(e))
                    if began.is_some()
                        && matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Err(IoError::new(
                        ErrorKind::TimedOut,
                        "the request's head stalled",
                    ));
                }
                Some(Err(e)) => return Err(e),
                None => return Err(IoError::new(ErrorKind::ConnectionAborted, "Unexpected EOF")),
            };

            let start = *began.get_or_insert_with(Instant::now);
            if self
                .limits
                .head_timeout
                .map_or(false, |t| start.elapsed() > t)
            {
                return Err(IoError::new(
                    ErrorKind::TimedOut,
                    "the request's head took too long",
                ));
            }
            if *left == 0 {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    "the request's head is too large",
                ));
            }
            *left -= 1;

            if byte == b'\n' && prev_byte_was_cr {
                buf.pop(); // removing the '\r'
                return AsciiString::from_ascii(buf)
                    .map_err(|_| IoError::new(ErrorKind::InvalidInput, "Header is not in ASCII"));
            }

            prev_byte_was_cr = byte == b'\r';

            buf.push(byte);
        }
    }

    /// Landscaper patch: waits for the last request to be answered, then throws away what was left of its body, if it wasn't
    /// read to its end: here, on the connection's own thread, not on the one that answered it. Too much of it (over
    /// `MAX_DISCARD_BYTES`), or a chunked body, can't be: an error, and the connection closes.
    fn discard_last_body(&mut self) -> IoResult<()> {
        self.next_header_source.wait_turn();
        match self.abandoned.swap(0, Ordering::SeqCst) {
            0 => Ok(()),
            left if left > MAX_DISCARD_BYTES => Err(IoError::new(
                ErrorKind::ConnectionAborted,
                "the last request's body was not read",
            )),
            left => self.discard(left),
        }
    }

    /// Landscaper patch: reads `n` bytes and throws them away.
    fn discard(&mut self, mut n: u64) -> IoResult<()> {
        let mut buf = [0u8; 8192];
        while n > 0 {
            let want = n.min(buf.len() as u64) as usize;
            match self.next_header_source.read(&mut buf[..want])? {
                0 => return Err(IoError::new(ErrorKind::ConnectionAborted, "Unexpected EOF")),
                got => n -= got as u64,
            }
        }
        Ok(())
    }

    /// Reads a request from the stream.
    /// Blocks until the header has been read.
    fn read(&mut self) -> Result<Request, ReadError> {
        let (method, path, version, headers) = {
            // reading the request line
            // (Landscaper patch: the last request's body first, then the head's size and time limits, shared by its request
            // line and headers.)
            self.discard_last_body().map_err(ReadError::ReadIoError)?;
            let mut left = self.limits.max_head_bytes;
            let mut began = None;
            let (method, path, version) = {
                let line =
                    self.read_next_line(&mut left, &mut began)
                        .map_err(|e| match e.kind() {
                            ErrorKind::InvalidData => ReadError::RequestLineTooLong,
                            _ => ReadError::ReadIoError(e),
                        })?;

                parse_request_line(
                    line.as_str().trim(), // TODO: remove this conversion
                )?
            };

            // getting all headers
            let headers = {
                let mut headers = Vec::with_capacity(8);
                loop {
                    let line =
                        self.read_next_line(&mut left, &mut began)
                            .map_err(|e| match e.kind() {
                                ErrorKind::InvalidData => ReadError::HeadTooLarge(version.clone()),
                                _ => ReadError::ReadIoError(e),
                            })?;

                    if line.is_empty() {
                        break;
                    };
                    headers.push(match FromStr::from_str(line.as_str().trim()) {
                        // TODO: remove this conversion
                        Ok(h) => h,
                        _ => return Err(ReadError::WrongHeader(version)),
                    });
                }

                headers
            };

            (method, path, version, headers)
        };

        // building the writer for the request
        let writer = self.sink.next().unwrap();

        // follow-up for next potential request
        let mut data_source = self.source.next().unwrap();
        std::mem::swap(&mut self.next_header_source, &mut data_source);

        // building the next reader
        let remote_addr = match self.remote_addr.as_ref() {
            Ok(addr) => *addr,
            Err(e) => return Err(ReadError::ReadIoError(IoError::from(e.kind()))),
        };

        let request = crate::request::new_request(
            self.secure,
            method,
            path,
            version.clone(),
            headers,
            remote_addr,
            data_source,
            writer,
            Some(self.abandoned.clone()),
        )
        .map_err(|e| {
            use crate::request;
            match e {
                request::RequestCreationError::CreationIoError(e) => ReadError::ReadIoError(e),
                request::RequestCreationError::ExpectationFailed => {
                    ReadError::ExpectationFailed(version)
                }
                request::RequestCreationError::InvalidTransferEncoding => {
                    ReadError::InvalidTransferEncoding(version)
                }
            }
        })?;

        // return the request
        Ok(request)
    }
}

impl Iterator for ClientConnection {
    type Item = Request;

    /// Blocks until the next Request is available.
    /// Returns None when no new Requests will come from the client.
    fn next(&mut self) -> Option<Request> {
        use crate::{Response, StatusCode};

        // the client sent a "connection: close" header in this previous request
        //  or is using HTTP 1.0, meaning that no new request will come
        if self.no_more_requests {
            // (Landscaper patch: the rest of the last body is thrown away before the connection closes too. Closed with it
            // unread, the connection is reset, and a client still sending it can lose its answer.)
            let _ = self.discard_last_body();
            return None;
        }

        loop {
            let rq = match self.read() {
                Err(ReadError::WrongRequestLine) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(400));
                    response
                        .raw_print(writer, HTTPVersion(1, 1), &[], false, None)
                        .ok();
                    return None; // we don't know where the next request would start,
                    // se we have to close
                }

                Err(ReadError::WrongHeader(ver)) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(400));
                    response.raw_print(writer, ver, &[], false, None).ok();
                    return None; // we don't know where the next request would start,
                    // se we have to close
                }

                // (Landscaper patch: a head over the limits is refused, and the connection closed: the rest of it is
                // not read.)
                Err(ReadError::RequestLineTooLong) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(414));
                    response
                        .raw_print(writer, HTTPVersion(1, 1), &[], false, None)
                        .ok();
                    return None;
                }

                Err(ReadError::HeadTooLarge(ver)) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(431));
                    response.raw_print(writer, ver, &[], false, None).ok();
                    return None;
                }

                Err(ReadError::ReadIoError(ref err)) if err.kind() == ErrorKind::TimedOut => {
                    // request timeout
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(408));
                    response
                        .raw_print(writer, HTTPVersion(1, 1), &[], false, None)
                        .ok();
                    return None; // closing the connection
                }

                Err(ReadError::ExpectationFailed(ver)) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(417));
                    response.raw_print(writer, ver, &[], true, None).ok();
                    return None; // TODO: should be recoverable, but needs handling in case of body
                }

                Err(ReadError::InvalidTransferEncoding(ver)) => {
                    let writer = self.sink.next().unwrap();
                    let response = Response::new_empty(StatusCode(400));
                    response.raw_print(writer, ver, &[], false, None).ok();
                    return None; // the message framing is ambiguous, so we
                    // cannot safely process any further data on this
                    // connection (see CVE-2026-66752)
                }

                Err(ReadError::ReadIoError(_)) => return None,

                Ok(rq) => rq,
            };

            // checking HTTP version
            if *rq.http_version() > (1, 1) {
                let writer = self.sink.next().unwrap();
                let response = Response::from_string(
                    "This server only supports HTTP versions 1.0 and 1.1".to_owned(),
                )
                .with_status_code(StatusCode(505));
                response
                    .raw_print(writer, HTTPVersion(1, 1), &[], false, None)
                    .ok();
                continue;
            }

            // updating the status of the connection
            let connection_header = rq
                .headers()
                .iter()
                .find(|h| h.field.equiv("Connection"))
                .map(|h| h.value.as_str());

            let lowercase = connection_header.map(|h| h.to_ascii_lowercase());

            match lowercase {
                Some(ref val) if val.contains("close") => self.no_more_requests = true,
                Some(ref val) if val.contains("upgrade") => self.no_more_requests = true,
                Some(ref val)
                    if !val.contains("keep-alive") && *rq.http_version() == HTTPVersion(1, 0) =>
                {
                    self.no_more_requests = true
                }
                None if *rq.http_version() == HTTPVersion(1, 0) => self.no_more_requests = true,
                _ => (),
            };

            // returning the request
            return Some(rq);
        }
    }
}

/// Parses a "HTTP/1.1" string.
fn parse_http_version(version: &str) -> Result<HTTPVersion, ReadError> {
    let (major, minor) = match version {
        "HTTP/0.9" => (0, 9),
        "HTTP/1.0" => (1, 0),
        "HTTP/1.1" => (1, 1),
        "HTTP/2.0" => (2, 0),
        "HTTP/3.0" => (3, 0),
        _ => return Err(ReadError::WrongRequestLine),
    };

    Ok(HTTPVersion(major, minor))
}

/// Parses the request line of the request.
/// eg. GET / HTTP/1.1
fn parse_request_line(line: &str) -> Result<(Method, String, HTTPVersion), ReadError> {
    let mut parts = line.split(' ');

    let method = parts.next().and_then(|w| w.parse().ok());
    let path = parts.next().map(ToOwned::to_owned);
    let version = parts.next().and_then(|w| parse_http_version(w).ok());

    method
        .and_then(|method| Some((method, path?, version?)))
        .ok_or(ReadError::WrongRequestLine)
}

#[cfg(test)]
mod test {
    #[test]
    fn test_parse_request_line() {
        let (method, path, ver) = super::parse_request_line("GET /hello HTTP/1.1").unwrap();

        assert!(method == crate::Method::Get);
        assert!(path == "/hello");
        assert!(ver == crate::common::HTTPVersion(1, 1));

        assert!(super::parse_request_line("GET /hello").is_err());
        assert!(super::parse_request_line("qsd qsd qsd").is_err());
    }
}
