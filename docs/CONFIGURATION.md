# Configuration

The Landscaper runs on a fresh machine with no setup beyond the game's files; everything below is optional and read from the
environment. (`src/config.rs` is the source of truth.)

The command line's options win over the environment. `landscaper --help` lists the commands; the options below work before or
after any command (the same names as nwn-bake's options):

| Option | What | Default |
|---|---|---|
| `--root DIR` | The game install | `NWN_ROOT`, then where the game is usually installed |
| `--user DIR` | The game's user folder (`hak/`, `tlk/`, `modules/`) | `NWN_HOME`, then the game's own rule |
| `--tlk TLK` | Your module's custom talk table: a file name in the user folder's `tlk/` or `hak/`, or a path | `LANDSCAPER_TLK` |
| `--haks HAKS` | The haks the editor's placeables come from: hak names in your module's order, comma separated, or `module` | `LANDSCAPER_HAKS` |
| `--jobs N` | How many threads an import, an export or a check works on | `LANDSCAPER_JOBS` |

| Variable | What | Default |
|---|---|---|
| `NWN_ROOT`, `NWN_HOME` | The game install and the user folder (haks, modules, tlk). A script check compiles against the install's `ovr/nwscript.nss`, so a module whose scripts need functions added to it (NWNX, say) needs that file there. The install must be the game itself, not the dedicated server's download: the shared hak's tile shaders are made from the game's own `fslit`, `vslit` and `inc_lighting` shaders, which the server download lacks | found like the game finds them |
| `XDG_DATA_HOME` | Where the Landscaper keeps its data (workspaces, exports, the texture library) | `~/.local/share` (folder `landscaper`); on Windows `%LOCALAPPDATA%` |
| `LANDSCAPER_WEB` (or `--web`) | The editor's files (the `nwn-landscaper-web` repository) | `web/` beside the program, `./nwn-landscaper-web`, `./landscaper-web`, `../nwn-landscaper-web`, `../landscaper-web` |
| `LANDSCAPER_MODULE_REPO` | A checkout of your module's source as JSON (what `nwn_gff` or nasher write): areas, blueprints, conversations, scripts and `module.ifo.json` | `./nwn-module` |
| `LANDSCAPER_LAYOUT` | Where those files are in the checkout: `nasher` (nasher's own layout: everything in `src/`, `src/module.ifo.json`) or `folders` (a folder per kind: `areas/<group>/`, `core/<ext>/`, `core/nss/`, `module.ifo.json` at the root; new areas in `areas/core/`). A team server refuses any other value at start | `nasher` |
| `LANDSCAPER_AREAS_DIR`, `LANDSCAPER_BLUEPRINTS_DIR` (with `{ext}`: `utc`, `uti`, `dlg`, ...), `LANDSCAPER_SCRIPTS_DIR`, `LANDSCAPER_IFO`, `LANDSCAPER_AREA_FOLDER` | Override one place of the layout; `LANDSCAPER_AREA_FOLDER` is the group a new area's file goes in (inside the areas folder; empty for none) | from the layout |
| `LANDSCAPER_AREA_TEMPLATE` | A JSON file of what a new area starts with (scripts, PvP, weather, music, ...); see `src/area_template.json` | a plain template |
| `LANDSCAPER_TLK` (or `--tlk`) | Your module's custom talk table: a file name in the user folder's `tlk/` or `hak/`, or a path | the only `.tlk` there, when there is exactly one |
| `LANDSCAPER_HAKS` (or `--haks`) | The haks the editor reads the game with: the Place and Scatter panels offer their placeables (placeables.2da's rows, from the haks' table when one has it), and their models, textures and blueprints are drawn, baked and placed from them. Hak names in your module's order, first highest, comma separated (`my_plc,my_tex`; `.hak` may be added), found in the user folder's `hak/`; or `module` for the `Mod_HakList` of the checkout's `module.ifo.json` (`LANDSCAPER_MODULE_REPO`). An export lists these haks in the module it builds, and adds the ones a module lacks when it installs an area in it, after the module's own: what was placed from them is there in the game only with them. The game with a module's haks opens once, when the editor first needs it (see "The editor's haks" below) | on a team server with a review service, the module's hak list from it (`GET /api/v1/areas/module`); otherwise none: the game's own placeables |
| `LANDSCAPER_PROJECT` | What your module is called in messages | "the module" |
| `LANDSCAPER_JOBS` (or `--jobs N`) | How many threads an import, an export or a check works on. What they make is the same whatever it is; fewer threads leave more of a shared server to everything else, more make them quicker. A team server started with it keeps that limit for every job | every core for an export; half the machine's cores (at least 1) for an import or a check |
| `LANDSCAPER_GAME` | `none`: the install the engine is given holds none of the game's files (the web demo's stand-in). `/api/me` answers `"game": false`, and the editor's Place, Scatter, Things, Scripts and Import say they need the game | the game's files are there |
| `LANDSCAPER_PARTICLES` | Another particle pack's folder (its `pack.json`, `models/` and textures): the kinds an area's `ambience` placements bake into its tiles. A pack that fails its checks makes an export skip them and say why | the pack built into the program (`packs/particles`) |
| `LANDSCAPER_LOG` | Set to anything: print every request (address, method, path, browser) to stderr | off |
| `LS_*` | Debugging switches; none is needed. Some change what an export writes, so the program names the ones that are set at start. [DEBUGGING.md](DEBUGGING.md) lists them | off |

At start `landscaper serve` prints the game install, the user folder and the module it found, and where each came from, so a wrong
guess shows at once.

## The editor's haks

The Landscaper opens the game once, when the editor first asks for a placeable, a model or a blueprint, and prints the haks it
opened it with and any it couldn't find in the user folder's `hak/` or the game's `data/hk/`. A team server with a review service
asks the service for the module's hak list; when the service doesn't answer then, the editor offers the game's own placeables
and asks again on later requests until it does. A module that adds a hak later is read with it after the Landscaper restarts.

The haks are mapped, not read whole, so their size costs little: with a module's 57 haks (16 GB, placeables.2da of 21,201 rows)
the first list of placeables took 0.16 s against 0.10 s for the game alone, and the server held 108 MB against 76 MB.

## Textures

The library lives in `textures/` in the data folder (`XDG_DATA_HOME`). It is built from the game's files the first time there is none;
`landscaper textures init [--pictures <folder>] | add <list.json> | build <folder> [--category NAME] [--surface N] | list`
fill it otherwise (see SELF-HOSTING.md, "Textures"). Texture packs add to it ([TEXTURE-PACKS.md](TEXTURE-PACKS.md)):

| Variable | What it is | Default |
|----------|------------|---------|
| `LANDSCAPER_PACKS_URL` | The list of texture and sky packs on offer (a URL or a file); a pack's `url` in it may be relative to it | the `texture-packs` release of nwn-landscaper on GitHub |
| `LANDSCAPER_PACK_MAX_MB` | The largest pack downloaded, in MB | 512 |

## Running it for yourself

```sh
landscaper serve --port 8765 --web path/to/nwn-landscaper-web
```

and open <http://127.0.0.1:8765/> in a browser. Exports go into the game's user folder (`modules/`, `hak/`), so both must be
writable. `--port` (default 8765) and `--bind` (default `127.0.0.1`) say where it listens. Without `--team` every request is you,
with no sign-in, so an address other machines can reach (`--bind 0.0.0.0`) is refused unless you also pass `--allow-remote`;
for other people, run a team server.

`--reload-on-rebuild` is for working on the Landscaper itself: after each request, if the `landscaper` program file has
changed since the server started (a new build), the server restarts on the new one. Off by default, so installing an
update never restarts a session in the middle of an edit.

## Running it for a team

`landscaper serve --team --bind 0.0.0.0 --port 8766` behind a reverse proxy that signs people in and passes who they are in
headers (see TEAM-SERVER.md). Each person gets a workspace of their own. `LANDSCAPER_TEAM` set to anything does what `--team`
does (the container images set it); `--bind 0.0.0.0` listens on every address, as a container needs. Settings for the team layer:

| Variable | What |
|---|---|
| `LANDSCAPER_AUTH` | Which headers say who is signed in: `authentik`, `oauth2-proxy`, `authelia`, or `headers` (the oauth2-proxy names, for any forward-auth setup). **Required:** a team server stops at start without it |
| `LANDSCAPER_USER_HEADER`, `LANDSCAPER_GROUPS_HEADER`, `LANDSCAPER_NAME_HEADER`, `LANDSCAPER_GROUPS_SEPARATOR` | Override a header name, or how the groups header is split (`,` `\|` or `space`) |
| `LANDSCAPER_ADMIN_GROUPS`, `LANDSCAPER_ADMIN_USERS` | Who may do everything (groups / user names, comma separated). Unset, `none` or empty `LANDSCAPER_ADMIN_GROUPS` makes no group an admin, only the users named; with neither set there is no admin (the server says so at start) |
| `LANDSCAPER_ALLOWED_GROUPS` | Who may use it (groups, comma separated); `*` is everyone the proxy signs in (a team without groups). **Required:** a team server stops at start without it |
| `LANDSCAPER_PROXY_SECRET` | A secret the proxy sends in `X-Landscaper-Proxy` so nobody can reach the server around it. Required: at least 16 characters, not `change-me` (e.g. `openssl rand -hex 24`) |
| `LANDSCAPER_INSECURE` | `1` (or `--insecure`): run a team server without `LANDSCAPER_PROXY_SECRET`, only where nothing but the proxy can reach it |
| `LANDSCAPER_PUBLIC_HOST` | The name people reach the server by, as the browser sends it in `Host` (e.g. `landscaper.example.com`; several comma separated; a name without a port matches any port). Requests for any other name are refused (DNS rebinding). Unset on a team server: any name is answered, with a warning at start. On your own machine `127.0.0.1`, `localhost` and `[::1]` with the port always work; set it only to reach the editor from another machine on purpose |
| `LANDSCAPER_REVIEW_URL`, `LANDSCAPER_REVIEW_KEY` | A review service that holds your module and opens review pull requests (you write your own; [REVIEW-SERVICE.md](REVIEW-SERVICE.md) is every call it must answer); without it a team server reads `LANDSCAPER_MODULE_REPO` and cannot publish. A team server stops at start when one of the two is set and the other empty |
| `LANDSCAPER_SERVICE_KEY` | Optional: lets a trusted service list and fetch builders' exports through the read-only `/api/service/*` routes ([REVIEW-SERVICE.md](REVIEW-SERVICE.md#fetching-exports)). At least 24 characters; unset, the routes answer 404 |
| `LANDSCAPER_REVIEWERS` | Who reviews published areas, as the editor names them ("Nothing is merged; *the builders' leads* review the pull requests"). Default: "your reviewers" |
| `LANDSCAPER_DEV_MODULE`, `LANDSCAPER_DEV_JOIN` | For a dev server: the module file `landscaper dev-pack` writes (default `Landscaper_dev`, giving `Landscaper_dev.mod`; letters, digits and `_ . -`), and how the editor's Try on dev tells a builder to join (for example `dev.example.org port 5121`; nothing is said when unset) |
| `LANDSCAPER_GITEA_URL`, `LANDSCAPER_GITEA_TOKEN`, `LANDSCAPER_GITEA_REPO` | Optional: keep saved versions of areas in a Gitea repository |
| `LANDSCAPER_NOREPLY_DOMAIN` | The domain of the commit address for saved versions (default `users.noreply.invalid`) |
| `LANDSCAPER_DISCORD_WEBHOOK` | Optional: a Discord webhook told when a builder imports an existing area, tries one that isn't theirs, or hits the search limit |
| `LANDSCAPER_SEARCH_PER_MIN`, `LANDSCAPER_SEARCH_PER_DAY` | How many searches a builder may make (default 30 a minute, 300 a day); admins are not limited |

Presets send: `authentik` `X-authentik-username` / `-groups` (split on `|`) / `-name`; `oauth2-proxy` and `headers`
`X-Forwarded-User` / `X-Forwarded-Groups` (split on `,`) / `X-Forwarded-Preferred-Username`; `authelia` `Remote-User` /
`Remote-Groups` / `Remote-Name`. Whatever the proxy sends, strip those headers from requests coming from outside (SELF-HOSTING.md
lists them), and set `LANDSCAPER_PROXY_SECRET` so the server only answers requests that came through it.

Every API request must carry the header `X-Landscaper: 1` (the editor sends it; see TEAM-SERVER.md, "Requests from other
sites"), so a script that calls the API sends it too.

A small team with no groups: `LANDSCAPER_AUTH=headers LANDSCAPER_ALLOWED_GROUPS='*' LANDSCAPER_ADMIN_USERS=alice,bob`.
