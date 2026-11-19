// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversations a builder writes: a DLG in the builder's workspace (`blueprints/<resref>.dlg.json`, like their other new
//! blueprints), edited through a plain model the editor shows as a tree.
//!
//! The model: `starts` (the lines the NPC may open with), `entries` (what the NPC says) and `replies` (what the player may
//! answer), each with the links to the lines that follow. A line may be reached from several places (a link, `IsChild`
//! 1 in the file). Nodes keep their places between a read and the save that follows it, so what the file holds beyond the
//! model (sounds, animations, quests) is kept for every node that was there; the save drops the lines nothing reaches.

use anyhow::{Context, Result};
use mg_core::StrRef;
use mg_rules::GameData;
use serde_json::{Value as Json, json};

const MAX_NODES: usize = 400;

fn text_of(game: &GameData, v: &Json) -> (String, Option<u64>) {
    let v = &v["value"];
    if let Some(t) = v["0"].as_str() {
        return (t.to_string(), None);
    }
    match v["id"].as_u64() {
        Some(id) => (game.string(StrRef(id as u32)).unwrap_or_default(), Some(id)),
        None => (String::new(), None),
    }
}

fn str_of(v: &Json) -> String {
    v["value"].as_str().unwrap_or("").to_string()
}

fn links(v: &Json) -> Vec<Json> {
    v["value"].as_array().map_or_default(|a| {
        a.iter().map(|l| json!({ "index": l["Index"]["value"].as_u64().unwrap_or(0), "active": str_of(&l["Active"]) })).collect()
    })
}

/// A DLG as the editor's model.
pub fn to_model(game: &GameData, resref: &str, doc: &Json) -> Json {
    let node = |n: &Json, children: &str| {
        let (text, id) = text_of(game, &n["Text"]);
        json!({
            "text": text, "strref": id, "speaker": str_of(&n["Speaker"]), "script": str_of(&n["Script"]),
            "comment": str_of(&n["Comment"]), "links": links(&n[children]),
        })
    };
    let list = |label: &str| doc[label]["value"].as_array().cloned().unwrap_or_default();
    json!({
        "resref": resref,
        "starts": links(&doc["StartingList"]),
        "entries": list("EntryList").iter().map(|n| node(n, "RepliesList")).collect::<Vec<_>>(),
        "replies": list("ReplyList").iter().map(|n| node(n, "EntriesList")).collect::<Vec<_>>(),
        "end": str_of(&doc["EndConversation"]), "abort": str_of(&doc["EndConverAbort"]),
    })
}

/// A new conversation: the NPC says one line.
pub fn blank() -> Json {
    json!({
        "__data_type": "DLG ", "DelayEntry": { "type": "dword", "value": 0 }, "DelayReply": { "type": "dword", "value": 0 },
        "EndConverAbort": { "type": "resref", "value": "" }, "EndConversation": { "type": "resref", "value": "" },
        "EntryList": { "type": "list", "value": [] }, "NumWords": { "type": "dword", "value": 0 },
        "PreventZoomIn": { "type": "byte", "value": 0 }, "ReplyList": { "type": "list", "value": [] },
        "StartingList": { "type": "list", "value": [] },
    })
}

fn loc(text: &str) -> Json {
    json!({ "type": "cexolocstring", "value": if text.is_empty() { json!({}) } else { json!({ "0": text }) } })
}

fn resref_node(v: &str, what: &str) -> Result<Json> {
    let v = v.trim().to_ascii_lowercase();
    anyhow::ensure!(v.is_empty() || crate::newbp::resref_ok(&v), "{what}: '{v}' isn't a script name (up to 16 letters, digits or _)");
    Ok(json!({ "type": "resref", "value": v }))
}

/// One line as a DLG struct: what `old` (the line as the file had it) holds is kept, the model's settings on top.
fn build_node(game: &GameData, old: Option<&Json>, n: &Json, entry: bool, children: Vec<Json>) -> Result<Json> {
    let mut o = old.cloned().unwrap_or_else(|| {
        let mut base = json!({
            "__struct_id": 0, "ActionParams": { "type": "list", "value": [] }, "Animation": { "type": "dword", "value": 0 },
            "AnimLoop": { "type": "byte", "value": 1 }, "Comment": { "type": "cexostring", "value": "" },
            "Delay": { "type": "dword", "value": 4294967295u32 }, "Quest": { "type": "cexostring", "value": "" },
            "Script": { "type": "resref", "value": "" }, "Sound": { "type": "resref", "value": "" },
            "Text": loc(""),
        });
        if entry {
            base["Speaker"] = json!({ "type": "cexostring", "value": "" });
        }
        base
    });
    // A line whose text wasn't changed keeps its string reference (the game's own text).
    let text = n["text"].as_str().unwrap_or("");
    match n["strref"].as_u64() {
        Some(id) if text_of(game, &json!({ "value": { "id": id } })).0 == text => {
            o["Text"] = json!({ "type": "cexolocstring", "value": { "id": id } });
        }
        _ => o["Text"] = loc(text),
    }
    if entry {
        let speaker = n["speaker"].as_str().unwrap_or("");
        anyhow::ensure!(speaker.len() <= 32, "a speaker tag is up to 32 characters");
        o["Speaker"] = json!({ "type": "cexostring", "value": speaker });
    }
    o["Script"] = resref_node(n["script"].as_str().unwrap_or(""), "an action script")?;
    o["Comment"] = json!({ "type": "cexostring", "value": n["comment"].as_str().unwrap_or("").chars().take(200).collect::<String>() });
    o[if entry { "RepliesList" } else { "EntriesList" }] = json!({ "type": "list", "value": children });
    Ok(o)
}

/// Which new number each line gets, and in what order they are reached.
struct Numbering {
    new: [Vec<Option<usize>>; 2],
    order: [Vec<usize>; 2],
}

impl Numbering {
    fn walk(&mut self, side: usize, i: usize, nodes: &[&Vec<Json>; 2]) {
        if i >= nodes[side].len() || self.new[side][i].is_some() {
            return;
        }
        self.new[side][i] = Some(self.order[side].len());
        self.order[side].push(i);
        let kids: Vec<usize> = nodes[side][i]["links"]
            .as_array()
            .map_or_default(|a| a.iter().filter_map(|l| l["index"].as_u64().map(|x| x as usize)).collect());
        for k in kids {
            self.walk(1 - side, k, nodes);
        }
    }
}

/// The model back as a DLG, on top of `old` (the file as it was, when there is one). With `compact`, lines nothing leads to
/// are dropped and the rest numbered from the starts, depth first; otherwise every line keeps its place.
pub fn from_model(game: &GameData, old: &Json, m: &Json, compact: bool) -> Result<Json> {
    let arr = |k: &str| m[k].as_array().cloned().unwrap_or_default();
    let (entries, replies, starts) = (arr("entries"), arr("replies"), arr("starts"));
    anyhow::ensure!(entries.len() + replies.len() <= MAX_NODES, "a conversation of up to {MAX_NODES} lines");
    let idx = |l: &Json| l["index"].as_u64().map(|i| i as usize);
    // Numbering: the order the lines are first reached (0 the NPC's lines, 1 the player's).
    let nodes = [&entries, &replies];
    let mut num = Numbering { new: [vec![None; entries.len()], vec![None; replies.len()]], order: [Vec::new(), Vec::new()] };
    for s in &starts {
        let i = idx(s).context("a start without a line")?;
        anyhow::ensure!(i < entries.len(), "a start leads to a line that isn't there");
        if compact {
            num.walk(0, i, &nodes);
        }
    }
    if !compact {
        // Saved as the editor goes: every line keeps its place (what nothing leads to is dropped when the editor closes).
        num.new = [(0..entries.len()).map(Some).collect(), (0..replies.len()).map(Some).collect()];
        num.order = [(0..entries.len()).collect(), (0..replies.len()).collect()];
    }
    // A line reached again from another parent is a link (`IsChild` 1); starts don't count.
    let mut claimed: [std::collections::HashSet<usize>; 2] = Default::default();
    let mut link = |l: &Json, side: usize, sid: usize, with_child: bool| -> Result<Json> {
        let i = idx(l).context("a link without a line")?;
        let n = num.new[side].get(i).copied().flatten().context("a link leads to a line that isn't there")?;
        let first = !with_child || claimed[side].insert(n);
        let mut o = json!({
            "__struct_id": sid,
            "Active": resref_node(l["active"].as_str().unwrap_or(""), "a condition script")?,
            "ConditionParams": { "type": "list", "value": [] },
            "Index": { "type": "dword", "value": n },
        });
        if with_child {
            o["IsChild"] = json!({ "type": "byte", "value": if first { 0 } else { 1 } });
        }
        Ok(o)
    };
    let mut doc = if old.is_object() { old.clone() } else { blank() };
    let mut starting = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        starting.push(link(s, 0, k, false)?);
    }
    let old_entries = old["EntryList"]["value"].as_array().cloned().unwrap_or_default();
    let old_replies = old["ReplyList"]["value"].as_array().cloned().unwrap_or_default();
    let order = num.order.clone();
    let mut new_entries = Vec::new();
    for &i in &order[0] {
        let kids = entries[i]["links"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(k, l)| link(l, 1, k, true))
            .collect::<Result<Vec<_>>>()?;
        new_entries.push(build_node(game, old_entries.get(i), &entries[i], true, kids)?);
    }
    let mut new_replies = Vec::new();
    for &i in &order[1] {
        let kids = replies[i]["links"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(k, l)| link(l, 0, k, true))
            .collect::<Result<Vec<_>>>()?;
        new_replies.push(build_node(game, old_replies.get(i), &replies[i], false, kids)?);
    }
    doc["StartingList"] = json!({ "type": "list", "value": starting });
    doc["EntryList"] = json!({ "type": "list", "value": new_entries });
    doc["ReplyList"] = json!({ "type": "list", "value": new_replies });
    doc["EndConversation"] = resref_node(m["end"].as_str().unwrap_or(""), "the end script")?;
    doc["EndConverAbort"] = resref_node(m["abort"].as_str().unwrap_or(""), "the abort script")?;
    let words: usize = order[0]
        .iter()
        .map(|&i| &entries[i])
        .chain(order[1].iter().map(|&i| &replies[i]))
        .map(|n| n["text"].as_str().unwrap_or("").split_whitespace().count())
        .sum();
    doc["NumWords"] = json!({ "type": "dword", "value": words });
    Ok(doc)
}

/// The builder's conversations: (resref).
pub fn list() -> Vec<String> {
    crate::newbp::list("dlg").into_iter().map(|(r, _)| r).collect()
}

/// A conversation of the builder's as the model.
pub fn get(game: &GameData, resref: &str) -> Result<Json> {
    let doc: Json =
        serde_json::from_str(&crate::newbp::read("dlg", resref).with_context(|| format!("no conversation {resref} of yours"))?)?;
    Ok(to_model(game, resref, &doc))
}

/// A conversation to look at: yours, else the module's, else the game's, as the model, with `mine` saying which.
pub fn get_any(game: &GameData, resref: &str) -> Result<Json> {
    let resref = resref.to_ascii_lowercase();
    let mine = crate::newbp::read("dlg", &resref).is_some();
    let doc = crate::newbp::source(game, "dlg", &resref)?;
    let mut m = to_model(game, &resref, &doc);
    m["mine"] = json!(mine);
    Ok(m)
}

/// Conversations to pick from: yours, then the module's, by name: `[{resref, source}]`.
pub fn search(q: &str, limit: usize) -> Vec<Json> {
    let q = q.trim().to_lowercase();
    let mut out: Vec<Json> =
        list().into_iter().filter(|n| q.is_empty() || n.contains(&q)).map(|n| json!({ "resref": n, "source": "mine" })).collect();
    if let Some(refs) = crate::review::refs() {
        let mut names: Vec<&String> = refs.dialogs.iter().filter(|n| q.is_empty() || n.contains(&q)).collect();
        names.sort();
        out.extend(names.into_iter().map(|n| json!({ "resref": n, "source": "module" })));
    }
    out.truncate(limit);
    out
}

/// Makes a conversation, a copy of `from` (the module's, the game's or your own) or a new one with a first line.
pub fn create(game: &GameData, resref: &str, from: &str) -> Result<Json> {
    anyhow::ensure!(crate::newbp::resref_ok(resref), "a conversation name is 1 to 16 lower case letters, digits or _");
    anyhow::ensure!(
        crate::newbp::read("dlg", resref).is_none() && !crate::newbp::taken(game, "dlg", resref)?,
        "{resref} is already a conversation: pick another name"
    );
    anyhow::ensure!(list().len() < 100, "that's a lot of new conversations: publish or delete some first");
    let doc = if from.trim().is_empty() {
        let model = json!({ "starts": [{ "index": 0, "active": "" }], "entries": [{ "text": "Hello.", "links": [] }], "replies": [] });
        from_model(game, &blank(), &model, true)?
    } else {
        crate::newbp::source(game, "dlg", &from.to_ascii_lowercase())?
    };
    crate::newbp::write("dlg", resref, &doc)?;
    Ok(to_model(game, resref, &doc))
}

/// Saves the editor's model of one of the builder's conversations.
pub fn save(game: &GameData, resref: &str, model: &Json, compact: bool) -> Result<Json> {
    let old: Json =
        serde_json::from_str(&crate::newbp::read("dlg", resref).with_context(|| format!("no conversation {resref} of yours"))?)?;
    let doc = from_model(game, &old, model, compact)?;
    crate::newbp::write("dlg", resref, &doc)?;
    Ok(to_model(game, resref, &doc))
}

/// A conversation's text with the lines nothing reaches dropped and the rest numbered (what is sent with Publish).
pub fn compacted(text: &str) -> Result<String> {
    let game = crate::models::game()?;
    let doc: Json = serde_json::from_str(text)?;
    let out = from_model(game, &doc, &to_model(game, "", &doc), true)?;
    Ok(serde_json::to_string_pretty(&out)? + "\n")
}

/// Deletes the builder's own conversation `resref` (the module's and the game's are never touched); none there is no error.
pub fn delete(resref: &str) -> Result<()> {
    anyhow::ensure!(crate::newbp::resref_ok(resref), "bad conversation name");
    let _ = std::fs::remove_file(crate::newbp::path("dlg", resref));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // (No game data in unit tests: the strref path is covered by checking a line with no string reference.)
    fn model() -> Json {
        json!({
            "starts": [{ "index": 0, "active": "" }, { "index": 1, "active": "cond_night" }],
            "entries": [
                {
                    "text": "Welcome.", "speaker": "", "script": "", "comment": "",
                    "links": [{ "index": 0, "active": "" }, { "index": 1, "active": "" }],
                },
                { "text": "Back again?", "links": [{ "index": 1, "active": "" }] },
                { "text": "Nobody gets here.", "links": [] },
            ],
            "replies": [
                { "text": "Hi.", "links": [{ "index": 1, "active": "" }] },
                { "text": "Bye.", "script": "act_leave", "links": [] },
            ],
        })
    }

    #[test]
    #[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test -- --ignored"]
    fn unreached_lines_go_and_a_second_arrival_is_a_link() {
        let doc = blank();
        let game = crate::models::game().expect("this test needs the game's files (NWN_ROOT)");
        let d = from_model(game, &doc, &model(), true).unwrap();
        // saved as one goes, nothing is dropped or moved
        let kept = from_model(game, &doc, &model(), false).unwrap();
        assert_eq!(kept["EntryList"]["value"].as_array().unwrap().len(), 3);
        // entry 2 isn't reached from any start: dropped
        assert_eq!(d["EntryList"]["value"].as_array().unwrap().len(), 2);
        assert_eq!(d["ReplyList"]["value"].as_array().unwrap().len(), 2);
        let first = &d["EntryList"]["value"][0]["RepliesList"]["value"];
        assert_eq!(first[0]["IsChild"]["value"], 0);
        assert_eq!(first[1]["IsChild"]["value"], 0);
        // "Hi." is the first line to lead to "Back again?" (a start isn't a parent), so that one is its child...
        let hi = &d["ReplyList"]["value"][0]["EntriesList"]["value"][0];
        assert_eq!(hi["IsChild"]["value"], 0);
        // ...and "Back again?" leading to "Bye." (already a child of "Welcome.") is a link
        let back = &d["EntryList"]["value"][1]["RepliesList"]["value"][0];
        assert_eq!(back["IsChild"]["value"], 1);
        assert_eq!(d["StartingList"]["value"][1]["Active"]["value"], "cond_night");
        assert_eq!(d["ReplyList"]["value"][1]["Script"]["value"], "act_leave");
        assert_eq!(d["NumWords"]["value"], 5);
    }

    #[test]
    fn a_bad_script_name_is_refused() {
        assert!(resref_node("way_too_long_for_a_script_name", "x").is_err());
        assert!(resref_node("Fine_One", "x").is_ok());
    }
}
