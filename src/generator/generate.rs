// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `generate`: builds the hak and the module (tiles, tileset, area, start point) from the current terrain.

use std::collections::HashMap;

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_erf::ErfWriter;
use mg_gff::Value;
use mg_module::new::{AreaSpec, add_area_of, new_module};
use mg_resman::ResKey;
use mg_set::Tileset;

use super::{
    MINIMAP, Painter, SHADOW_MAP, bake_size, baked_name, choose_path_nodes, crlf, edge, entrance, hak_name, in_parallel, minimap,
    minimap_name, set_file, shadow_map, shadow_name, shadow_strength, start_point, surfaced, tile_level, tile_model, tile_name, tile_wok,
    tiles_x, tiles_y, tileset_name, walkmesh,
};
use crate::{atmosphere, heightmap, import, models, objects, paint, settings, shared, textures, things, workspace};

/// The import the current terrain comes from (its palette or objects name one: `@import:<name>`), if any.
fn import_name() -> Option<String> {
    let hm = heightmap::current()?;
    hm.palette
        .iter()
        .map(String::as_str)
        .chain(hm.objects.iter().map(|o| o.model.as_str()))
        .find_map(|id| id.strip_prefix(paint::IMPORT))
        .map(|rest| rest.split(':').next().unwrap_or(rest).to_string())
}

/// The test module's script (`test_path`), up to its `main`: each walk sends a dog from one point to another and checks it
/// got there (`mg test` runs it on a dev server).
const TEST_LIB: &str = r#"#include "mg_test"

void LogPos(object o, string sCase)
{
    vector p = GetPosition(o);
    WriteTimestampedLogEntry("LSPOS " + sCase + " " + FloatToString(p.x, 0, 2) + " " + FloatToString(p.y, 0, 2) + " "
        + FloatToString(p.z, 0, 2));
}

void Report(object o, location lDest, string sCase)
{
    vector a = GetPosition(o);
    vector b = GetPositionFromLocation(lDest);
    float d = VectorMagnitude(Vector(a.x - b.x, a.y - b.y, 0.0));
    Assert(d < 2.5, sCase, "distance left " + FloatToString(d, 0, 2));
}

void Walk(string sCase, float x0, float y0, float x1, float y1)
{
    object oArea = GetFirstArea();
    location lEnd = Location(oArea, Vector(x1, y1, 0.0), 0.0);
    object o = CreateObject(OBJECT_TYPE_CREATURE, "nw_dog", Location(oArea, Vector(x0, y0, 0.0), 0.0));
    AssertValid(o, sCase + ": spawned");
    // No player is in the area, so creatures would run at a low AI level.
    SetAILevel(o, AI_LEVEL_HIGH);
    AssignCommand(o, ActionMoveToLocation(lEnd, TRUE));
    int i;
    for (i = 1; i <= 30; i++)
        DelayCommand(IntToFloat(i), LogPos(o, sCase));
    DelayCommand(30.5, Report(o, lEnd, sCase));
}

"#;

/// A walk of the test script: its name, and where the dog starts and should end (x, y in metres).
type TestWalk = (&'static str, [f32; 4]);

/// The walks of the test script: up a ramp, round a cliff, and across most of the area.
const TEST_WALKS: [TestWalk; 3] =
    [("ramp", [16.0, 6.0, 33.0, 6.0]), ("detour", [21.0, 15.0, 28.0, 15.0]), ("long", [6.0, 24.0, 34.0, 24.0])];

/// The test script with these walks.
fn test_script(walks: &[TestWalk]) -> String {
    let mut s = format!("{TEST_LIB}void main()\n{{\n");
    for (case, [x0, y0, x1, y1]) in walks {
        s += &format!("    Walk(\"{case}\", {x0:.1}, {y0:.1}, {x1:.1}, {y1:.1});\n");
    }
    s + "}\n"
}

/// `LS_ONLOAD`: the test script as a plain OnModuleLoad script, its asserts written to the log (for tracing under gdb, where
/// there is no `mg test`).
fn onload_script(walks: &[TestWalk]) -> String {
    test_script(walks).replace(
        "#include \"mg_test\"",
        "void Assert(int b, string s, string d) { WriteTimestampedLogEntry(\"LSRESULT \" + s + \" \" + IntToString(b) + \" \" + d); \
         }\nvoid AssertValid(object o, string s) {}",
    )
}

/// Generates the loaded area's tileset, hak and module into `out_dir`, on the export's threads. `test` is the module's test
/// script (`test_path`): None, the built-in walks; `Some("")`, none (an export to build on, not for `mg test`).
pub(crate) fn generate(out_dir: &std::path::Path, test: Option<&str>) -> Result<()> {
    super::plan::as_export(|| build(out_dir, test))
}

/// [`generate`]'s work, run as an export (on every core unless `--jobs` says otherwise).
fn build(out_dir: &std::path::Path, test: Option<&str>) -> Result<()> {
    let gen_start = std::time::Instant::now();
    let game = models::open_game(nwn_bake::game::Haks::None, nwn_bake::game::Tlk::None)?;
    let mut hak = ErfWriter::new(*b"HAK ");
    let mut owned: Vec<(ResRef, ResType, Vec<u8>)> = Vec::new();

    textures::ensure_starter();
    let painter = Painter::load()?;
    let strength = shadow_strength();
    // The library textures, the entrance black and the shadow shaders are in the shared hak.
    shared::ensure(&workspace::nwn_user_dir().context("no user dir")?.join("hak"))?;
    let (mut walk_faces, mut render_bytes, mut most, mut baked) = (0, 0, 0, 0);
    // The whole area's minimap, for a look at it outside the game.
    let mut whole = mg_image::Rgba::new(MINIMAP * tiles_x() as u32, MINIMAP * tiles_y() as u32);
    // And its baked shadows.
    let mut shadows = mg_image::Rgba::new(SHADOW_MAP * tiles_x() as u32, SHADOW_MAP * tiles_y() as u32);
    let timing = std::env::var_os("LS_TIMING").is_some();
    // LS_DUMP: each baked texture as a PNG and each model's text beside the export, to look at.
    let dump = std::env::var_os("LS_DUMP").is_some();
    // The baked ground textures and the shadow maps take most of an export; each tile's are its own, so they are made side by
    // side before the tiles are put together (the result is the same as one after another).
    let tiles: Vec<(usize, usize)> = (0..tiles_y()).flat_map(|ty| (0..tiles_x()).map(move |tx| (tx, ty))).collect();
    let mut baked_dds: HashMap<(usize, usize), Vec<u8>> = match &painter {
        Some(p) => {
            let baked_tiles: Vec<(usize, usize)> = tiles.iter().copied().filter(|&(tx, ty)| p.baked(tx, ty)).collect();
            let bake_start = std::time::Instant::now();
            let made = in_parallel(&baked_tiles, |(tx, ty)| paint::dds(&p.bake(tx, ty, bake_size())));
            if timing {
                eprintln!("  baked {} tiles in {:.1}s", baked_tiles.len(), bake_start.elapsed().as_secs_f32());
            }
            baked_tiles.into_iter().zip(made).collect()
        }
        None => HashMap::new(),
    };
    // Row by row from the south, as `tiles`; none without shadows.
    let mut shadow_maps = if strength > 0.0 {
        let shadow_start = std::time::Instant::now();
        let maps = in_parallel(&tiles, |(tx, ty)| shadow_map(tx, ty, strength));
        if timing {
            eprintln!("  shadow maps in {:.1}s", shadow_start.elapsed().as_secs_f32());
        }
        maps
    } else {
        Vec::new()
    }
    .into_iter();
    let mut ph = [0f32; 6];
    let mut mark = std::time::Instant::now();
    let pre_loop = gen_start.elapsed().as_secs_f32();
    macro_rules! phase {
        ($i:expr) => {
            ph[$i] += mark.elapsed().as_secs_f32();
            mark = std::time::Instant::now();
        };
    }
    for ty in 0..tiles_y() {
        for tx in 0..tiles_x() {
            let name = tile_name(tx, ty);
            let w = walkmesh(tx, ty);
            walk_faces += w.faces.len();
            most = most.max(w.faces.len());
            // The game's surfaces from here on (the plan knows walkable or not).
            // LS_NO_SURFACES: grass and non-walkable only (experiments).
            let w = if std::env::var_os("LS_NO_SURFACES").is_some() { w } else { surfaced(&w, tx, ty, painter.as_ref()) };
            phase!(0);
            let mut mtrs = Vec::new();
            let text = tile_model(&name, tx, ty, &w, painter.as_ref(), strength > 0.0, &mut mtrs);
            phase!(1);
            if let Some(map) = shadow_maps.next() {
                let row = (SHADOW_MAP * tiles_x() as u32 * 4) as usize;
                for y in 0..SHADOW_MAP as usize {
                    let at = (ty * SHADOW_MAP as usize + y) * row + tx * SHADOW_MAP as usize * 4;
                    shadows.data[at..at + SHADOW_MAP as usize * 4]
                        .copy_from_slice(&map.data[y * SHADOW_MAP as usize * 4..][..SHADOW_MAP as usize * 4]);
                }
                owned.push((ResRef::from_str(&shadow_name(tx, ty))?, ResType::DDS, paint::dds(&map)));
                owned.push((ResRef::from_str(&shadow_name(tx, ty))?, ResType::TXI, b"clamp 3\r\n".to_vec()));
                for (mat, text) in mtrs {
                    owned.push((ResRef::from_str(&mat)?, ResType::MTR, text.into_bytes()));
                }
            }
            phase!(2);
            if let Some(dds) = baked_dds.remove(&(tx, ty)) {
                if dump {
                    // As the game will decode it.
                    paint::write_png(&out_dir.join(format!("{}.png", baked_name(tx, ty))), &mg_image::read(ResType::DDS, &dds)?.to_rgba())?;
                }
                owned.push((ResRef::from_str(&baked_name(tx, ty))?, ResType::DDS, dds));
                // Clamped, so the edges don't pick up the opposite side's pixels.
                owned.push((ResRef::from_str(&baked_name(tx, ty))?, ResType::TXI, b"clamp 3\r\n".to_vec()));
                baked += 1;
            }
            phase!(3);
            let mini = minimap(tx, ty, painter.as_ref());
            let row = (MINIMAP * tiles_x() as u32 * 4) as usize;
            for y in 0..MINIMAP as usize {
                let at = (ty * MINIMAP as usize + y) * row + tx * MINIMAP as usize * 4;
                whole.data[at..at + MINIMAP as usize * 4].copy_from_slice(&mini.data[y * MINIMAP as usize * 4..][..MINIMAP as usize * 4]);
            }
            owned.push((ResRef::from_str(&minimap_name(tx, ty))?, ResType::TGA, mg_image::write_tga(&mini)));
            phase!(4);
            let compiled = mg_mdl::compile::compile(text.as_bytes(), &Default::default()).with_context(|| format!("compiling {name}"))?;
            // (The tile model has no aabb list of its own: Moonglow building one from its faces is expected, not news.)
            for (line, note) in compiled.notes.iter().filter(|(_, n)| !n.ends_with("no aabb list, a tree was made of its faces")) {
                eprintln!("  {name}:{line}: {note}");
            }
            render_bytes += compiled.binary.len();
            if dump {
                std::fs::write(out_dir.join(format!("{name}.mdl.txt")), &text)?;
            }
            owned.push((ResRef::from_str(&name)?, ResType::MDL, compiled.binary));
            owned.push((ResRef::from_str(&name)?, ResType::WOK, crlf(&tile_wok(&name, &w))));
            phase!(5);
        }
    }
    if timing {
        let t = gen_start.elapsed().as_secs_f32();
        eprintln!(
            "  generate per tile: walkmesh+surfaces {:.1}s, tile model {:.1}s, shadow map {:.1}s, baked texture {:.1}s, minimap {:.1}s, \
             compile+wok {:.1}s (before the loop {pre_loop:.1}s; so far {t:.1}s)",
            ph[0], ph[1], ph[2], ph[3], ph[4], ph[5]
        );
    }
    paint::write_png(&out_dir.join("minimap.png"), &whole)?;
    if strength > 0.0 {
        paint::write_png(&out_dir.join("shadows.png"), &shadows)?;
    }
    // The scenery the client draws beyond the edge (LS_NO_EDGE: none).
    // (A cave has no edge scenery: its border is off, see generate's area.)
    let underground = heightmap::current().is_some_and(|h| h.underground);
    if std::env::var_os("LS_NO_EDGE").is_none() && !underground {
        for (name, text) in edge::models(painter.as_ref()) {
            let compiled = mg_mdl::compile::compile(text.as_bytes(), &Default::default()).with_context(|| format!("compiling {name}"))?;
            if dump {
                std::fs::write(out_dir.join(format!("{name}.mdl.txt")), &text)?;
            }
            owned.push((ResRef::from_str(&name)?, ResType::MDL, compiled.binary));
        }
        owned.push((ResRef::from_str(&format!("{}_edge", tileset_name()))?, ResType::TWODA, crlf(&edge::table())));
    }
    if let Some(p) = &painter {
        let mut used: Vec<u8> = p.hm.materials.clone();
        used.sort_unstable();
        used.dedup();
        println!("textures: {} from the library (shared hak), {baked} tiles baked at {} px", used.len(), bake_size());
    }
    // An import's tileset textures (its cliffs' and objects'): they may only be in its module's haks.
    if let Some(hm) = heightmap::current() {
        let imports: std::collections::BTreeSet<&str> = hm
            .palette
            .iter()
            .map(String::as_str)
            .chain(hm.objects.iter().map(|o| o.model.as_str()))
            .filter_map(|id| id.strip_prefix(paint::IMPORT))
            .map(|rest| rest.split(':').next().unwrap_or(rest))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for name in imports {
            for e in std::fs::read_dir(paint::import_dir(name).join("tex")).into_iter().flatten().flatten() {
                let path = e.path();
                let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else {
                    continue;
                };
                let restype = match ext {
                    "dds" => ResType::DDS,
                    "tga" => ResType::TGA,
                    "txi" => ResType::TXI,
                    _ => continue,
                };
                if seen.insert((stem.to_string(), ext.to_string())) {
                    owned.push((ResRef::from_str(stem)?, restype, std::fs::read(&path)?));
                }
            }
        }
    }
    let nodes = choose_path_nodes();
    let set = set_file(&nodes);
    owned.push((ResRef::from_str(&tileset_name())?, ResType::SET, crlf(&set)));
    for (r, t, d) in &owned {
        hak.add(*r, *t, d.as_slice())?;
    }
    let hak_path = workspace::nwn_user_dir().context("no user dir")?.join("hak").join(format!("{}.hak", hak_name()));
    crate::atomic::write(&hak_path, &hak.to_bytes()?)?;
    if timing {
        eprintln!("  hak written at {:.1}s", gen_start.elapsed().as_secs_f32());
    }
    let levels: Vec<String> =
        (0..tiles_y()).rev().map(|ty| (0..tiles_x()).map(|tx| tile_level(tx, ty).to_string()).collect::<Vec<_>>().join("")).collect();
    println!("tile levels (north row first): {}", levels.join(" "));
    println!(
        "hak: {} ({} tiles, {walk_faces} walkmesh faces (at most {most} a tile), {render_bytes} bytes of models)",
        hak_path.display(),
        tiles_x() * tiles_y()
    );

    // The module: one area on the generated tileset, each cell its own tile.
    let mut rng = fastrand::Rng::with_seed(1);
    let mut m = new_module(&game, "Landscaper", &mut rng)?;
    let tileset = Tileset::parse(set.as_bytes(), game.codepage())?;
    // Named after the area: several of them can share a module.
    let area_name = heightmap::current().map_or_else(|| "Landscape".to_string(), |h| h.name.clone());
    let spec = AreaSpec { name: area_name, tileset: ResRef::from_str(&tileset_name())?, width: tiles_x() as u32, height: tiles_y() as u32 };
    if std::env::var_os("LS_CONTROL").is_some() {
        // Control: BioWare's rural tiles, water painted down the middle
        // (corners x=2, y=2..4), dry land to the south to walk around it.
        let spec = AreaSpec { name: "Landscape".into(), tileset: ResRef::from_str("ttr01")?, width: 4, height: 4 };
        let area = mg_module::new::add_area(&mut m, &game, &spec, &mut rng)?;
        let key = ResKey { resref: area, restype: ResType::ARE };
        let mut are = m.gff(&key).context("no area")??;
        let set_data = game.resman.get(&ResKey { resref: ResRef::from_str("ttr01")?, restype: ResType::SET })?;
        let ttr = Tileset::parse(&set_data, game.codepage())?;
        let index = mg_tiles::TileIndex::new(&ttr);
        let rules = mg_tiles::paint::Rules::new(&index, &ttr);
        let list = are.root.list("Tile_List").context("tiles")?;
        let placements = list
            .iter()
            .map(|t| mg_tiles::Placement {
                tile: t.integer("Tile_ID").unwrap_or(0) as u32,
                orientation: t.integer("Tile_Orientation").unwrap_or(0) as u8,
                height: t.integer("Tile_Height").unwrap_or(0) as i32,
            })
            .collect();
        let mut grid = mg_tiles::paint::Grid::new(&index, 4, 4, placements).context("grid")?;
        let water = index.terrain("water").context("no water terrain")?;
        for y in 2..=4 {
            let stroke = grid.paint(&index, &rules, 2, y, water).context("paint refused")?;
            grid.apply(&index, stroke, &mut rng);
        }
        let tiles = are.root.list_mut("Tile_List").context("tiles")?;
        for (t, p) in tiles.iter_mut().zip(&grid.tiles) {
            t.set("Tile_ID", Value::Int(p.tile as i32));
            t.set("Tile_Orientation", Value::Int(i32::from(p.orientation)));
            t.set("Tile_Height", Value::Int(p.height));
        }
        m.set_gff(key, &are)?;
        m.set_start(area, [6.0, 4.0, 0.0], std::f32::consts::FRAC_PI_2)?;
        // Only a walk across the water (the test area's walks don't fit these tiles).
        let water: [TestWalk; 1] = [("water", [6.0, 28.0, 34.0, 28.0])];
        if std::env::var_os("LS_ONLOAD").is_some() {
            m.set(ResKey { resref: ResRef::from_str("ls_onload")?, restype: ResType::NSS }, onload_script(&water).into_bytes());
            let mut info = m.info()?;
            info.root.set("Mod_OnModLoad", Value::resref(ResRef::from_str("ls_onload")?));
            m.set_info(&info)?;
        } else {
            m.set(ResKey { resref: ResRef::from_str("test_path")?, restype: ResType::NSS }, test_script(&water).into_bytes());
        }
        std::fs::write(out_dir.join(format!("{}.mod", hak_name())), m.to_archive_bytes()?)?;
        let names: Vec<String> = grid.tiles.iter().map(|p| ttr.tiles.get(p.tile as usize).map_or_default(|t| t.model.clone())).collect();
        println!("control module (ttr01 + water): {}", names.join(" "));
        return Ok(());
    }
    let area = add_area_of(&mut m, &game, &spec, &tileset, &mut rng)?;
    let are_key = ResKey { resref: area, restype: ResType::ARE };
    let mut are = m.gff(&are_key).context("no area")??;
    let tiles = are.root.list_mut("Tile_List").context("no Tile_List")?;
    for (i, t) in tiles.iter_mut().enumerate() {
        t.set("Tile_ID", Value::Int(i as i32));
        t.set("Tile_Orientation", Value::Int(0));
        t.set("Tile_Height", Value::Int(tile_level(i % tiles_x(), i / tiles_x())));
        // As the toolset sets them (a tile's animations loop), so the builder's test module and the published area agree.
        for k in ["Tile_AnimLoop1", "Tile_AnimLoop2", "Tile_AnimLoop3"] {
            t.set(k, Value::Byte(1));
        }
    }
    if let Some(a) = heightmap::current().and_then(|h| h.atmosphere.clone()) {
        atmosphere::apply(&mut are.root, &a)?;
    }
    // A cave: interior, underground, natural; no sky, no edge scenery.
    let underground = heightmap::current().is_some_and(|h| h.underground);
    // Flags: 1 interior, 2 underground, 4 natural.
    let interior = heightmap::current().is_some_and(|h| h.interior);
    are.root.set(
        "Flags",
        Value::Dword(if interior {
            1
        } else if underground {
            7
        } else {
            4
        }),
    );
    if underground {
        are.root.set("SkyBox", Value::Byte(0));
        are.root.set("TileBrdrDisabled", Value::Byte(1));
    }
    // An imported area: the original's identity, scripts and objects. (LS_NO_ORIGINAL: without them, to test pathing alone.)
    let original = import_name().filter(|_| std::env::var_os("LS_NO_ORIGINAL").is_none()).and_then(|n| import::original(&n));
    if let Some(o) = &original {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        import::adopt(o, &mut are.root, &mut git.root);
        // (The original's sky, unless this is now a cave: a new area starts with none.)
        if !underground && let Some(sky) = o.are.get("SkyBox") {
            are.root.set("SkyBox", sky.clone());
        }
        m.set_gff(git_key, &git)?;
    }
    // The area's own settings from the editor (after an import's originals, so what was changed wins).
    if let Some(sett) = heightmap::current().and_then(|h| h.settings.clone()) {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        settings::apply(&mut are.root, &mut git.root, &sett)?;
        m.set_gff(git_key, &git)?;
    }
    m.set_gff(are_key, &are)?;
    // Object-mode placeables, to use and script.
    let placeables = objects::instances(&game)?;
    if !placeables.is_empty() {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        let mut list = git.root.list("Placeable List").map_or_default(<[_]>::to_vec);
        list.extend(placeables);
        git.root.set("Placeable List", Value::List(list));
        m.set_gff(git_key, &git)?;
    }
    // The things the editor placed: creatures, doors, triggers, waypoints, sounds...
    let things = things::instances(&game)?;
    if !things.is_empty() {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        for (list, s) in things {
            let mut items = git.root.list(list).map_or_default(<[_]>::to_vec);
            items.push(s);
            git.root.set(list, Value::List(items));
        }
        m.set_gff(git_key, &git)?;
    }
    // Entrances' doors, to link in the toolset.
    let doors = entrance::doors(&game)?;
    if !doors.is_empty() {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        let mut list = git.root.list("Door List").map_or_default(<[_]>::to_vec);
        list.extend(doors);
        git.root.set("Door List", Value::List(list));
        m.set_gff(git_key, &git)?;
    }
    let mut info = m.info()?;
    // (An imported area's module haks after ours: its objects come from them.)
    let mut list = vec![hak_name(), shared::HAK.to_string()];
    list.extend(original.iter().flat_map(|o| o.haks.iter().cloned()));
    let haks = mg_module::attach::hak_list(&info.root, &list);
    info.root.set("Mod_HakList", haks);
    if let Some(t) = original.as_ref().and_then(|o| o.tlk.as_ref()) {
        info.root.set("Mod_CustomTlk", Value::String(t.as_bytes().to_vec()));
    }
    m.set_info(&info)?;
    m.set_start(area, start_point(), std::f32::consts::FRAC_PI_2)?;
    // Some(""): no test script (an export for building, not for mg test).
    if test != Some("") {
        m.set(
            ResKey { resref: ResRef::from_str("test_path")?, restype: ResType::NSS },
            test.map_or_else(|| test_script(&TEST_WALKS), str::to_string).into_bytes(),
        );
    }
    if std::env::var_os("LS_ONLOAD").is_some() {
        // A plain OnModuleLoad that runs the detour walk, for tracing under gdb.
        let keep = std::env::var("LS_CASE").unwrap_or_else(|_| "detour".into());
        let walks: Vec<TestWalk> = TEST_WALKS.into_iter().filter(|(case, _)| *case == keep).collect();
        let script = onload_script(&walks);
        m.remove(&ResKey { resref: ResRef::from_str("test_path")?, restype: ResType::NSS });
        m.set(ResKey { resref: ResRef::from_str("ls_onload")?, restype: ResType::NSS }, script.into_bytes());
        let mut info = m.info()?;
        info.root.set("Mod_OnModLoad", Value::resref(ResRef::from_str("ls_onload")?));
        m.set_info(&info)?;
    }
    let mod_path = out_dir.join(format!("{}.mod", hak_name()));
    std::fs::write(&mod_path, m.to_archive_bytes()?)?;
    println!("module: {} (area {area})", mod_path.display());
    Ok(())
}
