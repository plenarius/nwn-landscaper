# Running the Landscaper yourself

Two ways: on your own machine, in your browser, or as a team server on your own domain where each builder gets a
workspace. Both need the same ingredients.

## What you need

- **Linux, Windows or macOS.** The Landscaper is developed and tested on Linux; the release downloads (README, "Install") are
  checked on all three. The Docker setup below runs on any Linux host.
- **Neverwinter Nights: Enhanced Edition's files.** The Landscaper reads the game's models, textures, tilesets, tables and its
  lit shaders (it contains none of them). It needs the game itself: the dedicated server's download has no shaders, and export
  stops with a message that says so. The Steam install is found automatically; otherwise set `NWN_ROOT` (or `--root`) to the
  install folder and `NWN_HOME` (or `--user`) to your user folder (the one with `hak/`, `modules/` and `tlk/`).
- **A user folder the Landscaper may write in:** exports go into its `hak/` and `modules/`, so both must be writable by the user
  that runs it. (A team server writes into each person's workspace instead, and can mount the user folder read-only.)
- **Rust 1.98 or newer** (`rustup update`), a C++ compiler and cmake, to build it from source.
- **Your haks and talk table**, if your module uses custom content: put the `.hak` files in `NWN_HOME/hak/` and the custom
  `.tlk` in `NWN_HOME/tlk/` (or name it with `LANDSCAPER_TLK`).
- **`nwn_gff`** from [neverwinter.nim](https://github.com/niv/neverwinter.nim) on your `PATH` (it converts game files to and
  from JSON).
- **The editor's files**: the `nwn-landscaper-web` repository, next to the program (see `LANDSCAPER_WEB`).
- **Optionally, a checkout of your module's source** as JSON files, set with `LANDSCAPER_MODULE_REPO` and laid out as
  `LANDSCAPER_LAYOUT` says: `nasher` (the default: nasher's own layout, everything in `src/`) or `folders` (a folder per kind:
  `areas/<group>/<resref>.are.json`, `core/<ext>/<resref>.<ext>.json`, `core/nss/**`, `module.ifo.json`); other layouts by setting
  the folders ([CONFIGURATION.md](CONFIGURATION.md)). It gives the editor your blueprints, scripts and conversations to pick from
  and your areas to import. Without it you can still build areas, and import areas from a `.mod` or `.erf` you upload (on your
  own machine, also from the modules in your user folder's `modules/`; a team server doesn't list that folder, because it would
  differ with whoever asked first).

All the settings are in [CONFIGURATION.md](CONFIGURATION.md).

## Textures

The editor paints with a texture library, a folder in the data folder. The Landscaper ships none: on the first run with no library
it builds a starter set (about 40 ground, rock, sand, snow, floor and wall textures) from your own game files, so a fresh
install works. To add your own:

```sh
landscaper textures build path/to/pictures --category Grass      # PNG, TGA or DDS; a subfolder is a category
landscaper textures add my-textures.json                         # textures of the game or your haks, by name
landscaper textures list
```

`add` takes `{ "haks": ["my_hak"], "textures": [{ "id": "mossRoad", "name": "Mossy road", "category": "Earth", "surface": 1,
"resref": "my_road_01" }] }`: `id` is at most 13 letters and digits, `resref` a texture of the game or of those haks in your user
folder's `hak/`, `surface` the footsteps row of `surfacemat.2da` (1 dirt, 3 grass, 4 stone, 5 wood, 19 snow, 20 sand). In the
Docker setup, run these inside the container (`docker compose exec landscaper landscaper textures list`). The whole library
is built into `ls_textures.hak` with your exports, so use art you may put in a hak.

In the editor, the Paint tool can also pick a texture up from an imported area's ground (`I`, then click): the tileset's own texture
under the click joins the library as a "Tileset" texture. It needs the tileset's haks in your user folder's `hak/` as any import does.

## On your own machine

The simplest way is a release download (README, "Install"): unpack it and start the program. To build it from source:

```sh
git clone https://github.com/plenarius/nwn-landscaper
git clone https://github.com/plenarius/nwn-landscaper-web
cd nwn-landscaper && cargo build --release    # needs a Rust toolchain, a C++ compiler and cmake
./target/release/landscaper serve --web ../nwn-landscaper-web
```

The engine and the editor change together: check out the same release tag (`vX.Y.Z`) in both, or use the engine's `master`
branch and the editor's `main` branch, and update them together ("Updating", below). An editor older than its engine, or newer,
can be refused on every call (the editor shows "Live: error" and the reason).

It starts by printing what it found:

```text
game: ~/.local/share/Steam/steamapps/common/Neverwinter Nights (--root, NWN_ROOT)
user folder: ~/.local/share/Neverwinter Nights (--user, NWN_HOME)
module: ~/nwn-module (LANDSCAPER_MODULE_REPO)
```

If one is wrong, give it with `--root`, `--user` or `LANDSCAPER_MODULE_REPO`. Open <http://127.0.0.1:8765/>. Draw terrain,
place things, **Export to NWN** to build the area into a module and hak in your NWN user folder, and open it in the toolset or
the game. The first export builds a texture library hak (`ls_textures.hak`); later ones reuse it. **Package for your module…**
(in the file menu) builds a zip of the area's files in your module's source layout to commit.

Nothing leaves your machine. It listens on 127.0.0.1 only; don't expose it without the team setup below.

## As a team server on your domain

Each person signs in through a proxy, gets a workspace of their own, and works in their browser. The Landscaper trusts
headers from the proxy that say who is signed in, so **nothing but the proxy may reach it** (keep it on a private network
or a Docker network, and set `LANDSCAPER_PROXY_SECRET`: the Landscaper refuses requests without it).

`deploy/self-host/` has a working starting point: the Landscaper built from source plus Caddy with basic auth. The image takes
the editor of the same release as the engine you build: the engine and the editor are tagged with the same `vX.Y.Z` at each
release, and the Dockerfile's `WEB_REF` defaults to the engine's own version. Build from a release tag of this repository, or
pass `--build-arg WEB_REF=<tag or branch>` (with `docker compose build`, under `build: args:`) to choose another editor.

To build the engine's `master` with the editor's `main`, uncomment the `args:` line in `compose.yml` (`args: { WEB_REF: main }`),
or run `docker compose build --build-arg WEB_REF=main`.

```sh
cd nwn-landscaper/deploy/self-host
cp compose.example.yml compose.yml && cp Caddyfile.example Caddyfile && cp env.example .env
# edit .env (domain, game folder, admins, a secret), and the Caddyfile (people and password hashes:
#   docker run --rm caddy:2 caddy hash-password)
docker compose build && docker compose up -d
```

The game folder and your haks are mounted read only; the workspaces live in the `landscaper-data` volume (back it up).

### Single sign-on

Basic auth is the simplest thing that works. For your own identity provider, put its forward-auth proxy in front and tell the Landscaper
which headers it sends (`LANDSCAPER_AUTH`: `authentik`, `oauth2-proxy`, `authelia`, or `headers`, with `LANDSCAPER_USER_HEADER` and friends
to override; see CONFIGURATION.md). Who may build and who is an admin comes from groups (`LANDSCAPER_ALLOWED_GROUPS`,
`LANDSCAPER_ADMIN_GROUPS`) or, without groups, from `LANDSCAPER_ALLOWED_GROUPS=*` and `LANDSCAPER_ADMIN_USERS`. `LANDSCAPER_AUTH` and
`LANDSCAPER_ALLOWED_GROUPS` have no default: a team server stops at start without them, and no group is an admin group unless you name it.
Whatever you use, make the proxy **remove** every header below from the browser's request and then set the ones it owns, so a browser can't
send its own. A proxy that passes headers through by default (Caddy, nginx, Traefik) lets a signed-in person add `X-Forwarded-Groups:
Administrator` and become an admin otherwise.

| Preset (`LANDSCAPER_AUTH`) | User | Groups | Display name |
|---|---|---|---|
| `authentik` | `X-authentik-username` | `X-authentik-groups` (split on `\|`) | `X-authentik-name` |
| `oauth2-proxy`, `headers` | `X-Forwarded-User` | `X-Forwarded-Groups` (split on `,`) | `X-Forwarded-Preferred-Username` |
| `authelia` | `Remote-User` | `Remote-Groups` (split on `,`) | `Remote-Name` |

Also strip or replace:

- whatever `LANDSCAPER_USER_HEADER`, `LANDSCAPER_GROUPS_HEADER` and `LANDSCAPER_NAME_HEADER` name, if you set them;
- `X-Landscaper-Proxy`: the proxy sets it to `LANDSCAPER_PROXY_SECRET` (replacing the browser's);
- `X-Forwarded-For`: the activity log records its last address as the person's, so the proxy should set or append it (Caddy does).

The safest is to strip all of the table, whichever preset you use, as `Caddyfile.example` does. If your team has no groups, set
`LANDSCAPER_ADMIN_GROUPS=none` too (the example does): then no groups header can make anyone an admin, only
`LANDSCAPER_ADMIN_USERS`.

Set `LANDSCAPER_PUBLIC_HOST` to the name people reach the server by (the example uses `SITE_ADDRESS`): requests for any other
name are refused.

### What admins and builders can do

Everyone allowed can build, export, test locally and package. Admins can also see everyone's workspace read-only, read the
activity log, and write scripts (a script runs in your game, so it is an admin's job). Builders can write creatures, items and
conversations. A team can limit who may import which of the module's areas (assignments, see TEAM-SERVER.md).

### Publishing

- **No extra service:** **Package for your module…** downloads a zip in the layout of your module's source; a person with
  access to the repository unzips it, builds the haks and commits. The check that stops broken areas runs first.
- **A review service:** with `LANDSCAPER_REVIEW_URL` and `LANDSCAPER_REVIEW_KEY` set, **Publish** sends the same files to a
  service that opens pull requests for review. Layonara runs its own, which isn't public; you would write one to fit your
  repositories. [REVIEW-SERVICE.md](REVIEW-SERVICE.md) describes every call the Landscaper makes to it, and it is optional: the
  package download needs none of it.

## What to expect

- Exports are CPU-heavy: giving the container a few cores helps, and a large area can take a minute or more.
- Areas are 3 to 32 tiles a side. An import of a bigger area, or of one whose tile list doesn't match its size, is refused before
  the game is opened.
- It needs the game's files but not the game running, and no GPU. (Pictures of the module's creatures and items come from a
  review service, if it draws them; [REVIEW-SERVICE.md](REVIEW-SERVICE.md), "Previews".)
- A team server limits uploads (150 MB a file; 20 files and 500 MB a workspace; one at a time per builder, four at once), what
  a workspace holds (100 areas and 5 GB besides its uploads), how slowly a request may arrive, and searches;
  [TEAM-SERVER.md](TEAM-SERVER.md) has the limits and how it answers requests from other sites (the security headers it sends,
  which a proxy in front must not replace).
- **Try on dev** in the Publish dialog (an area on the dev server with no restart) shows only when the review service answers
  its trial calls (REVIEW-SERVICE.md, "Trying areas on a dev server"); the line telling a builder how to join is
  `LANDSCAPER_DEV_JOIN`. New scripts can't be tried on dev: they go through Publish.
- Areas, blueprints and scripts are read from the module checkout's layout and a package uses it too (CONFIGURATION.md).
  Packages are checked in both presets, `nasher` (the default) and `folders` (`tests/package_layouts.rs`); a hand-set layout
  was checked for reading and importing, not for building a package.

## Updating

- **Docker:** check out the new release tag of this repository and run `docker compose build && docker compose up -d`; the image
  takes the editor of the same tag (`WEB_REF`). Pulling a checkout of nwn-landscaper-web beside it changes nothing unless you copy
  it into the repository's root as `./landscaper-web`, which the build then uses instead.
- **From source:** `git pull` both repositories (or check out the same `vX.Y.Z` in both) and `cargo build --release`.

Workspaces are plain files in the data folder (in Docker, the data volume) and are kept. CHANGELOG.md says what changed, and
what an existing setup has to change with it.
