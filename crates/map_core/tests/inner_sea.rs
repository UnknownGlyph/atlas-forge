//! Land drawn across a strait encloses the sea behind it. That sea is no
//! longer part of the background: it becomes a water face of its own, and
//! the map stays valid. (A local rebuild used to leave it in the
//! background, whose rings then had the wrong orientation.)

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::{EntityKind, FaceKind, BACKGROUND};

fn ring(pts: &[(f64, f64)]) -> Vec<LonLat> {
    pts.iter().map(|(x, y)| LonLat::new(*x, *y)).collect()
}

/// A C-shaped country: a 10° square with a 4° bay inside, open to the
/// east through a strait 1° wide.
fn bay() -> (Document, map_core::EntityId) {
    let mut d = Document::default();
    let c = ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 4.5), (7.0, 4.5), (7.0, 3.0), (3.0, 3.0), (3.0, 7.0), (7.0, 7.0), (7.0, 5.5), (10.0, 5.5), (10.0, 10.0), (0.0, 10.0)]);
    let f = d.add_ring_face(&c, FaceKind::Land).unwrap();
    let a = d.add_entity(EntityKind::Country, "湾国");
    d.assign(f, Some(a)).unwrap();
    (d, a)
}

/// The land that closes the strait: over its mouth and a little out to sea.
fn plug() -> Vec<LonLat> {
    ring(&[(8.0, 4.0), (11.0, 4.0), (11.0, 6.0), (8.0, 6.0)])
}

fn area(d: &Document, e: map_core::EntityId) -> f64 {
    d.areas_by_entity().get(&e).copied().unwrap_or(0.0)
}

fn check_closed(n: &Document, what: &str) {
    n.topo.validate().unwrap_or_else(|e| panic!("{what}: {e}"));
    let inner = n.topo.face_at(LonLat::new(5.0, 5.0));
    assert_ne!(inner, BACKGROUND, "{what}: the enclosed sea is a face of its own");
    assert_eq!(n.topo.faces[&inner].kind, FaceKind::Water, "{what}");
    assert!(n.owner.get(&inner).is_none(), "{what}");
    assert_eq!(n.topo.face_at(LonLat::new(20.0, 5.0)), BACKGROUND, "{what}: the open sea is still the background");
    // the strait is land now
    let plug = n.topo.face_at(LonLat::new(9.0, 5.0));
    assert_eq!(n.topo.faces[&plug].kind, FaceKind::Land, "{what}");
}

#[test]
fn closing_a_strait_makes_an_inner_sea() {
    for intent_of in [|a| DrawIntent::Expand(a), |_| DrawIntent::AddLand] {
        let (d, a) = bay();
        assert_eq!(d.topo.face_at(LonLat::new(5.0, 5.0)), BACKGROUND, "open bay: part of the sea");
        let (n, _) = overlay::apply(&d, &plug(), intent_of(a)).unwrap();
        check_closed(&n, "no year");
        // the same map as a full rebuild
        let (w, _) = overlay::apply_whole_world(&d, &plug(), intent_of(a)).unwrap();
        assert!((area(&n, a) - area(&w, a)).abs() < 1e-6);
    }
}

#[test]
fn closing_a_strait_from_a_year() {
    let (mut d, a) = bay();
    let before = area(&d, a);
    d.set_year(Some(1500));
    let (mut n, _) = overlay::apply(&d, &plug(), DrawIntent::Expand(a)).unwrap();
    check_closed(&n, "1500");
    assert!(area(&n, a) > before);
    // before that year the strait is open water and the country as it was
    n.set_year(Some(1499));
    n.topo.validate().unwrap();
    assert!((area(&n, a) - before).abs() < 1e-6, "1499 unchanged");
    let strait = n.topo.face_at(LonLat::new(9.0, 5.0));
    assert_eq!(n.topo.faces[&strait].kind, FaceKind::Water, "1499: the strait is still water");
}

/// Opening it again: carving the plug away joins the inner sea to the
/// open sea without breaking the map.
#[test]
fn opening_the_strait_again() {
    let (d, a) = bay();
    let (n, _) = overlay::apply(&d, &plug(), DrawIntent::Expand(a)).unwrap();
    let (m, _) = overlay::apply(&n, &plug(), DrawIntent::CarveWater).unwrap();
    m.topo.validate().unwrap();
    assert_eq!(m.topo.face_at(LonLat::new(9.0, 5.0)), BACKGROUND, "the strait is sea again");
    assert_eq!(m.topo.face_at(LonLat::new(5.0, 5.0)), BACKGROUND, "and the bay is part of the open sea again");
    // the carving also took the country's own shores of the strait
    let r = d.radius();
    let strip = |y0: f64, y1: f64| map_core::geom::spherical_area(&[[8.0, y0], [10.0, y0], [10.0, y1], [8.0, y1]], r).abs();
    let expected = area(&d, a) - strip(4.0, 4.5) - strip(5.5, 6.0);
    assert!((area(&m, a) - expected).abs() < 1e-6 * expected, "{} vs {expected}", area(&m, a));
}
