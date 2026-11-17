//! Hatched areas: stripes clipped to the shape, and their years.

use map_core::area::{centre, hatch, AreaMark};
use map_core::geom::LonLat;

fn len(s: &[[f32; 2]; 2]) -> f32 {
    ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt()
}

/// Horizontal stripes across a square: one per spacing, each as wide as
/// the square, on multiples of the spacing.
#[test]
fn stripes_fill_a_square() {
    let sq = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let h = hatch(&[sq.clone()], 10.0, 0.0);
    assert_eq!(h.len(), 10);
    for (i, s) in h.iter().enumerate() {
        assert!((len(s) - 100.0).abs() < 1e-3);
        assert!((s[0][1] - 10.0 * i as f32).abs() < 1e-3 && (s[0][1] - s[1][1]).abs() < 1e-3);
    }
    // at any angle the stripes' total length is about area / spacing
    for angle in [30.0, 45.0, -60.0, 90.0] {
        let total: f32 = hatch(&[sq.clone()], 5.0, angle).iter().map(len).sum();
        assert!((total - 10000.0 / 5.0).abs() < 0.03 * 2000.0, "{angle}: {total}");
    }
}

/// A stripe crossing a U is cut into two pieces; a hole is left empty.
#[test]
fn stripes_skip_gaps_and_holes() {
    let u = vec![[0.0, 0.0], [30.0, 0.0], [30.0, 70.0], [70.0, 70.0], [70.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let h = hatch(&[u], 10.0, 0.0);
    let at = |y: f32| h.iter().filter(|s| (s[0][1] - y).abs() < 1e-3).count();
    assert_eq!((at(20.0), at(80.0)), (2, 1));
    let outer = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let hole = vec![[40.0, 40.0], [60.0, 40.0], [60.0, 60.0], [40.0, 60.0]];
    let h = hatch(&[outer, hole], 10.0, 0.0);
    let total: f32 = h.iter().map(len).sum();
    assert!((total - (10000.0 - 400.0) / 10.0).abs() < 1e-2, "{total}");
    assert!(h.iter().all(|s| !(s[0][1] > 40.0 && s[0][1] < 60.0 && s[0][0] < 50.0 && s[1][0] > 50.0)));
}

/// The gap left for a name: nothing inside it, the rest kept.
#[test]
fn stripes_leave_room_for_the_name() {
    use map_core::area::cut_out;
    let sq = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let h = hatch(&[sq], 10.0, 45.0);
    let before: f32 = h.iter().map(len).sum();
    let gap = [30.0, 40.0, 70.0, 60.0];
    let cut = cut_out(h, gap);
    let after: f32 = cut.iter().map(len).sum();
    for s in &cut {
        let m = [(s[0][0] + s[1][0]) * 0.5, (s[0][1] + s[1][1]) * 0.5];
        assert!(!(m[0] > gap[0] + 1e-3 && m[0] < gap[2] - 1e-3 && m[1] > gap[1] + 1e-3 && m[1] < gap[3] - 1e-3), "{s:?}");
    }
    // the gap's area (800) is taken out of the stripes' area (spacing 10)
    assert!((before - after - 800.0 / 10.0).abs() < 8.0, "{before} {after}");
}

#[test]
fn centre_and_years() {
    let c = centre(&[[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]]).unwrap();
    assert!((c[0] - 5.0).abs() < 1e-4 && (c[1] - 10.0).abs() < 1e-4);
    let mut a = AreaMark::new(vec![LonLat::new(0.0, 0.0); 3]);
    assert!(a.shown_in(None) && a.shown_in(Some(1)));
    a.from = Some(1914);
    a.to = Some(1918);
    assert!(a.shown_in(None) && a.shown_in(Some(1916)) && !a.shown_in(Some(1913)) && !a.shown_in(Some(1919)));
    // saved before the optional fields existed
    let back: AreaMark = serde_json::from_str(r#"{"ring":[],"color":[1,2,3,255],"spacing":8.0,"angle":45.0}"#).unwrap();
    assert!(!back.tint && !back.dashed && back.from.is_none() && back.name.is_empty());
}

/// Dots lie inside the shape, about one per spacing², staggered row to
/// row; cross-hatching is the stripes both ways; the name's room stays
/// empty in every pattern.
#[test]
fn patterns_fill_the_shape() {
    use map_core::area::{dots, fill_pattern, Pattern};
    let sq = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let d = dots(&[sq.clone()], 10.0, 0.0);
    assert!(d.iter().all(|p| p[0] >= 0.0 && p[0] <= 100.0 && p[1] >= 0.0 && p[1] <= 100.0));
    assert!((95..=105).contains(&d.len()), "{}", d.len());
    // rows at y = 0 and y = 10: the second shifted by half a step
    let row = |y: f32| {
        let mut xs: Vec<f32> = d.iter().filter(|p| (p[1] - y).abs() < 1e-3).map(|p| p[0]).collect();
        xs.sort_by(f32::total_cmp);
        xs
    };
    assert!((row(0.0)[0] - 0.0).abs() < 1e-3 && (row(10.0)[0] - 5.0).abs() < 1e-3);
    let (cross, none) = fill_pattern(&[sq.clone()], Pattern::Cross, 10.0, 0.0, None);
    assert!(none.is_empty());
    assert_eq!(cross.len(), hatch(&[sq.clone()], 10.0, 0.0).len() + hatch(&[sq.clone()], 10.0, 90.0).len());
    let gap = [30.0, 30.0, 70.0, 70.0];
    for p in Pattern::ALL {
        let (segs, pts) = fill_pattern(&[sq.clone()], p, 10.0, 30.0, Some(gap));
        assert!(pts.iter().all(|q| !(q[0] > 30.0 && q[0] < 70.0 && q[1] > 30.0 && q[1] < 70.0)), "{p:?}");
        for s in &segs {
            let m = [(s[0][0] + s[1][0]) * 0.5, (s[0][1] + s[1][1]) * 0.5];
            assert!(!(m[0] > 30.001 && m[0] < 69.999 && m[1] > 30.001 && m[1] < 69.999), "{p:?}");
        }
    }
}

/// Rings filled even-odd sort into outer rings with their holes; two
/// separate pieces are two outers.
#[test]
fn rings_group_into_pieces_and_holes() {
    use map_core::area::group_rings;
    let sq = |x: f32, y: f32, s: f32| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
    let rings = vec![sq(0.0, 0.0, 100.0), sq(200.0, 0.0, 50.0), sq(40.0, 40.0, 20.0), sq(210.0, 10.0, 10.0)];
    let mut g = group_rings(&rings);
    g.sort();
    assert_eq!(g, vec![(0, vec![2]), (1, vec![3])]);
}

/// A country's outline as a hatched area covers exactly the country's
/// area in the year asked for: after it gains another country's land from
/// a year, the outline for that year is larger by that land, the earlier
/// one is unchanged.
#[test]
fn country_outline_in_a_year() {
    use map_core::geom;
    let mut d = map_core::sample::demo_world();
    let r = d.radius();
    let area_of = |d: &map_core::Document, id| {
        let a = &d.areas[&id];
        a.rings().iter().map(|ring| geom::spherical_area(&geom::unwrap_path(ring, None), r)).collect::<Vec<f64>>()
    };
    let countries = d.countries();
    let (e, other) = (countries[0], countries[1]);
    let whole = d.areas_by_entity()[&e];
    let id = d.area_from_entity(e, None).unwrap();
    // outers count one way, holes the other (opposite winding)
    let net = |v: Vec<f64>| v.iter().sum::<f64>().abs();
    assert!((net(area_of(&d, id)) / whole - 1.0).abs() < 1e-3, "{} vs {whole}", net(area_of(&d, id)));
    assert_eq!(d.areas[&id].name, d.entities[&e].name);
    // from 1950 it also holds the other country's largest face
    let f = *d.faces_of(other).iter().max_by(|a, b| d.topo.face_area(**a, r).total_cmp(&d.topo.face_area(**b, r))).unwrap();
    let gained = d.topo.face_area(f, r);
    d.set_year(Some(1950));
    d.record_owner(f, Some(e)).unwrap();
    let after = d.area_from_entity(e, Some(1950)).unwrap();
    let before = d.area_from_entity(e, Some(1900)).unwrap();
    assert!((net(area_of(&d, before)) / whole - 1.0).abs() < 1e-3, "1900 unchanged");
    assert!(((net(area_of(&d, after)) - whole) / gained - 1.0).abs() < 1e-3, "1950 larger by the gained land");
    assert!(d.areas[&after].name.contains("1950"));
    // a country with no land in a year gives an error, not an empty area
    let n = d.areas.len();
    let city = d.add_city(map_core::geom::LonLat::new(0.0, 0.0), "城");
    assert!(d.area_from_entity(city, Some(1950)).is_err());
    assert_eq!(d.areas.len(), n);
}

/// Union, difference and intersection of two overlapping squares: the
/// right areas, the target keeps its look, the other is used up; cutting
/// a square out of the middle leaves a hole; empty results are refused.
#[test]
fn areas_combine() {
    use map_core::area::AreaOp;
    use map_core::doc::{Document, LineId};
    use map_core::geom;
    let sq = |x: f64, y: f64, s: f64| vec![LonLat::new(x, y), LonLat::new(x + s, y), LonLat::new(x + s, y + s), LonLat::new(x, y + s)];
    // planar area in square degrees (holes subtract)
    let area = |d: &Document, id| d.areas[&id].rings().iter().map(|r| geom::signed_area(&geom::unwrap_path(r, None))).sum::<f64>().abs();
    let setup = |b: Vec<LonLat>| {
        let mut d = Document::default();
        let (ia, ib) = (LineId(d.topo.alloc()), LineId(d.topo.alloc()));
        let mut a = AreaMark::new(sq(0.0, 0.0, 10.0));
        a.name = "甲".into();
        a.color = [1, 2, 3, 255];
        d.areas.insert(ia, a);
        d.areas.insert(ib, AreaMark::new(b));
        (d, ia, ib)
    };
    for (op, want) in [(AreaOp::Union, 175.0), (AreaOp::Difference, 75.0), (AreaOp::Intersection, 25.0)] {
        let (mut d, ia, ib) = setup(sq(5.0, 5.0, 10.0));
        d.combine_areas(ia, ib, op).unwrap();
        assert!((area(&d, ia) - want).abs() < 1e-6, "{op:?}: {}", area(&d, ia));
        assert!(!d.areas.contains_key(&ib));
        assert_eq!((d.areas[&ia].name.as_str(), d.areas[&ia].color), ("甲", [1, 2, 3, 255]));
    }
    // a hole in the middle
    let (mut d, ia, ib) = setup(sq(3.0, 3.0, 4.0));
    d.combine_areas(ia, ib, AreaOp::Difference).unwrap();
    assert_eq!(d.areas[&ia].extra.len(), 1);
    assert!((area(&d, ia) - 84.0).abs() < 1e-6);
    // two separate squares merged stay two pieces
    let (mut d, ia, ib) = setup(sq(20.0, 0.0, 5.0));
    d.combine_areas(ia, ib, AreaOp::Union).unwrap();
    assert_eq!(d.areas[&ia].rings().len(), 2);
    // nothing in common, or nothing left: refused, nothing changed
    let (mut d, ia, ib) = setup(sq(20.0, 0.0, 5.0));
    assert!(d.combine_areas(ia, ib, AreaOp::Intersection).is_err());
    assert!(d.areas.contains_key(&ib) && d.areas[&ia].ring.len() == 4);
    let (mut d, ia, ib) = setup(sq(-5.0, -5.0, 30.0));
    assert!(d.combine_areas(ia, ib, AreaOp::Difference).is_err());
    assert!(d.combine_areas(ia, ia, AreaOp::Union).is_err());
    // across the antimeridian: two squares either side join into one
    let mut d = Document::default();
    let (ia, ib) = (LineId(d.topo.alloc()), LineId(d.topo.alloc()));
    d.areas.insert(ia, AreaMark::new(sq(170.0, 0.0, 10.0)));
    d.areas.insert(ib, AreaMark::new(vec![LonLat::new(-180.0, 0.0), LonLat::new(-170.0, 0.0), LonLat::new(-170.0, 10.0), LonLat::new(-180.0, 10.0)]));
    d.combine_areas(ia, ib, AreaOp::Union).unwrap();
    assert_eq!(d.areas[&ia].rings().len(), 1);
    assert!((area(&d, ia) - 200.0).abs() < 1e-6);
}

/// Mountains and forests: upright little pictures on the dots' grid, drawn
/// as strokes (so every renderer and the legend draw them as they draw
/// stripes): a peak of two strokes, a tree of a crown and a trunk.
#[test]
fn mountains_and_trees() {
    use map_core::area::{fill_pattern, Pattern};
    let sq = vec![[0.0f32, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    let (_, dots) = fill_pattern(std::slice::from_ref(&sq), Pattern::Dots, 12.0, 20.0, None);
    let (peaks, none) = fill_pattern(std::slice::from_ref(&sq), Pattern::Mountains, 12.0, 20.0, None);
    let (trees, _) = fill_pattern(std::slice::from_ref(&sq), Pattern::Trees, 12.0, 20.0, None);
    assert!(none.is_empty() && !dots.is_empty());
    assert_eq!(peaks.len(), dots.len() * 2);
    assert_eq!(trees.len(), dots.len() * 4);
    // upright whatever the grid's angle: each peak's top above its feet (y runs down)
    for pair in peaks.chunks(2) {
        let (foot, top) = (pair[0][0], pair[0][1]);
        assert!(top[1] < foot[1] && (pair[1][0][0] - top[0]).abs() < 1e-4);
    }
    assert_eq!(Pattern::Mountains.name(), "山地");
}
