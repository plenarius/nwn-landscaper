// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where an import comes from: an area file, a module's area, the review service or a kept original ([`Source`]), the list of
//! areas that can be imported ([`sources`]), and the whole import of one into a landscaper area ([`import_to`], `landscaper
//! import`).

use std::path::Path;

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_rules::GameData;

use super::files::write;
use super::{import, recorded, span, times};
use crate::{models, paint};

/// An area to import: its ARE and GIT, and its module's haks and talk table.
pub struct Source {
    /// The production area it is (its resref, lower case), if it is one.
    pub area: String,
    /// The area's ARE, as bytes.
    pub are: Vec<u8>,
    /// Its GIT, as bytes (None: the source had none).
    pub git: Option<Vec<u8>>,
    /// Its module's haks, in the module's order.
    pub haks: Vec<String>,
    /// Its module's custom talk table.
    pub tlk: Option<String>,
    /// Where it came from, in words for the report (`module:<area>`, `<file>:<area>`).
    pub label: String,
}

impl Source {
    /// `<module.mod>:<area resref>`, or an area file (`.are` or nwn_gff's `.are.json`, its GIT beside it, its module.ifo above it).
    pub fn parse(arg: &str) -> Result<Source> {
        if let Some(resref) = arg.strip_prefix("ops:") {
            return Source::from_review(resref);
        }
        if let Some((m, area)) = arg.rsplit_once(".mod:") {
            return Source::module(Path::new(&format!("{m}.mod")), area);
        }
        let src = Path::new(arg);
        let lower = arg.to_ascii_lowercase();
        #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "`lower` is the name in lower case already")]
        let is_area = lower.ends_with(".are") || lower.ends_with(".are.json");
        anyhow::ensure!(is_area, "{arg}: give an area file (.are or .are.json) or <module.mod>:<area>");
        let m = nwn_bake::module::ModuleSource::open(src)?;
        let area = m.areas().into_iter().next().with_context(|| format!("{} is no area file", src.display()))?;
        Source::of(&m, &area, src.display().to_string())
    }

    /// Area `area` of `m`.
    fn of(m: &nwn_bake::module::ModuleSource, area: &str, label: String) -> Result<Source> {
        let files = m.area(area)?;
        Ok(Source {
            area: files.resref,
            are: files.are.to_bytes()?,
            git: files.git.map(|g| g.to_bytes()).transpose()?,
            haks: m.haks().to_vec(),
            tlk: m.tlk().map(str::to_string),
            label,
        })
    }

    /// Area `resref` from the review service, for the person acting now (whose assignment the caller has checked).
    pub fn from_review(resref: &str) -> Result<Source> {
        let f = crate::review::fetch(&crate::workspace::acting_user(), resref)?;
        let area = resref.to_ascii_lowercase();
        // (nwn_gff's JSON, as the module repository keeps it.)
        let gff = |ext: &str, json: &[u8]| nwn_bake::module::gff_from(&format!("{area}.{ext}.json"), json).and_then(|g| Ok(g.to_bytes()?));
        Ok(Source {
            are: gff("are", f.are.as_bytes())?,
            git: f.git.as_deref().map(|g| gff("git", g.as_bytes())).transpose()?,
            haks: f.haks.clone(),
            tlk: f.tlk.clone(),
            label: format!("module:{area}"),
            area,
        })
    }

    /// The original an earlier import of `name` kept (its ARE, GIT, module haks and talk table), to import it again.
    pub fn kept(name: &str) -> Result<Source> {
        let dir = paint::import_dir(name);
        Ok(Source {
            area: recorded(name)?.unwrap_or_default(),
            are: std::fs::read(dir.join("area.are"))
                .with_context(|| format!("{name} wasn't imported (no {})", dir.join("area.are").display()))?,
            git: std::fs::read(dir.join("area.git")).ok(),
            haks: std::fs::read(dir.join("haks.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default(),
            tlk: std::fs::read_to_string(dir.join("tlk.txt")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
            label: format!("{name}'s original"),
        })
    }

    /// Area `area` of the module file `path` (a plain ERF of areas has no module.ifo: no haks, no talk table).
    pub fn module(path: &Path, area: &str) -> Result<Source> {
        Source::of(&nwn_bake::module::ModuleSource::open(path)?, area, format!("{}:{area}", path.display()))
    }
}

/// The module's own hak list: from the review service on a team server, else the module checkout's module.ifo.json.
fn default_haks() -> Vec<String> {
    let from_api = crate::review::module_haks();
    if !from_api.is_empty() {
        return from_api;
    }
    models::checkout_haks()
}

/// The most tiles an area may have a side: the game's own limit, and what the editor and an export take (`heightmap::load`).
/// An import needs about 6 MB a tile (a 12 by 12 area peaked at 897 MB), so this also bounds what one import can ask of the
/// machine: an uploaded module whose area claims 100 by 100 tiles would need tens of GB.
const MAX_SIDE: i64 = 32;

/// Whether the area whose ARE root is `are` can be imported: 3 to [`MAX_SIDE`] tiles a side, and a tile list as long as its
/// size says. Checked before the game is opened or anything is allocated for its tiles.
pub fn size_ok(are: &mg_gff::Struct) -> Result<()> {
    let (w, h) = (are.integer("Width").unwrap_or(0), are.integer("Height").unwrap_or(0));
    anyhow::ensure!(w >= 3 && h >= 3, "areas under 3 tiles a side can't be imported yet (this one is {w} by {h})");
    anyhow::ensure!(w <= MAX_SIDE && h <= MAX_SIDE, "an area is at most {MAX_SIDE} tiles a side (this one says {w} by {h})");
    let tiles = are.list("Tile_List").map_or(0, <[_]>::len);
    anyhow::ensure!(tiles as i64 == w * h, "its tile list has {tiles} tiles, not the {} that {w} by {h} tiles need", w * h);
    Ok(())
}

/// Imports `src` as the landscaper area `name` in `dir`; what it found.
pub fn import_to(src: &Source, name: &str, dir: &Path) -> Result<serde_json::Value> {
    let are_gff = mg_gff::Gff::read(&src.are)?;
    size_ok(&are_gff.root)?;
    // The module's haks: its tilesets are often there.
    let mut game = span("open game", || models::open_game(nwn_bake::game::Haks::List(src.haks.clone()), nwn_bake::game::Tlk::None))?;
    // A source with no hak list of its own (a plain ERF of areas has no module.ifo, a toolset module may not list the
    // tile haks) still means the module's tilesets: when its tileset isn't there, add the module's haks behind its own.
    let tileset = are_gff.root.resref("Tileset").map_or_default(|t| t.to_string());
    let have = |game: &GameData| mg_resman::ResKey::parse(&tileset, ResType::SET).is_some_and(|k| game.resman.contains(&k));
    let mut added_default = false;
    if !have(&game) {
        let more: Vec<String> = default_haks().into_iter().filter(|h| !src.haks.iter().any(|s| s.eq_ignore_ascii_case(h))).collect();
        if !more.is_empty() {
            game.add_haks(&more)?;
            added_default = true;
        }
    }
    let mut imp = span("import", || import(&game, &are_gff.root))?;
    if added_default {
        imp.notes.push("its tileset came from the module's haks (the file lists none that has it)".to_string());
    }
    // Its own settings, so the editor shows what it has and an export writes the same back.
    let git_root = src.git.as_ref().and_then(|g| mg_gff::Gff::read(g).ok()).map_or_else(|| mg_gff::Struct::new(0), |g| g.root);
    let name_text = are_gff.root.locstring("Name").and_then(|l| {
        l.text(mg_core::Language::ENGLISH, mg_core::Gender::Male).map(std::borrow::Cow::into_owned).or_else(|| game.string(l.strref))
    });
    imp.settings = crate::settings::read(&are_gff.root, &git_root, name_text);
    span("write", || write(&imp, name, dir))?;
    // The original's ARE, GIT, module haks and talk table, for the export.
    let keep = dir.join(format!("{name}.import"));
    std::fs::write(keep.join("area.are"), &src.are)?;
    match &src.git {
        Some(g) => std::fs::write(keep.join("area.git"), g)?,
        None => {
            let _ = std::fs::remove_file(keep.join("area.git"));
        }
    }
    std::fs::write(keep.join("haks.json"), serde_json::to_vec(&src.haks)?)?;
    // Where it came from, who imported it and when (what assignments, re-import and export are checked against).
    std::fs::write(
        keep.join("source.json"),
        serde_json::to_vec(
            &serde_json::json!({ "area": src.area, "by": crate::workspace::acting_user(), "at": crate::assignments::now() }),
        )?,
    )?;
    match &src.tlk {
        Some(t) => std::fs::write(keep.join("tlk.txt"), t)?,
        None => {
            let _ = std::fs::remove_file(keep.join("tlk.txt"));
        }
    }
    // The tileset textures the cliffs and objects use, for the hak (they may only be in the module's haks).
    let tex = keep.join("tex");
    std::fs::create_dir_all(&tex)?;
    for e in std::fs::read_dir(&tex)?.flatten() {
        std::fs::remove_file(e.path())?;
    }
    let mut notes = imp.notes.clone();
    let t_files = times::begin("texture files");
    for t in &imp.textures {
        // The one the engine would use there (an EE material of that name gives its texture0; a hak's TGA beats the game's DDS), and
        // its TXI (cut-outs, blending), kept under the mesh's name.
        let resolved = nwn_bake::texture::resolve(&game, t);
        match resolved.as_ref().and_then(|r| r.plain(&game)) {
            Some((rt, d)) => std::fs::write(tex.join(format!("{t}.{}", if rt == ResType::DDS { "dds" } else { "tga" })), &*d)?,
            None => notes.push(format!("texture {t} not found")),
        }
        if let Some(d) = resolved.as_ref().and_then(|r| r.txi(&game)) {
            std::fs::write(tex.join(format!("{t}.txi")), &*d)?;
        }
    }
    times::add("texture files", t_files);
    if !game.missing_haks().is_empty() {
        notes.push(format!("haks not found: {}", game.missing_haks().join(", ")));
    }
    let (lo, hi) = imp.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let objects = mg_gff::Gff::read(src.git.as_deref().unwrap_or_default()).ok().map_or(0, |g| {
        ["Creature List", "Door List", "Encounter List", "List", "Placeable List", "SoundList", "StoreList", "TriggerList", "WaypointList"]
            .iter()
            .map(|l| g.root.list(l).map_or(0, <[_]>::len))
            .sum::<usize>()
    });
    Ok(serde_json::json!({
        "name": name, "source": src.label, "tiles": [imp.width, imp.height], "tileset": imp.tileset,
        "heights": [lo, hi], "standing": imp.objects.iter().filter(|o| !o.flat.fixed).count(), "textures": imp.textures.len(),
        "objects": objects, "notes": notes,
    }))
}

/// What an import of `name` makes in its folder: its kept original, pictures and objects (`<name>.import/`), and the heights,
/// paint, header and the editor's picture beside it. (Also `<name>.import.png`, the whole area's picture earlier versions
/// wrote, so a re-import clears it away.)
fn import_files(name: &str) -> [String; 6] {
    ["import", "json", "f32", "materials", "import.png", "import.view.png"].map(|ext| format!("{name}.{ext}"))
}

/// Imports `src` again as `name` in `dir`, over an earlier import of it (see [`replacing`]).
pub fn reimport_to(src: &Source, name: &str, dir: &Path) -> Result<serde_json::Value> {
    replacing(dir, name, || import_to(src, name, dir))
}

/// Runs `import` (a new import of `name` in `dir`) with what an earlier import of `name` left set aside: on success that goes,
/// so nothing of the old import (a stale object, a ground patch) mixes with the new one; on a failure or a panic, what the
/// failed import wrote goes and the earlier import comes back as it was. A re-import that fails (the tileset's hak is gone,
/// the review service is down, the kept ARE doesn't parse) must not lose the kept original: it is the only copy, and its
/// `source.json` is what ties the area to its assignment.
pub(super) fn replacing<T>(dir: &Path, name: &str, import: impl FnOnce() -> Result<T>) -> Result<T> {
    let aside = Aside::new(dir, name)?;
    let made = import()?;
    aside.done();
    Ok(made)
}

/// What an earlier import left, moved into `.<name>.reimport/` while a new one runs; put back when dropped before [`Aside::done`].
struct Aside {
    dir: std::path::PathBuf,
    name: String,
    /// The hidden folder the files are in.
    held: std::path::PathBuf,
    /// The files moved there.
    moved: Vec<String>,
    done: bool,
}

impl Aside {
    fn new(dir: &Path, name: &str) -> Result<Aside> {
        let held = dir.join(format!(".{name}.reimport"));
        // (One left by a server that stopped in the middle: what it holds is older than what is in place now, if anything is.)
        if held.exists() {
            std::fs::remove_dir_all(&held).with_context(|| format!("removing {}", held.display()))?;
        }
        std::fs::create_dir_all(&held).with_context(|| format!("making {}", held.display()))?;
        let mut aside = Aside { dir: dir.to_path_buf(), name: name.to_string(), held, moved: Vec::new(), done: false };
        for f in import_files(name) {
            let from = dir.join(&f);
            if std::fs::symlink_metadata(&from).is_ok() {
                // (A failure here drops `aside`, which puts back what was moved so far.)
                std::fs::rename(&from, aside.held.join(&f)).with_context(|| format!("setting {} aside", from.display()))?;
                aside.moved.push(f);
            }
        }
        Ok(aside)
    }

    /// The new import worked: what was set aside goes.
    fn done(mut self) {
        self.done = true;
        let _ = std::fs::remove_dir_all(&self.held);
    }
}

impl Drop for Aside {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // What the failed import wrote goes, then what was there comes back.
        for f in import_files(&self.name) {
            let p = self.dir.join(&f);
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
        let mut lost = false;
        for f in &self.moved {
            lost |= std::fs::rename(self.held.join(f), self.dir.join(f)).is_err();
        }
        if lost {
            // Left where it is, for an admin to put back by hand.
            eprintln!("re-import of {}: couldn't put the earlier import back; it is in {}", self.name, self.held.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.held);
        }
    }
}

/// An area's name as the module's file writes it out, when it does (the first language's text); a name that is a talk table
/// reference is not text here, so it gives "" (the review service, which has the table, sends that kind already resolved).
fn area_name_text(v: &serde_json::Value) -> String {
    v.as_object()
        .and_then(|o| {
            o.iter().filter(|(k, _)| k.chars().all(|c| c.is_ascii_digit())).min_by_key(|(k, _)| k.parse::<u32>().unwrap_or(u32::MAX))
        })
        .and_then(|(_, t)| t.as_str())
        .map_or_default(|t| t.trim().to_string())
}

/// What can be imported: the module repository's areas (LANDSCAPER_MODULE_REPO,
/// see config.rs) and the areas of the modules in the
/// game's modules folder (landscaper's own left out). Read once. An error only from a review service that has never answered.
pub fn sources() -> Result<Vec<serde_json::Value>> {
    if crate::review::active() {
        return crate::review::sources();
    }
    static LIST: std::sync::OnceLock<Vec<serde_json::Value>> = std::sync::OnceLock::new();
    Ok(LIST
        .get_or_init(|| {
            let mut out = Vec::new();
            let mut files = Vec::new();
            let mut stack = vec![crate::config::Layout::from_env().areas_dir()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.to_string_lossy().ends_with(".are.json") {
                        files.push(p);
                    }
                }
            }
            files.sort();
            for p in files {
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&p).unwrap_or_default()) else { continue };
                let f = |k: &str| v[k]["value"].clone();
                let resref = p.file_name().map_or_default(|n| n.to_string_lossy().trim_end_matches(".are.json").to_string());
                out.push(serde_json::json!({
                    "source": p, "area": resref, "tag": f("Tag"), "name": area_name_text(&v["Name"]["value"]), "tileset": f("Tileset"),
                    "width": f("Width"), "height": f("Height"), "flags": f("Flags"), "from": "module repository",
                }));
            }
            let in_repo: std::collections::HashSet<String> = out.iter().filter_map(|s| s["area"].as_str().map(str::to_string)).collect();
            // (Your own machine only: this list is shared by every request, and on a
            // team server whose modules folder it was would depend on who asked first.)
            if let Some(user) = (!crate::workspace::team()).then(crate::workspace::nwn_user_dir).flatten() {
                let mut mods: Vec<_> = std::fs::read_dir(user.join("modules"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "mod"))
                    .collect();
                mods.sort();
                for p in mods {
                    let Ok(m) = mg_module::Module::open(&p) else { continue };
                    for a in m.areas().unwrap_or_default() {
                        let Some(Ok(are)) = m.gff(&mg_resman::ResKey { resref: a, restype: ResType::ARE }) else { continue };
                        let tileset = are.root.resref("Tileset").map_or_default(|t| t.to_string());
                        // Landscaper's own areas are on a tileset of their own
                        // name; a build from the module's repository keeps the repository's.
                        let stem = p.file_stem().map_or_default(|n| n.to_string_lossy().into_owned());
                        if tileset.eq_ignore_ascii_case(&a.to_string())
                            || tileset.eq_ignore_ascii_case(&stem)
                            || in_repo.contains(&a.to_string())
                        {
                            continue;
                        }
                        out.push(serde_json::json!({
                            "source": format!("{}:{a}", p.display()), "area": a.to_string(),
                            "tag": are.root.string("Tag").map(|b| String::from_utf8_lossy(b).into_owned()), "tileset": tileset,
                            "width": are.root.integer("Width"), "height": are.root.integer("Height"), "flags": are.root.integer("Flags"),
                            "from": p.file_name().map(|n| n.to_string_lossy().into_owned()),
                        }));
                    }
                }
            }
            out
        })
        .clone())
}

/// `landscaper import <area.are|area.are.json|module.mod:area> <name> [dir]`: into `dir`, else the exports folder.
pub fn run(source: &str, name: &str, dir: Option<&Path>) -> Result<()> {
    let src = Source::parse(source)?;
    let dir = dir.map_or_else(crate::workspace::exports_dir, Path::to_path_buf);
    let r = import_to(&src, name, &dir)?;
    times::report();
    println!("imported {}: {}", src.label, serde_json::to_string(&r)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;

    /// Every file under `dir` with its content (folders as empty entries), by its path inside `dir`.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let at = p.strip_prefix(dir).unwrap().to_path_buf();
                if p.is_dir() {
                    out.insert(at, None);
                    stack.push(p);
                } else {
                    out.insert(at, Some(std::fs::read(&p).unwrap()));
                }
            }
        }
        out
    }

    /// A scratch folder holding an earlier import of `x` (its kept original, an object, its pictures) and another area `y`.
    fn earlier_import(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-reimport-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("x.import/tex")).unwrap();
        for (f, d) in [
            ("x.import/area.are", &b"ARE V3.2 the kept original"[..]),
            ("x.import/source.json", br#"{"area":"forest1","by":"alice","at":1}"#),
            ("x.import/haks.json", br#"["forest"]"#),
            ("x.import/obj_0000.json", b"{}"),
            ("x.import/tex/tt01_grass.tga", b"tga"),
            ("x.json", br#"{"name":"x","imported":true}"#),
            ("x.f32", &[1, 2, 3, 4]),
            ("x.materials", &[0]),
            ("x.import.png", b"png"),
            ("x.import.view.png", b"small png"),
            ("y.json", br#"{"name":"y"}"#),
        ] {
            std::fs::write(dir.join(f), d).unwrap();
        }
        dir
    }

    fn source(are: Vec<u8>) -> Source {
        Source { area: "forest1".into(), are, git: None, haks: Vec::new(), tlk: None, label: "test".into() }
    }

    #[test]
    fn a_failed_reimport_leaves_the_earlier_import_as_it_was() {
        let dir = earlier_import("fail");
        let before = snapshot(&dir);
        // A kept ARE that doesn't parse.
        assert!(reimport_to(&source(b"not a gff".to_vec()), "x", &dir).is_err());
        assert_eq!(snapshot(&dir), before);
        // An import that fails halfway, having written some of its files (a hak gone, the review service down).
        let failed: Result<()> = replacing(&dir, "x", || {
            std::fs::create_dir_all(dir.join("x.import")).unwrap();
            std::fs::write(dir.join("x.import/obj_0001.json"), b"half").unwrap();
            std::fs::write(dir.join("x.json"), b"half").unwrap();
            anyhow::bail!("the tileset tccr0 isn't in the game or in the haks")
        });
        assert!(failed.is_err());
        assert_eq!(snapshot(&dir), before);
        // A bug (a panic) in the middle of one.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            replacing(&dir, "x", || -> Result<()> {
                std::fs::write(dir.join("x.f32"), b"half").unwrap();
                panic!("a bug in the importer")
            })
        }));
        assert!(panicked.is_err());
        assert_eq!(snapshot(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reimport_that_works_replaces_the_earlier_one_whole() {
        let dir = earlier_import("works");
        replacing(&dir, "x", || {
            std::fs::create_dir_all(dir.join("x.import")).unwrap();
            std::fs::write(dir.join("x.import/area.are"), b"new").unwrap();
            std::fs::write(dir.join("x.json"), b"new").unwrap();
            Ok(())
        })
        .unwrap();
        let after = snapshot(&dir);
        let names: Vec<String> = after.keys().map(|p| p.display().to_string().replace('\\', "/")).collect();
        assert_eq!(names, ["x.import", "x.import/area.are", "x.json", "y.json"], "the earlier import's object, pictures and heights go");
        assert_eq!(after[Path::new("x.json")].as_deref(), Some(&b"new"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_area_over_32_tiles_a_side_or_with_a_wrong_tile_list_is_refused_before_the_game_is_opened() {
        let are = |w: i32, h: i32, tiles: usize| {
            let mut g = mg_gff::Gff::new(*b"ARE ");
            g.root.set("Width", mg_gff::Value::Int(w));
            g.root.set("Height", mg_gff::Value::Int(h));
            g.root.set("Tileset", mg_gff::Value::ResRef(b"ttr01".to_vec()));
            g.root.set("Tile_List", mg_gff::Value::List(vec![mg_gff::Struct::new(1); tiles]));
            g.to_bytes().unwrap()
        };
        let dir = std::env::temp_dir().join(format!("ls-import-size-{}", std::process::id()));
        for (w, h, tiles, why) in [
            (33, 3, 99, "at most 32 tiles a side"),
            (3, 64, 192, "at most 32 tiles a side"),
            // The tile list needn't be long for the size to be refused: a 100 by 100 claim in a small file.
            (100, 100, 1, "at most 32 tiles a side"),
            (8, 8, 10, "tile list has 10 tiles, not the 64"),
            (2, 8, 16, "under 3 tiles a side"),
        ] {
            // (No game in CI: getting as far as opening it would be another error.)
            let e = format!("{:#}", import_to(&source(are(w, h, tiles)), "x", &dir).unwrap_err());
            assert!(e.contains(why), "{w} by {h}, {tiles} tiles: {e}");
        }
        assert!(!dir.exists(), "nothing is written");
        size_ok(&mg_gff::Gff::read(&are(32, 32, 1024)).unwrap().root).unwrap();
    }

    #[test]
    fn an_area_name_written_in_the_file_is_read_and_a_table_reference_is_not() {
        assert_eq!(area_name_text(&serde_json::json!({ "0": " Forest of Fog " })), "Forest of Fog");
        assert_eq!(area_name_text(&serde_json::json!({ "id": 16857690 })), "");
        assert_eq!(area_name_text(&serde_json::Value::Null), "");
    }
}
