//! Map text following an arc.

use map_core::doc::{arc_layout, Annotation};

/// Without a bend the glyphs sit side by side on a straight line,
/// centred on the text's middle.
#[test]
fn straight_text_stays_on_a_line() {
    let g = arc_layout(&[10.0, 20.0, 10.0], 0.0);
    assert_eq!(g, vec![([-15.0, 0.0], 0.0), ([0.0, 0.0], 0.0), ([15.0, 0.0], 0.0)]);
}

/// A bend makes an arc of that angle that keeps the text's length, is
/// symmetric, and is centred on its middle height; a negative bend is
/// the same arc upside down.
#[test]
fn bent_text_follows_an_arc() {
    let w = [10.0f32; 7];
    let up = arc_layout(&w, 90.0);
    // glyph centres one advance apart along the arc
    let r = 70.0 / 90f32.to_radians();
    for p in up.windows(2) {
        let chord = ((p[1].0[0] - p[0].0[0]).powi(2) + (p[1].0[1] - p[0].0[1]).powi(2)).sqrt();
        assert!((chord - 2.0 * r * (5.0 / r).sin()).abs() < 1e-3, "{chord}");
    }
    // symmetric, arching up, the outer glyphs turned outwards
    assert!((up[0].0[0] + up[6].0[0]).abs() < 1e-4 && (up[0].0[1] - up[6].0[1]).abs() < 1e-4);
    assert!(up[3].0[1] > up[0].0[1]);
    assert!(up[0].1 > 0.0 && (up[0].1 + up[6].1).abs() < 1e-4 && up[3].1.abs() < 1e-4);
    // the end glyphs turn by half the arc less half a glyph's share
    assert!((up[6].1 + 90.0 * 30.0 / 70.0).abs() < 1e-3, "{}", up[6].1);
    // centred on its middle height: the arc's top as far above as its ends below
    let sag = r * (1.0 - 45f32.to_radians().cos());
    assert!((up[3].0[1] - sag * 0.5).abs() < 1e-3, "{}", up[3].0[1]);
    let down = arc_layout(&w, -90.0);
    for (a, b) in up.iter().zip(&down) {
        assert!((a.0[0] - b.0[0]).abs() < 1e-4 && (a.0[1] + b.0[1]).abs() < 1e-4 && (a.1 + b.1).abs() < 1e-4);
    }
}

/// Text saved before bending existed loads straight.
#[test]
fn old_text_loads_straight() {
    let a = Annotation::new("海", map_core::geom::LonLat::new(0.0, 0.0));
    let mut v = serde_json::to_value(&a).unwrap();
    v.as_object_mut().unwrap().remove("bend");
    let back: Annotation = serde_json::from_value(v).unwrap();
    assert_eq!(back.bend, 0.0);
    assert!(!back.curved());
}

/// Text laid along a curve is grabbed at the curve's middle; moving it
/// turns the curve with it (true shape kept); a copy takes its own curve;
/// clearing the curve brings the straight text back where it was.
#[test]
fn text_on_a_curve_moves_with_it() {
    use map_core::doc::{Document, LineId};
    let ll = map_core::geom::LonLat::new;
    let mut a = Annotation::new("山", ll(0.0, 0.0));
    a.set_path(vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(20.0, 0.0)]);
    assert!(a.on_path());
    assert!((a.at.lon - 10.0).abs() < 0.05 && a.at.lat.abs() < 1e-6, "{:?}", a.at);
    let d0 = map_core::measure::great_circle(a.path[0], a.path[2], 1.0);
    a.move_to(ll(10.0, 30.0));
    assert!((map_core::measure::great_circle(a.path[0], a.path[2], 1.0) - d0).abs() < 1e-9);
    assert!(map_core::measure::great_circle(a.path[1], ll(10.0, 30.0), 1.0) < 1e-6, "the middle follows the grab point");
    let mut d = Document::default();
    let id = LineId(d.topo.alloc());
    d.annotations.insert(id, a.clone());
    let c = d.duplicate_annotation(id, 2.0, -2.0).unwrap();
    assert!(d.annotations[&c].on_path() && d.annotations[&c].path != a.path);
    assert_eq!(d.annotations[&id].path, a.path, "the original keeps its curve");
    a.set_path(vec![ll(5.0, 5.0)]);
    assert!(!a.on_path() && a.path.is_empty());
    assert_eq!(a.at, ll(10.0, 30.0));
}
