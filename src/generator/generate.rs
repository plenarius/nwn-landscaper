// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `generate`: builds the hak and the module (tiles, tileset, area, start point) from the current terrain.

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_erf::ErfWriter;
use mg_gff::Value;
use mg_module::new::{AreaSpec, add_area_of, new_module};
use mg_resman::ResKey;
use mg_set::Tileset;

use super::{
    MINIMAP, Painter, SHADOW_MAP_FINE, bake_size, baked_name, choose_path_nodes, crlf, edge, entrance, hak_name, in_parallel, minimap,
    minimap_name, set_file, shadow_map, shadow_name, shadow_strength, start_point, surfaced, tile_level, tile_model, tile_name, tile_wok,
    tiles_x, tiles_y, tileset_name, walkmesh,
};
use crate::{atmosphere, heightmap, import, models, objects, paint, settings, shared, skies, textures, things, workspace};

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

/// One tile's part of the hak, made on its own thread.
struct TileFiles {
    name: String,
    /// Its walkmesh's faces.
    faces: usize,
    /// The model's text, kept for LS_DUMP only.
    text: Option<String>,
    mtrs: Vec<(String, String)>,
    shadow: Option<mg_image::Rgba>,
    shadow_dds: Option<Vec<u8>>,
    baked: Option<Vec<u8>>,
    mini: mg_image::Rgba,
    compiled: mg_mdl::compile::Compiled,
    wok: Vec<u8>,
}

/// The whole area's minimap from each tile's (row by row from the south), for a look at it outside the game.
fn whole_of(minimaps: &[mg_image::Rgba]) -> mg_image::Rgba {
    let mut whole = mg_image::Rgba::new(MINIMAP * tiles_x() as u32, MINIMAP * tiles_y() as u32);
    let (side, row) = (MINIMAP as usize * 4, (MINIMAP * tiles_x() as u32 * 4) as usize);
    for (i, mini) in minimaps.iter().enumerate() {
        let (tx, ty) = (i % tiles_x(), i / tiles_x());
        for y in 0..MINIMAP as usize {
            let at = (ty * MINIMAP as usize + y) * row + tx * side;
            whole.data[at..at + side].copy_from_slice(&mini.data[y * side..][..side]);
        }
    }
    whole
}

/// An exported module's haks, first highest: `ours` (the area's and the shared one), then `original`'s (an imported area's
/// module's), then the editor's (`editor`), each once (the first place a name comes, whatever its case).
pub(crate) fn module_haks(ours: &[String], original: impl IntoIterator<Item = String>, editor: Vec<String>) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    for h in ours.iter().cloned().chain(original).chain(editor) {
        if !list.iter().any(|l| l.eq_ignore_ascii_case(&h)) {
            list.push(h);
        }
    }
    list
}

/// The haks an area's module lists first: the area's own (`hak`), the shared texture library, and the skies' when the area has a
/// Landscaper sky (`sky`).
pub(crate) fn our_haks(hak: &str, sky: bool) -> Vec<String> {
    let mut list = vec![hak.to_string(), shared::HAK.to_string()];
    if sky {
        list.push(skies::HAK.to_string());
    }
    list
}

/// Generates the loaded area's tileset, hak and module into `out_dir`, on the export's threads. `test` is the module's test
/// script (`test_path`): None, the built-in walks; `Some("")`, none (an export to build on, not for `mg test`).
pub(crate) fn generate(out_dir: &std::path::Path, test: Option<&str>) -> Result<()> {
    super::plan::as_export(|| build(out_dir, test))
}

/// [`generate`]'s work, run as an export (on every core unless `--jobs` says otherwise).
fn build(out_dir: &std::path::Path, test: Option<&str>) -> Result<()> {
    let gen_start = std::time::Instant::now();
    // The editor's game, with its haks: the object-mode placeables' blueprints and the things' come from them, as the editor
    // showed them.
    let game = models::game()?;
    let mut owned: Vec<(ResRef, ResType, Vec<u8>)> = Vec::new();

    let lap = |what: &str| {
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!("  {what} at {:.1}s", gen_start.elapsed().as_secs_f32());
        }
    };
    lap("game opened");
    textures::ensure_starter();
    let painter = Painter::load()?;
    lap("textures loaded");
    let strength = shadow_strength();
    // The library textures, the entrance black and the tile shaders are in the shared hak.
    shared::ensure(&workspace::nwn_user_dir().context("no user dir")?.join("hak"))?;
    lap("shared hak");
    let (mut walk_faces, mut render_bytes, mut most, mut baked) = (0, 0, 0, 0);
    // Each tile's minimap from the terrain alone, row by row from the south: what the hak has until the area is drawn from above.
    let mut terrain_minimaps = Vec::with_capacity(tiles_x() * tiles_y());
    // And its baked shadows.
    let mut shadows = mg_image::Rgba::new(SHADOW_MAP_FINE * tiles_x() as u32, SHADOW_MAP_FINE * tiles_y() as u32);
    let timing = std::env::var_os("LS_TIMING").is_some();
    // LS_DUMP: each baked texture as a PNG and each model's text beside the export, to look at.
    let dump = std::env::var_os("LS_DUMP").is_some();
    // Each tile's files are its own, so every tile is made side by side and the hak is put together in tile order after (the result
    // is the same as one after another).
    let tiles: Vec<(usize, usize)> = (0..tiles_y()).flat_map(|ty| (0..tiles_x()).map(move |tx| (tx, ty))).collect();
    // (The plan first: worked out once, for every tile.)
    super::area_plan();
    // The particle placements, by tile.
    let ambience = super::ambience_plan();
    let pack = crate::ambience::installed().ok();
    if ambience.placements > 0 {
        println!("{}", ambience.summary());
        for s in &ambience.skipped {
            eprintln!(
                "  ambience at ({:.1}, {:.1}){}: {}",
                s.at[0],
                s.at[1],
                if s.count > 1 { format!(" and {} more", s.count - 1) } else { String::new() },
                s.why
            );
        }
    }
    lap("planned");
    let pre_loop = gen_start.elapsed().as_secs_f32();
    let no_surfaces = std::env::var_os("LS_NO_SURFACES").is_some();
    // Thread time of each step, summed over the tiles (LS_TIMING).
    let ph: [std::sync::atomic::AtomicU64; 6] = Default::default();
    let made = in_parallel(&tiles, |(tx, ty)| -> Result<TileFiles> {
        let mut mark = std::time::Instant::now();
        let mut phase = |i: usize| {
            ph[i].fetch_add(mark.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
            mark = std::time::Instant::now();
        };
        let baked = match &painter {
            Some(p) if p.baked(tx, ty) => Some(p.baked_dds(tx, ty, bake_size())),
            _ => None,
        };
        phase(3);
        let shadow = (strength > 0.0).then(|| shadow_map(tx, ty, strength));
        let shadow_dds = shadow.as_ref().map(paint::dds);
        phase(2);
        let name = tile_name(tx, ty);
        let raw = walkmesh(tx, ty);
        let faces = raw.faces.len();
        // The game's surfaces from here on (the plan knows walkable or not).
        // LS_NO_SURFACES: grass and non-walkable only (experiments).
        let w = if no_surfaces { raw } else { surfaced(&raw, tx, ty, painter.as_ref()) };
        phase(0);
        let mut mtrs = Vec::new();
        let particles = pack.zip(ambience.tiles.get(&(tx, ty))).map(|(p, b)| (b.as_slice(), p));
        let text = tile_model(&name, tx, ty, &w, painter.as_ref(), strength > 0.0, &mut mtrs, particles);
        phase(1);
        let mini = minimap(tx, ty, painter.as_ref());
        phase(4);
        let compiled = mg_mdl::compile::compile(text.as_bytes(), &Default::default()).with_context(|| format!("compiling {name}"))?;
        let wok = crlf(&tile_wok(&name, &w));
        phase(5);
        let text = dump.then_some(text);
        Ok(TileFiles { name, faces, text, mtrs, shadow, shadow_dds, baked, mini, compiled, wok })
    });
    let ph = ph.map(|n| n.into_inner() as f32 / 1e9);
    for ((tx, ty), tile) in tiles.iter().copied().zip(made) {
        let TileFiles { name, faces, text, mtrs, shadow, shadow_dds, baked: baked_dds, mini, compiled, wok } = tile?;
        walk_faces += faces;
        most = most.max(faces);
        if let (Some(map), Some(dds)) = (shadow, shadow_dds) {
            // (Each tile at the finer size, a coarse map's pixels doubled.)
            let (n, step) = (SHADOW_MAP_FINE as usize, (SHADOW_MAP_FINE / map.width) as usize);
            let row = n * tiles_x() * 4;
            for y in 0..n {
                for x in 0..n {
                    let at = (ty * n + y) * row + (tx * n + x) * 4;
                    let from = ((y / step) * map.width as usize + x / step) * 4;
                    shadows.data[at..at + 4].copy_from_slice(&map.data[from..from + 4]);
                }
            }
            owned.push((ResRef::from_str(&shadow_name(tx, ty))?, ResType::DDS, dds));
            owned.push((ResRef::from_str(&shadow_name(tx, ty))?, ResType::TXI, b"clamp 3\r\n".to_vec()));
        }
        // (with shadows off, library textures and a chasm's walls still have theirs: the triplanar shader, the fade)
        for (mat, text) in mtrs {
            owned.push((ResRef::from_str(&mat)?, ResType::MTR, text.into_bytes()));
        }
        if let Some(dds) = baked_dds {
            if dump {
                // As the game will decode it.
                paint::write_png(&out_dir.join(format!("{}.png", baked_name(tx, ty))), &mg_image::read(ResType::DDS, &dds)?.to_rgba())?;
            }
            owned.push((ResRef::from_str(&baked_name(tx, ty))?, ResType::DDS, dds));
            // Clamped, so the edges don't pick up the opposite side's pixels.
            owned.push((ResRef::from_str(&baked_name(tx, ty))?, ResType::TXI, b"clamp 3\r\n".to_vec()));
            baked += 1;
        }
        owned.push((ResRef::from_str(&minimap_name(tx, ty))?, ResType::TGA, mg_image::write_tga(&mini)));
        terrain_minimaps.push(mini);
        // (The tile model has no aabb list of its own: Moonglow building one from its faces is expected, not news.)
        for (line, note) in compiled.notes.iter().filter(|(_, n)| !n.ends_with("no aabb list, a tree was made of its faces")) {
            eprintln!("  {name}:{line}: {note}");
        }
        render_bytes += compiled.binary.len();
        if let Some(text) = text {
            std::fs::write(out_dir.join(format!("{name}.mdl.txt")), text)?;
        }
        owned.push((ResRef::from_str(&name)?, ResType::MDL, compiled.binary));
        owned.push((ResRef::from_str(&name)?, ResType::WOK, wok));
    }
    if timing {
        let t = gen_start.elapsed().as_secs_f32();
        eprintln!(
            "  generate per tile (thread time): walkmesh+surfaces {:.1}s, tile model {:.1}s, shadow map {:.1}s, baked texture {:.1}s, \
             minimap {:.1}s, compile+wok {:.1}s (before the tiles {pre_loop:.1}s; so far {t:.1}s)",
            ph[0], ph[1], ph[2], ph[3], ph[4], ph[5]
        );
    }
    if strength > 0.0 {
        paint::write_png(&out_dir.join("shadows.png"), &shadows)?;
    }
    // The scenery the client draws beyond the edge (LS_NO_EDGE: none).
    // (A cave has no edge scenery: its border is off, see generate's area.)
    let underground = heightmap::current().is_some_and(|h| h.underground);
    if std::env::var_os("LS_NO_EDGE").is_none() && !underground {
        for (name, text, mtrs) in edge::models(painter.as_ref()) {
            let compiled = mg_mdl::compile::compile(text.as_bytes(), &Default::default()).with_context(|| format!("compiling {name}"))?;
            if dump {
                std::fs::write(out_dir.join(format!("{name}.mdl.txt")), &text)?;
            }
            owned.push((ResRef::from_str(&name)?, ResType::MDL, compiled.binary));
            for (mat, text) in mtrs {
                owned.push((ResRef::from_str(&mat)?, ResType::MTR, text.into_bytes()));
            }
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
    // The particle pack's own textures, for the kinds baked in.
    if let Some(pack) = pack {
        for (name, data) in pack.textures_for(&ambience.kinds()) {
            owned.push((ResRef::from_str(&name)?, ResType::TGA, data));
        }
    }
    let nodes = choose_path_nodes();
    let set = set_file(&nodes);
    owned.push((ResRef::from_str(&tileset_name())?, ResType::SET, crlf(&set)));
    let hak_path = workspace::nwn_user_dir().context("no user dir")?.join("hak").join(format!("{}.hak", hak_name()));
    let write_hak = |owned: &[(ResRef, ResType, Vec<u8>)]| -> Result<()> {
        let mut hak = ErfWriter::new(*b"HAK ");
        for (r, t, d) in owned {
            hak.add(*r, *t, d.as_slice())?;
        }
        crate::atomic::write(&hak_path, &hak.to_bytes()?)
    };
    write_hak(&owned)?;
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
    let mut m = new_module(game, "Landscaper", &mut rng)?;
    let tileset = Tileset::parse(set.as_bytes(), game.codepage())?;
    // Named after the area: several of them can share a module.
    let area_name = heightmap::current().map_or_else(|| "Landscape".to_string(), |h| h.name.clone());
    let spec = AreaSpec { name: area_name, tileset: ResRef::from_str(&tileset_name())?, width: tiles_x() as u32, height: tiles_y() as u32 };
    if std::env::var_os("LS_CONTROL").is_some() {
        // Control: BioWare's rural tiles, water painted down the middle
        // (corners x=2, y=2..4), dry land to the south to walk around it.
        let spec = AreaSpec { name: "Landscape".into(), tileset: ResRef::from_str("ttr01")?, width: 4, height: 4 };
        let area = mg_module::new::add_area(&mut m, game, &spec, &mut rng)?;
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
        paint::write_png(&out_dir.join("minimap.png"), &whole_of(&terrain_minimaps))?;
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
    let area = add_area_of(&mut m, game, &spec, &tileset, &mut rng)?;
    let are_key = ResKey { resref: area, restype: ResType::ARE };
    let mut are = m.gff(&are_key).context("no area")??;
    let tiles = are.root.list_mut("Tile_List").context("no Tile_List")?;
    for (i, t) in tiles.iter_mut().enumerate() {
        t.set("Tile_ID", Value::Int(i as i32));
        t.set("Tile_Orientation", Value::Int(0));
        t.set("Tile_Height", Value::Int(tile_level(i % tiles_x(), i / tiles_x())));
        // As the toolset sets them (a tile's animations loop), so the builder's test module and the published area agree. Baked
        // ambience that loops is keyed on in `animloop01`, which only plays with these on.
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
    // A Landscaper sky: its row of the skyboxes.2da in ls_skies.hak, made now (after the settings: the area's sky wins).
    let sky = match skies::area_sky() {
        Some(id) => {
            let row = skies::prepare(&id, &workspace::nwn_user_dir().context("no user dir")?.join("hak"))?;
            are.root.set("SkyBox", Value::Byte(row));
            true
        }
        None => false,
    };
    m.set_gff(are_key, &are)?;
    // Object-mode placeables, to use and script.
    let placeables = objects::instances(game)?;
    if !placeables.is_empty() {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        let mut list = git.root.list("Placeable List").map_or_default(<[_]>::to_vec);
        list.extend(placeables);
        git.root.set("Placeable List", Value::List(list));
        m.set_gff(git_key, &git)?;
    }
    // The things the editor placed: creatures, doors, triggers, waypoints, sounds...
    let things = things::instances(game)?;
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
    let doors = entrance::doors(game)?;
    if !doors.is_empty() {
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        let mut git = m.gff(&git_key).context("no GIT")??;
        let mut list = git.root.list("Door List").map_or_default(<[_]>::to_vec);
        list.extend(doors);
        git.root.set("Door List", Value::List(list));
        m.set_gff(git_key, &git)?;
    }
    let mut info = m.info()?;
    // (An imported area's module haks after ours: its objects come from them. Then the editor's haks: the placeables placed
    // and baked from them, their rows, models and textures, are there only with them.)
    let list = module_haks(&our_haks(&hak_name(), sky), original.iter().flat_map(|o| o.haks.iter().cloned()), models::game_haks());
    let haks = mg_module::attach::hak_list(&info.root, &list);
    info.root.set("Mod_HakList", haks);
    if let Some(t) = original.as_ref().and_then(|o| o.tlk.as_ref()) {
        info.root.set("Mod_CustomTlk", Value::String(t.as_bytes().to_vec()));
    }
    // (Else the editor's, with its haks: their placeables' names are rows of it.)
    models::give_tlk(&mut info.root);
    m.set_info(&info)?;
    m.set_start(area, start_point(), std::f32::consts::FRAC_PI_2)?;
    // The minimaps drawn from above by nwn-bake, buildings and all, once the hak and the area are there (LS_MINIMAP=terrain: the
    // terrain's alone). If it fails, the terrain's stay.
    let minimaps = if std::env::var("LS_MINIMAP").is_ok_and(|v| v == "terrain") {
        terrain_minimaps
    } else {
        let start = std::time::Instant::now();
        let git = m.gff(&ResKey { resref: area, restype: ResType::GIT }).transpose()?;
        match super::above::top_view(&list, &area.to_string(), m.gff(&are_key).context("no area")??, git) {
            Ok(top) => {
                let maps = super::above::slice(&top, [tiles_x(), tiles_y()], &terrain_minimaps);
                for (i, mini) in maps.iter().enumerate() {
                    let name = ResRef::from_str(&minimap_name(i % tiles_x(), i / tiles_x()))?;
                    if let Some(e) = owned.iter_mut().find(|(r, t, _)| *r == name && *t == ResType::TGA) {
                        e.2 = mg_image::write_tga(mini);
                    }
                }
                write_hak(&owned)?;
                if timing {
                    eprintln!("  minimaps from above in {:.1}s (drawing {:.1}s)", start.elapsed().as_secs_f32(), top.time.as_secs_f32());
                }
                maps
            }
            Err(e) => {
                eprintln!("  the minimaps are the terrain's alone: drawing the area from above failed: {e:#}");
                terrain_minimaps
            }
        }
    };
    paint::write_png(&out_dir.join("minimap.png"), &whole_of(&minimaps))?;
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

#[cfg(test)]
mod tests {
    /// Ours first, then the imported area's module's, then the editor's; each hak once, where it first comes.
    #[test]
    fn an_exported_module_lists_ours_then_the_originals_then_the_editors_haks() {
        let s = |v: &[&str]| v.iter().map(|h| (*h).to_string()).collect::<Vec<_>>();
        let list = super::module_haks(&s(&["lsa_x", "ls_textures"]), s(&["Orig_Tiles", "plc"]), s(&["PLC", "cacti", "lsa_x"]));
        assert_eq!(list, s(&["lsa_x", "ls_textures", "Orig_Tiles", "plc", "cacti"]));
        assert_eq!(super::module_haks(&s(&["lsa_x", "ls_textures"]), Vec::new(), Vec::new()), s(&["lsa_x", "ls_textures"]));
    }
}
