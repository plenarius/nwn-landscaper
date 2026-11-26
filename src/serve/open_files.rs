// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The server's open files: each connection is one, from the moment it is accepted until it closes, so the process's limit on
//! open files bounds the connections it can hold. At the start the server raises its own limit as far as the system lets it
//! ([`raise_limit`]), and the connections it drops when it runs out anyway are said in its log ([`log_connection_drops`]).
//! The Forge's team server, under Docker's default limit of 1,024, ran out when the open editor tabs all reconnected after an
//! update, and stopped answering anyone until it was restarted (the vendored tiny_http's patch, `vendor/tiny_http/LANDSCAPER.md`).

/// Below this many open files the server says, at its start, that it may run out: it holds up to 1,024 connections
/// ([`super::CONNECTIONS`]: 512 read, 512 waiting) besides the files it reads and writes.
#[cfg(unix)]
const ENOUGH: u64 = 2048;

/// Raises the process's soft limit on open files (`RLIMIT_NOFILE`) to its hard limit, as servers commonly do: the soft limit is
/// often 1,024 (Linux, Docker) or 256 (macOS) when the hard one is far higher. Says so if what it gets is under [`ENOUGH`].
/// Nothing on Windows, whose sockets have no such limit.
pub(super) fn raise_limit() {
    #[cfg(unix)]
    match rlimit::increase_nofile_limit(u64::MAX) {
        Ok(n) if n < ENOUGH => eprintln!(
            "WARNING: the server may only have {n} files open at once (RLIMIT_NOFILE, at its hard limit), and each connection is \
             one: past that it drops new connections until some close. Raise the hard limit (Docker: --ulimit \
             nofile=65536:65536)."
        ),
        Ok(_) => {}
        Err(e) => eprintln!("WARNING: raising the limit on open files (RLIMIT_NOFILE): {e}"),
    }
}

/// Writes the vendored tiny_http's warnings to standard error, where the rest of the server's log goes: the connections it
/// drops because it can't take them (no file descriptor left, every thread busy), once a burst (tiny_http's `DropLog`).
/// Nothing else logs through the `log` crate.
pub(super) fn log_connection_drops() {
    if log::set_logger(&TinyHttpWarnings).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

struct TinyHttpWarnings;

impl log::Log for TinyHttpWarnings {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("tiny_http")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tiny_http::{Response, Server};

    /// Set in the child process [`a_server_out_of_file_descriptors_answers_once_some_close`] runs its server in.
    const CHILD: &str = "LANDSCAPER_TEST_OPEN_FILES_CHILD";

    /// The child's limit on open files: enough for the test's own (the test harness, the listener, a few more), and for
    /// [`CONNECTIONS`] connections at one file each, but not at four, as before.
    const LIMIT: u64 = 160;
    const CONNECTIONS: usize = 50;

    /// What the server answers on `s` to a `GET /`, read up to its body.
    fn ask(s: &mut TcpStream, close: bool) -> String {
        let connection = if close { "close" } else { "keep-alive" };
        s.write_all(format!("GET / HTTP/1.1\r\nHost: x\r\nConnection: {connection}\r\n\r\n").as_bytes()).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        while !out.ends_with(b"\r\n\r\nok") {
            match s.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn connect(port: u16) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s
    }

    /// The server runs out of file descriptors: it can't accept the connection waiting for it, nor any other, and says so
    /// once. Once some close it takes that connection, answers it and the next, and no thread has panicked. Before, the first
    /// accept that failed ended its accept thread: the process ran on and answered no one again.
    #[test]
    fn a_server_out_of_file_descriptors_answers_once_some_close() {
        if std::env::var_os(CHILD).is_some() {
            return serve_out_of_files();
        }
        // The limit is the whole process's, and other tests run alongside this one: the server runs in a process of its own.
        // Its clients run here, so a client's socket never takes, or gives back, a file the server is short of.
        let name = "serve::open_files::tests::a_server_out_of_file_descriptors_answers_once_some_close";
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut to_child = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok);
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let (said, log) = std::sync::mpsc::channel();
        let log_lines = std::thread::spawn(move || {
            stderr.lines().map_while(Result::ok).inspect(|l| drop(said.send(l.clone()))).collect::<Vec<_>>().join("\n")
        });
        // (Each at the end of a line: the harness starts the first with the test's name.)
        let port: u16 = stdout.find_map(|l| l.rsplit_once("port ").map(|(_, p)| p.parse().unwrap())).expect("the child's server started");

        // Connections held open cost the server one file each.
        let mut held: Vec<TcpStream> = (0..CONNECTIONS).map(|_| connect(port)).collect();
        for s in &mut held {
            assert!(ask(s, false).ends_with("ok"), "a connection held open is answered");
        }

        // Every file descriptor the server may have taken. An accept waiting for a connection has its file already (Linux
        // takes it first): this connection, held open without a request, takes it if so, and the next accept fails. If not,
        // the server can't accept this one either. Either way it says so, and from then on can take no connection.
        writeln!(to_child, "fill").unwrap();
        assert!(stdout.any(|l| l.ends_with("full")), "the child took its files");
        let mut idle = connect(port);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match log.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(l) if l.contains("dropping connections") => break,
                Ok(_) => {}
                Err(e) => panic!("the server didn't say it couldn't accept a connection: {e}"),
            }
        }
        let mut waiting = connect(port);
        waiting.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        // Unanswered meanwhile, as the accept fails a few more times (5, 10, 20, 40... ms apart).
        waiting.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
        let unanswered = waiting.read(&mut [0u8; 1]);
        assert!(
            unanswered.as_ref().is_err_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
            "the waiting connection isn't answered while the server has no file for it: {unanswered:?}"
        );
        waiting.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

        writeln!(to_child, "release").unwrap();
        let start = Instant::now();
        let mut answer = String::new();
        let _ = waiting.read_to_string(&mut answer);
        assert!(answer.ends_with("ok"), "the waiting connection is answered once files close: {answer:?}");
        assert!(start.elapsed() < Duration::from_secs(5), "within the accept's longest pause: {:?}", start.elapsed());

        assert!(ask(&mut connect(port), true).ends_with("ok"), "and so is a new one");
        for s in held.iter_mut().chain([&mut idle]) {
            assert!(ask(s, false).ends_with("ok"), "and the connections held open throughout");
        }

        drop(to_child);
        let status = child.wait().unwrap();
        let (stdout, stderr) = (stdout.collect::<Vec<_>>().join("\n"), log_lines.join().unwrap());
        assert!(status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("1 passed"), "the child ran the test: {stdout}");
        assert!(!stderr.contains("panicked"), "no thread panicked: {stderr}");
        assert_eq!(stderr.matches("dropping connections: accepting a connection").count(), 1, "said once: {stderr}");
        assert!(stderr.contains("taking connections again"), "{stderr}");
    }

    /// The child's side: a server on [`LIMIT`] files, which takes every file left when the parent says `fill` and closes them
    /// when it says `release`.
    fn serve_out_of_files() {
        super::log_connection_drops();
        let (_, hard) = rlimit::Resource::NOFILE.get().unwrap();
        rlimit::Resource::NOFILE.set(LIMIT, hard).unwrap();

        let server = Arc::new(Server::http_with_limits("127.0.0.1:0", super::super::CONNECTIONS, super::super::LIMITS).unwrap());
        println!("port {}", server.server_addr().to_ip().unwrap().port());
        std::thread::spawn(move || {
            super::super::answer_on_threads(&server, 4, |req| {
                let _ = req.respond(Response::from_string("ok"));
            });
        });

        let said = |what: &str| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), what);
        };
        said("fill");
        let mut files = Vec::new();
        while let Ok(f) = std::fs::File::open("/dev/null") {
            files.push(f);
        }
        assert!(!files.is_empty());
        println!("full");
        said("release");
        drop(files);
        // Answering until the parent is done.
        said("");
    }
}
