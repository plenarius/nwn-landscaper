# The review service

A team server can work without a copy of the module: a **review service** holds the module's source, gives the team server what a
builder needs (the area list, an assigned area, blueprints, scripts), and turns a published area into pull requests. This page is
every call the Landscaper makes to it, so a team can write its own (as a service of its own or as part of a larger one).

The service is optional. Without it a team server reads the module from a checkout (`LANDSCAPER_MODULE_REPO`) and a builder hands
an area over as a package to commit by hand (TEAM-SERVER.md, "Publishing").

## Settings

| Variable | What |
|---|---|
| `LANDSCAPER_REVIEW_URL` | The service's address, such as `http://review.internal:8000`. A `/` at the end is dropped |
| `LANDSCAPER_REVIEW_KEY` | The key the Landscaper sends with every call |

The service is used only on a team server (`--team`) with both set.

## Every call

- **Authentication:** every call sends `Authorization: Bearer <LANDSCAPER_REVIEW_KEY>`.
- **Answers** are JSON (`Content-Type: application/json`) unless a call below says otherwise.
- **Errors:** any status other than 2xx is a failure. When the body is JSON with a `detail` string, the Landscaper shows that text
  to the builder (for example `{"detail": "new scripts can't be tried on dev"}`), so write it for them. `404` on a single area,
  blueprint or script means "there is none"; `429` on an area means the builder fetched too many today.
- **Names** in a path are lower case: an area, blueprint or script resref is letters, digits and `_`, at most 16 characters.
- **Who asks:** the team server checks who the builder is and what they may do (assignments, admin) before it calls. The service
  gets the builder's user name in `user` where it needs it (to log, limit or credit), and should trust the team server for it:
  the key is the proof.
- **Timeouts:** 15 seconds for the area list, the references and the hak list, 30 for other reads, 60 for the calls marked
  *60 s*, 15 minutes for publishing.
- **Caching:** the area list, the module's references and its hak list are kept ten minutes by the team server. One request at
  a time fetches each; the others meanwhile use the copy they had. When a fetch fails, the team server uses its last copy and
  asks again after 30 seconds, so a service that is down gets one call every 30 seconds, not one per request. The last area
  list that arrived is also kept on disk (`review-areas.json` in the server's data folder) and read at start, so a restart
  while the service is down changes nothing for builders. Before any area list has arrived (a new server, or one whose
  `review-areas.json` was removed), routes that check a new area's name against it are refused. docs/TEAM-SERVER.md, "While the
  review service is down", says what builders see.
- **GFF files** (areas, blueprints) travel as nwn_gff's JSON, as text inside the JSON answer.

## The module

### `GET /api/v1/areas/index`

The module's areas, for the import list. No layouts.

```json
{ "areas": [ { "area": "elmwood", "tag": "ElmWood", "name": "Elm Wood", "tileset": "ttr01", "width": 8, "height": 8, "flags": 4,
               "kind": "core" } ] }
```

`area` is the resref and the only field required. `name` is the area's name as players see it (the talk table already applied),
`flags` the ARE `Flags` (bit 0 interior, 1 underground, 2 natural), `kind` the folder the area is in. The examples on this page use
the `folders` layout (`LANDSCAPER_LAYOUT=folders`, areas in `areas/<group>/`); in nasher's layout, the default, an area's files
are `src/<resref>.are.json` and its group is empty.

### `GET /api/v1/areas/<resref>?user=<user>`

One area, for a builder the team server has checked is assigned to it. The service may log and limit it (answer `429`
after a daily number, say).

```json
{ "area": "elmwood", "kind": "core", "are": "<the .are as nwn_gff JSON>", "git": "<the .git, or null>",
  "haks": ["my_tiles1", "my_tiles2"], "tlk": "mymodule" }
```

`are` is required; `git`, `haks` (the module's hak list, in its order) and `tlk` (its custom talk table, without `.tlk`) may be
`null` or missing.

### `GET /api/v1/areas/module`

The module's hak list, in its own order, and its custom talk table: what its areas' tiles, models and textures are found in.

```json
{ "haks": ["my_tiles1", "my_tiles2"], "tlk": "mymodule" }
```

### `GET /api/v1/areas/refs`

What the rest of the module holds that an area can name, for the check before publishing (a door leading nowhere, a script that
doesn't exist). Every name lower case or not, the Landscaper folds them.

```json
{ "scripts": ["area_onenter"], "dialogs": ["guard01"],
  "waypoints": { "wp_gate": ["elmwood"] }, "doors": { "dr_elm_in": ["elmwood"] },
  "areas": { "elmwood": ["elmwood"] }, "links": { "dr_elm_in": ["elmwood_inn"] } }
```

`waypoints`, `doors` and `areas` map a tag to the areas that have it; `links` maps a door's tag to where it leads. A missing key
counts as empty.

### `GET /api/v1/areas/<resref>/waypoints` (*60 s*)

An existing area's waypoints, for a transition to lead to (*Pick a waypoint of an existing area*). The editor shows them as they come.

```json
{ "area": "elmwood", "waypoints": [ { "tag": "WP_Gate", "x": 12.5, "y": 40.0 } ] }
```

## Blueprints and scripts

The module's skeleton: the blueprints and scripts a builder can use in a new area.

### `GET /api/v1/blueprints/<ext>?q=<text>&limit=<n>`

Blueprints of one kind (`ext`: `utc`, `uti`, `utp`, `utd`, `dlg`, ...) whose name or resref has `q` (letters, digits, spaces, `_`
and `-`, at most 40; spaces sent as `%20`).

```json
{ "blueprints": [ { "resref": "guard01", "name": "Town Guard" } ] }
```

### `GET /api/v1/blueprints/<ext>/<resref>`

One blueprint: `{ "resref": "guard01", "json": "<the file as nwn_gff JSON>" }`, or `404`.

### `GET /api/v1/scripts?q=<text>&limit=<n>`

Scripts whose name has `q` (letters, digits and `_`, at most 40): `{ "scripts": [ { "resref": "area_onenter", "folder": "area" } ] }`.

### `GET /api/v1/scripts/<resref>`

One script's source: `{ "resref": "area_onenter", "source": "void main() { ... }" }`, or `404`. The script check compiles a
builder's script against these, so includes are fetched this way too.

## Previews (*60 s*, optional)

Pictures of the module's own creatures and items, for a service that can draw them. Without them the editor shows no picture.

- **`GET /api/v1/blueprints/utc/<resref>/preview`:** `{ "resref": "guard01", "key": "<64 hexadecimal digits>" }`, the name of a
  baked 3D scene, or an error.
- **`GET /api/v1/render/<key>/scene.json`** and **`GET /api/v1/render/<key>/tex/<name>`:** the scene and its textures (`<name>`
  is lower case letters, digits, `_ . -`, ending in `.webp` or `.png`). Bytes, with their own `Content-Type`; at most 64 MiB. The
  team server passes them to the browser as they are and lets it keep them for good, so a key must never name different files.
- **`GET /api/v1/blueprints/uti/<resref>/icon`:** `{ "resref": "sword01", "key": "<icon key>" }`: which inventory icon the item
  shows.
- **`GET /api/v1/icons/<key>.png`:** that icon as a PNG. `|` and `^` in the key are sent as `%7C` and `%5E`.

## Trying areas on a dev server (optional)

For a service whose dev game server can take a builder's areas and tile files with no restart. The editor's Publish dialog shows
**Try on dev** when `GET /api/v1/dev/trial` answers (and not when it answers `503` with `"available": false`: trials are off). One
trial per builder; a new one replaces their last. New scripts and `.2da` files need a restart, so a service may refuse them
here: they go through Publish. Refusals are a `detail` sentence, which the editor shows as it is.

- **`POST /api/v1/dev/trial`** (*15 min*): the `publish-module` form below, without `dry_run`, for the area or its whole module
  (always with `library_files`: without the shared texture library the floors show magenta). The answer, shown to the builder:

  ```json
  { "user": "alice", "revs": { "bay1": 4 }, "reconnect": true,
    "way_in": "Connect to dev, then type ;forge for your areas, ...", "commands": { "bay1": ";forge bay1" },
    "areas": [{ "akey": "bay1", "new": true, "rev": 4, "tiles_changed": true, "dev": { "applied": true, "message": "" } }],
    "errors": [{ "akey": "cave2", "detail": "..." }], "skipped": [], "dropped": [], "warnings": [], "expires_in": 3600 }
  ```

  `reconnect` is true when dev's files changed: everyone on dev with the older files is disconnected about 10 seconds later.
  `skipped` are areas unchanged since the last trial, `dropped` the last trial's areas this one no longer has (back to live).
- **`GET /api/v1/dev/trial?user=<user>&touch=<true|false>`** (*60 s*): every trial, `{ "enabled": true, "trials": { "alice":
  { "akeys": ["bay1"], "expires_in": 3400, ... } } }`; the editor shows only the builder's own. `touch=true` counts as activity
  (a trial nobody touches for an hour is released); the editor sends it only when the builder asks to keep it longer.
- **`POST /api/v1/dev/trial/release`** (*60 s*): `{ "user": "alice" }` puts the areas back to live on dev and takes the files
  off. Answer `{ "released": true, ... }`.

## Publishing

`POST`, `multipart/form-data`, 15 minutes. With `dry_run` true the service only checks what it was sent and says what the pull
requests would hold; the editor shows that before the builder confirms, then sends the same with `dry_run` false. The service opens
the pull requests; the Landscaper never merges anything. Each zip holds files at the paths they go to.

A dry run names the tile, library and skies files instead of sending them: `hak_manifest` (`tiles.json`), `library_manifest`
(`library.json`) and `skies_manifest` (`skies.json`) in place of `hak_files`, `library_files` and `skies_files`, each a JSON
object `{ "<file>": ["<git blob id>", <size>] }`, so the service can compare them with the files the module's haks already have. A
service that wants the files themselves answers 422 to a form without `hak_files` (FastAPI's own refusal does), and the team
server sends the check again with the zips. A real publish always sends the zips.

### `POST /api/v1/areas/publish`

One area. Fields:

| Field | What |
|---|---|
| `user` | The builder |
| `name` | The area's name in the editor |
| `area` | The resref the module will know it by: an existing area's for a rework, otherwise a new one |
| `new` | `true` for a new area, `false` for a rework of `area` |
| `export` | The export's id (see [Fetching exports](#fetching-exports)): the same as the service routes list for it |
| `note` | The builder's note for the reviewers, at most 300 characters |
| `dry_run` | `true` or `false` |
| `module_files` (`module.zip`) | The area's `.are.json`, `.git.json` and `.gic.json` in the module's source layout (CONFIGURATION.md, `LANDSCAPER_LAYOUT`), without the builder's hidden mark |
| `hak_files` (`tiles.zip`) | The area's tileset files: models, walkmeshes, textures, minimaps, the `.set` |
| `library_files` (`library.zip`) | Optional: the shared texture library hak's files, for the service to add when the module's haks lack them |
| `skies_files` (`skies.zip`) | Only when the area has a Landscaper sky ([SKY-PACKS.md](SKY-PACKS.md)): the skies' hak's files, its `skyboxes.2da` among them (the module's table with the skies' rows added). The area's `SkyBox` is one of those rows, so the service must add them, and list `ls_skies` above every hak of the module with its own `skyboxes.2da` |
| `blueprint_files` (`blueprints.zip`) | Optional: the builder's own blueprints the area uses |

### `POST /api/v1/areas/publish-module`

Several areas as one set of pull requests. Fields: `user`, `note`, `dry_run`, `areas` (a JSON list of
`{ "name", "area", "new", "kind", "export" }`), then for each area `module_<name>` and `hak_<name>` (zips as above; on a dry run
`hakmanifest_<name>` instead of `hak_<name>`), and the optional `library_files` (or `library_manifest`), `skies_files` (or
`skies_manifest`) and `blueprint_files`.
`kind` is the group (the folder inside the areas folder) the area's files go in, as in a package: the folder the area is already
in when the team server has a module checkout, else `LANDSCAPER_AREA_FOLDER` (`core` in the `folders` layout, empty, for none,
in nasher's). A service that knows where a reworked area is should keep it there.

### The answer

Both answer with what the pull requests hold (`plan`) and, when they were opened, where (`pr_urls`); the editor shows it.

```json
{ "plan": { "area": "elmwood", "name": "elmwood", "new": false, "areas": ["elmwood"],
            "module_files": ["areas/core/elmwood.are.json"], "hak_files": 412, "library": false,
            "warnings": ["..."], "review": "### What changed\n..." },
  "pr_urls": { "nwn_module": "https://...", "nwn_haks": "https://..." } }
```

`module_files` is a list of paths and `hak_files` a count; `warnings` are shown to the builder; `review` is optional Markdown of
what the pull request will say. `pr_urls` is `null` on a dry run; its two keys are the module's pull request and the haks' (a
service with one repository can give the same address twice).

## Fetching exports

The other way round, and optional: a team server can let a trusted service (the review service, or another of the team's own)
list a builder's exports and fetch one, for example to bring a builder's area into another editor. These are routes on the
team server that the service calls, read-only, GET only.

| Variable | What |
|---|---|
| `LANDSCAPER_SERVICE_KEY` | The key the service sends. At least 24 characters (e.g. `openssl rand -hex 24`); a team server stops at start on a shorter one. Unset: the routes answer 404 |

- **Authentication:** every call sends `Authorization: Bearer <LANDSCAPER_SERVICE_KEY>`, compared in constant time; a missing or
  wrong key is answered 401 and recorded in the refusal log. The key opens these routes only. It isn't the proxy secret and signs
  no one in to the editor's API.
- **Reach:** the routes don't go through the sign-in proxy, so the service must reach the team server's port without it. Give it
  a way that only it has (a private network, a port published on an internal address only); don't route `/api/service/` through
  the public proxy. The request gate (`Host`, `X-Landscaper`, `Origin`) doesn't apply to them: it guards against browsers, which
  can't send the key.
- **Who:** `user` is the builder, as the sign-in proxy names them (lower case; letters, digits and `. _ -`), and must have a
  workspace (else 404). The routes act as that builder: an export is listed and fetched only when Publish would take it (its name,
  and the assignment of the area it reworks). A builder is an admin here only when `LANDSCAPER_ADMIN_USERS` names them, as no
  groups are sent. The service decides which builder asks; it should take the name from its own sign-in, never from its caller.
- **Log:** each call is in the activity log as `service-export`, by the builder, with the area and the route.
- **Names:** `name` is an area's name in the editor (lower case letters, digits and `_`, at most 9); a bad one is answered 400.
- **The export id** is the hex SHA-256 of the export's header file (`exports/<name>.json`, its bytes as on disk) followed by the
  hex SHA-256 of its hak, as text. Every export changes it, and Publish sends it as `export`, so the service can tell whether the
  tiles it fetched are the ones published.
- **Downloads** are streamed with a `Content-Length`, `Cache-Control: private, no-store` and an `ETag` that is the file's
  SHA-256 in quotes; `If-None-Match` with it answers 304 with no body. At most 8 downloads run at once; more are answered 503.

### `GET /api/service/exports?user=<user>`

The builder's exports that can be fetched, by name.

```json
[ { "name": "elmwood", "area": "elmwood", "new": false, "module": "elmwood", "exported_at": 1795000000,
    "export": "<the export id>", "hak": { "bytes": 7340032, "sha256": "<hex>" },
    "library": { "bytes": 10485760, "sha256": "<hex>" }, "skies": null } ]
```

`area` is the resref the module will know it by (an existing area's for a rework, as `new` says), `module` the module the export
went into, `exported_at` when its hak was written (Unix seconds). `hak` is `nwn/hak/<name>.hak`; `library` is the workspace's
shared texture library hak (the same in every entry), or `null` when there is none; `skies` the same for the skies' hak.

### `GET /api/service/export?user=<user>&name=<name>`

One export's area files, as Publish packs them (nwn_gff JSON, the builder's hidden mark taken out, a rework under the resref it
replaces), with the new blueprints and scripts the area uses and what the pre-publish check finds.

```json
{ "name": "elmwood", "area": "elmwood", "new": false, "export": "<the export id>",
  "docs": { "are": { }, "git": { }, "gic": { } },
  "blueprints": { "my_chest.utp": { }, "my_script.nss": "void main() {}" },
  "findings": [ { "level": "warning", "what": "...", "msg": "..." } ] }
```

`gic` may be `null`. A blueprint's key is its file name without `.json` and its value nwn_gff JSON; a script's value is its
text. 404 when there is no such export, 403 when Publish would refuse it (the `error` says why), 422 when it can't be read
(export it again).

### `GET /api/service/hak?user=<user>&name=<name>`

The export's hak (`application/octet-stream`), checked as `export` is.

### `GET /api/service/library?user=<user>`

The workspace's shared texture library hak, the same way; 404 when there is none.

### `GET /api/service/skies?user=<user>`

The workspace's skies hak (`ls_skies.hak`, with its `skyboxes.2da`), the same way; 404 when there is none.
