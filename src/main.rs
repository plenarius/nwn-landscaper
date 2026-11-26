// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Landscaper: a heightfield becomes an NWN area with a tileset of its own. Each 10 m tile of the area gets a model and walkmesh
//! cut from the same continuous terrain, so neighbouring edges always match.
//!
//! This file runs the command line (cli.rs): the generator is in generator.rs, the editor's server in serve.rs and the
//! research commands in research.rs.

use std::path::PathBuf;

mod ambience;
mod assignments;
mod atmosphere;
mod atomic;
mod audit;
mod blueprints;
mod cli;
mod config;
mod dds;
mod devpack;
mod devtrial;
mod dialog;
mod export_id;
mod generator;
mod gitea;
mod heightmap;
mod import;
mod itemprops;
mod marker;
mod mincut;
mod models;
mod newbp;
mod newscript;
mod objects;
mod occluders;
mod oracle;
mod packs;
mod paint;
mod pathnode;
mod plants;
mod publish;
mod quota;
mod records;
mod remote;
mod research;
mod review;
mod rtin;
mod seams;
mod serve;
mod settings;
mod shade;
mod shared;
mod sim;
mod skies;
mod switches;
mod textures;
mod things;
mod triplanar;
mod update;
mod upload;
mod validate;
mod version;
mod workspace;

use anyhow::{Context, Result};
use mg_core::ResType;

/// This process's id, for naming temporary files: WASI (the browser build) has none, and one page runs one engine.
pub(crate) fn pid() -> u32 {
    #[cfg(target_family = "wasm")]
    return 1;
    #[cfg(not(target_family = "wasm"))]
    std::process::id()
}

/// The command line (cli.rs): a mistake there exits with 2 and the usage, a failed command with 1 and what went wrong. With no
/// arguments at all, as a double-click starts it: the editor on this machine ([`quick_start`]).
fn main() -> std::process::ExitCode {
    if std::env::args_os().len() <= 1 {
        return quick_start();
    }
    // The browser build: one API request, its answer (`serve::wasi_request`). Not a clap command: it is the page's, not a builder's.
    #[cfg(feature = "web")]
    if std::env::args().nth(1).as_deref() == Some("wasi-request") {
        let files: Vec<PathBuf> = std::env::args_os().skip(2).map(PathBuf::from).collect();
        return match serve::wasi_request(files.first().map(PathBuf::as_path), files.get(1).map(PathBuf::as_path)) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let cli = <cli::Cli as clap::Parser>::parse();
    match run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Started with no arguments (a double-click on the program): `serve` on 127.0.0.1:8765 (or the next free port when another
/// program has it) with the editor in `web/` beside the program, the browser opened at it. The window stays open while it serves; when it can't start, it says why and waits for
/// Enter, so the message can be read before the window closes. Without a game install it stops and says what to set.
fn quick_start() -> std::process::ExitCode {
    println!("NWN Landscaper {}: the editor on this machine. (`landscaper --help` lists the commands.)\n", version::text());
    let mut args = vec!["landscaper".into(), "serve".into()];
    if let Some(user) = onedrive_user_folder() {
        args.extend(["--user".into(), user.into_os_string()]);
    }
    let given = <cli::Cli as clap::Parser>::parse_from(&args);
    config::set_given(config::Given { root: None, user: given.game.user.clone(), tlk: None, haks: None, jobs: None });
    let outcome = if models::game_folders().0.is_none() {
        say_folders();
        Err(anyhow::anyhow!("the Landscaper needs the game's files: set NWN_ROOT as above, then start it again"))
    } else {
        match quick_port() {
            QuickPort::Running(port) => {
                // The Landscaper started earlier and still running in another window: show its editor.
                println!("The Landscaper is already running on port {port}: its editor opens now.");
                serve::open_browser(&format!("http://127.0.0.1:{port}/"));
                Ok(())
            }
            QuickPort::Free(port) => {
                if port != QUICK_PORTS.start {
                    println!("Another program is using port {}, so the editor takes port {port}.", QUICK_PORTS.start);
                }
                args.extend(["--port".into(), port.to_string().into()]);
                serve::OPEN_BROWSER.store(true, std::sync::atomic::Ordering::Relaxed);
                run(<cli::Cli as clap::Parser>::parse_from(args))
            }
            QuickPort::None => Err(anyhow::anyhow!(
                "ports {} to {} are all taken by other programs: start it from a Command Prompt or terminal with \
                 `landscaper serve --port <a free port>`",
                QUICK_PORTS.start,
                QUICK_PORTS.end - 1
            )),
        }
    };
    if let Err(e) = &outcome {
        eprintln!("\nerror: {e:#}");
    }
    println!("\nPress Enter to close this window.");
    let _ = std::io::stdin().read_line(&mut String::new());
    if outcome.is_ok() { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
}

/// The ports a double-click tries, in order: other local tools also like 8765.
const QUICK_PORTS: std::ops::Range<u16> = 8765..8785;

enum QuickPort {
    /// A Landscaper started before answers there.
    Running(u16),
    /// Nothing listens there.
    Free(u16),
    None,
}

/// Where a double-click's editor is: a Landscaper already running on one of [`QUICK_PORTS`], else the first free one. A port
/// another program holds is passed over.
fn quick_port() -> QuickPort {
    for port in QUICK_PORTS {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return QuickPort::Free(port);
        }
        if is_landscaper(port) {
            return QuickPort::Running(port);
        }
    }
    QuickPort::None
}

/// Whether what listens on `port` is the Landscaper: its `/healthz` answers with an `X-Landscaper` header.
fn is_landscaper(port: u16) -> bool {
    use std::io::{Read, Write};
    let ask = || -> std::io::Result<String> {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let mut s = std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(1))?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        s.set_write_timeout(Some(std::time::Duration::from_secs(1)))?;
        s.write_all(format!("GET /healthz HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())?;
        let mut head = Vec::new();
        // (What came before a reset or a timeout still counts.)
        let _ = s.take(4096).read_to_end(&mut head);
        Ok(String::from_utf8_lossy(&head).into_owned())
    };
    ask().is_ok_and(|r| r.lines().take_while(|l| !l.trim().is_empty()).any(|l| l.to_ascii_lowercase().starts_with("x-landscaper:")))
}

/// On Windows, `Documents\Neverwinter Nights` in OneDrive (`%OneDrive%`), where Windows moves the Documents folder when OneDrive
/// backs it up: when the user folder isn't given (`NWN_HOME`, `NWN_USER_DIRECTORY`) and isn't in the usual place.
fn onedrive_user_folder() -> Option<PathBuf> {
    if !cfg!(windows) || mg_resman::GameInstall::detect_user_dir().is_some() {
        return None;
    }
    let folder = PathBuf::from(std::env::var_os("OneDrive")?).join("Documents").join("Neverwinter Nights");
    folder.is_dir().then_some(folder)
}

/// What to set when the game or the user folder isn't found: the variable, and how to set it on this system.
fn not_found_help(what: &str, var: &str, example: &str) -> String {
    let set = if cfg!(windows) {
        format!(
            "In a Command Prompt run\n    setx {var} \"{example}\"\n  then start the Landscaper again (or start it from a              Command Prompt with --{} \"<folder>\").",
            if var == "NWN_ROOT" { "root" } else { "user" }
        )
    } else {
        let profile = if cfg!(target_os = "macos") { "~/.zshrc" } else { "~/.profile" };
        format!("Add this line to {profile}, then open a new terminal and start the Landscaper again:\n    export {var}=\"{example}\"")
    };
    format!("  {what}: set {var} to it. {set}")
}

/// The debugging switches that are set, said at start: some change what an export writes (docs/DEBUGGING.md).
fn say_switches() {
    let set: Vec<String> = std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()).filter(|k| k.starts_with("LS_")).collect();
    for line in switches::describe_set(&set) {
        eprintln!("{line}");
    }
}

fn run(cli: cli::Cli) -> Result<()> {
    use cli::Command as C;
    let g = cli.game;
    config::set_given(config::Given { root: g.root, user: g.user, tlk: g.tlk, haks: g.haks, jobs: g.jobs.map(|n| n as usize) });
    say_switches();
    if let Some(p) = std::env::var_os("LS_HEIGHTMAP") {
        heightmap::load(std::path::Path::new(&p))?;
    }
    match cli.command {
        C::Serve(s) => serve_command(s)?,
        C::Import { source, name, dir } => import::run(&source, &name, dir.as_deref())?,
        C::Export { area } => {
            generator::export(&area, &generator::Before::default())?;
        }
        C::Check { area, json } => {
            let report = generator::check(&area)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_check(&report);
            }
        }
        C::Textures(t) => textures::run(t)?,
        C::Skies(s) => skies::run(s)?,
        C::DevPack { base, workspace, hak_dir, area, waypoint, out_dir, entry_area } => {
            let report = devpack::pack(&base, &workspace, &hak_dir, &area, &waypoint, entry_area.as_deref(), &out_dir)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        C::Marker { id } => {
            let id = id.trim().to_ascii_lowercase();
            anyhow::ensure!(
                id.len() == 12 && id.chars().all(|c| c.is_ascii_hexdigit()),
                "a mark is 12 hexadecimal digits, such as 3f9a0c12b7de"
            );
            match marker::who(&id).with_context(|| marker::file().display().to_string())? {
                Some(who) => {
                    let (user, area) = who.split_once('/').unwrap_or((&who, ""));
                    println!("{id}: exported by {user}, area {area}");
                }
                None => anyhow::bail!("no export on this server has the mark {id} (the marks are in {})", marker::file().display()),
            }
        }
        C::Generate { out } => {
            let out = out.unwrap_or_else(|| PathBuf::from("out"));
            std::fs::create_dir_all(&out)?;
            generator::generate(&out, None)?;
        }
        C::Decompile { model } => {
            let game = models::open_game(nwn_bake::game::Haks::None, nwn_bake::game::Tlk::None)?;
            let data = game.resman.get_named(&model, ResType::MDL)?;
            print!("{}", mg_mdl::decompile(&data)?);
        }
        C::Shaders { out } => shared::write_shaders(&out)?,
        C::CheckTiles { limit } => research::check_tiles(limit.unwrap_or(200_000))?,
        C::ValidatePathnodes { tileset } => research::validate_pathnodes(tileset.as_deref().unwrap_or("ttr01"))?,
        C::WokStats { tilesets } => research::wok_stats(&tilesets)?,
        C::Oracle { out, n, seed } => research::oracle_run(n.unwrap_or(40), seed.unwrap_or(1), &out)?,
        C::Sim { x0, y0, x1, y1 } => research::sim_one([x0, y0, x1, y1])?,
        C::SimLine { x0, y0, x1, y1 } => research::sim_line(x0, y0, x1, y1),
        C::SimCheck { files } => {
            for p in &files {
                research::sim_check(p)?;
            }
        }
        C::SimPredict { n, seeds } => {
            research::sim_predict(n.unwrap_or(40), 1..=seeds.unwrap_or(5), std::env::var_os("LS_VERBOSE").is_some())?;
        }
    }
    Ok(())
}

/// `landscaper serve`: says which game, user folder and module it found, checks a team server's settings, then serves.
/// `check`'s report for people: one line, then each warning with where it is.
fn print_check(r: &serde_json::Value) {
    let point = |v: &serde_json::Value| format!("({:.1}, {:.1})", v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0));
    let warnings = r["warnings"].as_array().map_or(&[][..], Vec::as_slice);
    println!(
        "check {}: {}x{} tiles, routes {}/{}{}, coverage {:.1}%, {} warnings",
        r["name"].as_str().unwrap_or(""),
        r["tiles"],
        r["tilesY"],
        r["routes"]["ok"],
        r["routes"]["total"],
        generator::long_note(r["routes"]["long"].as_u64().unwrap_or(0) as usize),
        100.0 * r["coverage"].as_f64().unwrap_or(0.0),
        warnings.len()
    );
    for w in warnings {
        let place = if w["at"].is_array() {
            format!("at {}", point(&w["at"]))
        } else if w["from"].is_array() {
            format!("{} -> {}", point(&w["from"]), point(&w["to"]))
        } else {
            format!("tile {},{}", w["tile"][0], w["tile"][1])
        };
        println!("  {} {place}: {}", w["kind"].as_str().unwrap_or("warning"), w["why"].as_str().unwrap_or(""));
    }
}

fn serve_command(s: cli::Serve) -> Result<()> {
    let web = s.web.or_else(|| std::env::var_os("LANDSCAPER_WEB").map(PathBuf::from)).map_or_else(config::web_dir, Ok)?;
    // A team server: sign-in from the proxy's headers, a workspace for each person (see workspace.rs). --bind 0.0.0.0 for a container.
    if s.team || std::env::var_os("LANDSCAPER_TEAM").is_some() {
        workspace::enable_team();
        // A setting left unresolved or half set up would start fine and then fail quietly (an empty area list): say so and stop.
        let env = |k: &str| std::env::var(k).ok();
        let mut problems = config::team_problems(&env);
        problems.extend(config::setting_problems(&env));
        // No proxy secret (or a guessable one) lets anything that reaches the server say who it is: refused unless
        // --insecure says nothing but the proxy can reach it.
        let insecure = s.insecure || std::env::var("LANDSCAPER_INSECURE").is_ok_and(|v| v.trim() == "1");
        problems.extend(workspace::proxy_secret_problem(config::AuthConfig::from_env().proxy_secret.as_deref(), insecure));
        if !problems.is_empty() {
            anyhow::bail!("the settings are wrong:\n  {}", problems.join("\n  "));
        }
        let auth = config::AuthConfig::from_env();
        if auth.admin_groups.is_empty() && auth.admin_users.is_empty() {
            eprintln!("note: no admins (LANDSCAPER_ADMIN_GROUPS and LANDSCAPER_ADMIN_USERS are both unset): nobody can assign areas");
        }
    }
    if !workspace::team() {
        match (cli::loopback(&s.bind), s.allow_remote) {
            (true, _) => {}
            (false, false) => anyhow::bail!(
                "--bind {} lets other machines reach the editor, and without --team every request is you, an admin, with no sign-in \
                 (scripts, imports, exports into your modules). Use --team behind a sign-in proxy (docs/TEAM-SERVER.md), or pass \
                 --allow-remote if everyone who can reach this address may do anything you can",
                s.bind
            ),
            (false, true) => eprintln!(
                "WARNING: listening on {} with no sign-in (--allow-remote): anyone who can reach it can write scripts, import, \
                 export into your modules and publish as you",
                s.bind
            ),
        }
    }
    say_folders();
    // The first run on a machine: no texture library yet, so build the starter set from the game's files.
    textures::ensure_starter();
    serve::serve(&s.bind, s.port, &web, s.reload_on_rebuild)
}

/// The start line's first part: which game install, user folder and module the server found, so a wrong guess shows at once.
fn say_folders() {
    let shown = |p: Option<PathBuf>, none: &str| p.map_or_else(|| none.to_string(), |p| p.display().to_string());
    let (root, user) = models::game_folders();
    let (no_root, no_user) = (root.is_none(), user.is_none());
    println!("game: {} (--root, NWN_ROOT)", shown(root, "none found"));
    if no_root {
        // NWN_ROOT given but not the game: say what is wrong with it.
        if let Some(given) = std::env::var_os("NWN_ROOT").map(PathBuf::from) {
            let why = mg_resman::GameInstall::why_not(&given).unwrap_or_default();
            println!("  NWN_ROOT is {}, which isn't the game's folder: {why}", given.display());
        }
        let example = match std::env::consts::OS {
            "windows" => r"C:\Program Files (x86)\Steam\steamapps\common\Neverwinter Nights",
            "macos" => "/Users/you/Library/Application Support/Steam/steamapps/common/Neverwinter Nights",
            _ => "/home/you/.local/share/Steam/steamapps/common/Neverwinter Nights",
        };
        println!(
            "  Looked in Steam's libraries, GOG's and Beamdog's usual folders. The game's folder is the one with data{}nwn_base.key in it.",
            std::path::MAIN_SEPARATOR
        );
        println!("{}", not_found_help("Wherever yours is", "NWN_ROOT", example));
    }
    println!("user folder: {} (--user, NWN_HOME)", shown(user, "none found"));
    if no_user {
        let example = match std::env::consts::OS {
            "windows" => r"C:\Users\you\Documents\Neverwinter Nights",
            "macos" => "/Users/you/Documents/Neverwinter Nights",
            _ => "/home/you/.local/share/Neverwinter Nights",
        };
        println!(
            "  The user folder (with hak, modules and tlk in it) is made when the game first starts; exports go into it. \
             Start the game once, or if yours is elsewhere:"
        );
        println!("{}", not_found_help("The user folder", "NWN_HOME", example));
    }
    let module = if review::active() {
        "from the review service (LANDSCAPER_REVIEW_URL)".to_string()
    } else {
        let repo = config::module_repo();
        let found = if repo.is_dir() { "" } else { ": not there, so there are no areas to import" };
        format!("{} (LANDSCAPER_MODULE_REPO){found}", std::fs::canonicalize(&repo).unwrap_or(repo).display())
    };
    println!("module: {module}");
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    /// A one-request server on a free port that answers with `reply`.
    fn answering(reply: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut asked = Vec::new();
            let mut buf = [0; 256];
            while !asked.ends_with(b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => asked.extend_from_slice(&buf[..n]),
                }
            }
            let _ = s.write_all(reply.as_bytes());
        });
        port
    }

    #[test]
    fn a_second_double_click_opens_only_a_landscaper_and_passes_over_other_programs() {
        assert!(super::is_landscaper(answering("HTTP/1.1 200 OK\r\nX-Landscaper: 0.2.2\r\nContent-Length: 2\r\n\r\nok")));
        // Another local tool on the port (a dashboard, or one with its own /healthz).
        assert!(!super::is_landscaper(answering("HTTP/1.0 404 Not Found\r\nServer: SimpleHTTP/0.6\r\n\r\nnope")));
        assert!(!super::is_landscaper(answering("HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\nX-Landscaper: 1")));
        // Nothing listening.
        let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        assert!(!super::is_landscaper(free));
    }
}
