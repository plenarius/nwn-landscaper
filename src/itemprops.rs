// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The properties of a builder's new item: what the game lists for its base item (itemprops.2da), chosen type, subtype,
//! value and parameter, and the item's gold value as the game works it out (and stores in `Cost`).

use anyhow::{Context, Result};
use mg_rules::GameData;
use mg_rules::items::{ItemProperty, ItemValue};
use serde_json::{Value as Json, json};

fn doc_of(ext_resref: &str) -> Result<Json> {
    let text = crate::newbp::read("uti", ext_resref).with_context(|| format!("no new item {ext_resref}"))?;
    Ok(serde_json::from_str(&text)?)
}

fn value_of(doc: &Json, resref: &str) -> Result<ItemValue> {
    let s = crate::things::from_json(&serde_json::to_string(doc)?, "uti", resref)?;
    Ok(ItemValue::from_gff(&s))
}

fn base_item(doc: &Json) -> u32 {
    doc["BaseItem"]["value"].as_u64().unwrap_or(0) as u32
}

fn entry(p: &ItemProperty) -> Json {
    json!({
        "__struct_id": 0, "ChanceAppear": { "type": "byte", "value": p.chance }, "CostTable": { "type": "byte", "value": p.cost_table },
        "CostValue": { "type": "word", "value": p.cost_value }, "Param1": { "type": "byte", "value": p.param1 },
        "Param1Value": { "type": "byte", "value": p.param1_value }, "PropertyName": { "type": "word", "value": p.property },
        "Subtype": { "type": "word", "value": p.subtype },
    })
}

/// Works the item's `Cost` out again (after its properties, stack, charges or added cost changed).
pub fn recost(game: &GameData, doc: &mut Json, resref: &str) -> Result<u32> {
    let cost = game.item_cost(&value_of(doc, resref)?);
    doc["Cost"] = json!({ "type": "dword", "value": cost });
    Ok(cost)
}

/// The item's properties as text, and its value.
pub fn get(game: &GameData, resref: &str) -> Result<Json> {
    let doc = doc_of(resref)?;
    let v = value_of(&doc, resref)?;
    let list: Vec<Json> = v
        .properties
        .iter()
        .map(|p| {
            json!({
                "property": p.property, "subtype": p.subtype, "costTable": p.cost_table, "costValue": p.cost_value, "param1": p.param1,
                "param1Value": p.param1_value, "text": game.property_text(p),
            })
        })
        .collect();
    Ok(json!({ "resref": resref, "properties": list, "cost": game.item_cost(&v) }))
}

/// The property types the item's base item can have: `[{row, name}]`.
pub fn types(game: &GameData, resref: &str) -> Result<Json> {
    let doc = doc_of(resref)?;
    let types: Vec<Json> = game.available_properties(base_item(&doc)).iter().map(|t| json!({ "row": t.row, "name": t.name })).collect();
    Ok(json!({ "types": types }))
}

/// What a property type offers on this item: its subtypes, values and parameters, and which are first.
pub fn choices(game: &GameData, resref: &str, property: u16, subtype: u16) -> Result<Json> {
    let base = base_item(&doc_of(resref)?);
    let t = game.property_type(property).with_context(|| format!("no property type {property}"))?;
    let rows = |v: Vec<mg_rules::Choice>| v.into_iter().map(|c| json!({ "row": c.row, "text": c.text })).collect::<Vec<_>>();
    let param = game.property_param_table(&t, subtype);
    Ok(json!({
        "subtypes": rows(game.property_subtypes(&t, base)),
        "costTable": t.cost_table,
        "costs": rows(game.property_costs(t.cost_table, base)),
        "param1": param, "params": param.map_or_default(|p| rows(game.property_params(p))),
    }))
}

/// Adds a property (its type, and the subtype, value and parameter rows when the type has them; else the first of each).
pub fn add(game: &GameData, resref: &str, req: &Json) -> Result<Json> {
    let mut doc = doc_of(resref)?;
    let base = base_item(&doc);
    let row = req["property"].as_u64().context("which property?")? as u16;
    let t = game.available_properties(base).into_iter().find(|t| t.row == row).context("the item can't have that property")?;
    let mut p = game.new_property(&t, base);
    if let Some(v) = req["subtype"].as_u64() {
        p.subtype = v as u16;
        // (the parameter table can depend on the subtype)
        p.param1 = game.property_param_table(&t, p.subtype).unwrap_or(255);
        p.param1_value = p.param1_value_default(game);
    }
    if let Some(v) = req["costValue"].as_u64() {
        p.cost_value = v as u16;
    }
    if let Some(v) = req["param1Value"].as_u64() {
        p.param1_value = v as u8;
    }
    let list =
        doc.as_object_mut().context("not an item")?.entry("PropertiesList").or_insert_with(|| json!({ "type": "list", "value": [] }));
    let arr = list["value"].as_array_mut().context("a bad property list")?;
    anyhow::ensure!(arr.len() < 60, "that's a lot of properties on one item");
    let mut e = entry(&p);
    e["__struct_id"] = json!(0);
    arr.push(e);
    recost(game, &mut doc, resref)?;
    crate::newbp::write("uti", resref, &doc)?;
    get(game, resref)
}

trait Param {
    fn param1_value_default(&self, game: &GameData) -> u8;
}

impl Param for ItemProperty {
    fn param1_value_default(&self, game: &GameData) -> u8 {
        if self.param1 == 255 { 0 } else { game.property_params(self.param1).first().map_or(0, |c| c.row as u8) }
    }
}

/// Takes property `index` off the builder's item `resref` and prices it again; the item as [`get`] answers it.
pub fn remove(game: &GameData, resref: &str, index: usize) -> Result<Json> {
    let mut doc = doc_of(resref)?;
    let arr = doc["PropertiesList"]["value"].as_array_mut().context("it has no properties")?;
    anyhow::ensure!(index < arr.len(), "no such property");
    arr.remove(index);
    recost(game, &mut doc, resref)?;
    crate::newbp::write("uti", resref, &doc)?;
    get(game, resref)
}
