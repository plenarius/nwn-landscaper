# The team server

`landscaper serve --team` runs the Landscaper for several people at once: each signs in through a proxy, gets a workspace of
their own, and can be limited to the existing areas they are given. Setting it up is in [SELF-HOSTING.md](SELF-HOSTING.md) and
every setting is in [CONFIGURATION.md](CONFIGURATION.md); this page is how it behaves.

## What it protects

A module's source (every area's layout, scripts, blueprints, conversations) is a team's work. The team server lets builders build
without holding all of it:

- **Not protected, because it is public anyway:** the game's own content and the haks players download (models, textures,
  tilesets, tables). The Landscaper's model and texture routes serve those.
- **Protected:** the module. A builder can start any new area, but can only import an *existing* area of the module when an admin has
  assigned it to them, and the server never holds a copy of the module when a review service is configured (below).
- **Not prevented:** a builder who is assigned an area sees that area. The aim is that each person sees what they need, with a
  record of it (audit log, hidden mark).

## Game files

The server reads the game from `NWN_ROOT` (the install: the key and bif files, `lang/<language>/data/dialog.tlk`, `ovr/nwscript.nss`)
and the module's haks and talk table from `NWN_HOME` (`hak/`, `tlk/`). Both can be mounted read-only; builders' exports go to their
own workspace, never there. Two things are worth knowing:

- Script checks compile against `ovr/nwscript.nss`, and names come from `dialog.tlk`. An install that is older than the one your game
  server runs, or lacks functions your module adds to `nwscript.nss`, makes the check reject scripts the game accepts. Keep the
  server's copy of the game the same as the one your module is built against.
- `NWN_HOME` only needs the haks and the talk table. A whole server folder (vault, database) mounted there works but is not needed.

## Identity and workspaces

- The server trusts the sign-in proxy's headers for who is asking and never a name the browser sends. Header names are
  configurable (`LANDSCAPER_AUTH`). A request must also carry `LANDSCAPER_PROXY_SECRET` in `X-Landscaper-Proxy`, so
  nothing that bypasses the proxy is answered. A request with no user is 401, a user outside the allowed groups 403.
- A team server doesn't start without a proxy secret of at least 16 characters (and not the example's `change-me`). Where
  nothing but the proxy can reach it, `--insecure` (or `LANDSCAPER_INSECURE=1`) lets it run without one.
- Each person has `workspaces/<user>/{areas,exports,nwn,blueprints,scripts,uploads,deleted}` under the data folder. `nwn/` stands in for
  the game's user folder (their own `hak/` and `modules/`), so exports never touch the game's. User names are cleaned to
  `[a-z0-9._-]`, upper case folded to lower: `Alice` and `alice` are one person with one workspace, so the sign-in proxy
  must not give two people names that differ only in case.
- **Admins** (`LANDSCAPER_ADMIN_GROUPS`, `LANDSCAPER_ADMIN_USERS`) can look at anyone's workspace: a cookie (`ls_as`, HttpOnly,
  SameSite=Strict) that applies only to reads, so the editor is read-only there (no autosave, export, check or import) with a
  "Back to mine" button. The engine holds that too: while the cookie names another workspace, it answers 403 ("`<user>`'s
  workspace is read-only here: nothing was changed") to every request but reads (`GET`, `HEAD`), going back (`POST
  /api/view-as`) and checking paths (`POST /api/check`, `POST /api/canvas-check`), whatever client sends it. Admins also read
  the activity log, manage assignments, and may write scripts.

## Requests from other sites

Before it looks at who is asking, the server (team or not) refuses with 403 and `{"error": "refused: ..."}` (the reason):

- a request whose `Host` isn't the server's: on your own machine `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`; on a
  team server those and the names in `LANDSCAPER_PUBLIC_HOST`. This stops DNS rebinding (a page that points its own name at the
  server to read its answers). A team server without `LANDSCAPER_PUBLIC_HOST` warns at start and answers any name;
- an API request (`/api/...`) without the header `X-Landscaper: 1`. The editor sends it with every call; a page on another site
  can't add it without the browser asking the server first (CORS), and the server never agrees. The pictures the editor loads as
  images (`/api/texture/`, `/api/import/`, `/api/import-file/`, `/api/preview-icon/`, a preview's `/tex/`) are read without it.
  A script or tool that calls the API sends it too (`curl -H 'X-Landscaper: 1' ...`);
- a request that changes something (not GET) with an `Origin` that isn't the server's own (its `Host`, the proxy's
  `X-Forwarded-Host`, or a name in `LANDSCAPER_PUBLIC_HOST`);
- an API request, or one that changes something, marked `Sec-Fetch-Site: cross-site` by the browser.

Every answer (the page, its files, API answers, refusals) carries:

- `Content-Security-Policy`: the editor's own policy, read from the `<meta http-equiv="Content-Security-Policy">` in its
  `index.html` (so that tag is the one place to change it), with `frame-ancestors 'none'` added. An editor without such a tag
  gets only `frame-ancestors 'none'`.
- `X-Frame-Options: DENY`, for browsers too old for `frame-ancestors`, and `X-Content-Type-Options: nosniff`.

A proxy in front must pass these through and not replace them: the policy names the page's import map by its hash, so a policy
of the proxy's own stops the page loading. To give the proxy's own answers (a sign-in prompt, an error while the engine is down)
the same protection, set them only where the answer has none, as `deploy/self-host/Caddyfile.example` does with Caddy's `?`
form (`header ?X-Frame-Options DENY`).

Names that become file names (an area's, a blueprint's, a script's, a file's) are checked before anything is read or written: an
area's name is 1 to 9 lower case letters, digits or `_`; anything else (`../`, a slash or backslash, NUL) is refused.

## Assignments

An *assignment* is: this person may import and rework this existing area (with an optional end date and a note). Stored in
`assignments.json` beside the workspaces. A file that doesn't parse (a hand edit, a damaged restore) stops the server at start,
and is never read as empty or written over: until it is fixed, every route that needs assignments answers an error naming it.
`markers.json` is treated the same.

- Default deny. Import, re-import, export and check are checked on the server for anything that is an existing area of the
  module; a new area may not reuse an existing area's resref or tag. Saving the editor's copy (`/api/save`) is not checked against
  the assignment: the copy stays in the builder's own workspace, and nothing built from it leaves without an export.
- Admins grant and revoke: `POST /api/assignments?user=&area=[&days=][&note=]`, `POST /api/assignments/revoke?user=&area=`,
  `GET /api/assignments` (admins see all, a builder their own). `days` is a whole number from 1 to 36,525 (100 years); without it,
  or 0, the grant has no end.
- Searching for areas to import (`GET /api/import-sources?q=`) is limited for builders: three letters or more, at most 25
  results, only name, tag, tileset, size and whether it is theirs (no paths or layout). Imports are by area name, never by file path.
- Revoking removes what was built from the area (the kept original, its pictures and objects, the build folder, the hak) and
  refuses further check, export and re-import of it, including of a renamed copy. The builder's saved editor copy stays. An
  import that another of the builder's assignments still covers stays (revoking `bay1` from someone who also holds `*`, or `*`
  from someone who also holds `bay1`), and a revoke that ends nothing removes nothing.
- A new area's name is checked against the module's area list. See "While the review service is down" below for when that list
  isn't available.
- A builder can also upload their own `.mod` or `.erf` and import areas from it. Those are theirs and need no assignment, but their
  names may not collide with the module's. A file is at most 150 MB, a workspace keeps at most 20 files and 500 MB of them, and
  uploads take turns: one at a time per builder, four at once in all (more answer 429).
- **A workspace has a disk quota:** at most 100 areas (working copies, exports and imports, by name; a canvas counts as one) and
  5 GB, its uploads aside.
  A save, import, export or check that would go over is answered 413 and writes nothing. A builder makes room by deleting areas
  they no longer need (see "Deleted areas" below). Of the copies kept when a save replaces another area of the same name
  (`<name>.json.replaced-<Unix seconds>`), the newest 3 stay, and so do the newest 3 deleted copies of each name; both count
  towards the 5 GB, so a workspace full of them needs an admin to clear `workspaces/<user>/deleted/`.
- **Answers name files as the builder knows them:** relative to their workspace (`exports/.check/cave_nwn/cave.mod`), never
  by the server's path; errors read `your workspace`, `<data>`, `<module>` or `<game>` where the server's folders were. The
  server's log keeps the whole paths.

### Deleted areas

A builder deletes the area open in the editor with **File, Delete area** (`DELETE /api/area?name=<name>&base=<version>`). On a
team server it works only in their own workspace: never in an admin's read-only view of another (403), and not on an area
whose assignment has ended (403, as an export). It is refused with 409 while another of their areas uses its import (its
ground or objects; the answer's `used_by` names them), and when the area was saved since the version the editor holds (see
"Requests and concurrency").

- **Set aside, for an admin to bring back:** the working copy (`areas/<name>.json`, with its underlay picture
  `areas/<name>.underlay`) and the import (`exports/<name>.import/`
  and its picture `exports/<name>.import.view.png`), moved as they are into `workspaces/<user>/deleted/<name>-<Unix seconds>/`,
  under the same `areas/` and `exports/` folders. The newest 3 deleted copies of each name stay.
- **Removed, since an export makes them again:** the copy before the last save (`<name>.json.bak`), the last export
  (`exports/<name>.json`, `.f32`, `.materials`), its build folder (`exports/<name>_nwn/`) and what the editor's Check left
  (`exports/.check/<name>.*`).
- **Kept as they are:** the installed module (`nwn/modules/<module>.mod`) and the area's hak (`nwn/hak/<name>.hak`). The
  module is the builder's own work and may hold other areas, and it needs the hak to load this one. Assignments stay too: they
  give a production area, not one copy of it, so the builder can import it again.

The name is free at once: a new area or an import may take it, and it no longer counts towards the 100 areas.

To bring an area back, as an admin on the server, move its files back into the workspace while no area of that name is there
(rename the builder's new one first if there is). In the container the data folder is `/data/landscaper`:

```sh
cd /data/landscaper/workspaces/bob/deleted/town-1763500000
mv areas/town.json ../../areas/
mv exports/* ../../exports/
```

The builder then opens it and exports it, which builds its export again. An area that had no working copy (imported, never
changed) comes back with its import alone: the builder imports it again from the Open list (**Re-import**). On your own
machine the same folder is `deleted/` in the data folder (`~/.local/share/landscaper/deleted/`), and every deleted copy stays
until you remove it.

### While the review service is down

The team server keeps the module's area list, its references and its hak list ten minutes, and the last area list that arrived
also on disk, in `review-areas.json` beside `assignments.json` (written in one step whenever a new list differs from it, read at
start). When the service stops answering:

- **Builders carry on with what they have:** opening, painting and saving areas, exporting, checking and testing them, and
  searching and creating areas against the last area list there was, from before a restart too. The server asks the service
  again at most once every 30 seconds, and each failed ask takes at most 15 seconds.
- **What needs the service itself fails with its reason:** importing a module area (its ARE and GIT come from the service),
  the skeleton module's blueprints and scripts, previews of its creatures, and publishing.
- **The reference checks** (scripts, conversations, tags an area refers to) are skipped while the service has never answered
  since the server started.
- **With no area list at all** (a new server, or `review-areas.json` removed, before the service first answers), saving a new
  area, exporting, checking and importing are refused with "the module's area list isn't available": a new area's name can't
  be checked against the module's.

## Canvases

A canvas ([the design](DESIGN-canvas-cut.md), [the handbook](handbook/canvases.md)) is one more item of a builder's workspace,
next to their areas: `workspaces/<user>/canvases/<name>/`, a header (`canvas.json`) and its ground in blocks of 16 by 16 tiles.

- **The same rules as an area.** It counts as one item, by its size on disk, against the quota; an admin viewing another
  workspace reads it without changing it. Its name (1 to 7 characters) can't be an area's or a canvas's in the workspace, nor
  an existing area's of the module.
- **Assignments:** making, saving, checking and exporting a canvas need the builder's assignment of every existing area it is
  built from (the imports its paint and objects use), and of every existing area a piece is named like, since its publish would
  replace that area.
- **Make a canvas from this area** names the first piece as the area, so a canvas made from an assigned area needs that
  assignment to stand, as the area does; the area's working copy is not changed.
- **Saves** send the header and only the blocks that changed, each with the version it started from, up to 48 MB a save (a
  whole 128 by 128 canvas). A save is refused whole (409) when another tab saved any of them since. A block file that is
  damaged stops whatever needs it, names the file, and is never written over.
- **Export and check** (`POST /api/canvas-export`, `POST /api/canvas-check`) build each piece into one module named after the
  canvas, in the builder's `nwn/`, as an area's export does, and take turns with the other heavy jobs.
- **Publish, Package and Try on dev** (`/api/canvas-publish`, `/api/canvas-package`, `/api/canvas-trial`) send the pieces
  together as one `publish-module` request, under the same assignment checks; Publish sends the pieces changed since the last
  publish unless others are named. The editor's buttons for a canvas come next.

## Audit log, hidden mark, notices

- **Audit log:** one JSON line per event in `audit/<year>-<month>.jsonl`: when, who, the client address (the last
  `X-Forwarded-For` entry), what, which area, allowed or refused, and a few words. It records searches, imports, exports,
  assignments, packages and publishes, an admin viewing a workspace, and refusals. Admins read it with `GET /api/audit` and the
  **Activity** button. A request without the proxy secret is logged with the address it connected from and no user: its
  `X-Forwarded-For` and user header are its sender's choice. Refusals are logged once a minute per address, for at most 1,024
  addresses a minute.
- **Search limit:** builders may search `LANDSCAPER_SEARCH_PER_MIN` (30) and `LANDSCAPER_SEARCH_PER_DAY` (300) times; more is a 429
  and is logged. Admins are not limited. The count is in memory.
- **Hidden mark:** every export on a team server carries a per-builder, per-area id (12 hex digits, kept in `markers.json`) in the
  area's variables and comments and on its objects, so a leaked area can be traced to a workspace: `landscaper marker <id>` on
  the server (in a container, `docker compose exec landscaper landscaper marker 3f9a0c12b7de`) says which builder and area it
  belongs to. The id is the `ls_ref` variable, or `ref:<id>` in the area's comments. It deters; someone who knows can strip it.
  Publishing and packaging take it out.
- **Notices:** with `LANDSCAPER_DISCORD_WEBHOOK` set, a message is posted when a builder imports an existing area, tries one that
  isn't theirs, or hits the search limit: at most one of each kind per person every ten minutes, the rest only in the activity
  log. A failed post never fails the request, and its log line never holds the webhook's address (that is its token).

## Modules, blueprints and scripts

- **With a review service** (`LANDSCAPER_REVIEW_URL`, `LANDSCAPER_REVIEW_KEY`): the area list, the areas themselves (fetched only
  after the assignment check and converted in a temporary folder that is removed straight after), blueprints, conversations and
  scripts come from that service, so the team server holds no copy of the module and no repository token. Every call is in
  [REVIEW-SERVICE.md](REVIEW-SERVICE.md).
- **Without one:** they are read from a checkout (`LANDSCAPER_MODULE_REPO`, laid out as `LANDSCAPER_LAYOUT` says).

## Exports and testing

A test module is a skeleton: the area, a start point and the hak list, with no scripts or blueprints of the module. Export builds it
into the person's `nwn/modules` and `nwn/hak`. If the review service runs a dev server, **Try on dev** in the Publish dialog
puts the export there with no restart, one trial per builder; new scripts can't be tried there and go through Publish.

## Publishing

- **A package** (no review service needed): files in the layout of the module's source, to commit yourself.
- **Pull requests** (with a review service): the same files, checked and opened for review. Nothing is merged by the Landscaper.
- Either way it is what the **last export** built, and a pre-publish check must pass first (see BUILDER-WORKFLOW.md).

## Saved versions

With `LANDSCAPER_GITEA_URL`, `LANDSCAPER_GITEA_TOKEN` and `LANDSCAPER_GITEA_REPO` set, **Save version** commits the editor's copy of
an area (and where an import came from) to a branch `build/<user>/<area>` of that Gitea repository, authored by the builder. The
repository should be readable by admins only: an area built from an existing one carries its ground. Autosave never commits.

## Requests and concurrency

Each request is answered on its own thread and a person's identity belongs to that thread. Heavy jobs (check, export, import,
re-import) take turns, one at a time; everything else carries on. On your own machine requests are answered one at a time.

- **Saves of one area take turns**, and a save never leaves the area missing: the editor's copy is replaced in one step, and the
  one before is kept as `<name>.json.bak`.
- **A new area never replaces another without the builder agreeing.** The first save of a new area (`/api/save?name=<name>&new=1`)
  is answered 409 when the workspace already has an area of that name (a working copy, an export or an import), and so is a save
  whose area was created separately from the saved copy (its `created` differs). The answer is
  `{ "error": "there is already an area called <name>: open it, pick another name, or replace it", "exists": "<name>" }`, and
  nothing is written. The editor then asks; `&replace=1` replaces the working copy, keeping the old one as
  `<name>.json.replaced-<Unix seconds>` (on a team server, the newest 3 of them).
- **Two tabs or computers on one area don't save over each other.** Opening an area (`GET /api/area`) answers its version in
  the `X-Landscaper-Version` header (16 hex digits of the working copy's FNV-1a hash, or `none` without a working copy), and a
  save answers the new one (`{ "saved", "version" }`). The editor sends the version its area started from with each save
  (`&base=<version>`); when the working copy has changed since, the save is answered 409 with
  `{ "error", "exists": "<name>", "moved": true, "saved": <Unix seconds>, "version": "<the one there now>" }` and nothing is
  written. The editor then keeps its copy in the browser and asks which to keep; `&replace=1` saves over the other, keeping it
  as `<name>.json.replaced-<Unix seconds>`. The version comes from the file's bytes, not its time, so a tab's own autosaves in
  quick succession never refuse each other; a save without `base` (an older editor) isn't checked.
- **A delete never removes what another tab has saved since.** `DELETE /api/area` takes the area's lock as a save does, and needs
  the version the editor holds (`&base=<version>`, `none` for an area with no working copy): when the working copy has changed
  since, it is answered 409 with the same body as a save's (`"moved": true`), and nothing is deleted. A re-import takes the
  area's lock as well, so a save never lands in the middle of one.
- **An import never takes an existing area's name** either (a working copy, an export or an import): `/api/import` answers 409
  with the same body, and nothing is imported. Over a working copy alone, the editor would open the old copy, and its next
  export would take in the import's objects.
- **A client can't hold a thread.** Every read and write on a connection waits 30 s at most, and a request's line and headers
  must arrive within 30 s and 1 MB (read as they come, before anything else looks at the request: 414 for a longer request
  line, 431 for longer headers, 408 for a slow one). A request's body must keep up 16 KB/s on average after its first 20 s, or
  it is answered 408 and an upload gives back its turn. One person's requests may be sending at most 8 bodies at once (429).
  A body a request is refused without (413, 429, 403) is thrown away by its connection, never by an answer thread. A
  connection idle for 30 s between requests is closed. At most 512 connections are read at once and 512 more wait; past that a
  new connection is closed at once. A request whose body is framed two ways (a `Transfer-Encoding` other than `chunked` alone, or
  one with a `Content-Length`) is answered 400 and its connection closed, so nothing can be hidden in a body past the proxy
  (request smuggling). The engine's HTTP server is tiny_http_dh, the maintained fork of tiny_http, with a patch of ours for this
  (`vendor/tiny_http/LANDSCAPER.md`).
- **A failed export or re-import changes nothing.** An export replaces the module in one step (the one before is kept as
  `<module>.mod.bak`) and puts back its own files if it fails; a failed re-import keeps the earlier import.
