//! Curves on rivers, routes and hatched areas: their points follow the
//! curve, edits keep the shape where they should, and moving, copying,
//! undoing and saving carry the curve.

use map_core::curve::{Curve, CurveNode, Handle};
use map_core::doc::{Document, LineFeature, LineId};
use map_core::features::FeatureRef;
use map_core::geom::{lon_delta, LonLat};
use map_core::history::History;

fn p(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

fn close(a: LonLat, b: LonLat, tol: f64) -> bool {
    lon_delta(a.lon, b.lon).abs() < tol && (a.lat - b.lat).abs() < tol
}

/// Largest distance (degrees) of the polyline's vertices from the curve's
/// own points sampled densely: how closely the flattening follows it.
fn deviation(c: &Curve, pts: &[LonLat]) -> f64 {
    // (flattening halves segments, at most ten times: every point it makes
    // is at a multiple of 1/1024 of its segment)
    let mut samples = Vec::new();
    for i in 0..c.segments() {
        for k in 0..=1024 {
            samples.push(c.point(i, k as f64 / 1024.0));
        }
    }
    pts.iter().map(|q| samples.iter().map(|s| (lon_delta(s.lon, q.lon).powi(2) + (s.lat - q.lat).powi(2)).sqrt()).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
}

#[test]
fn a_curve_of_corners_is_the_polyline() {
    let pts = vec![p(0.0, 0.0), p(10.0, 5.0), p(20.0, 0.0)];
    let c = Curve::corners(&pts, false);
    assert_eq!(c.flatten(), pts, "straight segments add no points");
    let ring = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    assert_eq!(Curve::corners(&ring, true).flatten(), ring, "a closed ring is not closed twice");
}

#[test]
fn curve_and_group_snapshots_share_storage_and_preserve_ordered_wire_maps() {
    use map_core::{change::ChangeSet, stack::GroupId};
    use std::collections::BTreeMap;

    let mut doc = Document::default();
    let id = LineId(47);
    let curve = Curve::corners(&[p(1.0, 2.0), p(3.0, 4.0)], false);
    doc.curves.insert(id, curve.clone());
    doc.group_of.insert(id, GroupId(11));
    let mut snapshot = doc.clone();
    assert!(snapshot.curves.ptr_eq(&doc.curves) && snapshot.group_of.ptr_eq(&doc.group_of));
    let wire = serde_json::to_value(&doc).unwrap();
    assert_eq!(wire["curves"], serde_json::to_value(BTreeMap::from([(id, &curve)])).unwrap());
    assert_eq!(wire["group_of"], serde_json::to_value(BTreeMap::from([(id, GroupId(11))])).unwrap());
    snapshot.curves.remove(&id);
    snapshot.group_of.remove(&id);
    assert!(ChangeSet::between(&doc, &snapshot).old_bytes >= curve.nodes.len() * std::mem::size_of::<CurveNode>());
    assert!(doc.curves.contains_key(&id) && doc.group_of.contains_key(&id));
}

#[test]
fn flattening_follows_the_curve_closely() {
    let c = Curve::smooth_through(&[p(0.0, 0.0), p(10.0, 8.0), p(20.0, -3.0), p(30.0, 6.0)], false);
    let pts = c.flatten();
    assert!(pts.len() > 20 && pts.len() < 2000, "{}", pts.len());
    // through every anchor, in order, ends exact
    assert_eq!((pts[0], *pts.last().unwrap()), (p(0.0, 0.0), p(30.0, 6.0)));
    for a in &c.nodes {
        assert!(pts.iter().any(|q| close(*q, a.at, 1e-9)));
    }
    assert!(deviation(&c, &pts) < 1e-9, "every point is on the curve");
    // and the straight pieces between them stay within 0.02°: sample the
    // curve and find the nearest flattened segment
    for i in 0..c.segments() {
        for k in 0..100 {
            let s = c.point(i, k as f64 / 100.0);
            let d = pts.windows(2).map(|w| map_core::geom::closest_on_segment(s, w[0], w[1]).1).fold(f64::INFINITY, f64::min);
            assert!(d < 0.02, "{d}");
        }
    }
    // smooth anchors: the handles on each side are in one line
    let n = c.nodes[1];
    let (a, i, o) = (n.at, n.h_in.unwrap(), n.h_out.unwrap());
    let cross = (i.lon - a.lon) * (o.lat - a.lat) - (i.lat - a.lat) * (o.lon - a.lon);
    assert!(n.smooth && cross.abs() < 1e-9);
}

#[test]
fn across_the_antimeridian_and_round_a_ring() {
    // a curve from 170°E to 170°W goes the short way, over 180°
    let c = Curve::smooth_through(&[p(170.0, 0.0), p(178.0, 4.0), p(-170.0, 0.0)], false);
    let pts = c.flatten();
    assert!(pts.iter().all(|q| q.lon >= 169.999 || q.lon <= -169.999), "{pts:?}");
    assert!(pts.windows(2).all(|w| lon_delta(w[0].lon, w[1].lon).abs() < 5.0));
    // a closed smooth curve: no point repeated, the last segment back to the first anchor
    let ring = Curve::smooth_through(&[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)], true);
    let r = ring.flatten();
    assert!(r.len() > 8 && r.first() != r.last());
    assert!(deviation(&ring, &r) < 1e-9);
}

#[test]
fn editing_anchors_and_handles() {
    let mut c = Curve::smooth_through(&[p(0.0, 0.0), p(10.0, 5.0), p(20.0, 0.0)], false);
    let before = c.nodes[1];
    // moving an anchor carries its handles
    c.move_node(1, p(12.0, 7.0));
    let n = c.nodes[1];
    assert!(close(n.at, p(12.0, 7.0), 1e-12));
    assert!(close(n.h_in.unwrap(), p(before.h_in.unwrap().lon + 2.0, before.h_in.unwrap().lat + 2.0), 1e-9));
    // a smooth anchor's other handle turns to stay opposite, keeping its length
    let len_in = |n: &CurveNode| ((n.h_in.unwrap().lon - n.at.lon).powi(2) + (n.h_in.unwrap().lat - n.at.lat).powi(2)).sqrt();
    let l = len_in(&c.nodes[1]);
    c.move_handle(1, Handle::Out, p(12.0, 12.0));
    let n = c.nodes[1];
    assert!((len_in(&n) - l).abs() < 1e-9 && close(n.h_in.unwrap(), p(12.0, 7.0 - l), 1e-9));
    // a corner's handles are independent
    c.nodes[1].smooth = false;
    c.move_handle(1, Handle::Out, p(15.0, 7.0));
    assert!(close(c.nodes[1].h_in.unwrap(), p(12.0, 7.0 - l), 1e-9));
    // pulling handles as the pen does: symmetric, smooth
    c.pull_handles(0, p(2.0, 3.0));
    assert_eq!((c.nodes[0].h_in, c.nodes[0].h_out, c.nodes[0].smooth), (Some(p(-2.0, -3.0)), Some(p(2.0, 3.0)), true));
    c.make_corner(0);
    assert!(c.nodes[0].h_in.is_none() && c.nodes[0].h_out.is_none());
    // inserting an anchor keeps the shape
    let shape = c.flatten();
    let i = c.insert_node(1, 0.4);
    assert_eq!((i, c.nodes.len()), (2, 4));
    assert!(deviation(&c, &shape) < 0.02, "the old points are still on the curve");
    // removing anchors, down to two
    c.remove_node(2).unwrap();
    c.remove_node(0).unwrap();
    assert!(c.remove_node(0).is_err());
    assert!(Curve::corners(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)], true).remove_node(0).is_err(), "an area keeps three");
}

fn river(d: &mut Document, pts: Vec<LonLat>) -> LineId {
    let id = LineId(d.topo.alloc());
    d.rivers.insert(id, LineFeature { name: "河".into(), pts, route: None, legend: String::new(), show_name: false, hidden: false });
    id
}

#[test]
fn features_drawn_with_curves() {
    let mut d = Document::default();
    // a new line and area from curves: their points are the flattened curve
    let c = Curve::smooth_through(&[p(0.0, 0.0), p(10.0, 8.0), p(20.0, 0.0)], false);
    let l = d.add_curve_line(c.clone(), None).unwrap();
    assert_eq!(d.rivers[&l].pts, c.flatten());
    let ac = Curve::smooth_through(&[p(30.0, 0.0), p(40.0, 0.0), p(40.0, 10.0), p(30.0, 10.0)], true);
    let a = d.add_curve_area(ac.clone()).unwrap();
    assert_eq!(d.areas[&a].ring, ac.flatten());
    assert!(d.add_curve_line(ac.clone(), None).is_err() && d.add_curve_area(c.clone()).is_err());
    assert!(d.set_curve(l, ac).is_err(), "a line's curve is open");
    // an edit of the curve moves the points
    d.edit_curve(l, |c| {
        c.move_node(1, p(10.0, 20.0));
        Ok(())
    })
    .unwrap();
    assert!(d.rivers[&l].pts.iter().any(|q| q.lat > 15.0));
    // a point dragged directly: an ordinary line again, its shape kept
    let shape = d.rivers[&l].pts.clone();
    d.delete_river_point(l, 3).unwrap();
    assert!(d.curve_of(l).is_none() && d.rivers[&l].pts.len() == shape.len() - 1);
    // an ordinary line made a curve: smooth through its points, or as it is
    let r = river(&mut d, vec![p(0.0, 30.0), p(5.0, 35.0), p(10.0, 30.0)]);
    d.curve_from_points(r, false).unwrap();
    assert_eq!(d.rivers[&r].pts, vec![p(0.0, 30.0), p(5.0, 35.0), p(10.0, 30.0)]);
    d.curve_from_points(r, true).unwrap();
    assert!(d.rivers[&r].pts.len() > 3);
    // back to straight lines: the anchors, or the shape as drawn
    let drawn = d.rivers[&r].pts.clone();
    let mut keep = d.clone();
    keep.drop_curve(r, true).unwrap();
    assert_eq!(keep.rivers[&r].pts, drawn);
    d.drop_curve(r, false).unwrap();
    assert_eq!(d.rivers[&r].pts, vec![p(0.0, 30.0), p(5.0, 35.0), p(10.0, 30.0)]);
    assert!(d.drop_curve(r, false).is_err());
}

#[test]
fn moving_copying_undoing_and_saving_carry_the_curve() {
    let mut d = Document::default();
    let c = Curve::smooth_through(&[p(0.0, 0.0), p(10.0, 8.0), p(20.0, 0.0), p(25.0, 5.0)], false);
    let l = d.add_curve_line(c, None).unwrap();
    let f = FeatureRef::River(l);
    // moved and turned: the curve with it, the points on the moved curve
    d.move_features(&[f], p(10.0, 4.0), p(60.0, 30.0)).unwrap();
    d.rotate_features(&[f], p(60.0, 30.0), 30.0).unwrap();
    let c = d.curve_of(l).unwrap().clone();
    assert!(c.nodes.iter().all(|n| n.at.lon > 40.0) && d.rivers[&l].pts == c.flatten());
    // copied and pasted: the copy has its own curve
    let clip = d.copy_features(&[f]);
    assert_eq!(clip.curves.len(), 1);
    let new = d.paste_features(&clip, Some(p(-50.0, -20.0))).unwrap();
    let nc = d.curve_of(new[0].id()).unwrap();
    assert!(nc.nodes.iter().all(|n| n.at.lon < -30.0) && d.rivers[&new[0].id()].pts == nc.flatten());
    assert_eq!(d.curve_of(l).unwrap(), &c, "the original is untouched");
    let dup = d.duplicate_features(&[f], 1.0, 1.0).unwrap();
    assert!(d.curve_of(dup[0].id()).is_some());
    let line_copy = d.duplicate_line(l, 2.0, 2.0).unwrap();
    assert_eq!(d.rivers[&line_copy].pts, d.curve_of(line_copy).unwrap().flatten());
    // put back from a clipboard (as a drag does each frame), curve and all
    let mut e = d.clone();
    e.edit_curve(l, |c| {
        c.move_node(0, p(0.0, 0.0));
        Ok(())
    })
    .unwrap();
    e.set_feature_from(f, &clip, 0).unwrap();
    assert_eq!((e.curve_of(l), &e.rivers[&l]), (d.curve_of(l), &d.rivers[&l]));
    // deleted: the curve goes too
    assert_eq!(d.delete_features(&dup), 1);
    assert!(d.curve_of(dup[0].id()).is_none());
    // undone: the curve as it was
    let mut h = History::default();
    h.apply(&mut d, "edit", |d| {
        d.edit_curve(l, |c| {
            c.make_corner(1);
            Ok(())
        })
    })
    .unwrap();
    assert!(d.curve_of(l).unwrap().nodes[1].h_in.is_none());
    h.undo(&mut d);
    assert_eq!(d.curve_of(l).unwrap(), &c);
    // saved and read back
    let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    // (JSON may change a float's last digit; the project file is exact)
    assert_eq!(back.curves.len(), d.curves.len());
    for (id, c) in &d.curves {
        let b = &back.curves[id];
        assert_eq!((b.closed, b.nodes.len()), (c.closed, c.nodes.len()));
        for (x, y) in b.nodes.iter().zip(&c.nodes) {
            assert!(close(x.at, y.at, 1e-9) && x.smooth == y.smooth && x.h_in.is_some() == y.h_in.is_some() && x.h_out.is_some() == y.h_out.is_some());
        }
    }
    // an older document (no curves at all) reads as one without curves
    let mut v = serde_json::to_value(&d).unwrap();
    v.as_object_mut().unwrap().remove("curves");
    assert!(serde_json::from_value::<Document>(v).unwrap().curves.is_empty());
}
