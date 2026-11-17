//! Land drawn around a pole: a polar continent, countries on it, and
//! what is drawn across its coast afterwards.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::{EntityKind, FaceKind, BACKGROUND};

/// A ring along a parallel, eastward or westward.
fn parallel(lat: f64, step: f64, east: bool) -> Vec<LonLat> {
    let n = (360.0 / step) as usize;
    let mut v: Vec<LonLat> = (0..n).map(|i| LonLat::new(-180.0 + step * i as f64 + 1.0, lat + if i % 2 == 0 { 0.0 } else { 0.7 })).collect();
    if !east {
        v.reverse();
    }
    v
}

fn rect(w: f64, s: f64, e: f64, n: f64) -> Vec<LonLat> {
    vec![LonLat::new(w, s), LonLat::new(e, s), LonLat::new(e, n), LonLat::new(w, n)]
}

fn land_area(d: &Document) -> f64 {
    d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).map(|(f, _)| d.topo.face_area(*f, d.radius())).sum()
}

/// Area of the cap poleward of a latitude, km².
fn cap(d: &Document, lat: f64) -> f64 {
    let r = d.radius();
    2.0 * std::f64::consts::PI * r * r * (1.0 - lat.abs().to_radians().sin())
}

#[test]
fn a_continent_around_either_pole() {
    for (lat, east) in [(70.0, true), (70.0, false), (-65.0, true), (-65.0, false)] {
        let d = Document::default();
        let (n, rep) = overlay::apply(&d, &parallel(lat, 10.0, east), DrawIntent::AddLand).unwrap_or_else(|e| panic!("{lat} {east}: {e}"));
        n.topo.validate().unwrap_or_else(|e| panic!("{lat} {east}: {e}"));
        let pole = LonLat::new(30.0, if lat > 0.0 { 89.0 } else { -89.0 });
        let f = n.topo.face_at(pole);
        assert_ne!(f, BACKGROUND, "{lat} {east}: the pole is on the land");
        assert_eq!(n.topo.faces[&f].kind, FaceKind::Land);
        assert_eq!(n.topo.face_at(LonLat::new(30.0, 0.0)), BACKGROUND, "the equator is still sea");
        assert_eq!(n.topo.face_at(LonLat::new(30.0, if lat > 0.0 { -89.0 } else { 89.0 })), BACKGROUND, "the other pole is still sea");
        let a = land_area(&n);
        // the coast zigzags between the parallel and 0.7° north of it
        let (lo, hi) = if lat > 0.0 { (cap(&n, lat + 0.7), cap(&n, lat)) } else { (cap(&n, lat), cap(&n, lat + 0.7)) };
        assert_eq!(rep.faces_after, 1, "one face, not cut at the antimeridian");
        assert!(a > lo * 0.98 && a < hi * 1.02, "{lat} {east}: area {a:.0} not between {lo:.0} and {hi:.0}");
        assert!((rep.added_km2 - a).abs() < a * 0.02, "reported {} of {a}", rep.added_km2);
    }
}

/// The polar continent, then things drawn on and across it.
fn continent() -> Document {
    overlay::apply(&Document::default(), &parallel(70.0, 10.0, true), DrawIntent::AddLand).unwrap().0
}

fn check(n: &Document, what: &str) {
    n.topo.validate().unwrap_or_else(|e| panic!("{what}: {e}"));
}

#[test]
fn countries_on_a_polar_continent() {
    let mut d = continent();
    let all = land_area(&d);
    let a = d.add_entity(EntityKind::Country, "北国");
    let b = d.add_entity(EntityKind::Country, "极国");
    // a country inside the continent, away from the pole
    let (n, _) = overlay::apply(&d, &rect(10.0, 74.0, 60.0, 82.0), DrawIntent::Expand(a)).unwrap();
    check(&n, "inside");
    assert_eq!(n.owner.get(&n.topo.face_at(LonLat::new(30.0, 78.0))), Some(&a));
    assert!(n.owner.get(&n.topo.face_at(LonLat::new(-100.0, 80.0))).is_none());
    assert!((land_area(&n) - all).abs() < all * 1e-6, "no land gained or lost");
    // a country around the pole itself, inside the continent
    let (n2, _) = overlay::apply(&n, &parallel(85.0, 20.0, false), DrawIntent::Expand(b)).unwrap();
    check(&n2, "around the pole");
    assert_eq!(n2.owner.get(&n2.topo.face_at(LonLat::new(0.0, 89.0))), Some(&b));
    // its label is by the pole, inside it, not at the edge of the map
    let (at, width) = n2.label_anchor(b).unwrap();
    assert!(at.lat > 86.0 && at.lon.abs() < 1.0 && width > 3.0, "{at:?} {width}");
    assert_eq!(n2.owner.get(&n2.topo.face_at(at)), Some(&b));
    assert_eq!(n2.owner.get(&n2.topo.face_at(LonLat::new(30.0, 78.0))), Some(&a));
    assert!(n2.owner.get(&n2.topo.face_at(LonLat::new(-100.0, 80.0))).is_none());
    assert!((land_area(&n2) - all).abs() < all * 1e-6);
    let areas = n2.areas_by_entity();
    assert!((areas[&b] - cap(&n2, 85.35)).abs() < cap(&n2, 85.35) * 0.1, "{}", areas[&b]);
    // taking the whole continent for one country, by a ring further south
    let (n3, _) = overlay::apply(&n2, &parallel(60.0, 10.0, true), DrawIntent::Take(a)).unwrap();
    check(&n3, "take all");
    // (the sea inside the ring becomes its land too)
    let got = n3.areas_by_entity()[&a];
    assert!((got - land_area(&n3)).abs() < got * 1e-6, "all the land is its own");
    assert!(got > cap(&n3, 60.7) * 0.98 && got < cap(&n3, 60.0) * 1.02, "{got}");
    for (lon, lat) in [(0.0, 89.0), (30.0, 78.0), (-100.0, 80.0), (179.5, 65.0), (-90.0, 63.0)] {
        assert_eq!(n3.owner.get(&n3.topo.face_at(LonLat::new(lon, lat))), Some(&a), "{lon} {lat}");
    }
    // the land around the pole country is in two faces (a face cannot
    // have the antimeridian inside it), and they can not be merged into one
    let ring_faces: Vec<_> = n2.topo.faces.keys().copied().filter(|f| *f != BACKGROUND && n2.owner.get(f).is_none()).collect();
    assert_eq!(ring_faces.len(), 2);
    let mut t = n2.clone();
    assert_eq!(t.tidy_faces(), 0);
    check(&t, "tidied");
}

#[test]
fn land_and_water_across_a_polar_coast() {
    let d = continent();
    let all = land_area(&d);
    // a peninsula from the coast southward
    let (n, _) = overlay::apply(&d, &rect(20.0, 55.0, 40.0, 75.0), DrawIntent::AddLand).unwrap();
    check(&n, "peninsula");
    assert!(land_area(&n) > all * 1.01);
    assert_ne!(n.topo.face_at(LonLat::new(30.0, 58.0)), BACKGROUND);
    // a bay cut into the coast
    let (n, _) = overlay::apply(&n, &rect(-60.0, 65.0, -30.0, 78.0), DrawIntent::CarveWater).unwrap();
    check(&n, "bay");
    assert_eq!(n.topo.face_at(LonLat::new(-45.0, 75.0)), BACKGROUND, "the bay is open sea");
    // a lake over the pole
    let (n, _) = overlay::apply(&n, &parallel(87.0, 30.0, true), DrawIntent::CarveWater).unwrap();
    check(&n, "polar lake");
    let lake = n.topo.face_at(LonLat::new(0.0, 89.5));
    assert_eq!(n.topo.faces[&lake].kind, FaceKind::Water);
    assert_eq!(n.topo.faces[&n.topo.face_at(LonLat::new(100.0, 80.0))].kind, FaceKind::Land);
    // the whole continent sunk again
    let (n, _) = overlay::apply(&n, &parallel(50.0, 10.0, false), DrawIntent::CarveWater).unwrap();
    check(&n, "sunk");
    assert!(land_area(&n) < 1.0);
}

#[test]
fn a_polar_continent_in_a_year() {
    let mut d = Document::default();
    d.add_ring_face(&rect(0.0, 0.0, 20.0, 20.0), FaceKind::Land).unwrap();
    d.set_year(Some(1500));
    let (n, _) = overlay::apply(&d, &parallel(-70.0, 10.0, true), DrawIntent::AddLand).unwrap();
    check(&n, "dated");
    let f = n.topo.face_at(LonLat::new(0.0, -89.0));
    assert_eq!(n.topo.faces[&f].kind, FaceKind::Land, "land from 1500");
    let mut before = n.clone();
    before.set_year(Some(1400));
    assert_eq!(before.topo.faces[&before.topo.face_at(LonLat::new(0.0, -89.0))].kind, FaceKind::Water, "sea before 1500");
    assert_eq!(before.topo.faces[&before.topo.face_at(LonLat::new(10.0, 10.0))].kind, FaceKind::Land);
}

/// A lake across the antimeridian on a polar continent: the land has to
/// be cut along a meridian, and the pieces meet at the pole. Nothing is
/// left over there (no face without area), and the map is valid.
#[test]
fn a_lake_on_the_antimeridian_of_a_polar_continent() {
    let d = continent();
    let all = land_area(&d);
    let (n, _) = overlay::apply(&d, &rect(170.0, 76.0, -170.0, 82.0), DrawIntent::CarveWater).unwrap();
    check(&n, "lake on the seam");
    let lake = n.topo.face_at(LonLat::new(179.0, 79.0));
    assert_eq!(n.topo.faces[&lake].kind, FaceKind::Water);
    assert_eq!(n.topo.faces[&n.topo.face_at(LonLat::new(-179.0, 79.0))].kind, FaceKind::Water);
    for (lon, lat) in [(0.0, 89.0), (90.0, 80.0), (-90.0, 80.0), (179.0, 73.0), (-179.0, 85.0)] {
        let f = n.topo.face_at(LonLat::new(lon, lat));
        assert!(f != BACKGROUND && n.topo.faces[&f].kind == FaceKind::Land, "{lon} {lat} is land");
    }
    let r = n.radius();
    let mut sum = 0.0;
    for f in n.topo.faces.keys().filter(|f| **f != BACKGROUND) {
        let a = n.topo.face_area(*f, r);
        assert!(a > 1.0, "{f:?} has no area ({a})");
        sum += a;
    }
    let lake_area: f64 = n.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Water).map(|(f, _)| n.topo.face_area(*f, r)).sum();
    assert!((sum - all).abs() < all * 1e-6, "land and lake together are the continent");
    assert!((land_area(&n) + lake_area - all).abs() < all * 1e-6 && lake_area > 100_000.0);
    // one outline for the whole of it: the coast and the lake, nothing at the pole
    let land: std::collections::HashSet<_> = n.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).map(|(f, _)| *f).collect();
    let shape = n.dissolve_faces(&land);
    assert_eq!(shape.len(), 1, "one polygon");
    let r = n.radius();
    let outline_area = map_core::geom::spherical_area(&shape[0].outer, r).abs() - shape[0].holes.iter().map(|h| map_core::geom::spherical_area(h, r).abs()).sum::<f64>();
    assert!((outline_area - land_area(&n)).abs() < all * 1e-6, "{outline_area} for {}", land_area(&n));
    // edges on the pole itself are not borders
    assert!(n.topo.edges.values().filter(|e| n.topo.on_pole(e)).count() >= 1);
    // tidying may or may not join the pieces, but leaves a valid map of the same land
    let mut t = n.clone();
    t.tidy_faces();
    check(&t, "tidied");
    assert!((land_area(&t) - land_area(&n)).abs() < all * 1e-6);
    for (lon, lat) in [(0.0, 89.0), (90.0, 80.0), (-90.0, 80.0)] {
        assert_eq!(t.topo.faces[&t.topo.face_at(LonLat::new(lon, lat))].kind, FaceKind::Land);
    }
    // drawing on goes on working: a country over the pole, the lake filled in again
    let mut n = n;
    let c = n.add_entity(EntityKind::Country, "极国");
    let (n, _) = overlay::apply(&n, &parallel(86.0, 20.0, true), DrawIntent::Expand(c)).unwrap();
    check(&n, "country over the pole");
    assert_eq!(n.owner.get(&n.topo.face_at(LonLat::new(40.0, 89.0))), Some(&c));
    let (n, _) = overlay::apply(&n, &rect(169.0, 75.0, -169.0, 83.0), DrawIntent::AddLand).unwrap();
    check(&n, "lake filled");
    assert!((land_area(&n) - all).abs() < all * 1e-6);
    // with the lake gone and the country given up, tidying makes one face of the continent again
    let mut t = n.clone();
    t.delete_entity(c);
    t.tidy_faces();
    check(&t, "one continent again");
    let faces: Vec<_> = t.topo.faces.keys().copied().filter(|f| *f != BACKGROUND).collect();
    assert_eq!(faces.len(), 1, "one face");
    assert!(t.topo.is_polar(&t.topo.faces[&faces[0]].outer) && t.topo.edges.values().all(|e| !t.topo.on_pole(e)));
    assert!((land_area(&t) - all).abs() < all * 1e-6);
}

/// Cuts that must be refused rather than leave an invalid map: a step
/// half way round the globe, and a line through a vertex of the border.
#[test]
fn cuts_that_are_refused() {
    let mut d = Document::default();
    // a land 180° wide, as the pieces of land around a pole are
    d.add_ring_face(&[LonLat::new(-170.0, 40.0), LonLat::new(-90.0, 40.0), LonLat::new(10.0, 40.0), LonLat::new(10.0, 60.0), LonLat::new(-80.0, 60.0), LonLat::new(-170.0, 60.0)], FaceKind::Land).unwrap();
    let before = d.clone();
    let e = d.split_face(&[LonLat::new(-170.0, 40.0), LonLat::new(10.0, 60.0)]).unwrap_err();
    assert!(e.contains("半个地球"), "{e}");
    // along the south coast, through its middle vertex
    let e = d.split_face(&[LonLat::new(-170.0, 40.0), LonLat::new(-80.0, 40.0), LonLat::new(-40.0, 50.0), LonLat::new(10.0, 60.0)]).unwrap_err();
    assert!(e.contains("顶点") || e.contains("相交"), "{e}");
    assert_eq!(d.topo.faces.len(), before.topo.faces.len());
    d.topo.validate().unwrap();
    // with a point in between it is an ordinary cut
    d.split_face(&[LonLat::new(-170.0, 40.0), LonLat::new(-85.0, 50.0), LonLat::new(10.0, 60.0)]).unwrap();
    d.topo.validate().unwrap();
    assert_eq!(d.topo.faces.len(), before.topo.faces.len() + 1);
}
