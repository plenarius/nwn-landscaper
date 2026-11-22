// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Publishing an area: what a builder built, as pull requests on the team's repositories (through the review service).
//!
//! The team server holds no GitHub token. It packs the area's files and sends them to the review service
//! (`/api/v1/areas/publish`), which checks them and opens the PRs. Packed here:
//!
//! - the area's ARE, GIT and GIC from the module the last export installed, as nwn_gff JSON,
//!   under the resref the module will know it by (an area built from an existing one keeps that
//!   one's resref and replaces it), with the builder's hidden mark taken out (it is for
//!   tracing leaks, not for the module);
//! - the area's tile files (its hak), without the textures an import copied from the game or the module's haks;
//! - the shared texture library hak, which the review service sends on only if the module's haks lack it.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_module::Module;
use mg_resman::ResKey;
use serde_json::{Value, json};

use crate::marker;

/// GFF bytes as nwn_gff's JSON text.
pub(crate) fn gff_to_json(bytes: &[u8], ext: &str) -> Result<Value> {
    let dir = std::env::temp_dir().join(format!("ls-publish-{}-{}", std::process::id(), fastrand::u32(..)));
    std::fs::create_dir_all(&dir)?;
    let r = (|| {
        let src = dir.join(format!("a.{ext}"));
        let out = dir.join(format!("a.{ext}.json"));
        std::fs::write(&src, bytes)?;
        let st = std::process::Command::new("nwn_gff")
            .args(["-i", &src.to_string_lossy(), "-o", &out.to_string_lossy()])
            .status()
            .context("nwn_gff isn't installed")?;
        anyhow::ensure!(st.success(), "nwn_gff could not convert the {ext}");
        Ok(serde_json::from_slice(&std::fs::read(&out)?)?)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    r
}

/// Takes the builder's mark (`ls_ref` variables and the `ref:` line in Comments) out of a GFF JSON tree.
fn strip_mark(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if let Some(list) = map.get_mut("VarTable").and_then(|t| t.get_mut("value")).and_then(Value::as_array_mut) {
                let ours = |n: &str| n == marker::VAR || n == crate::things::MARK || n == crate::things::ORIG;
                list.retain(|e| !e["Name"]["value"].as_str().is_some_and(ours));
            }
            if map.get("VarTable").is_some_and(|t| t["value"].as_array().is_some_and(Vec::is_empty)) {
                map.remove("VarTable");
            }
            if let Some(c) = map.get_mut("Comments").and_then(|c| c.get_mut("value"))
                && let Some(text) = c.as_str()
            {
                let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with("ref:")).collect();
                *c = Value::String(kept.join("\n"));
            }
            for child in map.values_mut() {
                strip_mark(child);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_mark),
        _ => {}
    }
}

fn zip_of(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        z.start_file(name, opts)?;
        z.write_all(data)?;
    }
    Ok(z.finish()?.into_inner())
}

/// Every resource of a hak as (`resref.ext`, bytes).
fn hak_files(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let data = std::fs::read(path).with_context(|| format!("no {}", path.display()))?;
    let erf = mg_erf::Erf::read(&data)?;
    erf.entries.iter().map(|e| Ok((e.filename(), erf.data(e)?.into_owned()))).collect()
}

/// A multipart/form-data body: text fields, then files (field, file name, bytes).
fn multipart(fields: &[(&str, String)], files: &[(&str, &str, Vec<u8>)]) -> (String, Vec<u8>) {
    let boundary = format!("----ls{:016x}", fastrand::u64(..));
    let mut body = Vec::new();
    for (k, v) in fields {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    for (k, name, data) in files {
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"; filename=\"{name}\"\r\nContent-Type: application/zip\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend(data);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// One area ready to send: where it will be in nwn-module, whether it replaces an existing area, its files.
struct Packed {
    name: String,
    target: String,
    rework: bool,
    module_files: Vec<(String, Vec<u8>)>,
    tiles: Vec<(String, Vec<u8>)>,
    /// The builder's new blueprints this area uses (`<resref>.<ext>.json`).
    blueprints: Vec<(String, Vec<u8>)>,
}

/// Where `name`'s last export is: the module it installed, the area on its tileset, and what the area will be in
/// nwn-module (its own name, or the production area it replaces).
struct Exported {
    module: Module,
    res: mg_core::ResRef,
    target: String,
    rework: bool,
    user_dir: std::path::PathBuf,
}

fn exported(name: &str) -> Result<Exported> {
    let exports = crate::workspace::exports_dir();
    let header: Value = serde_json::from_slice(
        &std::fs::read(exports.join(format!("{name}.json"))).with_context(|| format!("{name} hasn't been exported yet"))?,
    )?;
    let module = header["module"].as_str().unwrap_or(name).to_string();
    let user_dir = crate::workspace::nwn_user_dir().context("no module folder")?;
    let installed = user_dir.join("modules").join(format!("{module}.mod"));
    anyhow::ensure!(installed.is_file(), "export {name} to NWN first: publishing sends what the last export built");

    // The area on this tileset, from the module the export installed.
    let m = Module::open(&installed)?;
    let mut found = None;
    for a in m.areas()? {
        let Some(Ok(are)) = m.gff(&ResKey { resref: a, restype: ResType::ARE }) else { continue };
        if are.root.resref("Tileset").is_some_and(|t| t.to_string().eq_ignore_ascii_case(name)) {
            anyhow::ensure!(found.is_none(), "more than one area in {module}.mod is on {name}'s tileset");
            found = Some(a);
        }
    }
    let res = found.with_context(|| format!("{module}.mod has no area on {name}'s tileset: export again"))?;
    // An area built from an existing one replaces it; any other is new.
    let rework = crate::import::recorded(name);
    let target = rework.clone().unwrap_or_else(|| name.to_string());
    Ok(Exported { module: m, res, target, rework: rework.is_some(), user_dir })
}

/// What a reviewer would find wrong with `name`'s last export (see `validate`).
pub fn findings(name: &str) -> Result<Vec<crate::validate::Finding>> {
    let e = exported(name)?;
    let gff = |t: ResType| e.module.gff(&ResKey { resref: e.res, restype: t }).and_then(Result::ok);
    let are = gff(ResType::ARE).context("the exported area has no ARE")?;
    let git = gff(ResType::GIT).context("the exported area has no GIT")?;
    // Every waypoint and door tag in the builder's own module (links between its areas), and each door's way back.
    let (mut waypoints, mut doors): (HashMap<String, Vec<String>>, HashMap<String, (Vec<String>, String)>) =
        (HashMap::new(), HashMap::new());
    for a in e.module.areas()? {
        let Some(Ok(g)) = e.module.gff(&ResKey { resref: a, restype: ResType::GIT }) else { continue };
        let tag = |s: &mg_gff::Struct| s.string("Tag").map_or_default(|b| String::from_utf8_lossy(b).to_ascii_lowercase());
        for w in g.root.list("WaypointList").unwrap_or(&[]) {
            waypoints.entry(tag(w)).or_default().push(a.to_string());
        }
        for d in g.root.list("Door List").unwrap_or(&[]) {
            let link = d.string("LinkedTo").map_or_default(|b| String::from_utf8_lossy(b).into_owned());
            doors.entry(tag(d)).or_insert_with(|| (Vec::new(), link)).0.push(a.to_string());
        }
    }
    let mut local_links: HashMap<String, Vec<String>> = HashMap::new();
    for a in e.module.areas()? {
        let Some(Ok(g)) = e.module.gff(&ResKey { resref: a, restype: ResType::GIT }) else { continue };
        for l in ["Door List", "TriggerList"] {
            for o in g.root.list(l).unwrap_or(&[]) {
                let to = o.string("LinkedTo").map_or_default(|b| String::from_utf8_lossy(b).to_ascii_lowercase());
                if !to.is_empty() && o.integer("LinkedToFlags").unwrap_or(0) != 0 {
                    local_links.entry(to).or_default().push(a.to_string());
                }
            }
        }
    }
    let refs = crate::review::refs();
    let game = crate::models::game().ok();
    let has = |n: &str, t: ResType| game.is_some_and(|g| g.resman.get_named(n, t).is_ok());
    let ctx = crate::validate::Ctx {
        target: &e.target,
        is_new: !e.rework,
        local_waypoints: &waypoints,
        local_doors: &doors,
        local_links: &local_links,
        refs: refs.as_ref(),
        game_script: &|n| has(n, ResType::NCS),
        game_dialog: &|n| has(n, ResType::DLG),
    };
    let mut out = crate::validate::check(&are.root, &git.root, &ctx);
    // The builder's own scripts this area (or what it uses) names must compile: a broken script would reach the live server.
    if let Some(g) = game {
        let docs: Vec<Value> =
            [(&are, "are"), (&git, "git")].iter().filter_map(|(f, ext)| gff_to_json(&f.to_bytes().ok()?, ext).ok()).collect();
        #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "used_by() writes script names as `<resref>.nss`")]
        for (file, text) in crate::newbp::used_by(&docs.iter().collect::<Vec<_>>()).into_iter().filter(|(n, _)| n.ends_with(".nss")) {
            let resref = file.trim_end_matches(".nss");
            match crate::newscript::check(g, resref, &text) {
                Ok(r) if r["ok"] == true => {}
                Ok(r) => out.push(crate::validate::Finding {
                    level: "error",
                    what: resref.to_string(),
                    msg: format!("your script {resref} doesn't compile: {}", r["error"]["message"].as_str().unwrap_or("")),
                }),
                Err(e) => out.push(crate::validate::Finding {
                    level: "error",
                    what: resref.to_string(),
                    msg: format!("your script {resref} couldn't be checked: {e:#}"),
                }),
            }
        }
    }
    Ok(out)
}

/// `name`'s last export from the module it installed, as the files the review service takes.
fn pack_area(name: &str) -> Result<Packed> {
    let e = exported(name)?;
    let (m, res, target, user_dir) = (&e.module, e.res, e.target.clone(), &e.user_dir);
    let mut module_files = Vec::new();
    for (t, ext) in [(ResType::ARE, "are"), (ResType::GIT, "git"), (ResType::GIC, "gic")] {
        let Some(Ok(g)) = m.gff(&ResKey { resref: res, restype: t }) else {
            anyhow::ensure!(t == ResType::GIC, "the exported area has no {ext}");
            continue;
        };
        let mut json = gff_to_json(&g.to_bytes()?, ext)?;
        strip_mark(&mut json);
        if t == ResType::ARE {
            json["ResRef"] = json!({ "type": "resref", "value": target });
        }
        module_files.push((format!("{target}.{ext}.json"), serde_json::to_vec_pretty(&json)?));
    }
    let tiles = own_files(hak_files(&user_dir.join("hak").join(format!("{name}.hak")))?, &import_textures(name));
    let docs: Vec<Value> = module_files.iter().filter_map(|(_, b)| serde_json::from_slice(b).ok()).collect();
    let blueprints = crate::newbp::used_by(&docs.iter().collect::<Vec<_>>()).into_iter().map(|(n, t)| (n, t.into_bytes())).collect();
    Ok(Packed { name: name.to_string(), target, rework: e.rework, module_files, tiles, blueprints })
}

/// The texture files imports copied from the game and the module's haks (`<import>/tex/`, lower case): an export puts
/// them in its hak so it looks right on its own, but they are the game's, so a publish doesn't send them.
fn import_textures(name: &str) -> std::collections::HashSet<String> {
    let mut dirs = vec![crate::paint::import_dir(name).join("tex")];
    for e in std::fs::read_dir(crate::workspace::exports_dir()).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().ends_with(".import") {
            dirs.push(e.path().join("tex"));
        }
    }
    dirs.iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect()
}

/// The hak's files the Landscaper made: its textures that are copies an import made are left out.
fn own_files(files: Vec<(String, Vec<u8>)>, copied: &std::collections::HashSet<String>) -> Vec<(String, Vec<u8>)> {
    const TEXTURES: [&str; 3] = [".dds", ".tga", ".txi"];
    files
        .into_iter()
        .filter(|(f, _)| {
            let f = f.to_lowercase();
            !(TEXTURES.iter().any(|x| f.ends_with(x)) && copied.contains(&f))
        })
        .collect()
}

/// The shared texture library the exports carry (the review service sends it on only if the module's haks lack it).
fn library() -> Option<Vec<(String, Vec<u8>)>> {
    let user_dir = crate::workspace::nwn_user_dir()?;
    hak_files(&user_dir.join("hak").join(format!("{}.hak", crate::shared::HAK))).ok()
}

fn send(path: &str, fields: &[(&str, String)], files: &[(String, String, Vec<u8>)]) -> Result<Value> {
    let (url, key) = crate::review::config().context("publishing isn't set up on this server (the review service)")?;
    let borrowed: Vec<(&str, &str, Vec<u8>)> = files.iter().map(|(f, n, d)| (f.as_str(), n.as_str(), d.clone())).collect();
    let (ctype, body) = multipart(fields, &borrowed);
    let r = ureq::post(&format!("{url}{path}"))
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", &ctype)
        .timeout(std::time::Duration::from_secs(900))
        .send_bytes(&body);
    match r {
        Ok(r) => Ok(r.into_json()?),
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_json::<Value>().ok().and_then(|v| v["detail"].as_str().map(str::to_string)).unwrap_or_default();
            anyhow::bail!("the publishing service said {code}: {detail}")
        }
        Err(e) => anyhow::bail!("the publishing service didn't answer: {}", crate::remote::why(&e)),
    }
}

/// Packs `name`'s last export and sends it to the review service. With `dry_run`, the review service only checks
/// it and says what the PRs would hold.
pub fn publish(name: &str, note: &str, dry_run: bool) -> Result<Value> {
    crate::review::config().context("publishing isn't set up on this server (the review service)")?;
    let a = pack_area(name)?;
    let fields = [
        ("user", crate::workspace::acting_user()),
        ("name", name.to_string()),
        ("area", a.target.clone()),
        ("new", (!a.rework).to_string()),
        ("note", note.chars().take(300).collect()),
        ("dry_run", dry_run.to_string()),
    ];
    let mut files = vec![
        ("module_files".to_string(), "module.zip".to_string(), zip_of(&a.module_files)?),
        ("hak_files".to_string(), "tiles.zip".to_string(), zip_of(&a.tiles)?),
    ];
    if let Some(lib) = library() {
        files.push(("library_files".to_string(), "library.zip".to_string(), zip_of(&lib)?));
    }
    if !a.blueprints.is_empty() {
        files.push(("blueprint_files".to_string(), "blueprints.zip".to_string(), zip_of(&a.blueprints)?));
    }
    send("/api/v1/areas/publish", &fields, &files)
}

/// Several areas of one module as one pair of PRs.
pub fn publish_module(names: &[String], note: &str, dry_run: bool) -> Result<Value> {
    crate::review::config().context("publishing isn't set up on this server (the review service)")?;
    anyhow::ensure!(!names.is_empty(), "no areas to publish");
    let mut meta = Vec::new();
    let mut files = Vec::new();
    let mut blueprints: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    for n in names {
        let a = pack_area(n).with_context(|| n.clone())?;
        blueprints.extend(a.blueprints.iter().cloned());
        meta.push(module_entry(&a, &area_folder(&a.target)));
        files.push((format!("module_{}", a.name), "module.zip".to_string(), zip_of(&a.module_files)?));
        files.push((format!("hak_{}", a.name), "tiles.zip".to_string(), zip_of(&a.tiles)?));
    }
    if let Some(lib) = library() {
        files.push(("library_files".to_string(), "library.zip".to_string(), zip_of(&lib)?));
    }
    if !blueprints.is_empty() {
        files.push(("blueprint_files".to_string(), "blueprints.zip".to_string(), zip_of(&blueprints.into_iter().collect::<Vec<_>>())?));
    }
    let fields = [
        ("user", crate::workspace::acting_user()),
        ("note", note.chars().take(300).collect()),
        ("dry_run", dry_run.to_string()),
        ("areas", serde_json::to_string(&meta)?),
    ];
    send("/api/v1/areas/publish-module", &fields, &files)
}

/// One area of `publish-module`'s `areas` list: its `kind` is the group its files go in, as in a package ([`area_folder`]).
fn module_entry(a: &Packed, kind: &str) -> Value {
    json!({ "name": a.name, "area": a.target, "new": !a.rework, "kind": kind })
}

/// Where a new area's files go in the module's source: the folder the same area is already in (in the module checkout), else
/// `LANDSCAPER_AREA_FOLDER` (see config::Layout). Relative to the areas folder; empty when areas sit directly in it.
fn area_folder(target: &str) -> String {
    let layout = crate::config::Layout::from_env();
    area_folder_in(&layout, &layout.areas_dir(), target)
}

/// [`area_folder`] with the layout and the checkout's areas folder given.
fn area_folder_in(layout: &crate::config::Layout, areas: &Path, target: &str) -> String {
    for e in std::fs::read_dir(areas).into_iter().flatten().flatten() {
        if e.path().join(format!("{target}.are.json")).is_file() {
            return e.file_name().to_string_lossy().into_owned();
        }
    }
    layout.area_folder.clone()
}

/// What `names`' last exports are, as a package for a team to commit to its own module: files in the layout of the module's
/// source (nasher's JSON), `haks/<area>/` for each area's tile files, and a `MANIFEST.md`. Returns the files and a plan of
/// what is in it (the shape Publish's check shows).
pub fn package(names: &[String]) -> Result<(Vec<(String, Vec<u8>)>, Value)> {
    anyhow::ensure!(!names.is_empty(), "no areas to package");
    let layout = crate::config::Layout::from_env();
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    let mut manifest = String::from(
        "# Landscaper package\n\nFiles in the layout of the module's source (nasher's JSON): copy them into the module's repository, \
         build the haks, and commit.\n\n",
    );
    let (mut areas, mut new_areas, mut tile_files) = (Vec::new(), Vec::new(), 0usize);
    let mut blueprints: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    let mut notes = Vec::new();
    for n in names {
        let a = pack_area(n).with_context(|| n.clone())?;
        let folder = area_folder(&a.target);
        for (f, b) in &a.module_files {
            files.insert(crate::config::rel(&[&layout.areas, &folder, f]), b.clone());
        }
        for (f, b) in &a.tiles {
            files.insert(format!("haks/{}/{f}", a.name), b.clone());
        }
        tile_files += a.tiles.len();
        blueprints.extend(a.blueprints.iter().cloned());
        manifest += &format!(
            "- **{}** ({}): `{}/`, tile files in `haks/{}/` (put them in a hak that your module lists)\n",
            a.target,
            if a.rework { "replaces the area of that name" } else { "a new area" },
            crate::config::rel(&[&layout.areas, &folder]),
            a.name
        );
        areas.push(a.target.clone());
        if !a.rework {
            new_areas.push(a.target.clone());
        }
    }
    if let Some(lib) = library() {
        for (f, b) in lib {
            files.insert(format!("haks/{}/{f}", crate::shared::HAK), b);
        }
        manifest +=
            &format!("- the shared texture library every Landscaper area uses: `haks/{}/` (one hak for all areas)\n", crate::shared::HAK);
    }
    for (f, b) in &blueprints {
        let path = if let Some(ext) = f.strip_suffix(".json").and_then(|s| s.rsplit('.').next()) {
            crate::config::rel(&[&layout.blueprint_rel(ext), f])
        } else {
            crate::config::rel(&[&layout.scripts, "landscaper", f])
        };
        files.insert(path, b.clone());
    }
    if !blueprints.is_empty() {
        manifest += &format!(
            "- new blueprints, conversations and scripts: {}\n",
            blueprints.keys().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")
        );
    }
    // A new area goes into the module's area list: written out when there is a checkout to start from.
    if !new_areas.is_empty() {
        let ifo_path = layout.ifo_path();
        match std::fs::read(&ifo_path).ok().and_then(|d| serde_json::from_slice::<Value>(&d).ok()) {
            Some(mut ifo) => {
                let list = ifo["Mod_Area_list"]["value"].as_array_mut();
                if let Some(list) = list {
                    for a in &new_areas {
                        if !list.iter().any(|x| x["Area_Name"]["value"] == json!(a)) {
                            list.push(json!({ "__struct_id": 6, "Area_Name": { "type": "resref", "value": a } }));
                        }
                    }
                    files.insert(layout.ifo.clone(), (serde_json::to_string_pretty(&ifo)? + "\n").into_bytes());
                    manifest += &format!("- `{}`: your module's with the new area(s) added to `Mod_Area_list`\n", layout.ifo);
                }
            }
            None => {
                manifest += &format!("\nAdd the new area(s) to `Mod_Area_list` in `{}`: {}\n", layout.ifo, new_areas.join(", "));
                notes.push(format!("add {} to Mod_Area_list in module.ifo.json (no module checkout to do it from)", new_areas.join(", ")));
            }
        }
    }
    files.insert("MANIFEST.md".to_string(), manifest.into_bytes());
    let plan = json!({
        "area": areas.join(", "), "areas": areas, "new": new_areas.len() == areas.len(),
        "module_files": files.keys().filter(|k| !k.starts_with("haks/") && *k != "MANIFEST.md").collect::<Vec<_>>(),
        "hak_files": tile_files, "library": library().is_some(), "warnings": notes, "review": "",
    });
    Ok((files.into_iter().collect(), plan))
}

/// A zip of the package's files.
pub fn package_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    zip_of(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_imports_copied_textures_stay_out_of_the_publish() {
        let copied: std::collections::HashSet<String> = ["tcn01_cobb02.dds", "ttr01_runes.txi"].map(String::from).into();
        let files = ["hlint_mai_a01.mdl", "TCN01_COBB02.dds", "ttr01_runes.txi", "hlint_mai_a01.dds", "hlint_mai.set"]
            .map(|f| (f.to_string(), vec![1u8]));
        let kept: Vec<String> = own_files(files.into(), &copied).into_iter().map(|(f, _)| f).collect();
        assert_eq!(kept, ["hlint_mai_a01.mdl", "hlint_mai_a01.dds", "hlint_mai.set"]);
    }

    #[test]
    fn the_mark_is_taken_out_everywhere() {
        let mut v = json!({
            "Comments": { "type": "cexostring", "value": "keep\nref:abc123" },
            "VarTable": { "type": "list", "value": [{ "Name": { "type": "cexostring", "value": "ls_ref" } }] },
            "Placeable List": { "type": "list", "value": [
                { "VarTable": { "type": "list", "value": [
                    { "Name": { "type": "cexostring", "value": "ls_ref" } },
                    { "Name": { "type": "cexostring", "value": "mine" } } ] } },
                { "VarTable": { "type": "list", "value": [{ "Name": { "type": "cexostring", "value": "ls_ref" } }] } } ] },
        });
        strip_mark(&mut v);
        assert_eq!(v["Comments"]["value"], "keep");
        assert!(v.get("VarTable").is_none());
        let list = v["Placeable List"]["value"].as_array().unwrap();
        assert_eq!(list[0]["VarTable"]["value"].as_array().unwrap().len(), 1);
        assert_eq!(list[0]["VarTable"]["value"][0]["Name"]["value"], "mine");
        assert!(list[1].get("VarTable").is_none());
    }

    #[test]
    fn publish_module_sends_the_group_the_areas_files_go_in_not_always_core() {
        // It used to send "kind": "core" whatever the layout said.
        let layout =
            |pairs: &[(&str, &str)]| crate::config::Layout::from(&|k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()));
        let areas = std::env::temp_dir().join(format!("ls-publish-kind-{}-{}", std::process::id(), fastrand::u32(..)));
        std::fs::create_dir_all(areas.join("towns")).unwrap();
        std::fs::write(areas.join("towns/elmwood.are.json"), "{}").unwrap();
        let packed = |target: &str, rework: bool| Packed {
            name: "mine".into(),
            target: target.into(),
            rework,
            module_files: Vec::new(),
            tiles: Vec::new(),
            blueprints: Vec::new(),
        };
        let kind = |l: &crate::config::Layout, target: &str| area_folder_in(l, &areas, target);
        // nasher's layout (the default): new areas in no group.
        let nasher = layout(&[]);
        assert_eq!(
            module_entry(&packed("newarea", false), &kind(&nasher, "newarea")),
            json!({ "name": "mine", "area": "newarea", "new": true, "kind": "" })
        );
        // The folders layout, and a group of the team's own.
        assert_eq!(module_entry(&packed("newarea", false), &kind(&layout(&[("LANDSCAPER_LAYOUT", "folders")]), "newarea"))["kind"], "core");
        let towns = layout(&[("LANDSCAPER_LAYOUT", "folders"), ("LANDSCAPER_AREA_FOLDER", "/wild/")]);
        assert_eq!(module_entry(&packed("newarea", false), &kind(&towns, "newarea"))["kind"], "wild");
        // Set but empty: no group, even with the folders layout.
        assert_eq!(kind(&layout(&[("LANDSCAPER_LAYOUT", "folders"), ("LANDSCAPER_AREA_FOLDER", "")]), "newarea"), "");
        // A rework goes where the area already is in the module checkout.
        assert_eq!(
            module_entry(&packed("elmwood", true), &kind(&nasher, "elmwood")),
            json!({ "name": "mine", "area": "elmwood", "new": false, "kind": "towns" })
        );
        let _ = std::fs::remove_dir_all(&areas);
    }

    #[test]
    fn multipart_has_every_field_and_file() {
        let (ctype, body) = multipart(&[("a", "1".into())], &[("f", "x.zip", vec![1, 2, 3])]);
        let boundary = ctype.split("boundary=").nth(1).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"a\"\r\n\r\n1\r\n") && text.contains("filename=\"x.zip\""));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }
}
