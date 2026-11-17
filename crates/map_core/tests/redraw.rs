//! Redrawing a border moves land between the two sides, as an ownership
//! (or land/water) change that holds from the selected year on.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::overlay::DrawIntent;
use map_core::redraw;
use map_core::topo::FaceKind;
use map_core::{EntityId, EntityKind};
use std::collections::HashMap;

fn ll(x: f64, y: f64) -> LonLat {
    LonLat::new(x, y)
}

fn state(d: &Document, year: Option<i32>) -> (i64, HashMap<EntityId, i64>) {
    let mut v = d.clone();
    v.set_year(year);
    let r = v.radius();
    let land: f64 = v.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| v.topo.face_area(*id, r)).sum();
    (land.round() as i64, v.areas_by_entity().into_iter().map(|(e, a)| (e, a.round() as i64)).collect())
}

/// X holds the west half and Y the east half of a 20°×20° block.
fn world() -> (Document, EntityId, EntityId) {
    let mut d = Document::default();
    let f = d.add_ring_face(&[ll(0.0, 0.0), ll(20.0, 0.0), ll(20.0, 20.0), ll(0.0, 20.0)], FaceKind::Land).unwrap();
    let x = d.add_entity(EntityKind::Country, "X");
    let y = d.add_entity(EntityKind::Country, "Y");
    d.assign(f, Some(x)).unwrap();
    d.split_face(&[ll(10.0, 0.0), ll(10.0, 20.0)]).unwrap();
    let east = d.topo.face_at(ll(15.0, 10.0));
    d.assign(east, Some(y)).unwrap();
    (d, x, y)
}

#[test]
fn pushing_the_border_moves_land_to_the_far_side_from_the_year() {
    let (mut d, x, y) = world();
    let before = state(&d, Some(1900));
    d.set_year(Some(1950));
    // a bulge into Y: X gains it
    let p = redraw::plan(&d, &[ll(10.0, 4.0), ll(14.0, 10.0), ll(10.0, 16.0)], 0.01).unwrap();
    assert_eq!((p.gainer, p.loser), (Some(x), Some(y)));
    assert_eq!(p.intent, DrawIntent::Take(x));
    let (n, rep) = redraw::apply(&d, &p).unwrap();
    n.topo.validate().unwrap();
    let moved = rep.taken.iter().find(|(e, _)| *e == Some(y)).map(|(_, a)| a.round() as i64).unwrap();
    assert!(moved > 0);
    assert_eq!(state(&n, Some(1900)), before, "earlier years untouched");
    assert_eq!(state(&n, Some(1949)), before);
    let (land, own) = state(&n, Some(1950));
    assert_eq!(land, before.0, "no land gained or lost");
    assert!((own[&x] - (before.1[&x] + moved)).abs() <= 1);
    assert!((own[&y] - (before.1[&y] - moved)).abs() <= 1);
    // the other way: a bulge into X goes to Y
    let p = redraw::plan(&d, &[ll(10.0, 4.0), ll(6.0, 10.0), ll(10.0, 16.0)], 0.01).unwrap();
    assert_eq!((p.gainer, p.loser), (Some(y), Some(x)));
}

#[test]
fn redrawing_a_coast_adds_land_or_water() {
    let (d, _, y) = world();
    let before = state(&d, None);
    // out to sea: the region becomes Y's land
    let p = redraw::plan(&d, &[ll(20.0, 4.0), ll(24.0, 10.0), ll(20.0, 16.0)], 0.01).unwrap();
    assert_eq!((p.gainer, p.loser, p.intent), (Some(y), None, DrawIntent::Take(y)));
    let (n, rep) = redraw::apply(&d, &p).unwrap();
    n.topo.validate().unwrap();
    assert!(rep.added_km2 > 0.0);
    assert!((state(&n, None).1[&y] - (before.1[&y] + rep.added_km2.round() as i64)).abs() <= 1);
    // inland: the region becomes sea
    let p = redraw::plan(&d, &[ll(20.0, 4.0), ll(17.0, 10.0), ll(20.0, 16.0)], 0.01).unwrap();
    assert_eq!((p.gainer, p.loser, p.intent), (None, Some(y), DrawIntent::CarveWater));
    let (n, rep) = redraw::apply(&d, &p).unwrap();
    n.topo.validate().unwrap();
    assert!(rep.carved_km2 > 0.0);
    assert!(state(&n, None).0 < before.0);
}

#[test]
fn unusable_lines_are_refused() {
    let (d, _, _) = world();
    // an end off any boundary
    assert!(redraw::plan(&d, &[ll(10.0, 4.0), ll(14.0, 10.0)], 0.01).is_err());
    // crossing the old border
    assert!(redraw::plan(&d, &[ll(10.0, 4.0), ll(14.0, 8.0), ll(6.0, 12.0), ll(10.0, 16.0)], 0.01).is_err());
    // from the X/Y border to Y's coast: the far side is partly X, partly sea
    assert!(redraw::plan(&d, &[ll(10.0, 4.0), ll(15.0, 5.0), ll(20.0, 6.0)], 0.01).is_err());
    // back to the same point
    assert!(redraw::plan(&d, &[ll(10.0, 4.0), ll(14.0, 8.0), ll(10.0, 4.0)], 0.01).is_err());
}

/// Dragging a point of the border in a year moves land from that year;
/// the shared geometry (and so every earlier year) is untouched.
#[test]
fn dragging_a_border_point_in_a_year_is_a_dated_change() {
    use map_core::redraw::DragFrom;
    let (mut d, x, y) = world();
    let before = state(&d, Some(1900));
    d.set_year(Some(1950));
    // the X/Y border is one edge from (10,0) to (10,20); grab its middle
    let (edge, seg, _) = d.topo.nearest_on_edge(ll(10.0, 10.0), 0.01).unwrap();
    let line = redraw::drag_line(&d.topo, DragFrom::Segment { edge, seg }, ll(13.0, 10.0)).unwrap();
    assert_eq!(line.len(), 3);
    let p = redraw::plan(&d, &line, 0.01).unwrap();
    assert_eq!((p.gainer, p.loser), (Some(x), Some(y)));
    let (n, _) = redraw::apply(&d, &p).unwrap();
    n.topo.validate().unwrap();
    assert_eq!(state(&n, Some(1949)), before);
    let (land, own) = state(&n, Some(1950));
    assert_eq!(land, before.0);
    assert!(own[&x] > before.1[&x] && own[&y] < before.1[&y]);
    // junction nodes are refused
    let node = *d.topo.nodes.keys().next().unwrap();
    assert!(redraw::drag_line(&d.topo, DragFrom::Vertex(map_core::VertexRef::Node(node)), ll(1.0, 1.0)).is_err());
}

/// After dated border changes, pieces that agree in every year are merged
/// back; pieces that differ in some year stay apart, and no year changes.
#[test]
fn tidying_merges_only_pieces_with_the_same_history() {
    let (mut d, _, _) = world();
    // split X's land twice without changing any owner: pure clutter
    d.split_face(&[ll(0.0, 5.0), ll(10.0, 5.0)]).unwrap();
    d.split_face(&[ll(0.0, 12.0), ll(10.0, 12.0)]).unwrap();
    // a dated change that must survive: Y's bulge goes to X from 1950
    d.set_year(Some(1950));
    let p = redraw::plan(&d, &[ll(10.0, 14.0), ll(14.0, 16.0), ll(10.0, 18.0)], 0.01).unwrap();
    let (mut n, _) = redraw::apply(&d, &p).unwrap();
    n.set_year(None);
    let years = [Some(1900), Some(1949), Some(1950), None];
    let before: Vec<_> = years.iter().map(|y| state(&n, *y)).collect();
    let faces = n.topo.faces.len();
    let merged = n.tidy_faces();
    n.topo.validate().unwrap();
    assert!(merged >= 2, "the two clutter splits go ({merged})");
    assert!(n.topo.faces.len() < faces);
    let after: Vec<_> = years.iter().map(|y| state(&n, *y)).collect();
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(b.0, a.0);
        for (e, v) in &b.1 {
            assert!((a.1[e] - v).abs() <= 1, "areas the same in every year");
        }
    }
    // the bulge differs in 1949 from both neighbours, so it stays its own piece
    let bulge = n.topo.face_at(ll(11.0, 16.0));
    let x_piece = n.topo.face_at(ll(5.0, 16.0));
    assert_ne!(bulge, x_piece);
    // tidying again finds nothing
    assert_eq!(n.tidy_faces(), 0);
}
