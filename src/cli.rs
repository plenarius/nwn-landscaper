// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line: the commands, their options and `--help`. One usage text, the commands people use first; the research
//! commands (for working on the Landscaper itself) are listed after them, under their own heading. A mistake on the command line
//! (an unknown command, a missing argument, `--port abc`) is reported with the usage and exit status 2; a command that fails
//! exits with 1.
//!
//! The game options are `--root`, `--user`, `--tlk`, `--haks` and `--jobs` (nwn-bake's names, `--haks` aside). Each goes before or
//! after the command, and wins over its environment variable (docs/CONFIGURATION.md).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The research commands, listed after the others in `--help` (they are hidden from clap's own list, which can't group them).
const RESEARCH: &str = "\
Research commands (for working on the Landscaper itself; they print what they find):
  generate [OUT]                       Generate the test area into OUT (default: out); also writes landscaper.hak and
                                       ls_textures.hak into the user folder's hak/
  decompile <MODEL>                    Print a game model as ASCII MDL
  shaders <OUT>                        Write the shared hak's three shaders, as built from the game's, into OUT
  check-tiles [LIMIT]                  Check every tile's walkmesh against the planner's work limit (default 200000)
  validate-pathnodes [TILESET]         Compare the path node table with a tileset's tiles (default ttr01)
  wok-stats <TILESET>...               Count walkmesh faces per tile of game tilesets
  oracle --out <DIR> [N] [SEED]        Run N random routes on a real server (needs `mg`) and record what it did
  sim <X0> <Y0> <X1> <Y1>              Simulate the server's route planner on the test area
  sim-line <X0> <Y0> <X1> <Y1>         Print walkmesh coverage along a line, every 0.25 m
  sim-check <ORACLE.JSON>...           Compare the simulator with an oracle run's results
  sim-predict [N] [SEEDS]              Predict how N random routes go, for SEEDS seeds

Debugging switches (LS_*) are listed in docs/DEBUGGING.md; the ones that are set are printed at start.";

/// Paint terrain, get a Neverwinter Nights area with its own generated tileset.
#[derive(Parser, Debug)]
#[command(name = "landscaper", version = crate::version::text(), about, after_help = RESEARCH, subcommand_required = true, arg_required_else_help = true)]
pub struct Cli {
    /// Where the game is and how many threads to use, before or after any command.
    #[command(flatten)]
    pub game: GameArgs,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Where the game is, and how many threads to use: as nwn-bake takes them.
#[derive(Args, Debug, Default, Clone)]
pub struct GameArgs {
    /// The game install [default: NWN_ROOT, then the usual places]
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// The game's user folder (hak/, tlk/, modules/) [default: NWN_HOME, then the game's own rule]
    #[arg(long, global = true, value_name = "DIR")]
    pub user: Option<PathBuf>,
    /// The module's custom talk table: a file name in the user folder's tlk/ or hak/, or a path [default: LANDSCAPER_TLK, then the
    /// one .tlk in the user folder]
    #[arg(long, global = true, value_name = "TLK")]
    pub tlk: Option<String>,
    /// The haks the editor's placeables, models, blueprints and textures come from, in the module's order (comma separated), or
    /// `module` for the module checkout's list [default: LANDSCAPER_HAKS, then the review service's module haks on a team server]
    #[arg(long, global = true, value_name = "HAKS")]
    pub haks: Option<String>,
    /// Threads an import, an export or a check works on [default: LANDSCAPER_JOBS, then every core for an export and half the
    /// cores for the others]
    #[arg(long, global = true, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub jobs: Option<u32>,
}

/// The commands, as `landscaper --help` lists them.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the editor: open the address it prints in a browser
    Serve(Serve),
    /// Import an existing area into the editor
    Import {
        /// The area: an area file (.are, or nwn_gff's .are.json, its .git beside it) or <module.mod>:<area resref>
        source: String,
        /// The name the editor shows it by
        name: String,
        /// Where the import goes, and where `export <DIR>/<NAME>.json` finds it [default: the exports folder in the data folder]
        dir: Option<PathBuf>,
    },
    /// Export a saved area (its .json): its hak into the user folder's hak/, its module into modules/
    Export {
        /// The area's file, as the editor saves it
        area: PathBuf,
    },
    /// Check a saved area without exporting it: a summary and its warnings (the whole report with --json)
    Check {
        /// The area's file, as the editor saves it
        area: PathBuf,
        /// Print the whole report as JSON instead
        #[arg(long)]
        json: bool,
    },
    /// Fill the texture library
    #[command(subcommand)]
    Textures(Textures),
    /// Make the dev server's test module from the production module and a builder's export
    DevPack {
        /// The production module (.mod)
        base: PathBuf,
        /// The builder's exported module (.mod)
        workspace: PathBuf,
        /// The folder of the builder's haks
        hak_dir: PathBuf,
        /// The existing area the test enters from
        area: String,
        /// The waypoint in that area
        waypoint: String,
        /// Where the test module goes
        out_dir: PathBuf,
        /// The builder's area the way in leads to (a resref or a tileset name) [default: the first]
        entry_area: Option<String>,
    },
    /// Say whose export carries a hidden mark (a team server's `ls_ref` variable, or `ref:<id>` in an area's comments)
    Marker {
        /// The mark: 12 hexadecimal digits
        id: String,
    },

    // The research commands: listed in RESEARCH above.
    #[command(hide = true)]
    Generate { out: Option<PathBuf> },
    #[command(hide = true)]
    Decompile { model: String },
    #[command(hide = true)]
    Shaders { out: PathBuf },
    #[command(hide = true)]
    CheckTiles { limit: Option<i64> },
    #[command(hide = true)]
    ValidatePathnodes { tileset: Option<String> },
    #[command(hide = true)]
    WokStats {
        #[arg(required = true)]
        tilesets: Vec<String>,
    },
    #[command(hide = true)]
    Oracle {
        #[arg(long)]
        out: PathBuf,
        n: Option<usize>,
        seed: Option<u64>,
    },
    #[command(hide = true, allow_negative_numbers = true)]
    Sim { x0: f32, y0: f32, x1: f32, y1: f32 },
    #[command(hide = true, allow_negative_numbers = true)]
    SimLine { x0: f32, y0: f32, x1: f32, y1: f32 },
    #[command(hide = true)]
    SimCheck {
        #[arg(required = true)]
        files: Vec<String>,
    },
    #[command(hide = true)]
    SimPredict { n: Option<usize>, seeds: Option<u64> },
}

/// `landscaper serve`'s options.
#[derive(Args, Debug)]
pub struct Serve {
    /// The port to listen on
    #[arg(long, default_value_t = 8765)]
    pub port: u16,
    /// The address to listen on (0.0.0.0 in a container)
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,
    /// The editor's files (the nwn-landscaper-web repository) [default: LANDSCAPER_WEB, then web/ beside the program,
    /// ./nwn-landscaper-web, ./landscaper-web, ../nwn-landscaper-web, ../landscaper-web]
    #[arg(long, value_name = "DIR")]
    pub web: Option<PathBuf>,
    /// A team server behind a sign-in proxy (also LANDSCAPER_TEAM): docs/TEAM-SERVER.md
    #[arg(long)]
    pub team: bool,
    /// Run a team server without LANDSCAPER_PROXY_SECRET (also LANDSCAPER_INSECURE=1): only where nothing but the proxy can reach it
    #[arg(long)]
    pub insecure: bool,
    /// Without --team, listen on an address other machines can reach: anyone who reaches it can do anything you can
    #[arg(long)]
    pub allow_remote: bool,
    /// Restart on a new build of the program (for working on the Landscaper)
    #[arg(long)]
    pub reload_on_rebuild: bool,
}

/// Whether `--bind` is an address only this machine can reach (127.0.0.0/8, ::1, `localhost`).
pub fn loopback(bind: &str) -> bool {
    let host = bind.trim().trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// `landscaper textures`' commands: the texture library.
#[derive(Subcommand, Debug)]
pub enum Textures {
    /// Build the starter library from the game's files
    Init,
    /// Add the game's or the haks' textures named in a JSON list
    Add {
        /// A JSON list of textures (docs/SELF-HOSTING.md, "Textures")
        list: PathBuf,
    },
    /// Add the pictures in a folder (PNG, TGA or DDS; a subfolder is a category)
    Build {
        /// The folder of pictures (PNG, TGA or DDS; a subfolder is a category)
        folder: PathBuf,
        /// The category the pictures go in
        #[arg(long, value_name = "NAME")]
        category: Option<String>,
        /// The surface they walk as (a surfacemat.2da row)
        #[arg(long, value_name = "N")]
        surface: Option<u32>,
    },
    /// List the library
    List,
    /// Install a texture pack: its id in the list of packs, a URL or a .zip (docs/TEXTURE-PACKS.md)
    Get {
        /// A pack's id, a URL or a .zip file
        pack: String,
        /// The list of packs to look the id up in [default: LANDSCAPER_PACKS_URL, else the Landscaper's own]
        #[arg(long, value_name = "URL")]
        index: Option<String>,
    },
    /// List the installed texture packs and those available
    ListPacks {
        /// The list of packs [default: LANDSCAPER_PACKS_URL, else the Landscaper's own]
        #[arg(long, value_name = "URL")]
        index: Option<String>,
    },
    /// Remove an installed texture pack (refused while an area paints with one of its textures)
    RemovePack {
        /// The pack's id
        id: String,
        /// Remove it even so: those areas then don't export until it is installed again
        #[arg(long)]
        force: bool,
    },
    /// Make a texture pack's zip from a list of pictures (docs/TEXTURE-PACKS.md, "Making a pack")
    Pack {
        /// The JSON list of the pack's fields and pictures
        list: PathBuf,
        /// The folder the zip goes in
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// An index file to add the pack to (made if missing)
        #[arg(long, value_name = "FILE")]
        index: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("landscaper").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_mistake_is_an_error_not_a_default() {
        for bad in [
            &["serve", "--port", "abc"][..],
            &["frobnicate"],
            &["decompile"],
            &["sim-line", "1", "2"],
            &["import", "only-one"],
            &["import", "a", "b", "--jobs", "0"],
            &["textures"],
            &[],
        ] {
            let e = parse(bad).expect_err(&format!("{bad:?}"));
            assert_eq!(e.exit_code(), 2, "{bad:?}: {e}");
        }
    }

    #[test]
    fn help_lists_the_user_commands_first_and_the_research_ones_after() {
        let help = Cli::command().render_long_help().to_string();
        let at = |s: &str| help.find(s).unwrap_or_else(|| panic!("{s} not in the help:\n{help}"));
        assert!(at("serve") < at("Research commands"));
        assert!(at("import") < at("Research commands") && at("export") < at("Research commands") && at("check ") < at("Research"));
        assert!(at("Research commands") < at("sim-line"));
        for flag in ["--root", "--user", "--tlk", "--haks", "--jobs"] {
            at(flag);
        }
    }

    #[test]
    fn only_loopback_addresses_are_this_machine_only() {
        for local in ["127.0.0.1", "127.0.0.2", "::1", "[::1]", "localhost", "LOCALHOST"] {
            assert!(loopback(local), "{local}");
        }
        for remote in ["0.0.0.0", "::", "192.168.1.5", "example.com", ""] {
            assert!(!loopback(remote), "{remote}");
        }
    }

    #[test]
    fn the_game_options_go_before_or_after_the_command() {
        let c = parse(&["--root", "/g", "import", "x.are", "x", "--jobs", "3", "--user", "/u"]).unwrap();
        assert_eq!((c.game.root.as_deref(), c.game.user.as_deref(), c.game.jobs), (Some("/g".as_ref()), Some("/u".as_ref()), Some(3)));
        let Command::Serve(s) = parse(&["serve", "--port", "9000", "--team"]).unwrap().command else { panic!() };
        assert_eq!((s.port, s.bind.as_str(), s.team, s.reload_on_rebuild), (9000, "127.0.0.1", true, false));
        let Command::Sim { x0, .. } = parse(&["sim", "-1.5", "2", "3", "4"]).unwrap().command else { panic!() };
        assert_eq!(x0, -1.5);
    }
}
