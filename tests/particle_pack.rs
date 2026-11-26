// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The particle pack (`packs/particles`) keeps its models, `pack.json` and sample `placeables.2da` in step: each kind's model
//! compiles, carries the look `pack.json` says it does, stays within its live particle budget by day and by night, and can be
//! switched on and off as a placeable.

use std::path::{Path, PathBuf};

use mg_mdl::{Model, NodeKind};
use serde_json::Value;

/// What one placement may keep alive, whatever its kind's budget.
const MAX_LIVE: f64 = 60.0;
/// The look's values the pack gives for day and for night, as the model's controllers name them.
const LOOK: [(&str, &str); 5] = [
    ("birthrate", "birthrate"),
    ("colorStart", "colorstart"),
    ("colorEnd", "colorend"),
    ("alphaStart", "alphastart"),
    ("alphaEnd", "alphaend"),
];

fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/particles")
}

fn pack() -> Value {
    serde_json::from_str(&std::fs::read_to_string(pack_dir().join("pack.json")).unwrap()).unwrap()
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a.iter().map(|x| x.as_f64().unwrap()).collect(),
        x => vec![x.as_f64().unwrap()],
    }
}

fn close(a: &[f64], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - f64::from(*y)).abs() < 1e-4)
}

fn model(name: &str) -> (String, Model) {
    let path = pack_dir().join("models").join(format!("{name}.mdl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let model = Model::read(text.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
    (text, model)
}

/// A rest-pose value of a node, by its lower-case controller name.
fn value<'a>(m: &'a Model, node: &str, ctrl: &str) -> &'a [f32] {
    let i = m.node(node).unwrap_or_else(|| panic!("{}: no node {node}", m.name));
    m.nodes[i].value(ctrl).unwrap_or_else(|| panic!("{}: {node} has no {ctrl}", m.name))
}

/// The single key of `ctrl` on `node` in animation `anim`.
fn key(m: &Model, anim: &str, node: &str, ctrl: &str) -> f32 {
    let a = m.animation(anim).unwrap_or_else(|| panic!("{}: no {anim} animation", m.name));
    let n = a.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node)).unwrap_or_else(|| panic!("{}: {anim} lacks {node}", m.name));
    let c = n.controllers.iter().find(|c| c.name == ctrl).unwrap_or_else(|| panic!("{}: {anim} {node} keys no {ctrl}", m.name));
    c.row(0)[0]
}

#[test]
fn every_kind_matches_its_model() {
    let pack = pack();
    let own: Vec<&str> = pack["textures"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    let stock: Vec<&str> = pack["stock_textures"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    for t in pack["textures"].as_array().unwrap() {
        assert!(pack_dir().join(t["file"].as_str().unwrap()).is_file(), "{t}: no file");
        assert!(t["name"].as_str().unwrap().len() <= 16, "{t}: a resref has at most 16 characters");
    }
    let kinds = pack["kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 11, "the design's eleven kinds");
    for kind in kinds {
        let id = kind["id"].as_str().unwrap();
        let name = kind["model"].as_str().unwrap();
        assert!(name.len() <= 16 && name == name.to_lowercase(), "{id}: model resref {name}");
        let (text, m) = model(name);
        assert_eq!(m.name, name, "{id}: the model's own name");
        mg_mdl::compile::compile(text.as_bytes(), &Default::default()).unwrap_or_else(|e| panic!("{id}: {e}"));

        // The placeable carries the look it shows in: night for night-only kinds, day for the others.
        let shows = kind["shows"].as_str().unwrap();
        assert!(kind["loops"].is_boolean(), "{id}: loops is true or false");
        let worn = if shows == "night" { "night" } else { "day" };
        let emitters = kind["emitters"].as_object().unwrap();
        let in_model = m.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Emitter(_))).count();
        assert_eq!(in_model, emitters.len(), "{id}: the model's emitters are the ones pack.json lists");
        let mut live = [0.0, 0.0];
        for (node, looks) in emitters {
            let i = m.node(node).unwrap_or_else(|| panic!("{id}: no emitter {node}"));
            let NodeKind::Emitter(e) = &m.nodes[i].kind else { panic!("{id}: {node} is no emitter") };
            let tex = e.texture.as_deref().unwrap_or("");
            assert!(own.contains(&tex) || stock.contains(&tex), "{id}: {node}'s texture {tex} is neither the pack's nor listed as stock");
            for (key, ctrl) in LOOK {
                let want = floats(&looks[worn][key]);
                assert!(close(&want, value(&m, node, ctrl)), "{id}: {node} {ctrl} is not the {worn} look's {want:?}");
            }
            let life = f64::from(value(&m, node, "lifeexp")[0]);
            for (n, t) in ["day", "night"].into_iter().enumerate() {
                let rate = looks[t]["birthrate"].as_f64().unwrap();
                // In the client, an emitter born at 0.5 a second drew nothing.
                assert!(rate == 0.0 || rate >= 1.0, "{id}: {node}'s {t} birth rate {rate} is between 0 and 1");
                live[n] += rate * life;
            }
            // A script switches the placeable: `off` stops the particles, `on` and `default` give the look's.
            let rate = looks[worn]["birthrate"].as_f64().unwrap();
            assert_eq!(key(&m, "off", node, "birthrate"), 0.0, "{id}: off");
            for a in ["on", "default"] {
                assert!((f64::from(key(&m, a, node, "birthrate")) - rate).abs() < 1e-4, "{id}: {a}");
            }
        }
        for (n, t) in ["day", "night"].into_iter().enumerate() {
            let said = kind["live"][t].as_f64().unwrap();
            assert!((said - live[n]).abs() < 0.01, "{id}: pack.json says {said} live by {t}, the model makes {}", live[n]);
            let budget = kind["budget"].as_f64().unwrap();
            assert!(live[n] <= budget && live[n] < MAX_LIVE, "{id}: {} live by {t}, over its budget of {budget}", live[n]);
        }
        let shown = |t: usize| live[t] > 0.0;
        match shows {
            "always" => assert!(shown(0) && shown(1), "{id}: shows always"),
            "day" => assert!(shown(0) && !shown(1), "{id}: shows by day only"),
            "night" => assert!(!shown(0) && shown(1), "{id}: shows at night only"),
            s => panic!("{id}: shows {s}"),
        }

        // Meshes (the light shafts): their alpha is the look's, and night hides a day-only kind's.
        if let Some(meshes) = kind.get("meshes").and_then(Value::as_object) {
            for (node, looks) in meshes {
                let i = m.node(node).unwrap_or_else(|| panic!("{id}: no mesh {node}"));
                let mesh = m.nodes[i].mesh().unwrap_or_else(|| panic!("{id}: {node} is no mesh"));
                let tex = mesh.textures[0].as_deref().unwrap_or("");
                assert!(own.contains(&tex) || stock.contains(&tex), "{id}: {node}'s texture {tex}");
                let alpha = floats(&looks[worn]["alpha"]);
                assert!(close(&alpha, value(&m, node, "alpha")), "{id}: {node}'s alpha");
                if shows == "day" {
                    assert_eq!(floats(&looks["night"]["alpha"]), [0.0], "{id}: {node} at night");
                }
                assert_eq!(key(&m, "off", node, "alpha"), 0.0, "{id}: off hides {node}");
            }
        }
    }
}

#[test]
fn the_sample_placeables_2da_has_a_row_per_kind() {
    let text = std::fs::read_to_string(pack_dir().join("placeables.2da")).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().map(str::trim), Some("2DA V2.0"));
    let columns: Vec<&str> = lines.by_ref().find(|l| !l.trim().is_empty()).unwrap().split_whitespace().collect();
    let model_col = columns.iter().position(|c| *c == "ModelName").unwrap() + 1;
    let rows: Vec<Vec<&str>> = lines.filter(|l| !l.trim().is_empty()).map(|l| l.split_whitespace().collect()).collect();
    for row in &rows {
        assert_eq!(row.len(), columns.len() + 1, "{row:?}: one value a column");
    }
    let pack = pack();
    let kinds = pack["kinds"].as_array().unwrap();
    assert_eq!(rows.len(), kinds.len());
    for kind in kinds {
        let model = kind["model"].as_str().unwrap();
        assert_eq!(rows.iter().filter(|r| r[model_col] == model).count(), 1, "{model}: one row");
    }
}
