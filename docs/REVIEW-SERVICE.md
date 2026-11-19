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
  to the builder (for example `{"detail": "the dev server is held by alice"}`), so write it for them. `404` on a single area,
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

An existing area's waypoints, to choose where a test enters from. The editor shows them as they come.

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

## Testing on a dev server (optional)

For a service that runs a dev game server builders can test on, one at a time. The editor shows the **Test** button on any team
server with a review service, and these calls are made only when a builder uses it; a service without a dev server answers them
with an error that says so. The module tested is the builder's last export,
`LANDSCAPER_DEV_MODULE.mod` as `landscaper dev-pack` writes it; how it reaches the dev server is the service's business.

- **`GET /api/v1/dev/test?user=<user>&touch=<true|false>`** (*60 s*): who holds the dev server. `touch=true` from the holder
  counts as activity (the service may release a claim nobody uses).

  ```json
  { "claim": { "user": "alice", "module": "Mymodule_dev", "area": "elmwood", "waypoint": "WP_Gate", "entry": "",
               "expires_in": 5400 },
    "busy": ["deploying"] }
  ```

  `claim` is `null` when nobody holds it; `expires_in` is seconds. `busy` is what the service is doing to the dev server (shown to
  the builder, joined with commas), empty or `null` when nothing.
- **`POST /api/v1/dev/test`** (*60 s*): put the builder's module on the dev server.

  ```json
  { "user": "alice", "module": "Mymodule_dev", "area": "elmwood", "waypoint": "WP_Gate", "entry": "bay1", "takeover": false }
  ```

  The way in is from `waypoint` in the existing `area`, leading to the builder's area `entry` (empty: the first). `takeover` is
  `true` only for an admin taking it from someone. Answer `{ "queued": true, "claim": { ... } }`, or `409` with a `detail` when
  someone else holds it.
- **`POST /api/v1/dev/test/release`** (*60 s*): `{ "user": "alice", "admin": false }` gives it back; an admin may release anyone's.
  Answer `{ "queued": true }`, or `403` with a `detail`.

## Publishing

`POST`, `multipart/form-data`, 15 minutes. With `dry_run` true the service only checks what it was sent and says what the pull
requests would hold; the editor shows that before the builder confirms, then sends the same with `dry_run` false. The service opens
the pull requests; the Landscaper never merges anything. Each zip holds files at the paths they go to.

### `POST /api/v1/areas/publish`

One area. Fields:

| Field | What |
|---|---|
| `user` | The builder |
| `name` | The area's name in the editor |
| `area` | The resref the module will know it by: an existing area's for a rework, otherwise a new one |
| `new` | `true` for a new area, `false` for a rework of `area` |
| `note` | The builder's note for the reviewers, at most 300 characters |
| `dry_run` | `true` or `false` |
| `module_files` (`module.zip`) | The area's `.are.json`, `.git.json` and `.gic.json` in the module's source layout (CONFIGURATION.md, `LANDSCAPER_LAYOUT`), without the builder's hidden mark |
| `hak_files` (`tiles.zip`) | The area's tileset files: models, walkmeshes, textures, minimaps, the `.set` |
| `library_files` (`library.zip`) | Optional: the shared texture library hak's files, for the service to add when the module's haks lack them |
| `blueprint_files` (`blueprints.zip`) | Optional: the builder's own blueprints the area uses |

### `POST /api/v1/areas/publish-module`

Several areas as one set of pull requests. Fields: `user`, `note`, `dry_run`, `areas` (a JSON list of
`{ "name", "area", "new", "kind" }`), then for each area `module_<name>` and `hak_<name>` (zips as above), and the optional
`library_files` and `blueprint_files`. `kind` is the group (the folder inside the areas folder) the area's files go in, as in a
package: the folder the area is already in when the team server has a module checkout, else `LANDSCAPER_AREA_FOLDER` (`core` in
the `folders` layout, empty, for none, in nasher's). A service that knows where a reworked area is should keep it there.

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
