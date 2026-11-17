//! Coastlines through time: geometry is shared by all years, each face's
//! land/water has a history. An edit made in a later year never changes
//! an earlier year's land, sea or owners.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::topo::{FaceId, FaceKind};
use map_core::{EntityId, EntityKind};
use std::collections::HashMap;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<LonLat> {
    vec![LonLat::new(x0, y0), LonLat::new(x1, y0), LonLat::new(x1, y1), LonLat::new(x0, y1)]
}

/// Land area (all land faces) and owner areas in a year.
fn state(d: &Document, year: Option<i32>) -> (i64, HashMap<EntityId, i64>) {
    let mut v = d.clone();
    v.set_year(year);
    let r = v.radius();
    let land: f64 = v.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| v.topo.face_area(*id, r)).sum();
    (land.round() as i64, v.areas_by_entity().into_iter().map(|(e, a)| (e, a.round() as i64)).collect())
}

/// One country X on a 20°×20° block.
fn world() -> (Document, FaceId, EntityId) {
    let mut d = Document::default();
    let f = d.add_ring_face(&rect(0.0, 0.0, 20.0, 20.0), FaceKind::Land).unwrap();
    let x = d.add_entity(EntityKind::Country, "X");
    d.assign(f, Some(x)).unwrap();
    (d, f, x)
}

#[test]
fn an_island_drawn_in_a_year_rises_from_the_sea_then() {
    let (mut d, _, _) = world();
    let before = state(&d, Some(1900));
    d.set_year(Some(1950));
    let isl = d.add_ring_face(&rect(40.0, 0.0, 45.0, 5.0), FaceKind::Land).unwrap();
    d.topo.validate().unwrap();
    assert_eq!(d.face_kind_at(isl, Some(1949)), Some(FaceKind::Water));
    assert_eq!(d.face_kind_at(isl, Some(1950)), Some(FaceKind::Land));
    assert_eq!(state(&d, Some(1900)), before, "nothing changes before 1950");
    let a = d.topo.face_area(isl, d.radius()).round() as i64;
    assert!((state(&d, Some(1950)).0 - (before.0 + a)).abs() <= 1, "rounded separately");
    assert!(d.change_years().contains(&1950));
    // the view follows the year
    d.set_year(Some(1900));
    assert_eq!(d.topo.faces[&isl].kind, FaceKind::Water);
    d.set_year(None);
    assert_eq!(d.topo.faces[&isl].kind, FaceKind::Land, "no year: latest");
}

#[test]
fn a_lake_drawn_in_a_year_was_the_countrys_land_before() {
    let (mut d, _, x) = world();
    let before = state(&d, Some(1900));
    d.set_year(Some(1950));
    let lake = d.add_ring_face(&rect(5.0, 5.0, 8.0, 8.0), FaceKind::Water).unwrap();
    let a = d.topo.face_area(lake, d.radius()).round() as i64;
    assert_eq!(state(&d, Some(1900)), before);
    let (land, own) = state(&d, Some(1950));
    assert_eq!(land, before.0 - a);
    assert!((own[&x] - (before.1[&x] - a)).abs() <= 1);
    d.set_year(Some(1940));
    assert_eq!(d.owner.get(&lake), Some(&x), "the area was X's land before");
    d.set_year(Some(1960));
    assert!(!d.owner.contains_key(&lake), "water has no owner");
}

#[test]
fn toggling_land_and_water_keeps_owners_for_later_years() {
    let (mut d, f, x) = world();
    d.set_year(Some(1980));
    d.record_kind(f, FaceKind::Water).unwrap();
    d.set_year(Some(2000));
    d.record_kind(f, FaceKind::Land).unwrap();
    for (y, kind, owner) in [(1970, FaceKind::Land, Some(x)), (1990, FaceKind::Water, None), (2010, FaceKind::Land, Some(x))] {
        d.set_year(Some(y));
        assert_eq!(d.topo.faces[&f].kind, kind, "{y}");
        assert_eq!(d.owner.get(&f).copied(), owner, "{y}");
    }
    assert_eq!(d.kind_history(f), (FaceKind::Land, vec![(1980, FaceKind::Water), (2000, FaceKind::Land)]));
    // all-years edits of a history are refused, the background is fixed
    d.set_year(None);
    let before = d.clone();
    assert!(d.record_kind(f, FaceKind::Water).is_err());
    assert_eq!(d, before);
    assert!(d.record_kind(map_core::topo::BACKGROUND, FaceKind::Land).is_err());
}

#[test]
fn splits_share_and_merges_require_equal_coast_histories() {
    let (mut d, f, _) = world();
    d.set_year(Some(1980));
    d.record_kind(f, FaceKind::Water).unwrap();
    d.set_year(None);
    // splitting needs a year-free edit of geometry only: shared by both halves
    d.set_year(Some(1970));
    let (a, b) = d.split_face(&[LonLat::new(10.0, 0.0), LonLat::new(10.0, 20.0)]).unwrap();
    assert_eq!(d.kind_history(a), d.kind_history(b));
    d.set_year(Some(1990));
    d.record_kind(b, FaceKind::Land).unwrap();
    let before = d.clone();
    assert!(d.merge_faces(a, b).is_err());
    assert_eq!(d, before);
    d.record_kind(b, FaceKind::Water).unwrap();
    assert_eq!(d.kind_history(a), d.kind_history(b));
    d.merge_faces(a, b).unwrap();
    d.topo.validate().unwrap();
}

#[test]
fn drawing_across_the_coast_in_a_year_leaves_earlier_years() {
    let (d0, _, x) = world();
    let mut d = d0.clone();
    let y_ = d.add_entity(EntityKind::Country, "Y");
    let early = state(&d, Some(1900));
    d.set_year(Some(1950));
    // new land reaching from X's coast into the sea, given to Y
    let (n, rep) = overlay::apply(&d, &rect(15.0, 5.0, 30.0, 10.0), DrawIntent::Take(y_)).unwrap();
    n.topo.validate().unwrap();
    assert!(rep.added_km2 > 0.0 && !rep.taken.is_empty());
    assert_eq!(state(&n, Some(1900)), early, "1900: no new land, X whole");
    let (land, own) = state(&n, Some(1950));
    assert_eq!(land, early.0 + rep.added_km2.round() as i64);
    assert!(own[&y_] > 0 && own[&x] < early.1[&x]);
    // carving a bay into X in 1960: X keeps the land until then
    let mut n = n;
    n.set_year(Some(1960));
    let mid = state(&n, Some(1955));
    let (m, rep) = overlay::apply(&n, &rect(-5.0, 12.0, 6.0, 16.0), DrawIntent::CarveWater).unwrap();
    m.topo.validate().unwrap();
    assert!(rep.carved_km2 > 0.0);
    assert_eq!(state(&m, Some(1955)), mid, "before 1960 the bay was land");
    assert_eq!(state(&m, Some(1900)), early);
    let (land, own) = state(&m, Some(1960));
    assert_eq!(land, mid.0 - rep.carved_km2.round() as i64);
    assert!((own[&x] - (mid.1[&x] - rep.carved_km2.round() as i64)).abs() <= 1);
    // saved and loaded, the coast history survives
    let mut back: Document = rmp_serde::from_slice(&rmp_serde::to_vec_named(&m).unwrap()).unwrap();
    back.topo.rebuild_index();
    back.refresh_time_view();
    assert_eq!(state(&back, Some(1955)), mid);
    assert_eq!(state(&back, Some(1960)), state(&m, Some(1960)));
}

/// Snapping makes a drawn island start exactly on an existing coast vertex
/// or edge. Whatever path handles it, the topology must stay valid.
#[test]
fn islands_touching_a_coast_at_a_snapped_point_stay_valid() {
    for touch in [LonLat::new(20.0, 0.0), LonLat::new(20.0, 5.0)] {
        let (d, _, _) = world();
        let ring = vec![touch, LonLat::new(30.0, 2.0), LonLat::new(30.0, 8.0), LonLat::new(25.0, 9.0)];
        let mut fast = d.clone();
        match fast.add_ring_face(&ring, FaceKind::Land) {
            Ok(_) => fast.topo.validate().unwrap_or_else(|e| panic!("fast path at {touch:?}: {e}")),
            Err(e) => eprintln!("fast path refused at {touch:?}: {e}"),
        }
        let (n, _) = overlay::apply(&d, &ring, DrawIntent::AddLand).unwrap();
        n.topo.validate().unwrap();
    }
}

/// Restyling a country's borders changes shared edges for both sides, keeps
/// every junction, and leaves a valid topology; an island's single-edge
/// coast is restyled as a closed ring.
#[test]
fn restyling_borders_keeps_topology_valid() {
    use map_core::shape::LineStyle;
    let mut d = map_core::sample::demo_world();
    let r = d.radius();
    let nodes_before = d.topo.nodes.len();
    let faces: Vec<FaceId> = d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    let total = |d: &Document| d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| d.topo.face_area(*id, r)).sum::<f64>();
    let before = total(&d);
    for (i, f) in faces.iter().enumerate() {
        let style = if i % 2 == 0 { LineStyle::Natural } else { LineStyle::Smooth };
        let mut t = d.clone();
        match t.restyle_face(*f, style, 0.5, 0.3, false) {
            Ok(n) => {
                t.topo.validate().unwrap_or_else(|e| panic!("face {f:?} {style:?}: {e}"));
                assert!(n > 0);
                d = t;
            }
            Err(e) => eprintln!("face {f:?} {style:?} refused: {e}"),
        }
    }
    assert_eq!(d.topo.nodes.len(), nodes_before, "junctions stay");
    // land area changes only a little (detail goes both ways)
    let after = total(&d);
    assert!((after - before).abs() < 0.05 * before, "{before} -> {after}");
}

/// With "keep area", reshaping borders leaves every country's area the
/// same in every year (only shapes change); without it areas drift.
#[test]
fn restyling_with_keep_area_keeps_every_countrys_area_in_every_year() {
    use map_core::shape::LineStyle;
    let mut d = map_core::sample::demo_world();
    // give the map some history: one country takes another's land in 1900
    let cs = d.countries();
    let f = d.faces_of(cs[1])[0];
    d.set_year(Some(1900));
    d.assign(f, Some(cs[0])).unwrap();
    d.set_year(None);
    let years = [Some(1899), Some(1900), None];
    let areas = |d: &Document| -> Vec<HashMap<EntityId, f64>> {
        years.iter().map(|y| {
            let mut v = d.clone();
            v.set_year(*y);
            v.areas_by_entity()
        }).collect()
    };
    let before = areas(&d);
    let land: Vec<FaceId> = d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    // fine detail, and coarse detail (zoomed out) where thinning alone
    // can turn a border into a straight line
    for min_len in [0.3, 2.0] {
        let (mut kept, mut drifted) = (d.clone(), d.clone());
        for (i, f) in land.iter().enumerate() {
            let style = if i % 2 == 0 { LineStyle::Natural } else { LineStyle::Smooth };
            let _ = kept.restyle_face(*f, style, 0.7, min_len, true);
            let _ = drifted.restyle_face(*f, style, 0.7, min_len, false);
        }
        kept.topo.validate().unwrap();
        let (k, dr) = (areas(&kept), areas(&drifted));
        let mut worst = (0.0f64, 0.0f64);
        for y in 0..years.len() {
            for (e, a) in &before[y] {
                worst.0 = worst.0.max((k[y][e] - a).abs() / a);
                worst.1 = worst.1.max((dr[y][e] - a).abs() / a);
            }
        }
        assert!(worst.0 < 1e-3, "min_len {min_len}: kept, worst relative change {}", worst.0);
        assert!(worst.1 > 10.0 * worst.0, "min_len {min_len}: without keep-area the change is larger: {:?}", worst);
    }
}
