//! The year's view of ownership (`Document::owner`, areas) follows the same
//! rules however it is refreshed: after one edit, after a year change, or
//! after loading. Water has no owner; an entity that does not exist in the
//! year owns nothing.

use map_core::doc::{Document, LayerKind, FULL_SHARE};
use map_core::{EntityKind, FaceKind};

/// The view equals a full refresh of the same document.
fn view_is_fresh(d: &Document, what: &str) {
    let mut full = d.clone();
    full.refresh_time_view();
    assert_eq!(d.owner, full.owner, "{what}: owners differ from a full refresh");
    for (id, x) in d.entities.iter() {
        assert_eq!(x.parent, full.entities[id].parent, "{what}: parent of {}", x.name);
    }
}

fn area(d: &Document, e: map_core::EntityId) -> f64 {
    d.areas_by_entity().get(&e).copied().unwrap_or(0.0)
}

/// Land made water with no year selected stays out of its former owner's
/// area after a year change (it used to come back: only faces with a
/// land/water history lost their owner in a full refresh).
#[test]
fn land_made_water_for_all_years_stays_unowned() {
    let mut d = map_core::sample::demo_world();
    let e = d.countries()[0];
    let f = d.faces_of(e)[0];
    let before = area(&d, e);
    let lost = d.topo.face_area(f, d.radius());
    d.record_kind(f, FaceKind::Water).unwrap();
    assert!((area(&d, e) - (before - lost)).abs() < 1e-6);
    view_is_fresh(&d, "after making it water");
    d.set_year(Some(1900));
    assert!((area(&d, e) - (before - lost)).abs() < 1e-6, "1900");
    d.set_year(None);
    assert!((area(&d, e) - (before - lost)).abs() < 1e-6, "back to no year");
    assert!(d.owner.get(&f).is_none());
    // made land again, it is its recorded owner's again
    d.record_kind(f, FaceKind::Land).unwrap();
    assert!((area(&d, e) - before).abs() < 1e-6);
    view_is_fresh(&d, "after making it land again");
}

/// With a year: water from that year on, the owner's before it.
#[test]
fn land_made_water_from_a_year() {
    let mut d = map_core::sample::demo_world();
    let e = d.countries()[0];
    let f = d.faces_of(e)[0];
    let before = area(&d, e);
    d.set_year(Some(1950));
    d.record_kind(f, FaceKind::Water).unwrap();
    view_is_fresh(&d, "1950");
    assert!(d.owner.get(&f).is_none());
    d.set_year(Some(1949));
    assert!((area(&d, e) - before).abs() < 1e-6, "earlier years keep the land");
    d.set_year(None);
    assert!(d.owner.get(&f).is_none(), "latest state: water");
}

/// Merging into, or deleting next to, an entity that does not exist in the
/// viewed year: the view right after the edit is the refreshed view (the
/// per-face refresh used to ignore lifespans).
#[test]
fn edits_keep_lifespans_in_the_view() {
    let mut d = map_core::sample::demo_world();
    let c = d.countries();
    d.set_lifespan(c[1], Some(1950), None).unwrap();
    d.set_year(Some(1900));
    assert!(d.faces_of(c[1]).is_empty());
    d.merge_entities(c[0], c[1]).unwrap();
    assert!(d.faces_of(c[1]).is_empty(), "not founded yet: owns nothing in 1900");
    view_is_fresh(&d, "after the merge");
    d.set_year(Some(1960));
    assert!(d.faces_of(c[1]).len() >= 2, "from its founding it holds both countries' land");
    // splitting and deleting keep the view fresh too
    d.set_year(Some(1900));
    d.delete_entity(c[2]);
    view_is_fresh(&d, "after a deletion");
}

/// On a share layer too: water carries no shares into the areas, and a
/// unit that does not exist in the year holds nothing.
#[test]
fn share_layers_follow_the_same_rules() {
    let mut d = map_core::sample::demo_world();
    let land: Vec<_> = d.countries().iter().flat_map(|c| d.faces_of(*c)).collect();
    let l = d.add_layer_of("宗教", LayerKind::Shares);
    d.set_active_layer(l).unwrap();
    let (a, b) = (d.add_entity(EntityKind::Country, "甲教"), d.add_entity(EntityKind::Country, "乙教"));
    for f in &land {
        d.set_share(*f, Some(a), FULL_SHARE / 2).unwrap();
        d.set_share(*f, Some(b), FULL_SHARE / 2).unwrap();
    }
    let total: f64 = land.iter().map(|f| d.topo.face_area(*f, d.radius())).sum();
    assert!((area(&d, a) - total / 2.0).abs() < 1e-3);
    // one face becomes water from 1950
    let f = land[0];
    let fa = d.topo.face_area(f, d.radius());
    d.set_year(Some(1950));
    d.record_kind(f, FaceKind::Water).unwrap();
    assert!((area(&d, a) - (total - fa) / 2.0).abs() < 1e-3, "water carries no shares");
    view_is_fresh(&d, "share layer, water");
    d.set_year(Some(1949));
    assert!((area(&d, a) - total / 2.0).abs() < 1e-3);
    // a faith founded in 1800 holds nothing before
    d.set_lifespan(b, Some(1800), None).unwrap();
    d.set_year(Some(1700));
    assert_eq!(area(&d, b), 0.0, "not founded yet");
    assert!((area(&d, a) - total / 2.0).abs() < 1e-3, "the other faith's share is its own");
    assert!(d.owner.values().all(|e| *e != b));
    view_is_fresh(&d, "share layer, lifespans");
    d.set_year(Some(1900));
    assert!((area(&d, b) - total / 2.0).abs() < 1e-3);
}
