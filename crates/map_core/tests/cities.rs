//! Finding and moving cities.

use map_core::doc::{CityMove, Document};
use map_core::geom::LonLat;
use map_core::topo::FaceKind;
use map_core::EntityKind;

fn ll(x: f64, y: f64) -> LonLat {
    LonLat::new(x, y)
}

/// X (west) and Y (east) on a 20°×20° block, a city in X.
fn world() -> (Document, map_core::EntityId, map_core::EntityId, map_core::EntityId) {
    let mut d = Document::default();
    let f = d.add_ring_face(&[ll(0.0, 0.0), ll(20.0, 0.0), ll(20.0, 20.0), ll(0.0, 20.0)], FaceKind::Land).unwrap();
    let x = d.add_entity(EntityKind::Country, "X");
    let y = d.add_entity(EntityKind::Country, "Y");
    d.assign(f, Some(x)).unwrap();
    d.split_face(&[ll(10.0, 0.0), ll(10.0, 20.0)]).unwrap();
    d.assign(d.topo.face_at(ll(15.0, 10.0)), Some(y)).unwrap();
    let c = d.add_city(ll(5.0, 5.0), "城");
    (d, x, y, c)
}

#[test]
fn cities_are_found_near_their_point() {
    let (d, _, _, c) = world();
    assert_eq!(d.city_at(ll(5.2, 5.1), 0.5), Some(c));
    assert_eq!(d.city_at(ll(7.0, 5.0), 0.5), None);
}

#[test]
fn a_moved_city_follows_its_country_unless_it_has_history() {
    let (mut d, x, y, c) = world();
    assert_eq!(d.entities[&c].parent, Some(x));
    assert_eq!(d.move_city(c, ll(6.0, 6.0)).unwrap(), CityMove::Same);
    // no history, no year: the parent follows into Y
    assert_eq!(d.move_city(c, ll(15.0, 6.0)).unwrap(), CityMove::Parent(Some(y)));
    assert_eq!(d.entities[&c].parent, Some(y));
    assert_eq!(d.entities[&c].pos, Some(ll(15.0, 6.0)));
    // the city changed hands in 1950: moving it keeps that history
    d.set_year(Some(1950));
    d.record_parent(c, Some(x)).unwrap();
    d.set_year(None);
    let before = d.entities[&c].parent_changes.clone();
    assert_eq!(d.move_city(c, ll(16.0, 6.0)).unwrap(), CityMove::Kept { here: Some(y) });
    assert_eq!(d.entities[&c].parent_changes, before);
    // countries are not cities
    assert!(d.move_city(x, ll(1.0, 1.0)).is_err());
}

/// Projects saved before labels could be placed load with automatic labels.
#[test]
fn entities_without_label_fields_load_with_automatic_labels() {
    let (d, x, _, _) = world();
    let mut v = serde_json::to_value(&d.entities[&x]).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("label");
    o.remove("label_hidden");
    let e: map_core::doc::Entity = serde_json::from_value(v).unwrap();
    assert_eq!((e.label, e.label_hidden), (None, false));
    // a placement keeps its size factor default when only the position was saved
    let p: map_core::doc::LabelPlace = serde_json::from_value(serde_json::json!({"at": {"lon": 1.0, "lat": 2.0}})).unwrap();
    assert_eq!((p.angle, p.scale), (0.0, 1.0));
}
