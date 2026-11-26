// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Saved versions of a builder's area, kept in the team's Gitea.
//!
//! "Save version" commits the editor's copy of an area to the branch
//! `build/<user>/<area>` of a repository (`LANDSCAPER_GITEA_REPO`, required) as `<user>/<area>/area.json` (and `source.json`, where it
//! came from, for one built from an existing area). Autosave never goes
//! there. The repository should be readable by admins only: an area built
//! from a production one carries its ground.

use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use serde_json::{Value, json};

struct Config {
    url: String,
    token: String,
    repo: String,
}

fn config() -> Result<Config> {
    let var = |k: &str| crate::config::setting(k).filter(|v| !v.is_empty());
    Ok(Config {
        url: var("LANDSCAPER_GITEA_URL")
            .context("saving versions isn't set up (LANDSCAPER_GITEA_URL, LANDSCAPER_GITEA_TOKEN)")?
            .trim_end_matches('/')
            .to_string(),
        token: var("LANDSCAPER_GITEA_TOKEN").context("saving versions isn't set up (LANDSCAPER_GITEA_TOKEN)")?,
        repo: var("LANDSCAPER_GITEA_REPO")
            .context("saving versions isn't set up (LANDSCAPER_GITEA_REPO, the repository to keep them in)")?,
    })
}

/// Whether saving versions is set up (Gitea's address, token and repository): the editor shows the button only then.
pub fn enabled() -> bool {
    config().is_ok()
}

fn call(c: &Config, method: &str, path: &str, body: Option<Value>) -> Result<Option<Value>> {
    let req = ureq::request(method, &format!("{}/api/v1{path}", c.url))
        .set("Authorization", &format!("token {}", c.token))
        .timeout(Duration::from_secs(30));
    let r = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match r {
        Ok(r) => Ok(Some(r.into_json().unwrap_or(Value::Null))),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(code, r)) => {
            anyhow::bail!("Gitea said {code}: {}", r.into_string().unwrap_or_default().chars().take(200).collect::<String>())
        }
        Err(e) => anyhow::bail!("Gitea didn't answer: {}", crate::remote::why(&e)),
    }
}

/// Commits `files` (path, bytes) to `branch`, making the branch from the default one if it is new. Returns the commit's address.
pub fn commit(branch: &str, files: &[(String, Vec<u8>)], message: &str, author: &str) -> Result<Value> {
    let c = config()?;
    let repo = &c.repo;
    let info = call(&c, "GET", &format!("/repos/{repo}"), None)?.with_context(|| format!("Gitea has no repository {repo}"))?;
    let default = info["default_branch"].as_str().unwrap_or("main").to_string();
    let enc = urlencode(branch);
    if call(&c, "GET", &format!("/repos/{repo}/branches/{enc}"), None)?.is_none() {
        call(&c, "POST", &format!("/repos/{repo}/branches"), Some(json!({ "new_branch_name": branch, "old_branch_name": default })))?
            .with_context(|| format!("can't make branch {branch} (is {repo} empty?)"))?;
    }
    let mut ops = Vec::new();
    for (path, bytes) in files {
        let existing = call(&c, "GET", &format!("/repos/{repo}/contents/{}?ref={enc}", urlencode_path(path)), None)?;
        let mut op = json!({ "path": path, "content": base64::engine::general_purpose::STANDARD.encode(bytes) });
        match existing.as_ref().and_then(|e| e["sha"].as_str()) {
            Some(sha) => {
                op["operation"] = "update".into();
                op["sha"] = sha.into();
            }
            None => op["operation"] = "create".into(),
        }
        ops.push(op);
    }
    let who = json!({ "name": author, "email": format!("{author}@{}", crate::config::noreply_domain()) });
    let r = call(
        &c,
        "POST",
        &format!("/repos/{repo}/contents"),
        Some(json!({ "branch": branch, "message": message, "files": ops, "author": who, "committer": who })),
    )?
    .context("Gitea refused the commit")?;
    Ok(json!({ "branch": branch, "commit": r["commit"]["sha"], "url": r["commit"]["html_url"] }))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn urlencode_path(s: &str) -> String {
    s.split('/').map(urlencode).collect::<Vec<_>>().join("/")
}
