# Changelog

All notable changes to the Landscaper engine; the editor's are in
[nwn-landscaper-web's CHANGELOG.md](https://github.com/plenarius/nwn-landscaper-web/blob/main/CHANGELOG.md). Versions follow
[Semantic Versioning](https://semver.org/); before 1.0, a minor version may change the commands, the HTTP API between the engine
and the editor, or the files it writes. [nwn-bake](https://github.com/plenarius/nwn-bake) has its own changelog.

## 0.2.3

- **No change to the engine.** Released with the editor's 0.2.3, whose Place list pictures are the right way up in every
  browser; the self-host image builds that editor (`WEB_REF`).

## 0.2.2

- **Export works on Windows.** Every export there stopped with "Access is denied (os error 5)" on the module's `.work` copy:
  the copy was flushed to disk through a read-only handle, which Windows refuses. The tests now run on Windows too.
- **A double-click passes over another program on port 8765.** It used to open whatever was listening there (another local
  tool's page) and say it was the Landscaper. Now it opens a Landscaper already running only when `/healthz` says it is one
  (an `X-Landscaper` header), and otherwise starts on the next free port up to 8784, and says which.
- **Export is faster, and an export again much faster.** Each tile's files (its baked texture, shadow map, model, walkmesh and
  minimap) are made on every core at once, the in-tile crossings check spreads single crossings instead of whole tiles, and an
  import's pictures load side by side. `landscaper serve` keeps the last exports' baked textures, shadow maps and simulated routes by
  what they are made from: exported again, only the tiles whose paint or ground changed are baked again, and the route checks
  run again only when the walkmeshes changed. What an export writes is the same, byte for byte.
- **Export's check for winding tiles is faster.** Each way across a tile stops being searched once it is over the limit
  (200,000 units of work), so the step takes about a second where one bad tile made it 8 (a 13x12 area). The warning says
  "over 200,000" instead of the exact cost (the report's `units` is now `over`, the limit), and its marker is on one of the
  tile's ways over the limit, not always the worst. What an export writes is the same, byte for byte.

- **A trusted service can list and fetch a builder's exports.** With `LANDSCAPER_SERVICE_KEY` set, a team server answers
  read-only `/api/service/*` routes: a builder's exports, one export's area files and new blueprints (as Publish packs them), its
  tile hak and the texture library, streamed with an `ETag`. Each is checked as Publish checks it (the name, the assignment) for
  the builder named, and recorded in the activity log. docs/REVIEW-SERVICE.md, "Fetching exports".
- **Publish sends the export's id** (`export`, and in each area of `publish-module`): a hash of the export's header and its hak,
  the same id the service routes list, so a service can tell whether the tiles it fetched are the ones published.
- **Publish's check is much faster.** It sends the review service the tile and library files' names, git blob ids and sizes
  instead of the files (a review service that wants the files gets them, as before), and a hak's files are read and packed once
  per export: the check, the pull requests and Try on dev reuse them, and a check that passes packs the zips in the background
  for what usually follows. docs/REVIEW-SERVICE.md, "Publishing".
- **The minimap shows what is built on the area.** An export draws its area straight down with nwn-bake, from the hak and module
  it has written (the tiles, the placeables and doors, their own textures), and cuts the picture into each tile's minimap:
  houses, walls, towers and platforms are on it now, not only the ground and water. Where nothing is drawn (a cave's rock) the
  terrain's minimap shows, as it does if the drawing fails. `LS_MINIMAP=terrain` keeps the terrain's alone.
  It is lit by the game's sun, from the north-east as the terrain's baked shadows are, so shadows fall south-west; they are
  drawn half as long, so a wall's or a house's shadow marks it and a 10 m tower's (4.3 m) stays off the next tile. Needs
  nwn-bake 0.7.0 (`--sun`, `--shadow-length`).

## 0.2.1

- **The dev server's full test is gone** from the team server: `/api/dev-test` (status, start and release) and the review
  service's `/api/v1/dev/test` calls it made. Try on dev replaces it; new scripts can't be tried on dev and go through Publish.
  `landscaper dev-pack` stays, as a command of its own. An existing area's waypoints, for a transition to lead to, are now
  `/api/waypoints` (was `/api/dev-test/waypoints`), still from the service's `/api/v1/areas/<resref>/waypoints`.

## 0.2.0

- **Try on dev.** A team server whose review service runs a dev game server can put an area, or its whole module, on dev with
  no restart (`/api/dev-trial`): it sends what Publish sends, checked the same way, always with the shared texture library. The
  builder walks in with `;forge <area>`, keeps the trial longer or releases it. docs/REVIEW-SERVICE.md, "Trying areas on a dev
  server".

## 0.1.2

- **Publishing sends only the files the Landscaper made.** An import copies the textures its cliffs and objects use into
  `<name>.import/tex/` (sometimes an EE material's texture under the plain name), and an export puts them in the area's hak so
  it looks right on its own. Those are the game's and the module's haks' own textures, so a publish and a package now leave
  them out: shipped in a hak under the same name, they would replace the originals in every area.
- `url` 2.5.9, for RUSTSEC-2026-0592.

## 0.1.1

- **Downloads to unzip and start.** Each release has an archive for Windows (x86-64), macOS (one program for Apple silicon and
  Intel) and Linux (x86-64), holding the program, the editor of the same release in `web/`, the licences and a QUICKSTART. No
  Rust, compiler or second repository needed. They are made by `.github/workflows/release.yml` on a version tag, as a draft
  release, with `SHA256SUMS` and the complete source. The builds are not signed (README, "Install").
- **Started with no arguments** (a double-click), the program runs `serve` on 127.0.0.1:8765, opens the editor in the default
  browser and keeps its window open, showing the game and user folder it found and how to stop it. Without a game install it says
  what to set (`NWN_ROOT`, and how on that system) and waits for Enter instead of closing. If the editor is already running, it
  opens the browser at it. On Windows it also finds the user folder in OneDrive's `Documents`. Every command, and `serve` with
  any option, works as before.
- **`serve` says what to set when it finds no game or user folder**, and why a `NWN_ROOT` that is set isn't the game's folder.
- **Windows: the data folder is `%LOCALAPPDATA%\landscaper`** (`XDG_DATA_HOME` still wins). Without a `HOME` it was a
  `.local/share/landscaper` folder under wherever the program was started.
- **The engine builds on Windows.** `--reload-on-rebuild` and the team server's disk quota used Unix-only calls; on Windows the
  first starts the new build in the same window, and the second counts a file with two names twice.

## 0.1.0

The first public release: the editor's engine (`serve`, on your own machine or as a team server), area import and export, the
route check, the texture library with a starter set read from the game's files, and the hidden mark on a team server's exports.
The editor, [nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web), is released with it under the same version tag
(README, "Releases"), and records its own changes.

It uses nwn-bake 0.6.1 for reading the game's files and drawing pictures.

### Since the private builds

**If you ran a private build, check these first:**

- **The module layout's default is now `nasher`** (nasher's own layout: everything in `src/`). A module laid out a folder per
  kind (`areas/<group>/`, `core/<ext>/`, `core/nss/`, `module.ifo.json` at the root), which was the default, now needs
  `LANDSCAPER_LAYOUT=folders`. A team server refuses to start with a `LANDSCAPER_LAYOUT` that is neither.
- **`serve` without `--team` refuses an address other machines can reach** (`--bind 0.0.0.0`) unless you add `--allow-remote`.
  On your own machine every request is you, with no sign-in. Team servers are not affected.
- **`X-Forwarded-For` is trusted only on requests that carry the proxy secret.** The activity log records the address a request
  came from otherwise, and no user name from its headers.
- **`LANDSCAPER_PROXY_SECRET` is trimmed**, so an env file with a trailing space or Windows line endings works.
- **Uploads have limits on a team server:** 150 MB a file, 20 files and 500 MB in a workspace, one upload at a time per builder
  and four at once in all. More answers 413 or 429 with the reason.
- **Imports are limited to 32 tiles a side.** A bigger area, or one whose tile list doesn't match its size, is refused before
  the game is opened.
- **Check and export refuse what an area can't hold:** an object, thing or entrance more than 10 m outside the area, an object
  scaled outside 0.1 to 5, a value that isn't a number, or an entrance wider or deeper than the area. The message names it.
  (The editor never saves such an area; this stops a hand-made request using all of the server's memory.)
- **A team server refuses to start when `assignments.json` or `markers.json` doesn't parse**, and names the file. Before, it
  read the file as empty, and the next grant or export wrote it back with one row. While it runs, a damaged file makes the
  routes that need it answer an error instead.
- **`--jobs` (`LANDSCAPER_JOBS`) now limits exports and checks too, not only imports.** When it isn't given, an export works on
  every core (it used to bake textures on up to 8 threads and check routes on every core), and an import or a check on half the
  cores (a check's routes used every core). A team server started with `LANDSCAPER_JOBS` or `--jobs` keeps that limit for its
  exports too. `LS_THREADS` is gone.
- **Imports no longer write `<name>.import.png`** (the whole area at full size: 144 MB for 16 by 15 tiles), **and exports no
  longer write each tile's model as text** (`<tile>.mdl.txt` beside the module: 58 MB for 32 by 32). Nothing read either;
  `LS_DUMP` writes the models' text again. A re-import, or removing the import, clears an old `.import.png` away.
- **A team server's workspaces have a disk quota:** 100 areas and 5 GB each, uploads aside. A save, import, export or check
  that would go over is answered 413, and only the newest 3 `<name>.json.replaced-<seconds>` copies of an area are kept. A
  builder makes room by deleting an area in the editor (below).
- **A slow request is cut off.** Reads and writes on a connection wait 30 s at most; a request's line and headers must come
  within 30 s and 1 MB (414, 431 or 408 otherwise), checked as they arrive; a body must average 16 KB/s after its first 20 s
  (408), and one person's requests may be sending at most 8 bodies at once (429). A connection idle for 30 s is closed. A
  body refused unread is thrown away by its connection, not the thread that answered it. The HTTP server is now tiny_http_dh
  0.12.4, the maintained fork of tiny_http, with a patch of ours, in `vendor/tiny_http`.
- **A request whose body is framed two ways is refused** (400, and the connection closed): a `Transfer-Encoding` other than
  `chunked` alone, or one with a `Content-Length`. tiny_http took any `Transfer-Encoding` for chunked, so behind a proxy that
  framed the body by its length a request could be hidden in another's body and answered without the proxy seeing it (request
  smuggling, CVE-2026-66752; fixed in tiny_http_dh).
- **`days` in a grant is 1 to 36,525** (`POST /api/assignments`); more, or a value that isn't a whole number, is refused.
  Before, a huge value overflowed into a grant that ended within a day, and a value that wasn't a number gave the area for good.
- **Answers on a team server name files relative to the builder's workspace:** the check and export reports' `module` and
  `installed_module` read `exports/...` or `nwn/modules/...`, and errors have the server's folders taken out. An upload import's
  `source` is `<file>:<area>` (on your own machine too).
- **`/api/preview-file` and `/api/preview-icon` answer 502 when the review service fails**, and 404 only when it has no such
  file. A preview is served as the type its name says, never the one the service sent.
- **The self-host image builds the editor of the engine's own version** (`WEB_REF`, default `v` and the version).

**New:**

- **Stable entrance ids.** Each entrance keeps its own id, and its door's tag is `<area>_ent<id>`, so taking one entrance out no
  longer renames the doors of the others. Areas saved before keep every door's tag. The export's header keeps the next id
  (`entranceNextId`), and an export that gives a new entrance the id of one taken out is refused.
- **An entrance taken out takes its door with it.** The next export removes that door from the installed module and lists its
  tag in the report (`updated.doors_removed`). Doors placed by hand in the toolset stay.
- **A blueprint's own settings for the editor:** `GET /api/blueprint-fields?kind=<kind>&resref=<resref>` answers the settings
  the blueprint has (on/off as `1` or `0`; one it doesn't have is left out), or 404.
- **Security headers on every answer:** the editor's Content-Security-Policy (read from its `index.html`) with
  `frame-ancestors 'none'`, `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff`. `Caddyfile.example` gives Caddy's
  own answers the same without replacing the engine's (docs/TEAM-SERVER.md, "Requests from other sites").
- **A new area never replaces another without asking.** The first save of a new area (`/api/save?new=1`) is answered 409 when
  the workspace already has an area of that name, unless `&replace=1`, which keeps the old copy as
  `<name>.json.replaced-<seconds>`.
- **Two tabs on one area no longer save over each other.** `GET /api/area` answers the working copy's version
  (`X-Landscaper-Version`), a save answers the new one, and a save that sends the version it started from (`&base=`) is answered
  409 (`"moved": true`, with when the other copy was saved) when the area was saved from elsewhere since. Areas saved before
  `created` ids are covered too. `&replace=1` keeps the other copy as `<name>.json.replaced-<seconds>`.
- **The read-only view is held by the engine:** while the `ls_as` cookie names another workspace, everything but reads, `POST
  /api/view-as` and `POST /api/check` is answered 403, whatever sends it. Before, such a request worked in the admin's own
  workspace.
- **A builder can delete an area** (`DELETE /api/area?name=<name>&base=<version>`, the editor's File, Delete area). Its working
  copy and import are set aside in `deleted/<name>-<seconds>/` for an admin to bring back (docs/TEAM-SERVER.md, "Deleted
  areas"); its last export, its build folder and the copy before its last save are removed; the installed module, its hak and
  any assignment stay. The name is free again and no longer counts towards the 100 areas. It takes the area's lock as a save
  does, and is answered 409 when the area was saved since the version the editor holds (`"moved": true`, as a save), or while
  another area uses its import (`used_by`); 404 when there is no such area. On a team server it works in the builder's own
  workspace only, never in the read-only view, refuses an area whose assignment has ended (403), and keeps the newest 3
  deleted copies of a name, which count towards the 5 GB.
- **The export's `palette`, `water`, `entrances` and `atmosphere` go in its body's JSON**, beside the objects, not in its query
  string, which keeps the area's name and its simple settings (`tiles`, `tilesY`, `shadow`, `module`, `kind`, `roof`,
  `clearance`). An export or check that still sends one in the query string (an editor older than this engine) is answered 400
  and told to reload the page. The editor of the same release sends the new form.
- **A re-import takes the area's lock** as a save does: another tab's save at that moment was moved aside unseen, or put the old
  area back over the import.
- **The review service's last area list is kept on disk** (`review-areas.json` in the data folder) and read at start, so a
  restart while the service is down no longer stops builders creating, importing and exporting areas (docs/TEAM-SERVER.md,
  "While the review service is down").
- **Releases** tag the engine and the editor with the same `vX.Y.Z`, and a test keeps the self-host image's `WEB_REF` in step.

**Quicker and lighter:**

- Exports draw the tiles' shadow maps on several threads, as the baked textures are: a 32 by 32 export took 38 s and now
  takes 15 s on 12 threads, with the same hak.
- A painted export holds each baked texture once, not twice (about 170 MB less at 32 by 32), and an import no longer holds the
  whole area's picture (1.45 GB at its peak for 16 by 15 tiles, now 0.84 GB, and half the time).
- An export no longer reads the area's hak into memory to put it back if it fails: it keeps it under a second name instead.
- The area list and the links (`/api/areas`, `/api/links`) read only a few fields of each saved area, not its heights: about
  seven times quicker with 40 areas of 32 by 32 tiles.

**Fixed:**

- An import is refused (409, the same answer as a save's) when the workspace has a working copy of that name. Before, the
  editor then opened the old copy, and its next export took in the import's objects.
- Revoking an area removes only the imports no other assignment of the builder still covers: revoking `bay1` from someone who
  also holds `*` keeps their import of it, and a revoke that ends nothing removes nothing. Before, `area=*` removed every
  import of a module area in the workspace, whatever the builder still held.
- A slow or hung review service no longer makes requests wait in turn: one request fetches the area list, references or hak
  list (15 s timeout, was 30 s) while the others use the copy there was, and a failure is kept 30 seconds instead of being
  retried by every request. With no area list at all, a new area's name is refused ("the module's area list isn't available")
  instead of let through unchecked, and `/api/import-sources` answers 503 instead of an empty list.
- Whether a refusal is logged as `denied`, and whether a missing blueprint or script counts as "none", no longer depend on the
  wording of a message.
- `serve` kept every area plan it had made, so a team server grew by one or more megabytes with each Check and export as people
  edited. It now keeps only the current area's.
- A failed export changes nothing: the module is replaced in one step (the one before is kept as `<module>.mod.bak`), and the
  export's own files and hak are put back. Before, the next export could lose changes to light, fog and settings.
- A failed re-import keeps the earlier import, instead of leaving the area with no kept original.
- One request could stop the engine, without signing in: a `Content-Length` of 100 TB on any request whose body wasn't read
  (one the gate refused, say) made it abort, unable to allocate a buffer of that size to throw the body away.
- A builder could read another builder's blueprints and conversations: the `from` of `POST /api/new-blueprint` and
  `POST /api/dialog`, and the `resref` of `GET /api/dialog?any=1`, took a path. They take a resref only now.
- 32 requests whose bodies stopped arriving held every answer thread of a team server, and a request line of hundreds of MB was
  kept in memory before the gate refused it. (See "A slow request is cut off" above.)
- A preview whose type the review service sent wrong could open as a page on the editor's origin; a type no header can hold
  answered 500, and a file over 64 MB was cut short and kept by the browser for a year. All three are refused now (502).
- Saves of one area take turns, and the area's file is replaced in one step: overlapping saves on a team server could leave only
  the `.bak`.
- The area's hak, the shared hak, the texture library's catalogue and the installed module are written in one step, so a crash
  or a full disk can't leave a cut-off file (a cut-off catalogue used to fail every export on a team server).
- A builder's own blueprint is never shared through the cache with another builder who asks for the same resref.
- An area imported into a folder of its own (`landscaper import <source> <name> <dir>`) now exports from there.
- Uploads are written to disk as they arrive instead of being held in memory, each in a part file of its own.
- `publish-module` sends the group the area's files go in as `kind`, instead of always `core`.
- Error answers and log lines no longer show a service's address or a Discord webhook's token. Discord notices are at most one
  of each kind per person every ten minutes, successful imports included.
- `%XX` in a request is decoded only when both characters are hex digits.
- `textures add` and `textures build` explain their arguments in `--help`.
- An admin's activity log (`GET /api/audit`) skipped February's events from 1 to 3 March: "last month" was taken as 31 days
  back, which is January then. It now reads this calendar month and the one before.

**Other:**

- `src/pathnodes.json`, the measured path node layouts, is under CC0-1.0.
- `GET /api/scripts` no longer answers `canWrite`; the editor stopped reading it.
- The self-host image checks the neverwinter.nim download (`NWN_NIM_SHA256`) before unpacking `nwn_gff`.
- New tests, some needing the game's files: the export, re-export and import cycle, an import into a folder of its own,
  entrance doors, and packages in both module layouts (README, "Development").
