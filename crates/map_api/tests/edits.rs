//! Edits on a draft (stage C): they change a draft, which reads then see,
//! never the world opened; each says its year; the draft reports what it
//! changes, can be previewed before and after, saved apart, or dropped.

use map_api::Session;
use serde_json::{json, Value};

fn call(s: &mut Session, name: &str, args: Value) -> Value {
    let (text, err) = map_api::mcp::call(s, name, &args);
    assert!(!err, "{name}: {text}");
    serde_json::from_str(&text).unwrap_or(json!({"text": text}))
}

#[test]
fn a_neighbour_takes_the_east_of_a_country_from_a_year() {
    let mut s = Session::new(map_core::testworld::history_world(&Default::default()).0, "测试世界");
    let original = s.original().clone();
    // two neighbouring countries in 1800
    let list = call(&mut s, "entities_list", json!({"kind": "country", "year": 1800}));
    let items = list["result"]["items"].as_array().unwrap();
    let a = &items[0];
    let (an, aid) = (a["name"].as_str().unwrap().to_string(), a["id"].as_u64().unwrap());
    let d = s.doc().clone();
    let mut d1800 = d.clone();
    d1800.set_year(Some(1800));
    let owner = &d1800.owner;
    let b = d1800
        .topo
        .edges
        .values()
        .find_map(|e| match (owner.get(&e.left), owner.get(&e.right)) {
            (Some(x), Some(y)) | (Some(y), Some(x)) if x.0 == aid && y.0 != aid => Some(*y),
            _ => None,
        })
        .expect("a neighbour");
    // the country's middle meridian
    let p = d1800.dissolve(map_core::EntityId(aid));
    let bb = map_core::geom::bbox(&p[0].outer);
    let mid = (bb[0] + bb[2]) * 0.5;
    let r = call(&mut s, "edit_transfer", json!({"from": aid, "to": b.0, "east_of": mid, "year": 1800}));
    assert!(r["result"]["result"]["moved_km2"].as_f64().is_some_and(|m| m > 0.0), "{r}");
    // reads see the draft; the world opened is as it was
    let after = call(&mut s, "entity_get", json!({"id": aid, "year": 1800}));
    let before = {
        let mut v = original.clone();
        v.set_year(Some(1800));
        v.areas_by_entity()[&map_core::EntityId(aid)]
    };
    let now = after["result"]["in_context_year"]["area_km2"].as_f64().unwrap_or_else(|| panic!("{after}"));
    assert!(now < before - 1.0, "{after}");
    assert!(s.original() == &original);
    // earlier years untouched
    let ch = call(&mut s, "draft_changes", json!({"year": 1799}));
    assert!(ch["result"]["units"].as_array().unwrap().is_empty(), "{ch}");
    let ch = call(&mut s, "draft_changes", json!({"year": 1800}));
    assert!(ch["result"]["units"].as_array().unwrap().iter().any(|u| u["id"] == aid), "{ch}");
    // more edits: a rename, a new country, an event
    call(&mut s, "edit_rename", json!({"entity": aid, "name": format!("{an}（残部）")}));
    call(&mut s, "edit_create", json!({"kind": "country", "name": "新国", "color": "#336699"}));
    call(&mut s, "edit_add_event", json!({"name": "东部之战", "year": 1800, "note": "测试"}));
    let ch = call(&mut s, "draft_changes", json!({"year": 1800}));
    assert_eq!(ch["result"]["edits"].as_array().unwrap().len(), 4);
    assert_eq!(ch["result"]["events_added"], 1);
    // previews before and after
    for before in [true, false] {
        let r = call(&mut s, "preview_render", json!({"year": 1800, "before": before, "width": 400, "height": 240}));
        assert!(r["result"]["objects"].is_object() || r["result"].is_object());
    }
    // saved apart (never over the world opened), then dropped
    let out = std::env::temp_dir().join(format!("atlas_draft_{}.atlas", std::process::id()));
    call(&mut s, "draft_save", json!({"path": out.display().to_string()}));
    let saved = map_io::project::load(&out).unwrap();
    assert!(saved.entities.values().any(|x| x.name == "新国"));
    let _ = std::fs::remove_file(&out);
    let (text, err) = map_api::mcp::call(&mut s, "draft_save", &json!({"path": "测试世界"}));
    assert!(err, "{text}");
    call(&mut s, "draft_discard", json!({}));
    assert!(s.draft().is_none() && s.doc() == &original);
}

#[test]
fn a_bad_edit_leaves_the_draft_as_it_was() {
    let mut s = Session::new(map_core::sample::demo_world(), "示例世界");
    let (text, err) = map_api::mcp::call(&mut s, "edit_transfer", &json!({"from": "没有这个国家", "to": 1, "east_of": 0.0}));
    assert!(err, "{text}");
    let first = s.doc().countries()[0].0;
    let (text, err) = map_api::mcp::call(&mut s, "edit_set_color", &json!({"entity": first, "color": "红色"}));
    assert!(err && text.contains("#rrggbb"), "{text}");
    assert!(s.draft().is_none(), "nothing kept from refused edits");
}

/// From a blank world: land for a new country, a lake in it, a river and a
/// city, all in the draft.
#[test]
fn a_world_from_nothing() {
    let mut s = Session::new(map_core::Document::new("新世界", 6371.0), "空白世界");
    let c = call(&mut s, "edit_create", json!({"kind": "country", "name": "甲国", "color": "#aa6633"}));
    let id = c["result"]["result"]["entity"].as_u64().unwrap();
    let r = call(&mut s, "edit_draw_land", json!({"polygon": [[0, 0], [30, 0], [30, 20], [0, 20]], "country": id}));
    assert!(r["result"]["result"]["added_km2"].as_f64().unwrap() > 1e6, "{r}");
    call(&mut s, "edit_draw_land", json!({"polygon": [[40, 0], [50, 0], [50, 10], [40, 10]]}));
    call(&mut s, "edit_draw_lake", json!({"polygon": [[10, 8], [14, 8], [14, 12], [10, 12]]}));
    call(&mut s, "edit_add_river", json!({"points": [[5, 15], [12, 10]], "name": "甲河"}));
    call(&mut s, "edit_create", json!({"kind": "city", "name": "甲城", "lon": 20.0, "lat": 5.0}));
    let d = s.doc();
    d.topo.validate().unwrap();
    let a = d.areas_by_entity()[&map_core::EntityId(id)];
    assert!(a > 1e6 && a < 30.0 * 20.0 * 12400.0, "{a}");
    assert_eq!(d.rivers.len(), 1);
    assert!(d.entities.values().any(|x| x.name == "甲城" && x.pos.is_some()));
    assert!(s.original().topo.faces.len() == 1, "the world opened stays blank");
}
