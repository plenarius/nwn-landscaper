// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scripts a builder with the right writes (an admin, for now): NWScript in the workspace (`scripts/<resref>.nss`), checked
//! by Moonglow's built-in compiler (the one the game and Aurora use) with the module's includes fetched from the review service.
//! Publish sends the ones an area, conversation or creature names (and the ones those include).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_resman::ResKey;
use serde_json::{Value as Json, json};

/// The largest script a builder may save: a limit, so one request can't fill the workspace.
pub const MAX_BYTES: usize = 256 * 1024;

/// The builder's own scripts: `scripts/` in their workspace.
pub fn dir() -> PathBuf {
    crate::workspace::root().join("scripts")
}

fn path(resref: &str) -> PathBuf {
    dir().join(format!("{resref}.nss"))
}

/// The builder's script, or None.
pub fn read(resref: &str) -> Option<String> {
    if !crate::newbp::resref_ok(resref) {
        return None;
    }
    std::fs::read_to_string(path(resref)).ok()
}

/// The builder's scripts, by name.
pub fn list() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".nss").map(str::to_string))
        .collect();
    v.sort();
    v
}

static MODULE: Mutex<Option<HashMap<String, (Instant, Option<String>)>>> = Mutex::new(None);

/// A script of the skeleton module (the review service, or the checkout locally), kept ten minutes.
fn module_script(resref: &str) -> Option<String> {
    if let Some((at, hit)) = MODULE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|m| m.get(resref))
        && at.elapsed() < Duration::from_secs(600)
    {
        return hit.clone();
    }
    let found = if crate::review::active() {
        crate::review::script(resref).ok().flatten()
    } else {
        let root = crate::config::Layout::from_env().scripts_dir();
        walk(&root, resref)
    };
    MODULE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(resref.to_string(), (Instant::now(), found.clone()));
    found
}

fn walk(dir: &std::path::Path, resref: &str) -> Option<String> {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(t) = walk(&p, resref) {
                return Some(t);
            }
        } else if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&format!("{resref}.nss"))) {
            return std::fs::read_to_string(&p).ok();
        }
    }
    None
}

/// A script's source as the compiler should see it: the builder's, the module's, the game's.
pub fn source(game: &mg_rules::GameData, resref: &str) -> Option<Vec<u8>> {
    if let Some(t) = read(resref) {
        return Some(t.into_bytes());
    }
    if let Some(t) = module_script(resref) {
        return Some(t.into_bytes());
    }
    let key = ResKey { resref: mg_core::ResRef::from_str(resref).ok()?, restype: ResType::NSS };
    game.resman.get(&key).ok().map(std::borrow::Cow::into_owned)
}

/// Compiles `text` as script `resref` (which may replace one of the builder's own, but sees the rest). The compiler stops at
/// the first error: `{ ok, error: { file, line, message } }`.
pub fn check(game: &mg_rules::GameData, resref: &str, text: &str) -> Result<Json> {
    anyhow::ensure!(crate::newbp::resref_ok(resref), "a script name is 1 to 16 lower case letters, digits or _");
    anyhow::ensure!(text.len() <= MAX_BYTES, "a script is up to {} KB", MAX_BYTES / 1024);
    let me = (resref.to_string(), text.as_bytes().to_vec());
    let mut compiler = mg_script::Compiler::new(|name: &str, t: ResType| {
        if t != ResType::NSS {
            return None;
        }
        let name = name.to_ascii_lowercase();
        if name == me.0 {
            return Some(me.1.clone());
        }
        source(game, &name)
    });
    // (A conditional script has no `main`.)
    compiler.set_require_entry_point(false);
    Ok(match compiler.compile(resref) {
        Ok(_) => json!({ "ok": true }),
        Err(e) => {
            let (file, line) = e.location().unwrap_or_else(|| (resref.to_string(), 0));
            json!({ "ok": false, "error": { "file": file, "line": line, "message": e.message } })
        }
    })
}

/// A new script: a copy of `from` (the module's, the game's or your own), or a start of the kind asked for.
pub fn create(game: &mg_rules::GameData, resref: &str, from: &str, kind: &str) -> Result<Json> {
    anyhow::ensure!(crate::newbp::resref_ok(resref), "a script name is 1 to 16 lower case letters, digits or _");
    let exists = read(resref).is_some()
        || module_script(resref).is_some()
        || game.resman.contains(&ResKey { resref: mg_core::ResRef::from_str(resref)?, restype: ResType::NSS });
    anyhow::ensure!(!exists, "{resref} is already a script: pick another name");
    anyhow::ensure!(list().len() < 100, "that's a lot of new scripts: publish or delete some first");
    let text = if from.trim().is_empty() {
        match kind {
            "condition" => {
                "// Return TRUE when this line of the conversation should show.\nint StartingConditional()\n{\n    return TRUE;\n}\n"
                    .to_string()
            }
            _ => "void main()\n{\n\n}\n".to_string(),
        }
    } else {
        let from = from.to_ascii_lowercase();
        String::from_utf8(source(game, &from).with_context(|| format!("no script {from}"))?).context("that script isn't text")?
    };
    std::fs::create_dir_all(dir())?;
    std::fs::write(path(resref), &text)?;
    Ok(json!({ "resref": resref, "source": text }))
}

/// Saves the builder's own script `resref` and compiles it; the compiler's answer.
pub fn save(game: &mg_rules::GameData, resref: &str, text: &str) -> Result<Json> {
    anyhow::ensure!(read(resref).is_some(), "no script {resref} of yours");
    anyhow::ensure!(text.len() <= MAX_BYTES, "a script is up to {} KB", MAX_BYTES / 1024);
    std::fs::write(path(resref), text)?;
    check(game, resref, text)
}

/// Deletes the builder's own script `resref` (the module's and the game's are never touched); none there is no error.
pub fn delete(resref: &str) -> Result<()> {
    anyhow::ensure!(crate::newbp::resref_ok(resref), "bad script name");
    let _ = std::fs::remove_file(path(resref));
    Ok(())
}

/// Scripts to pick from: the builder's own, then the module's, `[{resref, source}]` (`mine` or `module`).
pub fn search(q: &str, limit: usize) -> Vec<Json> {
    let q = q.trim().to_lowercase();
    let mut out: Vec<Json> =
        list().into_iter().filter(|n| q.is_empty() || n.contains(&q)).map(|n| json!({ "resref": n, "source": "mine" })).collect();
    if crate::review::active() {
        for s in crate::review::script_list(&q, limit).unwrap_or_default() {
            if let Some(r) = s["resref"].as_str() {
                out.push(json!({ "resref": r, "source": "module", "folder": s["folder"] }));
            }
        }
    }
    out.truncate(limit);
    out
}

/// What `text` includes: `#include "name"`.
pub fn includes(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start().strip_prefix("#include")?.trim();
            Some(l.trim_matches('"').trim().to_ascii_lowercase())
        })
        .filter(|n| !n.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_are_read_from_the_include_lines() {
        let t = "#include \"Inc_One\"\n  #include   \"two\"\n// #include \"commented\"\nvoid main() {}\n#include \"\"\n";
        assert_eq!(includes(t), vec!["inc_one".to_string(), "two".to_string()]);
    }
}
