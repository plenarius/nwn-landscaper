# tiny_http, patched for the Landscaper

[tiny_http_dh](https://github.com/dheijl/tiny-http-dh) 0.12.4 as published on crates.io (its `src/`, licences, README, changelog
and manifest), with one patch of ours. tiny_http_dh is the maintained fork of [tiny_http](https://github.com/tiny-http/tiny-http)
(last release 0.12.0, 2022). `Cargo.toml` at the repository's root uses it under tiny_http's name:
`tiny_http = { package = "tiny_http_dh", path = "vendor/tiny_http" }`.

## Why the fork

Over tiny_http 0.12.0 the fork has, among others (its `CHANGELOG.md`):

- the fix for request smuggling, CVE-2026-66752: a `Transfer-Encoding` other than `chunked` alone, or one with a
  `Content-Length`, is answered 400 and the connection closed. tiny_http read any `Transfer-Encoding` as chunked and preferred it
  to a `Content-Length`, so behind a proxy that frames the body the other way (Caddy in front of a team server) a request could
  be hidden in another's body and answered without the proxy seeing it;
- a bounded thread pool for connections (`PoolConfig`; the Landscaper sets its own, `CONNECTIONS` in `src/serve.rs`);
- a body that wasn't read thrown away through a fixed buffer. tiny_http's buffer was the size of what was left of the body,
  which the request's `Content-Length` chose: `Content-Length: 100000000000000` made the process abort ("memory allocation of
  100000000000000 bytes failed"). Our patch had the same fix; it is dropped, the fork's is kept.

## Why the patch

Neither tiny_http nor the fork sets a timeout on a connection (the fork has one for a TLS handshake only, which the Landscaper
doesn't use), and both read a request's head whole, however long it is. On a team server that let one client:

- hold an answer thread for as long as its connection stayed open, by sending a request's body slowly or not at all (32 such
  requests stopped the server answering anyone);
- fill the memory with one request line: a 400 MB line took the server from 287 MB to 1,091 MB before it refused the request;
- hold an answer thread for 30 s with a request refused without reading its body (413, 429, the gate's 403): throwing the body
  away read it, on that thread, until the connection's timeout.

## The patch

Every change is marked `Landscaper patch` in a comment. `git log -p vendor/tiny_http` shows it: the commit before it is the
crate unchanged.

- `Limits` (`src/lib.rs`): `io_timeout`, the longest a read or a write on a connection may wait; `head_timeout`, the longest
  reading a request's head may take from its first byte; `max_head_bytes`, the most bytes the head may hold. The default is no
  limits, as upstream.
- `Server::http_with_limits` (`http_with_pool` with them) and `Server::from_listener_with_limits` (`from_listener_with_options`
  with them) take them. Each connection gets `io_timeout` as its read and write timeout, or is dropped if it can't be set (with
  TLS, once its handshake is done, in place of the fork's none).
- `ClientConnection` (`src/client.rs`) counts the head's bytes and time as it reads them: a request line over the limit is answered
  414, longer headers 431, a head that stalls or takes too long 408, and the connection is closed without reading the rest. A
  connection that is idle between requests for `io_timeout` is closed without an answer, as an idle keep-alive connection.
- What is left of a body that wasn't read to its end is thrown away on the connection's own thread (`ClientConnection`, once
  the request is answered: `SequentialReader::wait_turn`), not on the thread that answered it, through an 8 KB buffer. Over
  256 MB of it, or a chunked body, whose length isn't known, closes the connection instead: what was left of a chunked body
  was read as the next request before. `EqualReader` (`src/util/equal_reader.rs`) and a chunked body's reader (`src/request.rs`)
  say how much was left; `TestRequest` still throws it away itself, through the fork's fixed buffer.

- `rustfmt.toml`: rustfmt's defaults, which the fork is formatted with, in place of the repository's (`cargo fmt --all` in the
  repository formats this folder too). The patch is formatted the fork's way.

The patch on tiny_http 0.12.0 also allowed two of its warnings in `Cargo.toml`; the fork has neither, so that part is gone.

## Updating

To move to a new tiny_http_dh release: replace the folder's files (not this one) with the new release's, as published on
crates.io (`Cargo.toml.orig` as `Cargo.toml`), in one commit, then apply the patch again in the next: `git diff` of the last
such pair of commits, applied with `patch -p3`, and by hand where the fork changed the same lines. Read the fork's changelog
for anything that changes what the Landscaper relies on (its connection pool, its timeouts). Then run the Landscaper's tests:
`src/serve.rs` tests the limits, the bodies left unread and request smuggling on a real socket.

## Licence

MIT or Apache-2.0, at your option, as upstream: `LICENSE-MIT` and `LICENSE-APACHE`. The patch is under the same terms.
