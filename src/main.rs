// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Landscaper: a heightfield becomes an NWN area with a tileset of its own. Each 10 m tile of the area gets a model and walkmesh
//! cut from the same continuous terrain, so neighbouring edges always match.
//!
//! This file runs the command line (cli.rs): the generator is in generator.rs, the editor's server in serve.rs and the
//! research commands in research.rs.

use std::path::PathBuf;

mod assignments;
mod atmosphere;
mod atomic;
mod audit;
mod blueprints;
mod cli;
mod config;
mod devpack;
mod devtest;
mod dialog;
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
mod oracle;
mod paint;
mod pathnode;
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
mod switches;
mod textures;
mod things;
mod update;
mod upload;
mod validate;
mod workspace;

use anyhow::{Context, Result};
use mg_core::ResType;

/// The command line (cli.rs): a mistake there exits with 2 and the usage, a failed command with 1 and what went wrong.
fn main() -> std::process::ExitCode {
    let cli = <cli::Cli as clap::Parser>::parse();
    match run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
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
    config::set_given(config::Given { root: g.root, user: g.user, tlk: g.tlk, jobs: g.jobs.map(|n| n as usize) });
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
        "check {}: {}x{} tiles, routes {}/{}, coverage {:.1}%, {} warnings",
        r["name"].as_str().unwrap_or(""),
        r["tiles"],
        r["tilesY"],
        r["routes"]["ok"],
        r["routes"]["total"],
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
    println!("game: {} (--root, NWN_ROOT)", shown(root, "none found"));
    println!("user folder: {} (--user, NWN_HOME)", shown(user, "none found"));
    let module = if review::active() {
        "from the review service (LANDSCAPER_REVIEW_URL)".to_string()
    } else {
        let repo = config::module_repo();
        let found = if repo.is_dir() { "" } else { ": not there, so there are no areas to import" };
        format!("{} (LANDSCAPER_MODULE_REPO){found}", std::fs::canonicalize(&repo).unwrap_or(repo).display())
    };
    println!("module: {module}");
}
