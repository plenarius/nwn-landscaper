// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An export's last step, the same from the command line and the editor: the module goes where the game lists it (the user
//! folder's `modules/`; the hak is already in `hak/`). A module already there, perhaps worked on in the toolset, only takes the
//! new tiles and what changed in the editor since the last export.

use std::path::Path;

use anyhow::{Context, Result};

/// What the last export of the area left, for a re-export to change only what is new. All empty for an export from the
/// command line: the installed module then takes the new tiles and keeps its own light, settings and objects.
#[derive(Default)]
pub(crate) struct Before {
    /// The ground the installed module was last built on, so its objects can follow the terrain's changes.
    pub heights: Option<crate::heightmap::Heightmap>,
    /// Light and fog at the last export.
    pub atmosphere: Option<serde_json::Value>,
    /// The area's settings at the last export.
    pub settings: Option<serde_json::Value>,
    /// An import's original objects (taken out, moved, edited) at the last export.
    pub originals: Option<serde_json::Value>,
    /// Freshly imported and not exported since: the installed module is the import's own.
    pub just_imported: bool,
    /// Whether this export carries the editor's things at all (an older editor's doesn't: then none are touched).
    pub things: bool,
}

/// Installs the module built from `json` (in `<name>_nwn/` beside it) and adds what was done to `report`.
pub(crate) fn install(json: &Path, before: &Before, report: &mut serde_json::Value) -> Result<()> {
    let Some(user) = crate::workspace::nwn_user_dir() else {
        return Ok(());
    };
    let header: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(json)?)?;
    let name = header["name"].as_str().context("the area has no name")?.to_string();
    let module = header["module"].as_str().unwrap_or(&name).to_string();
    let out = json.parent().unwrap_or(Path::new(".")).join(format!("{name}_nwn"));
    let modules = user.join("modules");
    std::fs::create_dir_all(&modules)?;
    let dest = modules.join(format!("{module}.mod"));
    let fresh = out.join(format!("{name}.mod"));
    // A module that hasn't this area yet (another area's): add it.
    let has_area = dest.exists()
        && mg_module::Module::open(&dest).is_ok_and(|m| {
            m.areas().unwrap_or_default().iter().any(|a| {
                m.gff(&mg_resman::ResKey { resref: *a, restype: mg_core::ResType::ARE })
                    .and_then(Result::ok)
                    .and_then(|g| g.root.resref("Tileset"))
                    .is_some_and(|t| t.to_string().eq_ignore_ascii_case(&name))
            })
        });
    let existed = dest.exists();
    // Every change is made to a copy, which replaces the module in one step once all of them worked (see `replace_module`).
    replace_module(&dest, |work| {
        if existed && !has_area {
            let added = crate::update::add_area(work, &fresh, &name)?;
            report["added_to_module"] = serde_json::json!({ "module": module, "area": added });
        } else if existed {
            let new = crate::heightmap::current().context("no terrain loaded")?;
            // Light, fog and settings only when changed in the editor since the last export: otherwise the toolset's stay.
            let atmosphere = header.get("atmosphere").filter(|a| a.is_object());
            let changed = atmosphere.filter(|a| before.atmosphere.as_ref() != Some(*a));
            let settings = header.get("settings").filter(|s| s.is_object());
            let changed_settings = settings.filter(|s| before.settings.as_ref() != Some(*s));
            let changes = crate::update::Changes {
                atmosphere: changed,
                settings: changed_settings,
                originals: header.get("originals"),
                old_originals: before.originals.as_ref(),
                things: before.things,
            };
            let u = crate::update::update_installed(work, &fresh, &name, &name, before.heights.as_ref(), &new, &changes)?;
            if before.just_imported {
                crate::update::adopt_import(work, &fresh, &name)?;
            }
            report["updated"] = serde_json::json!({
                "areas": u.areas,
                "objects": u.objects,
                "atmosphere": changed.is_some(),
                "doors_added": u.doors_added,
                "doors_removed": u.doors_removed,
                "things_added": u.things_added,
                "things_removed": u.things_removed,
                "placeables_added": u.placeables_added,
                "placeables_removed": u.placeables_removed,
                "moved": u.moved,
                "ground_known": before.heights.is_some(),
                "outside": u.outside.iter().map(|(tag, x, y)| serde_json::json!({ "tag": tag, "at": [x, y] })).collect::<Vec<_>>(),
            });
        } else {
            std::fs::copy(&fresh, work).with_context(|| format!("copying {}", fresh.display()))?;
        }
        // On a team server what leaves carries the builder's hidden mark.
        if crate::workspace::team() {
            let id = crate::marker::id(&crate::workspace::acting_user(), &name)?;
            for m in [work, fresh.as_path()] {
                crate::marker::stamp_module(m, &name, &id)?;
            }
            crate::audit::log("marker", Some(&name), true, &id);
        }
        Ok(())
    })?;
    report["installed_module"] = serde_json::json!(dest);
    Ok(())
}

/// Changes the module `dest` through `change`, which gets the path of a copy of it to change in place (nothing there yet when
/// there is no module): the module is replaced by the changed copy in one rename, and only once `change` has worked. The module
/// before is kept as `<module>.mod.bak`. That module is the builder's own (its doors, scripts, other areas, hours of work in the
/// toolset); an export rewrites it up to three times (the update, an import's first export, a team server's mark), and a crash,
/// a kill, a full disk or a failed step between them used to leave it cut off or half changed, with no copy.
pub(crate) fn replace_module(dest: &Path, change: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    let dir = dest.parent().context("the module has no folder")?;
    let file = dest.file_name().context("the module has no file name")?.to_string_lossy().into_owned();
    // (Hidden and not a .mod, so the game never lists it; a name of its own, so two exports never share one.)
    let work = dir.join(format!(".{file}.{}-{}.work", std::process::id(), WORK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let changed = (|| -> Result<()> {
        if dest.exists() {
            std::fs::copy(dest, &work).with_context(|| format!("copying {}", dest.display()))?;
        }
        change(&work)?;
        // (Opened for writing: Windows refuses to flush a file opened read-only, "Access is denied".)
        std::fs::OpenOptions::new()
            .write(true)
            .open(&work)
            .and_then(|f| f.sync_all())
            .with_context(|| format!("writing {}", work.display()))?;
        if dest.exists() {
            // The module before, kept: a hard link costs nothing and needs no room; a copy where links don't work.
            let bak = dir.join(format!("{file}.bak"));
            let _ = std::fs::remove_file(&bak);
            if std::fs::hard_link(dest, &bak).is_err() {
                std::fs::copy(dest, &bak).with_context(|| format!("keeping {}", bak.display()))?;
            }
        }
        std::fs::rename(&work, dest).with_context(|| format!("replacing {}", dest.display()))?;
        Ok(())
    })();
    if changed.is_err() {
        let _ = std::fs::remove_file(&work);
    }
    changed
}

static WORK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_is_replaced_whole_after_every_change_worked_and_the_one_before_is_kept() {
        let dir = std::env::temp_dir().join(format!("ls-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("cave.mod");
        let names = || {
            let mut n: Vec<String> =
                std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            n.sort();
            n
        };
        // A first install: no module yet.
        replace_module(&dest, |work| Ok(std::fs::write(work, b"first")?)).unwrap();
        assert_eq!((std::fs::read(&dest).unwrap(), names()), (b"first".to_vec(), vec!["cave.mod".to_string()]));
        // A change that fails after rewriting the copy twice (the update worked, the import's first export didn't): the module
        // is as it was, and no copy is left.
        let failed = replace_module(&dest, |work| {
            assert_eq!(std::fs::read(work).unwrap(), b"first", "the copy starts as the module");
            std::fs::write(work, b"updated")?;
            std::fs::write(work, b"upd")?;
            anyhow::bail!("adopting the import failed")
        });
        assert!(failed.is_err());
        assert_eq!((std::fs::read(&dest).unwrap(), names()), (b"first".to_vec(), vec!["cave.mod".to_string()]));
        // One that works: the new module, and the one before as .bak.
        replace_module(&dest, |work| Ok(std::fs::write(work, b"second")?)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"second");
        assert_eq!(std::fs::read(dir.join("cave.mod.bak")).unwrap(), b"first");
        assert_eq!(names(), ["cave.mod", "cave.mod.bak"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
