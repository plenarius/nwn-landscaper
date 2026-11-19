// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Checking an exported area before it is published: what a reviewer would otherwise find. Transitions that lead
//! nowhere, scripts and dialogs that don't exist, tags that collide, objects outside the area, an area far
//! heavier than the module's own. Read from the area's GIT and ARE in the module the last export built; what the
//! rest of the module holds (scripts, dialogs, every waypoint and door tag) comes from the review service.
//!
//! A finding is an `error` (publishing is refused), a `warn` (look at it) or `info`. Objects an import brought in
//! untouched (they carry `ls_orig`) can only ever warn: the production area was already like that.

use std::collections::{HashMap, HashSet};

use mg_gff::Struct;
use serde::Serialize;

use crate::things::{LISTS, ORIG, fields_of, kind_name, position_of};

/// One thing the check before publishing found.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Finding {
    /// `error` (stops a publish) or `warn`.
    pub level: &'static str,
    /// The object's tag (or blueprint), or empty for the area itself.
    pub what: String,
    /// What is wrong, and what to do about it.
    pub msg: String,
}

/// What the module has (from the review service): lower case throughout.
#[derive(Default, Clone)]
pub struct Refs {
    /// The module's scripts.
    pub scripts: HashSet<String>,
    /// The module's conversations.
    pub dialogs: HashSet<String>,
    /// Waypoint tag -> the areas that have one.
    pub waypoints: HashMap<String, Vec<String>>,
    /// Door tag -> the areas that have one.
    pub doors: HashMap<String, Vec<String>>,
    /// Area tag -> the areas that have it.
    pub areas: HashMap<String, Vec<String>>,
    /// A destination tag -> the areas whose doors or triggers lead to it.
    pub links: HashMap<String, Vec<String>>,
}

impl Refs {
    /// The references the review service answered (`/api/v1/areas/refs`).
    pub fn from_json(v: &serde_json::Value) -> Refs {
        let set = |k: &str| v[k].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_ascii_lowercase)).collect();
        let map = |k: &str| {
            v[k].as_object()
                .into_iter()
                .flatten()
                .map(|(t, a)| {
                    (
                        t.to_ascii_lowercase(),
                        a.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_ascii_lowercase)).collect(),
                    )
                })
                .collect()
        };
        Refs {
            scripts: set("scripts"),
            dialogs: set("dialogs"),
            waypoints: map("waypoints"),
            doors: map("doors"),
            areas: map("areas"),
            links: map("links"),
        }
    }
}

/// What the check needs besides the area's own files.
pub struct Ctx<'a> {
    /// The resref the area will have in nwn-module, and whether it is new there (a rework keeps its tags).
    pub target: &'a str,
    /// Whether the area is new in nwn-module.
    pub is_new: bool,
    /// Waypoint and door tags of every area in the builder's own module (lower case -> areas), and each door's
    /// destination, for the way back.
    pub local_waypoints: &'a HashMap<String, Vec<String>>,
    /// Door tag -> the areas that have one, and its destination.
    pub local_doors: &'a HashMap<String, (Vec<String>, String)>,
    /// Destination tag -> the areas of the builder's module that lead to it.
    pub local_links: &'a HashMap<String, Vec<String>>,
    /// None when the review service has nothing to say (a machine of your own): the checks that need it are skipped.
    pub refs: Option<&'a Refs>,
    /// A script / dialog the base game has.
    pub game_script: &'a dyn Fn(&str) -> bool,
    /// A conversation the base game has.
    pub game_dialog: &'a dyn Fn(&str) -> bool,
}

/// The busiest 1% of a large persistent world's areas (counted 2026-10): above these an area is heavier than 99 of 100 of them.
const HEAVY: [(&str, usize); 5] =
    [("Creature List", 30), ("Placeable List", 170), ("Encounter List", 12), ("TriggerList", 15), ("SoundList", 40)];
const HEAVY_TOTAL: usize = 200;
/// A waypoint or door tag in this many other areas is a convention (NW_SAFE, InnRestDoor), not a collision.
const COMMON: usize = 3;

fn text(s: &Struct, label: &str) -> String {
    s.string(label).map_or_default(|b| String::from_utf8_lossy(b).into_owned())
}

fn has_text(l: Option<&mg_core::LocString>) -> bool {
    l.is_some_and(|l| !l.strref.is_none() || l.strings.iter().any(|(_, b)| !b.is_empty()))
}

/// Checks an area's ARE and GIT before it is published: what would break in the module (a missing script, a tag that collides, an area
/// nothing leads into).
pub fn check(are: &Struct, git: &Struct, ctx: &Ctx<'_>) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |level: &'static str, what: &str, msg: String| out.push(Finding { level, what: what.to_string(), msg });

    // The area itself.
    if !has_text(are.locstring("Name")) {
        add("error", "", "the area has no name (Area settings)".into());
    }
    let area_tag = text(are, "Tag");
    if area_tag.is_empty() {
        add("error", "", "the area has no tag (Area settings)".into());
    } else if let Some(refs) = ctx.refs
        && let Some(others) = refs.areas.get(&area_tag.to_ascii_lowercase())
        && others.iter().any(|a| a != ctx.target)
    {
        add("error", &area_tag, format!("another area already has the tag {area_tag} ({})", others.join(", ")));
    }
    let known_script = |n: &str| {
        let n = n.to_ascii_lowercase();
        ctx.refs.is_none_or(|r| r.scripts.contains(&n)) || (ctx.game_script)(&n) || crate::newscript::read(&n).is_some()
    };
    let known_dialog = |n: &str| {
        let n = n.to_ascii_lowercase();
        ctx.refs.is_none_or(|r| r.dialogs.contains(&n)) || (ctx.game_dialog)(&n) || crate::newbp::read("dlg", &n).is_some()
    };
    for label in ["OnEnter", "OnExit", "OnHeartbeat", "OnUserDefined"] {
        let s = are.resref(label).map_or_default(|r| r.to_string());
        if !s.is_empty() && !known_script(&s) {
            add("error", &area_tag, format!("area script {label}: {s} isn't in the module or the game"));
        }
    }
    let (w, h) = (are.integer("Width").unwrap_or(0) as f32 * 10.0, are.integer("Height").unwrap_or(0) as f32 * 10.0);

    // Every object.
    let mut total = 0usize;
    let mut tags: HashMap<(&str, String), usize> = HashMap::new();
    for list in LISTS {
        let objects = git.list(list).unwrap_or(&[]);
        total += objects.len();
        let Some(kind) = mg_rules::ObjectKind::ALL.iter().copied().find(|k| k.list() == list) else { continue };
        let kname = kind_name(kind);
        for o in objects {
            let tag = text(o, "Tag");
            let what = if tag.is_empty() { text(o, "TemplateResRef") } else { tag.clone() };
            // An original the builder hasn't touched was already like this in production.
            let original = o.list("VarTable").is_some_and(|v| v.iter().any(|e| e.string("Name") == Some(ORIG.as_bytes())));
            let sev = |level: &'static str| if original && level == "error" { "warn" } else { level };
            let note = if original { " (already so in the original)" } else { "" };

            if let Some(([x, y, _], _)) = position_of(list, o)
                && (x < 0.0 || y < 0.0 || x > w || y > h)
            {
                add(sev("warn"), &what, format!("{kname} is outside the area ({x:.0}, {y:.0}; the area is {w:.0} by {h:.0} m){note}"));
            }
            for (label, ty) in fields_of(kind) {
                if ty != 'r' {
                    continue;
                }
                let v = o.resref(label).map_or_default(|r| r.to_string());
                if v.is_empty() {
                    continue;
                }
                if label == "Conversation" {
                    if !known_dialog(&v) {
                        add(sev("error"), &what, format!("{kname} conversation {v} isn't in the module or the game{note}"));
                    }
                } else if !known_script(&v) {
                    add(sev("error"), &what, format!("{kname} script {label}: {v} isn't in the module or the game{note}"));
                }
            }
            if matches!(kname, "waypoint" | "door" | "trigger" | "store") && !tag.is_empty() {
                *tags.entry((kname, tag.to_ascii_lowercase())).or_default() += 1;
            }
            // Where it leads.
            let (link, flags) = (text(o, "LinkedTo"), o.integer("LinkedToFlags").unwrap_or(0));
            let transition = kname == "trigger" && o.integer("Type") == Some(1);
            if transition && link.is_empty() {
                add(sev("error"), &what, format!("an area transition with no destination tag{note}"));
            } else if matches!(kname, "door" | "trigger") && !link.is_empty() {
                let key = link.to_ascii_lowercase();
                if flags == 0 {
                    add(sev("warn"), &what, format!("leads to {link} but 'leads to a' is 0, so it won't take anyone there{note}"));
                } else {
                    let find = |wp: bool| {
                        let local =
                            if wp { ctx.local_waypoints.get(&key).cloned() } else { ctx.local_doors.get(&key).map(|d| d.0.clone()) };
                        let far = ctx.refs.and_then(|r| if wp { r.waypoints.get(&key) } else { r.doors.get(&key) }).cloned();
                        (local, far)
                    };
                    let (lw, fw) = find(true);
                    let (ld, fd) = find(false);
                    let found_wp = lw.is_some() || fw.is_some() || ctx.refs.is_none();
                    let found_door = ld.is_some() || fd.is_some() || ctx.refs.is_none();
                    let ok = match flags {
                        1 => found_door,
                        2 => found_wp,
                        _ => found_door || found_wp,
                    };
                    if !ok {
                        let wanted = match flags {
                            1 => "door",
                            2 => "waypoint",
                            _ => "door or waypoint",
                        };
                        let wrong = if flags == 1 && (lw.is_some() || fw.is_some()) {
                            " (there is a waypoint of that tag: set 'leads to a' to 2)"
                        } else if flags == 2 && (ld.is_some() || fd.is_some()) {
                            " (there is a door of that tag: set 'leads to a' to 1)"
                        } else {
                            ""
                        };
                        add(
                            sev("error"),
                            &what,
                            format!("leads to {link}, but there is no {wanted} with that tag in the module{wrong}{note}"),
                        );
                    } else if kname == "door"
                        && flags == 1
                        && let Some((_, back)) = ctx.local_doors.get(&key)
                        && !back.eq_ignore_ascii_case(&tag)
                    {
                        add(
                            sev("warn"),
                            &what,
                            format!(
                                "leads to the door {link}, which leads {} (a one-way door)",
                                if back.is_empty() { "nowhere".to_string() } else { format!("to {back}") }
                            ),
                        );
                    }
                }
            }
        }
        if let Some((_, limit)) = HEAVY.iter().find(|(l, _)| *l == list)
            && objects.len() > *limit
        {
            add("warn", "", format!("{} {kname}s: more than areas usually have (over {limit} is heavy for one area)", objects.len()));
        }
    }
    if total > HEAVY_TOTAL {
        add("warn", "", format!("{total} objects in all: heavy for one area (over {HEAVY_TOTAL})"));
    }

    // Tags that should be one object's (not the ones production reuses everywhere on purpose, NW_SAFE and the like).
    let common = |kname: &str, tag: &str| {
        ctx.refs.is_some_and(|r| {
            let map = if kname == "waypoint" { &r.waypoints } else { &r.doors };
            map.get(tag).is_some_and(|a| a.iter().filter(|a| a.as_str() != ctx.target).count() >= COMMON)
        })
    };
    let mut dup: Vec<_> = tags.iter().filter(|((kname, tag), n)| **n > 1 && !common(kname, tag)).collect();
    dup.sort();
    for ((kname, tag), n) in dup {
        add("warn", tag, format!("{n} {kname}s share the tag {tag}"));
    }
    // A new area nothing leads into can't be reached from the world.
    if ctx.is_new {
        let mine: HashSet<String> = ["WaypointList", "Door List"]
            .iter()
            .flat_map(|l| git.list(l).unwrap_or(&[]).iter().map(|o| text(o, "Tag").to_ascii_lowercase()).collect::<Vec<_>>())
            .filter(|t| !t.is_empty())
            .collect();
        let led_to =
            |links: &HashMap<String, Vec<String>>| mine.iter().any(|t| links.get(t).is_some_and(|a| a.iter().any(|a| a != ctx.target)));
        // (Only said when the module's links are known: without them it can't be told.)
        if let Some(refs) = ctx.refs
            && !led_to(&refs.links)
            && !led_to(ctx.local_links)
        {
            add(
                "warn",
                "",
                "nothing leads into this area yet. Import a neighbouring area, place a Transition there leading to one of this area's \
                 waypoints (or doors), and publish them together; or ask a reviewer to connect it"
                    .into(),
            );
        }
    }
    // A waypoint or door tag one or two other areas have: scripts and links by tag could reach the wrong one.
    if let Some(refs) = ctx.refs {
        for list in ["WaypointList", "Door List"] {
            let map = if list == "WaypointList" { &refs.waypoints } else { &refs.doors };
            let mut seen = HashSet::new();
            for o in git.list(list).unwrap_or(&[]) {
                let tag = text(o, "Tag").to_ascii_lowercase();
                if tag.is_empty() || !seen.insert(tag.clone()) {
                    continue;
                }
                if let Some(others) = map
                    .get(&tag)
                    .map(|a| a.iter().filter(|a| a.as_str() != ctx.target).cloned().collect::<Vec<_>>())
                    .filter(|a| !a.is_empty() && a.len() < COMMON)
                {
                    add(
                        "warn",
                        &tag,
                        format!("{} tag {tag} is also in {}", if list == "WaypointList" { "waypoint" } else { "door" }, others.join(", ")),
                    );
                }
            }
        }
    }
    out.sort_by_key(|f| match f.level {
        "error" => 0,
        "warn" => 1,
        _ => 2,
    });
    out
}

/// Counts of each level.
#[cfg(test)]
pub fn counts(f: &[Finding]) -> (usize, usize) {
    (f.iter().filter(|x| x.level == "error").count(), f.iter().filter(|x| x.level == "warn").count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mg_gff::Value;

    fn obj(fields: &[(&str, Value)]) -> Struct {
        let mut s = Struct::new(0);
        for (k, v) in fields {
            s.set(k, v.clone());
        }
        s
    }
    fn st(s: &str) -> Value {
        Value::String(s.as_bytes().to_vec())
    }
    fn pos(x: f32, y: f32) -> [(&'static str, Value); 3] {
        [("XPosition", Value::Float(x)), ("YPosition", Value::Float(y)), ("ZPosition", Value::Float(0.0))]
    }
    fn at(tag: &str, x: f32, y: f32, more: &[(&str, Value)]) -> Struct {
        let mut f: Vec<(&str, Value)> = pos(x, y).to_vec();
        f.push(("Tag", st(tag)));
        f.extend(more.iter().cloned());
        obj(&f)
    }
    fn git(lists: &[(&str, Vec<Struct>)]) -> Struct {
        let mut g = Struct::new(0);
        for (l, v) in lists {
            g.set(l, Value::List(v.clone()));
        }
        g
    }
    fn are() -> Struct {
        let mut a = obj(&[("Tag", st("MyArea")), ("Width", Value::Int(4)), ("Height", Value::Int(4))]);
        a.set(
            "Name",
            Value::LocString(mg_core::LocString {
                strref: mg_core::StrRef::NONE,
                strings: vec![(mg_core::LocStringKey(0), b"My area".to_vec())],
            }),
        );
        a
    }
    fn run_all(git: &Struct, refs: Option<&Refs>) -> Vec<Finding> {
        let (lw, ld, ll) = (HashMap::new(), HashMap::new(), HashMap::new());
        let ctx = Ctx {
            target: "mine",
            is_new: true,
            local_waypoints: &lw,
            local_doors: &ld,
            local_links: &ll,
            refs,
            game_script: &|_| false,
            game_dialog: &|_| false,
        };
        check(&are(), git, &ctx)
    }
    /// Without the "nothing leads here" note, which most of these areas would have.
    fn run(git: &Struct, refs: Option<&Refs>) -> Vec<Finding> {
        run_all(git, refs).into_iter().filter(|f| !f.msg.starts_with("nothing leads into")).collect()
    }
    fn refs() -> Refs {
        let mut r = Refs::default();
        r.scripts.insert("lay_area_onenter".into());
        r.waypoints.insert("wp_far".into(), vec!["elsewhere".into()]);
        r.doors.insert("door_far".into(), vec!["elsewhere".into()]);
        r
    }
    fn msgs(f: &[Finding]) -> Vec<String> {
        f.iter().map(|x| format!("{}:{}", x.level, x.msg)).collect()
    }

    #[test]
    fn a_clean_area_has_no_findings() {
        let g = git(&[("WaypointList", vec![at("wp_here", 5.0, 5.0, &[])])]);
        let mut r = refs();
        r.links.insert("wp_here".into(), vec!["neighbour".into()]);
        assert!(run_all(&g, Some(&r)).is_empty());
    }

    #[test]
    fn a_new_area_nothing_leads_into_is_said() {
        let g = git(&[("WaypointList", vec![at("wp_here", 5.0, 5.0, &[])])]);
        let m = msgs(&run_all(&g, Some(&refs())));
        assert_eq!(m.len(), 1);
        assert!(m[0].starts_with("warn:nothing leads into this area"));
        // the area's own links don't count, a neighbour's do (also one in the builder's module)
        let mut r = refs();
        r.links.insert("wp_here".into(), vec!["mine".into()]);
        assert_eq!(msgs(&run_all(&g, Some(&r))).len(), 1);
        let (lw, ld, mut ll) = (HashMap::new(), HashMap::new(), HashMap::new());
        ll.insert("wp_here".to_string(), vec!["other_area".to_string()]);
        let ctx = Ctx {
            target: "mine",
            is_new: true,
            local_waypoints: &lw,
            local_doors: &ld,
            local_links: &ll,
            refs: Some(&refs()),
            game_script: &|_| false,
            game_dialog: &|_| false,
        };
        assert!(check(&are(), &g, &ctx).is_empty());
        // a rework keeps the connections it has: nothing is said
        let (none, r) = (HashMap::new(), refs());
        let ctx = Ctx {
            target: "mine",
            is_new: false,
            local_waypoints: &lw,
            local_doors: &ld,
            local_links: &none,
            refs: Some(&r),
            game_script: &|_| false,
            game_dialog: &|_| false,
        };
        assert!(check(&are(), &g, &ctx).is_empty());
    }

    #[test]
    fn a_transition_must_lead_somewhere_that_exists() {
        let g = git(&[(
            "TriggerList",
            vec![
                at("t_none", 5.0, 5.0, &[("Type", Value::Int(1))]),
                at("t_bad", 6.0, 6.0, &[("Type", Value::Int(1)), ("LinkedTo", st("wp_nowhere")), ("LinkedToFlags", Value::Byte(2))]),
                at("t_ok", 7.0, 7.0, &[("Type", Value::Int(1)), ("LinkedTo", st("WP_FAR")), ("LinkedToFlags", Value::Byte(2))]),
                at("t_wrongkind", 8.0, 8.0, &[("Type", Value::Int(1)), ("LinkedTo", st("wp_far")), ("LinkedToFlags", Value::Byte(1))]),
            ],
        )]);
        let m = msgs(&run(&g, Some(&refs())));
        assert_eq!(m.len(), 3, "{m:?}");
        assert!(m.iter().any(|x| x.contains("no destination tag")));
        assert!(m.iter().any(|x| x.contains("no waypoint with that tag")));
        assert!(m.iter().any(|x| x.contains("there is a waypoint of that tag: set 'leads to a' to 2")));
        assert!(m.iter().all(|x| x.starts_with("error:")));
    }

    #[test]
    fn missing_scripts_and_dialogs_are_errors_but_the_games_are_known() {
        let g = git(&[(
            "Creature List",
            vec![at(
                "c",
                5.0,
                5.0,
                &[
                    ("ScriptSpawn", Value::resref(mg_core::ResRef::from_str("nope").unwrap())),
                    ("ScriptDeath", Value::resref(mg_core::ResRef::from_str("lay_area_onenter").unwrap())),
                    ("Conversation", Value::resref(mg_core::ResRef::from_str("nodlg").unwrap())),
                ],
            )],
        )]);
        let m = msgs(&run(&g, Some(&refs())));
        assert_eq!(m.len(), 2, "{m:?}");
        assert!(m.iter().any(|x| x.contains("ScriptSpawn: nope")) && m.iter().any(|x| x.contains("conversation nodlg")));
        // without the module's lists nothing can be said about them
        assert!(run(&g, None).is_empty());
    }

    #[test]
    fn an_untouched_original_only_warns() {
        let orig = Value::List(vec![obj(&[("Name", st("ls_orig")), ("Type", Value::Int(1)), ("Value", st("Creature List:0"))])]);
        let g = git(&[(
            "Creature List",
            vec![at("c", 5.0, 5.0, &[("ScriptSpawn", Value::resref(mg_core::ResRef::from_str("nope").unwrap())), ("VarTable", orig)])],
        )]);
        let f = run(&g, Some(&refs()));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].level, "warn");
        assert!(f[0].msg.contains("already so in the original"));
    }

    #[test]
    fn outside_the_area_duplicates_and_collisions_warn() {
        let g = git(&[(
            "WaypointList",
            vec![at("wp_a", 5.0, 5.0, &[]), at("wp_a", 6.0, 6.0, &[]), at("wp_far", 7.0, 7.0, &[]), at("wp_out", 500.0, 5.0, &[])],
        )]);
        let m = msgs(&run(&g, Some(&refs())));
        assert!(m.iter().any(|x| x.contains("outside the area")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("2 waypoints share the tag wp_a")));
        assert!(m.iter().any(|x| x.starts_with("warn:") && x.contains("tag wp_far is also in elsewhere")));
        // a tag production uses in many areas is a convention
        let mut r = refs();
        r.waypoints.insert("wp_a".into(), vec!["x".into(), "y".into(), "z".into()]);
        r.waypoints.insert("wp_far".into(), vec!["x".into(), "y".into(), "z".into()]);
        let m = msgs(&run(&g, Some(&r)));
        assert!(!m.iter().any(|x| x.contains("share the tag wp_a") || x.contains("wp_far is also in")), "{m:?}");
    }

    #[test]
    fn a_nameless_untagged_area_is_two_errors() {
        let mut a = Struct::new(0);
        a.set("Width", Value::Int(4));
        a.set("Height", Value::Int(4));
        let (lw, ld, ll) = (HashMap::new(), HashMap::new(), HashMap::new());
        let ctx = Ctx {
            target: "mine",
            is_new: true,
            local_waypoints: &lw,
            local_doors: &ld,
            local_links: &ll,
            refs: None,
            game_script: &|_| false,
            game_dialog: &|_| false,
        };
        let f = check(&a, &Struct::new(0), &ctx);
        assert_eq!(counts(&f), (2, 0));
    }

    #[test]
    fn a_one_way_door_in_the_builders_module_warns() {
        let mut ld = HashMap::new();
        ld.insert("door_b".to_string(), (vec!["other".to_string()], String::new()));
        let (lw, ll) = (HashMap::new(), HashMap::new());
        let ctx = Ctx {
            target: "mine",
            is_new: true,
            local_waypoints: &lw,
            local_doors: &ld,
            local_links: &ll,
            refs: None,
            game_script: &|_| false,
            game_dialog: &|_| false,
        };
        let g = git(&[("Door List", vec![at("door_a", 5.0, 5.0, &[("LinkedTo", st("door_b")), ("LinkedToFlags", Value::Byte(1))])])]);
        let f = check(&are(), &g, &ctx);
        assert_eq!(msgs(&f), vec!["warn:leads to the door door_b, which leads nowhere (a one-way door)".to_string()]);
    }
}
