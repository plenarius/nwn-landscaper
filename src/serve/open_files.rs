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
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tiny_http::{Response, Server};

    /// Set in the child process [`a_server_out_of_file_descriptors_answers_once_some_close`] runs itself in.
    const CHILD: &str = "LANDSCAPER_TEST_OPEN_FILES_CHILD";

    /// The child's limit on open files: enough for the test's own (the test harness, the listener, a few more), and for
    /// [`CONNECTIONS`] connections at one file each on either end, but not at four on the server's end, as before.
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
        if std::env::var_os(CHILD).is_none() {
            // The limit is the whole process's, and other tests run alongside this one: it runs in a process of its own.
            let name = "serve::open_files::tests::a_server_out_of_file_descriptors_answers_once_some_close";
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success(), "{stdout}\n{stderr}");
            assert!(stdout.contains("1 passed"), "the child ran the test: {stdout}");
            assert!(!stderr.contains("panicked"), "no thread panicked: {stderr}");
            assert_eq!(stderr.matches("dropping connections: accepting a connection").count(), 1, "said once: {stderr}");
            assert!(stderr.contains("taking connections again"), "{stderr}");
            return;
        }

        super::log_connection_drops();
        let (_, hard) = rlimit::Resource::NOFILE.get().unwrap();
        rlimit::Resource::NOFILE.set(LIMIT, hard).unwrap();

        let server = Arc::new(Server::http_with_limits("127.0.0.1:0", super::super::CONNECTIONS, super::super::LIMITS).unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            super::super::answer_on_threads(&server, 4, |req| {
                let _ = req.respond(Response::from_string("ok"));
            });
        });

        // Connections held open cost the server one file each.
        let mut held: Vec<TcpStream> = (0..CONNECTIONS).map(|_| connect(port)).collect();
        for s in &mut held {
            assert!(ask(s, false).ends_with("ok"), "a connection held open is answered");
        }

        // Every file descriptor taken, but the one a client then connects with: the server can't accept it.
        let mut files = Vec::new();
        while let Ok(f) = std::fs::File::open("/dev/null") {
            files.push(f);
        }
        files.pop();
        let mut waiting = connect(port);
        waiting.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        // Long enough for the accept to fail a few times (5, 10, 20, 40... ms apart).
        std::thread::sleep(Duration::from_millis(400));

        drop(files);
        let start = Instant::now();
        let mut answer = String::new();
        let _ = waiting.read_to_string(&mut answer);
        assert!(answer.ends_with("ok"), "the waiting connection is answered once files close: {answer:?}");
        assert!(start.elapsed() < Duration::from_secs(5), "within the accept's longest pause: {:?}", start.elapsed());

        assert!(ask(&mut connect(port), true).ends_with("ok"), "and so is a new one");
        for s in &mut held {
            assert!(ask(s, false).ends_with("ok"), "and the connections held open throughout");
        }
    }
}
