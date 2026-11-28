// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `landscaper serve`: the engine behind the web editor (landscaper-web), on 127.0.0.1 or as a team server.
//!
//! This file holds what every request goes through: the request gate ([`Gate`]), who is asking, the body limits, the
//! threads and the catching of a panic. The routes are in `serve/`, one file per area of the API, each reading its
//! parameters through one parser (`serve/query.rs`).

pub(crate) mod app;
mod areas;
mod blueprints;
mod canvases;
mod dialogs;
mod export;
mod files;
mod import;
mod open_files;
mod packs;
mod previews;
mod query;
mod scripts;
mod service;
mod skies;
mod team;
mod underlay;

use std::path::Path;

use anyhow::Result;
use tiny_http::{Header, Method, Response, Server};

use self::query::Query;
use crate::workspace::exports_dir;

/// A response header the server writes itself. `Header::from_bytes` refuses only a name or value that isn't ASCII, and every one
/// given here is: a fixed text, hex digits, or a name the routes checked (an area or user name).
fn response_header(name: &str, value: impl Into<Vec<u8>> + AsRef<[u8]>) -> Header {
    Header::from_bytes(name, value).expect("a header the server writes is ASCII")
}

fn json_response(status: u16, value: &serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(serde_json::to_vec(value).unwrap_or_default())
        .with_status_code(status)
        .with_header(response_header("Content-Type", "application/json"))
}

/// The request header the editor sends with every API call. A web page on another site can't add a header of its own to a
/// request without asking first (a CORS preflight), and the server never says yes to one, so a request that carries it came
/// from the editor (or a tool), not from a page the builder happened to visit.
const API_HEADER: &str = "X-Landscaper";

/// Which requests the server answers at all, before it looks at who is asking: its `Host` (against DNS rebinding, a page
/// that renames its own site to the server's address to read its answers), and for the API, the [`API_HEADER`], `Origin`
/// and `Sec-Fetch-Site` (against cross-site requests: a page that sends the builder's browser to the API).
#[derive(Clone, Debug)]
struct Gate {
    port: u16,
    team: bool,
    /// `LANDSCAPER_PUBLIC_HOST`: the names the server is reached by through its proxy (comma separated, lower case; a name
    /// without a port matches any port).
    public: Vec<String>,
}

/// API answers a page loads as a picture (`<img>`, three.js's texture loader), which can't carry a header: a GET of one is
/// let through without it (still subject to the `Host`, `Origin` and `Sec-Fetch-Site` checks). Another site can at most
/// show such a picture, never read it. Everything else under `/api/` (a preview's `scene.json` too) needs the header.
fn picture_route(path: &str) -> bool {
    ["/api/texture/", "/api/import/", "/api/import-file/", "/api/preview-icon/", "/api/sky-picture/"].iter().any(|p| path.starts_with(p))
        || path.strip_prefix("/api/preview-file/").and_then(|rest| rest.split_once('/')).is_some_and(|(_, file)| file.starts_with("tex/"))
}

/// `host` (`name` or `name:port`, lower case) without its port.
fn host_name(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

impl Gate {
    fn new(port: u16, team: bool, public_host: Option<&str>) -> Gate {
        let public = public_host
            .unwrap_or_default()
            .split(',')
            .map(|h| {
                let h = h.trim().to_ascii_lowercase();
                let h = h.strip_prefix("https://").or_else(|| h.strip_prefix("http://")).unwrap_or(&h);
                h.trim_end_matches('/').to_string()
            })
            .filter(|h| !h.is_empty())
            .collect();
        Gate { port, team, public }
    }

    fn from_env(port: u16, team: bool) -> Gate {
        Gate::new(port, team, crate::config::setting("LANDSCAPER_PUBLIC_HOST").as_deref())
    }

    /// Whether `host` is one of the public names.
    fn public_host(&self, host: &str) -> bool {
        self.public.iter().any(|p| host == p || (!p.contains(':') && host_name(host) == p))
    }

    /// Whether a request for `host` (its `Host` header, lower case) is for this server: on this machine by its loopback
    /// address and port, or by a public name. A team server with no public name set takes any (said at start).
    fn host_ok(&self, host: &str) -> bool {
        let local =
            ["127.0.0.1", "localhost", "[::1]"].iter().any(|h| host == format!("{h}:{}", self.port) || (self.port == 80 && host == *h));
        local || self.public_host(host) || (self.team && self.public.is_empty())
    }

    /// Why a request is refused, or None to answer it. `header` reads a request header.
    fn refuse(&self, method: &Method, path: &str, header: &dyn Fn(&str) -> Option<String>) -> Option<&'static str> {
        let host = header("Host").map_or_default(|h| h.trim().to_ascii_lowercase());
        if !self.host_ok(&host) {
            return Some("refused: this server isn't reached by that name (the Host header); see LANDSCAPER_PUBLIC_HOST");
        }
        let api = path.starts_with("/api/");
        let reading = matches!(method, Method::Get | Method::Head);
        if (api || !reading) && header("Sec-Fetch-Site").is_some_and(|s| s.trim().eq_ignore_ascii_case("cross-site")) {
            return Some("refused: a request from another site");
        }
        // (A page's own GET of the editor's files carries no Origin; an API request or a change that carries one must be ours.)
        if (api || !reading)
            && let Some(origin) = header("Origin")
        {
            let origin = origin.trim().to_ascii_lowercase();
            let forwarded = header("X-Forwarded-Host").map(|h| h.split(',').next().unwrap_or("").trim().to_ascii_lowercase());
            let ours = origin
                .strip_prefix("https://")
                .or_else(|| origin.strip_prefix("http://"))
                .is_some_and(|o| !o.is_empty() && (o == host || forwarded.as_deref() == Some(o) || self.public_host(o)));
            if !ours {
                return Some("refused: a request from another site (the Origin header)");
            }
        }
        if api && !(*method == Method::Get && picture_route(path)) && header(API_HEADER).is_none_or(|v| v.trim() != "1") {
            return Some("refused: an API request without the X-Landscaper: 1 header (the editor sends it)");
        }
        None
    }
}

/// How many requests a team server answers at once; more wait in the listener's queue. Heavy jobs take turns anyway
/// ([`job`]); this bounds the light ones. Connections are bounded by [`CONNECTIONS`]. A client can't keep one of these threads:
/// [`LIMITS`] and [`Paced`] drop a connection that stops sending, or sends too slowly.
const TEAM_THREADS: usize = 32;

/// The threads that read connections (tiny_http_dh's pool, one thread a connection while it is open): at most 512, and 512 more
/// connections wait their turn; past that a new connection is closed at once. tiny_http_dh's default is 64 threads, which 64
/// connections held open for [`LIMITS`]' 30 s would fill, the server answering no one else meanwhile. The sign-in proxy in
/// front limits connections too.
const CONNECTIONS: tiny_http::PoolConfig = tiny_http::PoolConfig { min_threads: 4, max_threads: 512, max_queue: 512 };

/// What a connection may cost before it is dropped (the vendored tiny_http's patch, `vendor/tiny_http/LANDSCAPER.md`):
/// - 30 s at most waiting for a read or a write: a request whose body stops arriving fails to read, and a connection idle
///   between requests is closed;
/// - 30 s to send a request's line and headers, and 1 MB for them, read as they arrive, before the gate or the sign-in look at
///   the request: a longer request line is answered 414, longer headers 431. 1 MB is what Caddy and Go's servers take by
///   default, so a proxy in front never passes a request this refuses. (No route needs a long query string: the export's
///   palette, water, entrances and atmosphere come in its body.)
const LIMITS: tiny_http::Limits = tiny_http::Limits {
    io_timeout: Some(std::time::Duration::from_secs(30)),
    head_timeout: Some(std::time::Duration::from_secs(30)),
    max_head_bytes: 1 << 20,
};

/// `landscaper serve`. `reload_on_rebuild` (`--reload-on-rebuild`, your own machine only): after each request, restart on
/// the program file when it has changed since the start (a new build). For working on the Landscaper; off by default, so
/// installing an update never restarts a session in the middle of an edit.
pub fn serve(bind: &str, port: u16, web: &Path, reload_on_rebuild: bool) -> Result<()> {
    anyhow::ensure!(
        web.join("index.html").is_file(),
        "{} has no index.html (set LANDSCAPER_WEB or pass --web <landscaper-web folder>)",
        web.display()
    );
    // A team server's records must read before it answers anyone: one that doesn't parse would be replaced by the next change.
    if crate::workspace::team() {
        crate::assignments::check()?;
        crate::marker::check()?;
    }
    if crate::review::active() {
        crate::review::read_kept_areas();
    }
    open_files::raise_limit();
    open_files::log_connection_drops();
    let server = Server::http_with_limits((bind, port), CONNECTIONS, LIMITS).map_err(|e| anyhow::anyhow!("{e}"))?;
    // (`--port 0` takes a free port: the one it got is the one printed below and checked against `Host`.)
    let port = server.server_addr().to_ip().map_or(port, |a| a.port());
    let gate = Gate::from_env(port, crate::workspace::team());
    if gate.team && gate.public.is_empty() {
        eprintln!(
            "WARNING: LANDSCAPER_PUBLIC_HOST is not set, so requests for any host name are answered. Set it to the name people \
             reach the server by (e.g. landscaper.example.com) to refuse the rest (DNS rebinding)."
        );
    }
    if crate::workspace::team() {
        let secret = crate::config::setting("LANDSCAPER_PROXY_SECRET").is_some_and(|s| !s.is_empty());
        println!(
            "Landscaper team server on {bind}:{port} (serving {}; workspaces in {}; proxy secret {})",
            web.display(),
            crate::workspace::workspaces_dir().display(),
            if secret { "required" } else { "NOT set (--insecure): only run it where nothing but the sign-in proxy can reach it" }
        );
    } else {
        println!("Landscaper editor: http://{bind}:{port}/   (serving {}; exports in {})", web.display(), exports_dir().display());
        if OPEN_BROWSER.load(std::sync::atomic::Ordering::Relaxed) {
            open_browser(&format!("http://{bind}:{port}/"));
            println!("To stop the Landscaper, close this window or press Ctrl+C.");
        }
    }
    // The list of existing areas takes seconds to read: have it ready before the first search.
    std::thread::spawn(|| {
        let _ = crate::import::sources();
    });
    // A team server answers on a fixed set of threads; only heavy jobs take turns.
    if crate::workspace::team() {
        answer_on_threads(&server, TEAM_THREADS, |req| guarded(req, |r| answer(r, web, &gate)));
        return Ok(());
    }
    // With --reload-on-rebuild a rebuilt landscaper takes over: after answering a request, if the program file changed since
    // it started, it runs the new one in its place (same arguments). A stale server once hid a feature for a day.
    let exe = std::env::current_exe().ok().filter(|_| reload_on_rebuild);
    let built = exe.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    // On your own machine requests are answered one at a time, in order.
    while let Ok(req) = server.recv() {
        guarded(req, |r| answer(r, web, &gate));
        if let (Some(exe), Some(built)) = (&exe, built)
            && std::fs::metadata(exe).and_then(|m| m.modified()).is_ok_and(|now| now != built)
        {
            eprintln!("landscaper was rebuilt: restarting the server on the new build");
            drop(server);
            let mut again = std::process::Command::new(exe);
            again.args(std::env::args_os().skip(1));
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let err = again.exec();
                anyhow::bail!("restarting on the new build: {err}");
            }
            // No exec elsewhere: the new build runs in this window, and this one ends.
            #[cfg(not(unix))]
            {
                again.spawn().map_err(|e| anyhow::anyhow!("restarting on the new build: {e}"))?;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Set when the program was started with no arguments (a double-click, main.rs): once it listens, the browser opens at the editor.
pub static OPEN_BROWSER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Opens `url` in the default browser (`start` on Windows, `open` on macOS, `xdg-open` elsewhere); if that can't be run, says
/// to open it.
pub fn open_browser(url: &str) {
    let (program, first): (&str, &[&str]) = if cfg!(windows) {
        // `start`'s first quoted argument is a window title: an empty one, so the address is taken as what to open.
        ("cmd", &["/C", "start", ""])
    } else if cfg!(target_os = "macos") {
        ("open", &[])
    } else {
        ("xdg-open", &[])
    };
    let started = std::process::Command::new(program)
        .args(first)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match started {
        Ok(mut child) => {
            println!("Opening {url} in your browser.");
            std::thread::spawn(move || child.wait());
        }
        Err(_) => println!("Open {url} in your browser."),
    }
}

/// Heavy jobs (a check, an export, an import) take turns: they keep the area they work on in process-wide state and are heavy on the
/// machine. Everything else (searching, listing, loading, saving) goes on meanwhile.
fn job() -> std::sync::MutexGuard<'static, ()> {
    static JOB: std::sync::Mutex<()> = std::sync::Mutex::new(());
    JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The browser build (`landscaper wasi-request [REQUEST] [ANSWER]`, docs/DESIGN-web-demo.md): answers one API request, read whole
/// as HTTP/1.x (request line, headers, blank line, body) from the file REQUEST or standard input, and writes the answer to the file
/// ANSWER or standard output (a file keeps it apart from what the engine prints). The page runs the engine once per request.
#[cfg(any(feature = "web", test))]
pub fn wasi_request(request: Option<&Path>, answer_to: Option<&Path>) -> Result<()> {
    use std::io::{Read, Write};
    let raw = match request {
        Some(p) => std::fs::read(p)?,
        None => {
            let mut raw = Vec::new();
            std::io::stdin().read_to_end(&mut raw)?;
            raw
        }
    };
    let answer = answer_raw(&raw, Path::new("/web"))?;
    match answer_to {
        Some(p) => std::fs::write(p, answer)?,
        None => {
            let mut out = std::io::stdout().lock();
            out.write_all(&answer)?;
            out.flush()?;
        }
    }
    Ok(())
}

/// The answer to the HTTP/1.x request `raw`, as [`answer`] gives it, written out as HTTP/1.0 (no chunks to undo).
#[cfg(any(feature = "web", test))]
fn answer_raw(raw: &[u8], web: &Path) -> Result<Vec<u8>> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| anyhow::anyhow!("no blank line after the head"))?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    // (tiny_http's test request takes a body that lives for ever: one request per run, so it is let go of with the process.)
    let body: &'static [u8] = Box::leak(raw[split + 4..].to_vec().into_boxed_slice());
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method: Method = first.next().unwrap_or("GET").parse().map_err(|()| anyhow::anyhow!("not an HTTP method"))?;
    let path = first.next().unwrap_or("/").to_string();
    let mut req = tiny_http::TestRequest::new().with_method(method).with_path(&path).with_body_bytes(body);
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            req = req.with_header(Header::from_bytes(k.trim(), v.trim()).map_err(|()| anyhow::anyhow!("a header that isn't ASCII: {k}"))?);
        }
    }
    let mut req: tiny_http::Request = req.into();
    // (On the page's own origin, which the page's Service Worker checked: the gate sees the editor's local host.)
    let gate = Gate::new(80, false, None);
    crate::workspace::begin_request();
    let mut out = Vec::new();
    let v10 = tiny_http::HTTPVersion(1, 0);
    match answer(&mut req, web, &gate) {
        Reply::Data(a) => a.raw_print(&mut out, v10, &[], false, None)?,
        Reply::Stream(s) => s.raw_print(&mut out, v10, &[], false, None)?,
    }
    Ok(out)
}

/// What every route answers with.
type Answer = Response<std::io::Cursor<Vec<u8>>>;

/// An answer as it is sent: in memory, or streamed (a hak for the service routes, which can be 80 MB).
enum Reply {
    Data(Answer),
    Stream(tiny_http::ResponseBox),
}

impl From<Answer> for Reply {
    fn from(a: Answer) -> Self {
        Reply::Data(a)
    }
}

impl Reply {
    fn status(&self) -> u16 {
        match self {
            Reply::Data(a) => a.status_code().0,
            Reply::Stream(s) => s.status_code().0,
        }
    }

    fn with_headers(self, headers: Vec<Header>) -> Reply {
        match self {
            Reply::Data(a) => Reply::Data(headers.into_iter().fold(a, Response::with_header)),
            Reply::Stream(s) => Reply::Stream(headers.into_iter().fold(s, Response::with_header)),
        }
    }
}

/// Answers `server`'s requests on `threads` threads, each taking the next request when it is done with one; the rest wait
/// in the server's queue. Returns when the server stops.
fn answer_on_threads(server: &Server, threads: usize, answer: impl Fn(tiny_http::Request) + Sync) {
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Ok(req) = server.recv() {
                    answer(req);
                }
            });
        }
    });
}

/// Answers `req` with what `make` makes of it. A panic in `make` (a bug: an `unwrap` that failed, an index out of range)
/// answers 500 and the server goes on with the next request; the panic's message and place are in the server's log
/// (Rust's panic hook prints them). Who is asking is forgotten before and after, so it never carries over to the next
/// request on this thread.
fn guarded<R: Into<Reply>>(mut req: tiny_http::Request, make: impl FnOnce(&mut tiny_http::Request) -> R) {
    crate::workspace::begin_request();
    crate::audit::set_address(String::new());
    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(&mut req).into()));
    crate::workspace::begin_request();
    let response = made.unwrap_or_else(|_| {
        eprintln!("{} {}: answered 500 after a panic (above)", req.method(), req.url());
        json_response(500, &serde_json::json!({ "error": "the server hit a bug answering this; the server's log has the details" })).into()
    });
    let _ = match response {
        Reply::Data(a) => req.respond(a),
        Reply::Stream(s) => req.respond(s),
    };
}

/// The most a request body may hold, by what it carries. What the editor sends, with room to spare: an area is at most
/// 32 by 32 tiles, 513 x 513 samples, so its heights are 1.05 MB as f32 and its paint 0.26 MB (1.32 MB in an export's
/// body, 1.75 MB as the base64 of a save), plus the JSON of its objects. A body over its route's limit is refused with 413
/// before it is read, or as soon as it passes the limit when it doesn't say its length.
mod body_max {
    /// An area: `/api/save`, `/api/export`, `/api/check`.
    pub const AREA: usize = 8 << 20;
    /// A canvas save (`/api/canvas-save`): every block of a 128 by 128 tile canvas in one request, since a stroke that changes a
    /// row two blocks share has to save both at once (64 blocks of 257 by 257 samples with a chasm mask, 0.53 MB each as
    /// base64: 34 MB), and a header as large as an area's whole body.
    pub const CANVAS: usize = 48 << 20;
    /// A conversation's JSON (`/api/dialog-save`).
    pub const DIALOG: usize = 4 << 20;
    /// A script's source (`/api/script-save`); the game's own `nwscript.nss` is under 0.3 MB.
    pub const SCRIPT: usize = 1 << 20;
    /// A few fields of JSON (a blueprint's fields, its gear, an item property).
    pub const FIELDS: usize = 256 << 10;
    /// An area's underlay picture (`/api/underlay`): a battlemat or a scanned map page.
    pub const UNDERLAY: usize = 16 << 20;
}

/// The body of `req`, up to `max` bytes, or the 413 that refuses it. A body that stops arriving or comes too slowly
/// ([`Paced`]) is answered 408, and one person may have at most [`Reading::MOST`] bodies arriving at once (else 429).
fn read_body(req: &mut tiny_http::Request, max: usize) -> std::result::Result<Vec<u8>, Answer> {
    let Some(_reading) = Reading::begin(&crate::workspace::acting_user()) else {
        return Err(refuse(429, "other requests of yours are still sending: try again once they are done"));
    };
    read_body_from(req.body_length(), &mut Paced::new(req.as_reader()), max)
}

/// One person's request body being read (see [`read_body`]); counted until dropped. A body that stalls holds its answer
/// thread until [`LIMITS`]' 30 s timeout: without a limit per person, one builder sending 32 of them, again and again, kept a
/// team server from answering anyone else. Uploads take turns of their own (`upload.rs`).
struct Reading(String);

impl Reading {
    /// The most bodies one person's requests may be sending at once: a quarter of the team server's [`TEAM_THREADS`]. The
    /// editor sends one or two at a time (a save, a check); two tabs and an autosave a few more (`tests/serve_guard.rs` sends
    /// eight saves at once).
    const MOST: usize = 8;

    fn now() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, usize>> {
        static NOW: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> =
            std::sync::LazyLock::new(Default::default);
        NOW.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `user`'s turn to send a body, if they have fewer than [`Reading::MOST`] arriving.
    fn begin(user: &str) -> Option<Reading> {
        let mut now = Reading::now();
        let n = now.entry(user.to_string()).or_default();
        if *n >= Self::MOST {
            return None;
        }
        *n += 1;
        Some(Reading(user.to_string()))
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        let mut now = Reading::now();
        if let Some(n) = now.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                now.remove(&self.0);
            }
        }
    }
}

/// [`read_body`] from `body`, which says it is `length` bytes long (if it does).
fn read_body_from(length: Option<usize>, body: &mut dyn std::io::Read, max: usize) -> std::result::Result<Vec<u8>, Answer> {
    let too_big = || json_response(413, &serde_json::json!({ "error": format!("the request is over {} KB", max >> 10) }));
    if length.is_some_and(|n| n > max) {
        return Err(too_big());
    }
    let mut out = Vec::new();
    match std::io::Read::read_to_end(&mut std::io::Read::take(body, max as u64 + 1), &mut out) {
        Ok(_) if out.len() > max => Err(too_big()),
        Ok(_) => Ok(out),
        Err(e) if stalled(&e) => Err(refuse(408, "the request's body stopped arriving, or came too slowly: send it again")),
        Err(e) => Err(json_response(400, &serde_json::json!({ "error": format!("reading the request: {e}") }))),
    }
}

/// A request's body as it arrives, which fails once it comes slower than [`Paced::MIN_RATE`] on average, after the first
/// [`Paced::GRACE`]. A read that waits for nothing fails anyway after [`LIMITS`]' 30 s, but a client sending a byte every
/// 29 s would still have held its thread (and an upload its turn, `upload.rs`) for as long as the body's length allowed.
/// 16 KB/s is a 150 MB upload in 2.6 hours, slower than any connection a builder works on; an 8 MB area in 8.5 minutes.
pub(super) struct Paced<R> {
    inner: R,
    start: std::time::Instant,
    read: u64,
    grace: std::time::Duration,
    min_rate: f64,
}

impl<R: std::io::Read> Paced<R> {
    /// How long a body may take to get going.
    const GRACE: std::time::Duration = std::time::Duration::from_secs(20);
    /// The slowest a body may come on average after [`Paced::GRACE`], in bytes per second.
    const MIN_RATE: f64 = 16.0 * 1024.0;

    /// `inner`, cut off once it falls behind [`Paced::MIN_RATE`] after [`Paced::GRACE`].
    pub(super) fn new(inner: R) -> Self {
        Self::with(inner, Self::GRACE, Self::MIN_RATE)
    }

    fn with(inner: R, grace: std::time::Duration, min_rate: f64) -> Self {
        Paced { inner, start: std::time::Instant::now(), read: 0, grace, min_rate }
    }

    /// Whether the body is behind its pace now.
    fn behind(&self) -> bool {
        let late = self.start.elapsed().saturating_sub(self.grace).as_secs_f64();
        late > 0.0 && (self.read as f64) < late * self.min_rate
    }
}

/// Whether `e`, from reading a body, says it stopped arriving or came too slowly (the socket's timeout, which is `WouldBlock`
/// on Unix, or [`Paced`]'s own).
fn stalled(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

impl<R: std::io::Read> std::io::Read for Paced<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let too_slow = || std::io::Error::new(std::io::ErrorKind::TimedOut, "the request's body came too slowly");
        if self.behind() {
            return Err(too_slow());
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if n > 0 && self.behind() {
            return Err(too_slow());
        }
        Ok(n)
    }
}

/// A refusal or failure as JSON: `{ "error": why }` with `status`. On a team server, the server's own paths are taken out of
/// `why` ([`crate::workspace::scrub`]): an error that names a file names it as the builder knows it.
fn refuse(status: u16, why: &str) -> Answer {
    json_response(status, &serde_json::json!({ "error": crate::workspace::scrub(why) }))
}

/// [`refuse`] with an error and its causes (`{e:#}`).
fn fail(status: u16, e: &anyhow::Error) -> Answer {
    refuse(status, &format!("{e:#}"))
}

/// One request as a route sees it, once the gate has let it through and the server knows who is asking.
struct Ask<'a> {
    req: &'a mut tiny_http::Request,
    method: Method,
    /// The URL's path, not decoded.
    path: &'a str,
    query: Query<'a>,
    /// Who is asking (on your own machine, the local user, an admin).
    me: crate::workspace::Identity,
    /// The workspace an admin is looking at (the `ls_as` cookie, GET requests only), if not their own.
    viewing: Option<String>,
}

/// The API's areas, asked in turn: each answers the requests that are its own and returns `None` for the rest. No two
/// match the same request; anything left is a file of the editor (GET) or refused with 405.
const ROUTES: [fn(&mut Ask<'_>) -> Option<Answer>; 13] = [
    team::route,
    app::route,
    packs::route,
    skies::route,
    areas::route,
    canvases::route,
    underlay::route,
    export::route,
    import::route,
    blueprints::route,
    scripts::route,
    dialogs::route,
    previews::route,
];

/// Answers one request (on one of the pool's threads on a team server), with the security headers every answer carries
/// ([`security_headers`]).
fn answer(req: &mut tiny_http::Request, web: &Path, gate: &Gate) -> Reply {
    // (The service routes go before the gate: it guards against browsers, which can't send the service key.)
    let made = service::route(req).unwrap_or_else(|| answer_request(req, web, gate).into());
    made.with_headers(security_headers(&page_policy(web)))
}

/// The headers every answer carries:
/// - `Content-Security-Policy`: the editor page's own policy (its `<meta http-equiv>`, see [`page_policy`]) plus
///   `frame-ancestors 'none'`, which only works as a header: no other site may show the editor in a frame (clickjacking).
///   An editor without a policy of its own gets only `frame-ancestors 'none'`, so nothing it needs is refused.
/// - `X-Content-Type-Options: nosniff`: a file is only ever what its `Content-Type` says (an uploaded or imported file is never
///   run as a script or a page).
/// - `X-Frame-Options: DENY`: `frame-ancestors` for browsers too old for it.
fn security_headers(policy: &Option<String>) -> Vec<Header> {
    let csp = match policy {
        Some(p) if !p.to_ascii_lowercase().contains("frame-ancestors") => {
            format!("{}; frame-ancestors 'none'", p.trim_end().trim_end_matches(';'))
        }
        Some(p) => p.clone(),
        None => "frame-ancestors 'none'".to_string(),
    };
    [("Content-Security-Policy", csp.as_str()), ("X-Content-Type-Options", "nosniff"), ("X-Frame-Options", "DENY")]
        .into_iter()
        .filter_map(|(k, v)| Header::from_bytes(k, v).ok())
        .collect()
}

/// The Content-Security-Policy in the editor's `index.html` (its `<meta http-equiv="Content-Security-Policy" content="...">`),
/// read again when the file changes. Taken from the page rather than written here, so the header can never fall out of step
/// with it: the policy names the page's import map by its hash, which changes with the import map.
fn page_policy(web: &Path) -> Option<String> {
    type Read = (std::path::PathBuf, Option<std::time::SystemTime>, Option<String>);
    static LAST: std::sync::Mutex<Option<Read>> = std::sync::Mutex::new(None);
    let path = web.join("index.html");
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((p, m, policy)) = last.as_ref()
        && *p == path
        && *m == modified
    {
        return policy.clone();
    }
    let policy = std::fs::read_to_string(&path).ok().and_then(|html| meta_policy(&html));
    *last = Some((path, modified, policy.clone()));
    policy
}

/// The `content` of the page's `<meta http-equiv="Content-Security-Policy">`, if it has one that can be a header (printable
/// ASCII only).
fn meta_policy(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<meta").map(|k| k + from) {
        let end = lower[at..].find('>').map(|k| k + at)?;
        let tag = &lower[at..end];
        from = end;
        let equiv =
            ["http-equiv=\"content-security-policy\"", "http-equiv='content-security-policy'", "http-equiv=content-security-policy"];
        if !equiv.iter().any(|e| tag.contains(e)) {
            continue;
        }
        // (The value keeps its own case: hashes are case-sensitive.)
        let original = &html[at..end];
        let start = tag.find("content=")? + "content=".len();
        let quote = original[start..].chars().next()?;
        let value = match quote {
            '"' | '\'' => original[start + 1..].split(quote).next()?,
            _ => original[start..].split_whitespace().next()?,
        };
        let value = value.trim();
        return (!value.is_empty() && value.bytes().all(|b| (0x20..0x7f).contains(&b))).then(|| value.to_string());
    }
    None
}
/// What the editor may send while an admin views another builder's workspace (the `ls_as` cookie): reads, going back to their
/// own workspace (`/api/view-as`), and checking paths (`/api/check`, which writes nothing). The editor's `api()` sends nothing
/// else then; anything else is refused with 403.
fn read_only_allows(method: &Method, path: &str) -> bool {
    matches!(method, Method::Get | Method::Head) || (*method == Method::Post && matches!(path, "/api/view-as" | "/api/check"))
}

/// [`answer`] without the security headers.
fn answer_request(req: &mut tiny_http::Request, web: &Path, gate: &Gate) -> Answer {
    let url = req.url().to_string();
    if crate::config::setting_os("LANDSCAPER_LOG").is_some() {
        let ua = req.headers().iter().find(|h| h.field.equiv("User-Agent")).map_or_default(|h| h.value.to_string());
        eprintln!("{} {} {} [{ua}]", req.remote_addr().map_or_default(ToString::to_string), req.method(), url);
    }
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    // The container's own health check (from inside it only): no sign-in, no log.
    // `X-Landscaper` tells a second double-click that this is the Landscaper and not another program on the port.
    if path == "/healthz" && req.remote_addr().is_some_and(|a| a.ip().is_loopback()) {
        return Response::from_string("ok").with_header(response_header("X-Landscaper", env!("CARGO_PKG_VERSION")));
    }
    let headers: Vec<(String, String)> =
        req.headers().iter().map(|h| (h.field.as_str().as_str().to_ascii_lowercase(), h.value.to_string())).collect();
    let header = |name: &str| headers.iter().find(|(k, _)| *k == name.to_ascii_lowercase()).map(|(_, v)| v.clone());
    // The client's address as the proxy says it, but only once the request has shown it came through the proxy: anyone who
    // reaches the server around it chooses their own `X-Forwarded-For` (and would choose a new one for every refused request,
    // to dodge the refusal log's limit). Otherwise the connection's own address.
    let auth = crate::config::AuthConfig::from_env();
    let from_proxy = crate::workspace::team() && crate::workspace::from_proxy(&auth, header);
    let peer = req.remote_addr().map_or_default(|a| a.ip().to_string());
    crate::audit::set_address(if from_proxy { crate::audit::client_address(header("X-Forwarded-For").as_deref(), &peer) } else { peer });
    // Another site's page, or a name that isn't this server's: refused before anything else.
    if let Some(why) = gate.refuse(req.method(), path, &header) {
        crate::audit::log_refused(None, 403, why);
        // JSON like every other refusal, so the editor shows the reason (an editor too old for this engine is refused here).
        return refuse(403, why);
    }
    // Who is asking (the sign-in proxy's headers, in team mode), and whose workspace this request works in: their own, or for an admin
    // reading, the one they chose to look at.
    let user_hint;
    let who = {
        // (Who a request that didn't come through the proxy says it is, is its sender's choice: not recorded.)
        user_hint = header(&auth.user).filter(|_| from_proxy);
        crate::workspace::identify(&auth, header).map(|me| {
            let cookie = header("Cookie").unwrap_or_default();
            let looking = cookie.split(';').find_map(|c| c.trim().strip_prefix("ls_as=")).and_then(crate::workspace::clean_user);
            (me, looking)
        })
    };
    let (me, viewing) = match who {
        Ok((me, looking)) => {
            let looking = looking
                .filter(|u| crate::workspace::team() && me.admin && *u != me.user && crate::workspace::workspaces_dir().join(u).is_dir());
            // The read-only view, held here as well as in the editor: while the page shows another workspace, nothing it sends
            // may change one (a POST would otherwise land in the admin's own workspace, under the name of the area on show).
            if let Some(u) = &looking
                && !read_only_allows(req.method(), path)
            {
                let why = format!("{u}'s workspace is read-only here: nothing was changed");
                crate::audit::log_refused(Some(&me.user), 403, &why);
                return refuse(403, &why);
            }
            let viewing = looking.filter(|_| *req.method() == Method::Get);
            (me, viewing)
        }
        Err((code, why)) => {
            crate::audit::log_refused(user_hint.as_deref(), code, why);
            return json_response(code, &serde_json::json!({ "error": why }));
        }
    };
    crate::workspace::set_user(if crate::workspace::team() { Some(viewing.clone().unwrap_or_else(|| me.user.clone())) } else { None });
    // Who is acting (an admin reading someone's workspace still acts as themselves: assignments are theirs, not the workspace's).
    crate::workspace::set_acting(&me.user, me.admin);
    let method = req.method().clone();
    let mut ask = Ask { req, method, path, query: Query::new(query), me, viewing };
    for route in ROUTES {
        if let Some(answer) = route(&mut ask) {
            return answer;
        }
    }
    files::route(&mut ask, web)
}

#[cfg(test)]
mod pool_tests {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// A server on a free port answering with `make` on `threads` threads (left running: the test process ends it).
    fn start(threads: usize, make: impl Fn(&mut tiny_http::Request) -> Answer + Send + Sync + 'static) -> u16 {
        start_with(tiny_http::Limits::default(), threads, make)
    }

    /// [`start`] with `limits` on its connections.
    fn start_with(
        limits: tiny_http::Limits,
        threads: usize,
        make: impl Fn(&mut tiny_http::Request) -> Answer + Send + Sync + 'static,
    ) -> u16 {
        let server = Arc::new(Server::http_with_limits("127.0.0.1:0", CONNECTIONS, limits).unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || answer_on_threads(&server, threads, |req| guarded(req, &make)));
        port
    }

    fn get(port: u16, path: &str) -> (u16, String) {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let status = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (status, out.split_once("\r\n\r\n").map_or_default(|(_, b)| b.to_string()))
    }

    #[test]
    fn a_panic_answers_500_and_the_same_thread_answers_the_next_request_as_nobody() {
        // One thread: the one that panicked must be the one that answers next.
        let port = start(1, |req| match req.url() {
            "/boom" => {
                crate::workspace::set_user(Some("mallory".into()));
                panic!("a bug in a route");
            }
            _ => Response::from_string(crate::workspace::user().unwrap_or_else(|| "nobody".into())),
        });
        let (status, body) = get(port, "/boom");
        assert_eq!(status, 500, "{body}");
        assert!(body.contains("bug"), "{body}");
        for _ in 0..2 {
            assert_eq!(get(port, "/who"), (200, "nobody".to_string()), "the panicked request's user doesn't carry over");
        }
    }

    /// What the server answers to `bytes` sent as they are (whatever of it it reads before closing the connection), within 10 s.
    fn raw(port: u16, bytes: &[u8]) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        // (The server may close the connection before it has all of it: that is the point.)
        let _ = s.write_all(bytes);
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn a_request_line_or_headers_over_the_limit_are_refused_as_they_are_read_and_never_reach_a_route() {
        let limits = tiny_http::Limits { max_head_bytes: 4096, ..LIMITS };
        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        let port = start_with(limits, 1, move |_| {
            s.fetch_add(1, Ordering::SeqCst);
            Response::from_string("ok")
        });
        // A request line of 8 MB, never ended: refused once 4 KB of it are in, not kept whole (it was, before the gate).
        let line = format!("GET /?x={} HTTP/1.1\r\nHost: x\r\n\r\n", "a".repeat(8 << 20));
        let out = raw(port, line.as_bytes());
        assert!(out.is_empty() || out.starts_with("HTTP/1.1 414"), "{out:.80}");
        let headers = format!("GET / HTTP/1.1\r\nHost: x\r\nX-Big: {}\r\nConnection: close\r\n\r\n", "b".repeat(8000));
        let out = raw(port, headers.as_bytes());
        assert!(out.is_empty() || out.starts_with("HTTP/1.1 431"), "{out:.80}");
        assert_eq!(seen.load(Ordering::SeqCst), 0, "neither reached a route");
        // A request within the limit is answered as before.
        assert_eq!(get(port, "/"), (200, "ok".to_string()));
    }

    #[test]
    fn a_head_or_a_body_that_stops_arriving_lets_go_of_its_thread() {
        let limits = tiny_http::Limits { io_timeout: Some(Duration::from_secs(1)), head_timeout: Some(Duration::from_secs(2)), ..LIMITS };
        // One thread: the stalled request must give it back for the next one to be answered.
        let port = start_with(limits, 1, |req| match read_body(req, 1 << 20) {
            Ok(b) => Response::from_string(format!("{} bytes", b.len())),
            Err(refused) => refused,
        });
        let stalled = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            s.write_all(b"POST /save HTTP/1.1\r\nHost: x\r\nContent-Length: 5000\r\n\r\nab").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        });
        std::thread::sleep(Duration::from_millis(200));
        let t = std::time::Instant::now();
        assert_eq!(get(port, "/"), (200, "0 bytes".to_string()), "answered once the stalled body let go");
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        let out = stalled.join().unwrap();
        assert!(out.starts_with("HTTP/1.1 408"), "{out}");
        // A head sent a byte at a time, each within the read timeout: answered 408 once it has taken 2 s.
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let t = std::time::Instant::now();
        let mut out = Vec::new();
        // The answer is read as soon as it comes: on Windows a byte sent after the server closed resets the connection, and
        // the reset throws away an answer not read yet.
        s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        for b in b"GET / HTTP/1.1\r\nHost: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\n\r\n" {
            let mut got = [0; 256];
            if let Ok(n @ 1..) = s.read(&mut got) {
                out.extend_from_slice(&got[..n]);
                break;
            }
            if s.write_all(&[*b]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            if t.elapsed() > Duration::from_secs(4) {
                break;
            }
        }
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let _ = s.read_to_end(&mut out);
        let out = String::from_utf8_lossy(&out);
        assert!(out.starts_with("HTTP/1.1 408"), "{out}");
    }

    #[test]
    fn a_body_refused_unread_is_never_read_on_the_answer_thread_nor_taken_for_a_request() {
        // One thread, and the full 30 s timeouts: refusing must not wait for a body that never comes.
        let port = start_with(LIMITS, 1, |req| match req.body_length() {
            Some(n) if n > 1000 => refuse(413, "too big"),
            _ => Response::from_string("ok"),
        });
        let t = std::time::Instant::now();
        // 100 TB declared: tiny_http allocated what was left of it to throw it away, and the process aborted. Then 5 MB
        // declared and 2 bytes sent: thrown away by reading it, on the answer thread, for 30 s.
        for length in ["100000000000000", "5000000"] {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            s.write_all(format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {length}\r\n\r\nab").as_bytes()).unwrap();
            let mut answer = [0u8; 12];
            s.read_exact(&mut answer).unwrap();
            assert_eq!(&answer, b"HTTP/1.1 413", "{length}");
            assert_eq!(get(port, "/"), (200, "ok".to_string()), "the server is up, and its thread free");
            // A request inside the body is part of the body, never answered.
            let _ = s.write_all(b"GET /sneaky HTTP/1.1\r\nHost: x\r\n\r\n");
            let _ = s.shutdown(std::net::Shutdown::Write);
            let mut rest = String::new();
            let _ = s.read_to_string(&mut rest);
            assert!(!rest.contains("200 OK"), "{rest}");
        }
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        // A body sent whole and refused unread is thrown away, and the connection goes on: the 413 isn't lost to a reset.
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let body = "x".repeat(200_000);
        let two = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            body.len()
        );
        s.write_all(two.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.starts_with("HTTP/1.1 413") && out.contains("HTTP/1.1 200") && out.ends_with("ok"), "{out}");
    }

    #[test]
    fn a_body_framed_two_ways_is_refused_and_nothing_in_it_is_taken_for_a_request() {
        // Request smuggling (CVE-2026-66752, fixed in tiny_http_dh): a proxy in front that frames a body by its Content-Length,
        // or ignores a Transfer-Encoding it doesn't know, passes the whole of it on as one request. tiny_http read any
        // Transfer-Encoding as chunked and preferred it to a Content-Length, so the body below ended at its `0` chunk and the
        // request after it was answered as a request of its own, which the proxy never saw (and never signed in).
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let port = start_with(LIMITS, 1, move |req| {
            s.lock().unwrap().push(req.url().to_string());
            match read_body(req, 1 << 20) {
                Ok(b) => Response::from_string(format!("{} bytes", b.len())),
                Err(refused) => refused,
            }
        });
        let smuggled = "0\r\n\r\nGET /sneaky HTTP/1.1\r\nHost: x\r\n\r\n";
        for framing in [
            format!("Content-Length: {}\r\nTransfer-Encoding: chunked", smuggled.len()),
            format!("Transfer-Encoding: chunked\r\nContent-Length: {}", smuggled.len()),
            "Transfer-Encoding: identity".to_string(),
            "Transfer-Encoding: chunked, identity".to_string(),
            "Transfer-Encoding: xchunked".to_string(),
        ] {
            let out = raw(port, format!("POST /save HTTP/1.1\r\nHost: x\r\n{framing}\r\n\r\n{smuggled}").as_bytes());
            assert!(out.starts_with("HTTP/1.1 400"), "{framing}: {out}");
            assert_eq!(out.matches("HTTP/1.1").count(), 1, "{framing}: one answer, and the connection closed: {out}");
        }
        assert!(seen.lock().unwrap().is_empty(), "no route saw them: {:?}", seen.lock().unwrap());
        // A body framed one way, chunked or by its length, is read as before.
        let out =
            raw(port, b"POST /save HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n");
        assert!(out.starts_with("HTTP/1.1 200") && out.ends_with("3 bytes"), "{out}");
        let out = raw(port, b"POST /save HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc");
        assert!(out.starts_with("HTTP/1.1 200") && out.ends_with("3 bytes"), "{out}");
    }

    #[test]
    fn one_person_sends_at_most_eight_bodies_at_once() {
        let mine: Vec<Reading> = (0..Reading::MOST).map(|_| Reading::begin("test-mallory").unwrap()).collect();
        assert!(Reading::begin("test-mallory").is_none(), "a ninth is refused");
        let theirs = Reading::begin("test-bob");
        assert!(theirs.is_some(), "someone else still sends");
        drop(mine);
        assert!(Reading::begin("test-mallory").is_some(), "free again once read");
    }

    #[test]
    fn a_body_that_falls_behind_its_pace_fails_and_one_on_pace_reads_whole() {
        let mut fine = Paced::new(&b"hello"[..]);
        let mut got = String::new();
        Read::read_to_string(&mut fine, &mut got).unwrap();
        assert_eq!(got, "hello");
        // No grace and a pace no client keeps: behind at once, and said so as a stalled body (408).
        let mut slow = Paced::with(&b"hello"[..], Duration::ZERO, 1e12);
        std::thread::sleep(Duration::from_millis(5));
        let e = Read::read_to_end(&mut slow, &mut Vec::new()).unwrap_err();
        assert!(stalled(&e), "{e}");
        let answer = read_body_from(Some(5), &mut Paced::with(&b"hello"[..], Duration::ZERO, 1e12), 100).unwrap_err();
        assert_eq!(answer.status_code(), 408);
    }

    #[test]
    fn a_pool_answers_no_more_requests_at_once_than_it_has_threads_and_queues_the_rest() {
        let (now, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (n, m) = (now.clone(), most.clone());
        let port = start(2, move |_| {
            m.fetch_max(n.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(250));
            n.fetch_sub(1, Ordering::SeqCst);
            Response::from_string("ok")
        });
        let clients: Vec<_> = (0..6).map(|_| std::thread::spawn(move || get(port, "/"))).collect();
        for c in clients {
            assert_eq!(c.join().unwrap(), (200, "ok".to_string()), "every request is answered in the end");
        }
        assert_eq!(most.load(Ordering::SeqCst), 2, "at most two at once, and two did run at once");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::areas::imports_in;
    use crate::serve::export::export_request;
    use crate::serve::previews::{preview_tex_ok, preview_tex_type};

    /// What `gate` says to `method path` with `headers`.
    fn ask(gate: &Gate, method: &Method, path: &str, headers: &[(&str, &str)]) -> Option<&'static str> {
        gate.refuse(method, path, &|name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.to_string()))
    }

    const EDITOR: [(&str, &str); 3] = [("Host", "127.0.0.1:8765"), ("X-Landscaper", "1"), ("Origin", "http://127.0.0.1:8765")];

    #[test]
    fn the_pages_policy_is_read_from_its_meta_tag_and_frames_are_refused() {
        let page = concat!(
            r#"<!doctype html><meta charset="utf-8"><META HTTP-EQUIV="Content-Security-Policy" "#,
            r#"content="default-src 'self'; script-src 'self' 'sha256-TCQjodja3ktH01cTB7LwVQgTmMjeLTzCsuSoGtZ5I3U='"><title>t</title>"#
        );
        let policy = meta_policy(page);
        assert_eq!(policy.as_deref(), Some("default-src 'self'; script-src 'self' 'sha256-TCQjodja3ktH01cTB7LwVQgTmMjeLTzCsuSoGtZ5I3U='"));
        let csp = |p: &Option<String>| {
            security_headers(p).into_iter().find(|h| h.field.equiv("Content-Security-Policy")).map(|h| h.value.to_string())
        };
        assert_eq!(
            csp(&policy).as_deref(),
            Some("default-src 'self'; script-src 'self' 'sha256-TCQjodja3ktH01cTB7LwVQgTmMjeLTzCsuSoGtZ5I3U='; frame-ancestors 'none'")
        );
        // An editor without a policy of its own: only frames are refused, nothing it needs.
        assert_eq!(meta_policy("<!doctype html><meta name=viewport content=x><title>t</title>"), None);
        assert_eq!(csp(&None).as_deref(), Some("frame-ancestors 'none'"));
        // Other meta tags, a policy that already says frame-ancestors, and one that can't be a header.
        assert_eq!(
            meta_policy("<meta content=\"a\" name=\"x\"><meta http-equiv='content-security-policy' content='img-src *'>").as_deref(),
            Some("img-src *")
        );
        assert_eq!(
            csp(&Some("default-src 'self'; frame-ancestors 'self';".into())).as_deref(),
            Some("default-src 'self'; frame-ancestors 'self';")
        );
        assert_eq!(meta_policy("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src\u{7}x\">"), None);
        let names: Vec<String> = security_headers(&None).iter().map(|h| h.field.to_string()).collect();
        assert_eq!(names, ["Content-Security-Policy", "X-Content-Type-Options", "X-Frame-Options"]);
    }

    #[test]
    fn the_editors_own_requests_pass() {
        let g = Gate::new(8765, false, None);
        for (m, p) in [(Method::Post, "/api/export"), (Method::Get, "/api/me"), (Method::Put, "/api/save")] {
            assert_eq!(ask(&g, &m, p, &EDITOR), None, "{m} {p}");
        }
        // localhost and [::1] too, and a fetch the browser marks same-origin; a GET carries no Origin.
        for host in ["localhost:8765", "[::1]:8765", "LOCALHOST:8765"] {
            let h = [("Host", host), ("X-Landscaper", "1"), ("Sec-Fetch-Site", "same-origin")];
            assert_eq!(ask(&g, &Method::Get, "/api/areas", &h), None, "{host}");
        }
        // The editor's page, scripts and pictures (no header: the browser loads them).
        for p in ["/", "/js/app.js", "/textures/catalog.json", "/api/texture/tt01_grass", "/api/import/bay1", "/api/preview-icon/a.png"] {
            assert_eq!(ask(&g, &Method::Get, p, &[("Host", "127.0.0.1:8765")]), None, "{p}");
        }
        // A link from another site to the editor's page still opens it.
        assert_eq!(ask(&g, &Method::Get, "/", &[("Host", "127.0.0.1:8765"), ("Sec-Fetch-Site", "cross-site")]), None);
    }

    #[test]
    fn an_api_request_without_the_header_is_refused() {
        let g = Gate::new(8765, false, None);
        let h = [("Host", "127.0.0.1:8765")];
        for (m, p) in
            [(Method::Post, "/api/export"), (Method::Get, "/api/me"), (Method::Get, "/api/script"), (Method::Post, "/api/texture/x")]
        {
            assert!(ask(&g, &m, p, &h).is_some_and(|w| w.contains("X-Landscaper")), "{m} {p}");
        }
        assert!(ask(&g, &Method::Post, "/api/save", &[("Host", "127.0.0.1:8765"), ("X-Landscaper", "0")]).is_some());
        // scene.json is fetched (with the header); only its pictures are loaded as images.
        assert!(ask(&g, &Method::Get, &format!("/api/preview-file/{}/scene.json", "a".repeat(64)), &h).is_some());
        assert!(ask(&g, &Method::Get, &format!("/api/preview-file/{}/tex/a.png", "a".repeat(64)), &h).is_none());
        assert!(ask(&g, &Method::Get, &format!("/api/preview-file/{}/tex/a.webp", "a".repeat(64)), &h).is_none());
    }

    #[test]
    fn a_preview_texture_is_webp_or_png_and_lower_case() {
        // What the review service's baker names them now, and an older bake's.
        for t in ["pmh0_footr001_2_7_0_0_0_0_0_0_23_52.webp", "particle_fxpa_x.webp", "pmh0_head001.png", "a-b.c.webp"] {
            assert!(preview_tex_ok(t), "{t}");
        }
        // Served as the type its name says, whatever the review service sent.
        assert_eq!((preview_tex_type("pmh0_head001.png"), preview_tex_type("particle_fxpa_x.webp")), ("image/png", "image/webp"));
        for t in [
            "a.WEBP",
            "A.webp",
            "a.jpg",
            "a.webp.txt",
            "a.web",
            "awebp",
            "../a.webp",
            "a/b.webp",
            "a%2fb.webp",
            "a b.webp",
            "",
            &format!("{}.webp", "a".repeat(120)),
        ] {
            assert!(!preview_tex_ok(t), "{t}");
        }
        assert!(preview_tex_ok(&format!("{}.webp", "a".repeat(119))));
    }

    #[test]
    fn only_a_get_of_a_picture_goes_without_the_header() {
        let g = Gate::new(8765, false, None);
        let h = [("Host", "127.0.0.1:8765")];
        let key = "a".repeat(64);
        let pictures = [
            "/api/texture/tt01_grass".to_string(),
            "/api/import/bay1".to_string(),
            "/api/import-file/bay1/decal_x.png".to_string(),
            format!("/api/preview-file/{key}/tex/a.png"),
            format!("/api/preview-file/{key}/tex/a.webp"),
            "/api/preview-icon/k.png".to_string(),
        ];
        for p in &pictures {
            assert_eq!(ask(&g, &Method::Get, p, &h), None, "GET {p}");
            for m in [Method::Post, Method::Head, Method::Put, Method::Delete] {
                assert!(ask(&g, &m, p, &h).is_some_and(|w| w.contains("X-Landscaper")), "{m} {p}");
            }
            // Still for this server only, and not for another site.
            assert!(ask(&g, &Method::Get, p, &[("Host", "evil.example:8765")]).is_some(), "foreign Host {p}");
            assert!(ask(&g, &Method::Get, p, &[("Host", "127.0.0.1:8765"), ("Origin", "https://evil.example")]).is_some(), "Origin {p}");
            assert!(ask(&g, &Method::Get, p, &[("Host", "127.0.0.1:8765"), ("Sec-Fetch-Site", "cross-site")]).is_some(), "cross-site {p}");
            // A three.js loader asks in CORS mode: its Origin, when sent, is the server's own.
            assert_eq!(
                ask(&g, &Method::Get, p, &[("Host", "127.0.0.1:8765"), ("Origin", "http://127.0.0.1:8765")]),
                None,
                "own Origin {p}"
            );
        }
        // Any other API route needs it, a GET too, and a prefix alone isn't enough.
        for p in [
            "/api/me",
            "/api/areas",
            "/api/textures",
            "/api/importx",
            "/api/preview-file/x/scene.json",
            "/api/preview-file/x/scene.json/tex/a",
            "/api/import-sources",
        ] {
            assert!(ask(&g, &Method::Get, p, &h).is_some_and(|w| w.contains("X-Landscaper")), "GET {p}");
        }
    }

    #[test]
    fn another_sites_request_is_refused() {
        let g = Gate::new(8765, false, None);
        for origin in ["https://evil.example", "http://127.0.0.1:9999", "null", "http://localhost:8765.evil.example", ""] {
            let h = [("Host", "127.0.0.1:8765"), ("X-Landscaper", "1"), ("Origin", origin)];
            assert!(ask(&g, &Method::Post, "/api/export", &h).is_some_and(|w| w.contains("Origin")), "{origin:?}");
        }
        let h = [("Host", "127.0.0.1:8765"), ("X-Landscaper", "1"), ("Sec-Fetch-Site", "cross-site")];
        assert!(ask(&g, &Method::Post, "/api/save", &h).is_some_and(|w| w.contains("another site")));
        assert!(ask(&g, &Method::Get, "/api/area", &h).is_some_and(|w| w.contains("another site")));
        assert!(ask(&g, &Method::Get, "/api/texture/x", &h).is_some(), "not even a picture, for another site");
        // same-site (another name under the same domain) and none at all (curl) are not another site.
        let h = [("Host", "127.0.0.1:8765"), ("X-Landscaper", "1"), ("Sec-Fetch-Site", "same-site")];
        assert_eq!(ask(&g, &Method::Post, "/api/save", &h), None);
    }

    #[test]
    fn a_foreign_host_is_refused_on_this_machine() {
        let g = Gate::new(8765, false, None);
        for host in ["evil.example:8765", "127.0.0.1:8766", "127.0.0.1", "localhost", "", "127.0.0.1.evil.example:8765"] {
            let h = [("Host", host), ("X-Landscaper", "1")];
            assert!(ask(&g, &Method::Get, "/api/me", &h).is_some_and(|w| w.contains("Host")), "{host:?}");
            assert!(ask(&g, &Method::Get, "/", &[("Host", host)]).is_some(), "the page too: {host:?}");
        }
        assert_eq!(ask(&g, &Method::Get, "/api/me", &[]), Some(ask(&g, &Method::Get, "/api/me", &[("Host", "x")]).unwrap()));
        // On port 80 the browser leaves the port out.
        assert_eq!(ask(&Gate::new(80, false, None), &Method::Get, "/", &[("Host", "localhost")]), None);
        // A public name set on this machine (reached from the network on purpose) is allowed as well.
        let g = Gate::new(8765, false, Some("builder-pc.lan"));
        assert_eq!(ask(&g, &Method::Get, "/", &[("Host", "builder-pc.lan:8765")]), None);
    }

    #[test]
    fn a_team_server_answers_its_public_name_and_without_one_any() {
        let g = Gate::new(8766, true, Some("https://Landscaper.Example.com/, alias.example.com:8443"));
        assert_eq!(g.public, ["landscaper.example.com", "alias.example.com:8443"]);
        let good = [("Host", "landscaper.example.com"), ("X-Landscaper", "1"), ("Origin", "https://landscaper.example.com")];
        assert_eq!(ask(&g, &Method::Post, "/api/save", &good), None);
        assert_eq!(ask(&g, &Method::Get, "/", &[("Host", "alias.example.com:8443")]), None);
        assert!(ask(&g, &Method::Get, "/", &[("Host", "alias.example.com")]).is_some(), "a name with a port matches that port only");
        assert!(ask(&g, &Method::Get, "/", &[("Host", "evil.example")]).is_some());
        // Inside the container, by its own address.
        assert_eq!(ask(&g, &Method::Get, "/", &[("Host", "127.0.0.1:8766")]), None);
        // A proxy that rewrites Host to the container's name: the Origin is checked against X-Forwarded-Host (or the setting).
        let proxied = [("Host", "landscaper:8766"), ("X-Landscaper", "1"), ("Origin", "https://landscaper.example.com")];
        let g2 = Gate::new(8766, true, None);
        assert!(ask(&g2, &Method::Post, "/api/save", &proxied).is_some());
        let mut with_forwarded = proxied.to_vec();
        with_forwarded.push(("X-Forwarded-Host", "landscaper.example.com"));
        assert_eq!(ask(&g2, &Method::Post, "/api/save", &with_forwarded), None);
        // Without the setting: any name (the warning at start), but still no other site and no request without the header.
        assert_eq!(ask(&g2, &Method::Get, "/", &[("Host", "whatever.example")]), None);
        let evil = [("Host", "landscaper.example.com"), ("X-Landscaper", "1"), ("Origin", "https://evil.example")];
        assert!(ask(&g2, &Method::Post, "/api/save", &evil).is_some());
        assert!(ask(&g2, &Method::Post, "/api/save", &[("Host", "landscaper.example.com")]).is_some());
    }

    #[test]
    fn export_refuses_a_bad_name_or_size_before_writing() {
        let body = vec![0u8; 49 * 49 * 4];
        for (q, why) in [
            ("name=..%2F..%2Fprobe&tiles=3", "bad area name"),
            ("name=%2Fabs%2Fprobe&tiles=3", "bad area name"),
            ("name=a%5Cb&tiles=3", "bad area name"),
            ("name=a%00b&tiles=3", "bad area name"),
            ("name=abcdefghij&tiles=3", "bad area name"),
            ("name=ok&tiles=2", "3 to 32 tiles"),
            ("name=ok&tiles=3&tilesY=33", "3 to 32 tiles"),
            ("name=ok&tiles=100000000000", "3 to 32 tiles"),
            ("name=ok&tiles=3&halo=10", "whole tiles of samples"),
            ("name=ok&tiles=3&halo=96", "at most 5 tiles"),
            ("name=ok&tiles=3&halo=16", "a halo of 16"),
        ] {
            let e = format!("{:#}", export_request(q, &body, true).unwrap_err());
            assert!(e.contains(why), "{q}: {e}");
        }
    }

    #[test]
    fn export_refuses_a_bad_blend_layer_before_writing() {
        use base64::Engine;
        let n = 49 * 49;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let mut past = vec![0u8; n * 2];
        past[2 * 9..2 * 9 + 2].copy_from_slice(&[2, 40]);
        for (blend, why) in
            [(b64(&[0u8; 10]), "two a sample"), ("!!".to_string(), "not base64"), (b64(&past), "past the palette's 2 textures")]
        {
            let mut body = vec![0u8; n * 4];
            body.extend(vec![0u8; n]);
            body.extend(serde_json::to_vec(&serde_json::json!({ "palette": ["a", "b"], "blend": blend })).unwrap());
            let e = format!("{:#}", export_request("name=blendbad&tiles=3", &body, true).unwrap_err());
            assert!(e.contains(why), "{why}: {e}");
        }
    }

    #[test]
    fn imports_are_found_in_paint_and_objects() {
        let objects = serde_json::json!({ "objects": [
            { "model": "@import:bay2:0302_0" }, { "model": "plc_boulder" }, { "model": "@import:other:0001" },
        ] });
        let palette = vec!["grassMedium".to_string(), "@import:bay1".to_string()];
        assert_eq!(imports_in(&palette, &objects), vec!["bay1", "bay2", "other"]);
        assert!(imports_in(&[], &serde_json::Value::Null).is_empty());
    }
}

#[cfg(test)]
mod wasi_request_tests {
    use super::*;

    #[test]
    fn a_request_written_out_is_answered_as_the_server_answers_it() {
        let raw = b"GET /api/me HTTP/1.1\r\nHost: localhost\r\nX-Landscaper: 1\r\n\r\n";
        let out = String::from_utf8(answer_raw(raw, Path::new("/nowhere")).unwrap()).unwrap();
        assert!(out.starts_with("HTTP/1.0 200"), "{out}");
        let (_, body) = out.split_once("\r\n\r\n").unwrap();
        let me: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(me["version"].as_str().is_some_and(|v| v.starts_with(env!("CARGO_PKG_VERSION"))), "{body}");
        // Through the gate as the server's: an API request without the editor's header is refused.
        let out =
            String::from_utf8(answer_raw(b"GET /api/me HTTP/1.1\r\nHost: localhost\r\n\r\n", Path::new("/nowhere")).unwrap()).unwrap();
        assert!(out.starts_with("HTTP/1.0 403"), "{out}");
    }

    #[test]
    fn a_request_with_a_binary_body_reaches_its_route_and_the_answer_goes_to_a_file() {
        let dir = std::env::temp_dir().join(format!("ls-wasi-request-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A check of an area too small to be one: the route's refusal, not one of the request's framing.
        let mut raw =
            b"POST /api/check?name=x&tiles=1&tilesY=1 HTTP/1.1\r\nHost: localhost\r\nX-Landscaper: 1\r\nContent-Length: 4\r\n\r\n".to_vec();
        raw.extend([0xff, 0x00, 0x80, 0x7f]);
        std::fs::write(dir.join("req"), &raw).unwrap();
        wasi_request(Some(&dir.join("req")), Some(&dir.join("answer"))).unwrap();
        let out = String::from_utf8_lossy(&std::fs::read(dir.join("answer")).unwrap()).into_owned();
        assert!(out.starts_with("HTTP/1.0 4"), "{out}");
        assert!(out.contains("3 to 32 tiles"), "the route's own refusal: {out}");
        assert!(answer_raw(b"GET / HTTP/1.1\r\n", Path::new("/nowhere")).is_err(), "a head that never ends");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
