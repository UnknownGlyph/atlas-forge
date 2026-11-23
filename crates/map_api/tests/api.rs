//! The read-only world interface, checked against the test world: every
//! advertised operation answers, requests do not affect each other, and
//! the answers agree with one another and with the document.

use map_api::Session;
use map_core::testworld::{history_world, TestWorldSpec};
use serde_json::{json, Value};

fn session() -> Session {
    let (d, _) = history_world(&TestWorldSpec { nx: 5, ny: 3, events: 60, ..Default::default() });
    Session::new(d, "测试世界")
}

fn ok(s: &Session, req: Value) -> Value {
    let a = s.handle(&req);
    assert_eq!(a["ok"], true, "{req} -> {a}");
    a["result"].clone()
}

fn err(s: &Session, req: Value) -> String {
    let a = s.handle(&req);
    assert_eq!(a["ok"], false, "{req} -> {a}");
    a["error"]["kind"].as_str().unwrap().to_string()
}

#[test]
fn every_advertised_operation_answers() {
    let s = session();
    let caps = ok(&s, json!({"op": "capabilities"}));
    let dir = std::env::temp_dir().join(format!("atlas_api_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let e = ok(&s, json!({"op": "entities.list", "args": {"kind": "country", "limit": 1}, "context": {"year": 1500}}))["items"][0]["id"].clone();
    for op in caps["operations"].as_array().unwrap() {
        let name = op["op"].as_str().unwrap();
        assert_eq!(op["read_only"], true);
        let args = match name {
            "entity.get" => json!({"id": e}),
            "place.at" => json!({"lon": 0.0, "lat": 5.0}),
            "changes.between" => json!({"from": 1200, "to": 1800}),
            "preview.render" => json!({"path": dir.join("a.svg").display().to_string(), "width": 400, "height": 240}),
            _ => json!({}),
        };
        ok(&s, json!({"op": name, "args": args, "context": {"year": 1500}}));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn requests_carry_their_own_context_and_change_nothing() {
    let s = session();
    let before = s.doc().clone();
    let early = ok(&s, json!({"op": "entities.list", "args": {"kind": "country"}, "context": {"year": 1010}}));
    let late = ok(&s, json!({"op": "entities.list", "args": {"kind": "country"}, "context": {"year": 1990}}));
    assert_ne!(early, late, "different years, different answers");
    // the same request gives the same answer whatever came between
    assert_eq!(ok(&s, json!({"op": "entities.list", "args": {"kind": "country"}, "context": {"year": 1010}})), early);
    assert_eq!(s.doc(), &before, "the world is untouched");
    // every listed country exists that year
    for it in late["items"].as_array().unwrap() {
        assert_eq!(it["presence"], "exists");
    }
    // the answer says which context it used
    let a = s.handle(&json!({"op": "world.summary", "context": {"year": 1200, "layer": "文化区"}}));
    assert_eq!(a["context"]["year"], 1200);
    assert_eq!(a["context"]["layer"]["name"], "文化区");
    assert_eq!(a["version"], s.version());
}

#[test]
fn answers_agree_with_each_other() {
    let s = session();
    let (from, to) = (1300, 1700);
    let ch = ok(&s, json!({"op": "changes.between", "args": {"from": from, "to": to, "limit": 1000}}));
    // area bookkeeping: what owners gained net equals land gained from the sea
    let net: f64 = ch["area_changes"].as_array().unwrap().iter().map(|c| c["change_km2"].as_f64().unwrap()).sum();
    let sea = ch["water_became_land_km2"].as_f64().unwrap() - ch["land_became_water_km2"].as_f64().unwrap();
    assert!((net - sea).abs() < 1.0, "{net} vs {sea}");
    // an entity's change matches its own history
    let c = ch["area_changes"].as_array().unwrap().iter().find(|c| !c["entity"]["id"].is_null()).unwrap();
    let id = c["entity"]["id"].clone();
    let at = |y: i32| ok(&s, json!({"op": "entity.get", "args": {"id": id}, "context": {"year": y}}))["in_context_year"]["area_km2"].as_f64().unwrap_or(0.0);
    assert!((at(from) - c["area_from_km2"].as_f64().unwrap()).abs() < 1.0);
    assert!((at(to) - c["area_to_km2"].as_f64().unwrap()).abs() < 1.0);
    // a city lies in the land of its country (cities follow their land)
    let cities = ok(&s, json!({"op": "entities.list", "args": {"kind": "city"}, "context": {"year": 1600}}));
    for city in cities["items"].as_array().unwrap() {
        let p = &city["position"];
        let here = ok(&s, json!({"op": "place.at", "args": {"lon": p[0], "lat": p[1]}, "context": {"year": 1600}}));
        assert_eq!(here["layers"][0]["owner"], city["parent"], "{city}");
    }
    // the statistics table lists exactly the countries that exist
    let st = ok(&s, json!({"op": "stats.table", "args": {"fields": ["population"]}, "context": {"year": 1600}}));
    let list = ok(&s, json!({"op": "entities.list", "args": {"kind": "country"}, "context": {"year": 1600}}));
    assert_eq!(st["rows"].as_array().unwrap().len(), list["count"].as_u64().unwrap() as usize);
    assert!(st["rows"][0]["values"]["population"]["source"].as_str().unwrap().starts_with("interpolated") || st["rows"][0]["values"]["population"]["source"].as_str().unwrap().starts_with("recorded"));
    // the test world has nothing to confirm
    let chk = ok(&s, json!({"op": "checks.run", "context": {"year": 1600}}));
    assert_eq!(chk["topology"], "ok");
    assert!(chk["lifespan_conflicts"].as_array().unwrap().is_empty());
}

#[test]
fn place_uses_the_requested_year_for_all_layers() {
    use map_core::doc::{Document, EntityKind, LayerKind, POLITICAL};
    use map_core::{FaceKind, LonLat};

    let mut d = Document::default();
    let ring = [(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)]
        .map(|(lon, lat)| LonLat::new(lon, lat));
    let face = d.add_ring_face(&ring, FaceKind::Land).unwrap();
    let country = d.add_entity(EntityKind::Country, "Country");
    d.assign(face, Some(country)).unwrap();
    let exclusive = d.add_layer("Exclusive");
    d.set_active_layer(exclusive).unwrap();
    let governor = d.add_entity(EntityKind::Country, "Governor");
    d.assign(face, Some(governor)).unwrap();
    let shared = d.add_layer_of("Shared", LayerKind::Shares);
    d.set_active_layer(shared).unwrap();
    let early = d.add_entity(EntityKind::Country, "Early");
    let late = d.add_entity(EntityKind::Country, "Late");
    d.set_share(face, Some(early), 3000).unwrap();
    d.set_share(face, Some(late), 7000).unwrap();
    d.set_lifespan(late, Some(150), None).unwrap();
    d.set_year(Some(200));
    d.record_kind(face, FaceKind::Water).unwrap();
    d.set_year(None);

    let s = Session::new(d, "layers");
    let place = |year, layer| ok(&s, json!({
        "op": "place.at", "args": {"lon": 2.0, "lat": 2.0},
        "context": {"year": year, "layer": layer},
    }));
    let first = place(Some(100), POLITICAL.0);
    assert_eq!(first["face"], face.0);
    assert_eq!(first["land"], true);
    assert_eq!(first["layers"][0]["owner"]["id"], country.0);
    assert_eq!(first["layers"][1]["owner"]["id"], governor.0);
    assert_eq!(first["layers"][2]["owner"]["id"], early.0);
    assert_eq!(first["layers"][2]["shares"].as_array().unwrap().len(), 1);
    assert_eq!(place(Some(150), exclusive.0)["layers"][2]["owner"]["id"], late.0);
    let flooded = place(None, shared.0);
    assert_eq!(flooded["land"], false);
    assert!(flooded["layers"].as_array().unwrap().iter().all(|layer| layer["owner"].is_null()));
    assert_eq!(flooded["layers"][2]["shares"], json!([]));
}

#[test]
fn previews_say_where_things_are() {
    let s = session();
    let dir = std::env::temp_dir().join(format!("atlas_api_prev_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let id = ok(&s, json!({"op": "entities.list", "args": {"kind": "country", "limit": 1}, "context": {"year": 1500}}))["items"][0]["id"].clone();
    let path = dir.join("p.png");
    let r = ok(&s, json!({"op": "preview.render", "args": {"path": path.display().to_string(), "focus": {"entity": id}}, "context": {"year": 1500}}));
    assert!(std::fs::metadata(&path).unwrap().len() > 1000);
    let objs = r["objects"].as_array().unwrap();
    let me = objs.iter().find(|o| o["id"] == id).expect("the focused country is in the picture");
    let b: Vec<f64> = me["bbox_px"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    // focused: it fills most of the picture, and its name is inside it
    assert!(b[2] - b[0] > 600.0 || b[3] - b[1] > 350.0, "{b:?}");
    let a: Vec<f64> = me["anchor_px"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(a[0] >= b[0] && a[0] <= b[2] && a[1] >= b[1] && a[1] <= b[3]);
    // every reported position is in the image
    for o in objs {
        if let Some(a) = o["anchor_px"].as_array() {
            let (x, y) = (a[0].as_f64().unwrap(), a[1].as_f64().unwrap());
            assert!((0.0..=1200.0).contains(&x) && (0.0..=700.0).contains(&y), "{o}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_are_classified() {
    let s = session();
    assert_eq!(err(&s, json!({"op": "nope"})), "unsupported");
    assert_eq!(err(&s, json!({"args": {}})), "bad_request");
    assert_eq!(err(&s, json!({"op": "world.summary", "context": {"layer": "不存在的层"}})), "not_found");
    assert_eq!(err(&s, json!({"op": "world.summary", "context": {"year": "1900"}})), "bad_request");
    assert_eq!(err(&s, json!({"op": "entity.get", "args": {"id": 999999999}})), "not_found");
    assert_eq!(err(&s, json!({"op": "changes.between", "args": {"from": 1200}})), "bad_request");
    assert_eq!(err(&s, json!({"op": "preview.render", "args": {"path": "x.gif"}})), "bad_request");
    assert_eq!(s.handle_text("not json")["error"]["kind"], "bad_request");
    // the request id comes back
    assert_eq!(s.handle(&json!({"id": "q7", "op": "nope"}))["id"], "q7");
}

#[test]
fn the_version_identifies_the_world() {
    let (d, _) = history_world(&TestWorldSpec { nx: 3, ny: 2, events: 10, ..Default::default() });
    let a = Session::new(d.clone(), "a");
    let b = Session::new(d.clone(), "b");
    assert_eq!(a.version(), b.version());
    let mut d2 = d;
    d2.meta.name = "改名".into();
    assert_ne!(Session::new(d2, "c").version(), a.version());
}
