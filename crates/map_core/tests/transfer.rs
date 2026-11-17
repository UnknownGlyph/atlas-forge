//! Land handed from one country to another inside an area, from a year:
//! only the giver's land moves (not the sea, a lake or a third country's
//! land inside the area); earlier years stay; one call, one change.

use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::grid_world;
use map_core::{EntityId, FaceKind, BACKGROUND};

fn areas(d: &map_core::Document, y: Option<i32>) -> std::collections::HashMap<EntityId, f64> {
    let mut v = d.clone();
    v.set_year(y);
    v.areas_by_entity()
}

#[test]
fn the_east_of_a_country_to_its_neighbour_from_a_year() {
    let mut d = grid_world(6, 4, 2, 5);
    let cs = d.countries();
    let (a, b, c) = (cs[0], cs[1], cs[2]);
    // a lake in A, and a third country's piece of land: neither may change
    let pieces = d.dissolve(a);
    let bb = map_core::geom::bbox(&pieces[0].outer);
    let mid = (bb[0] + bb[2]) * 0.5;
    let (cx, cy) = (mid + (bb[2] - mid) * 0.5, (bb[1] + bb[3]) * 0.5);
    let sq = |x: f64, y: f64, r: f64| vec![LonLat::new(x - r, y - r), LonLat::new(x + r, y - r), LonLat::new(x + r, y + r), LonLat::new(x - r, y + r)];
    d = overlay::apply(&d, &sq(cx, cy, 0.4), DrawIntent::CarveWater).unwrap().0;
    d = overlay::apply(&d, &sq(cx, cy + 1.2, 0.4), DrawIntent::Take(c)).unwrap().0;
    let lakes = |d: &map_core::Document| d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Water).map(|(f, _)| d.topo.face_area(*f, d.radius())).sum::<f64>();
    let (lake0, before) = (lakes(&d), areas(&d, Some(1899)));
    // the east of A (and beyond, over the sea) to B, from 1900
    d.set_year(Some(1900));
    let east = vec![LonLat::new(mid, bb[1] - 5.0), LonLat::new(bb[2] + 10.0, bb[1] - 5.0), LonLat::new(bb[2] + 10.0, bb[3] + 5.0), LonLat::new(mid, bb[3] + 5.0)];
    let moved = d.transfer_land(a, b, &east).unwrap();
    d.topo.validate().unwrap();
    let after = areas(&d, Some(1900));
    let lost = before[&a] - after[&a];
    assert!(moved > 0.0 && (lost - moved).abs() < 1e-6 * moved, "{moved} km² moved, A lost {lost}");
    assert!((after[&b] - before[&b] - moved).abs() < 1e-6 * moved);
    assert!((after[&c] - before[&c]).abs() < 1e-6 * before[&c].max(1.0), "the third country keeps its piece");
    assert!((lakes(&d) - lake0).abs() < 1e-6 * lake0, "the lake stays water");
    // earlier years stay (to the rounding of rebuilt faces)
    let earlier = areas(&d, Some(1899));
    assert_eq!(earlier.len(), before.len());
    assert!(before.iter().all(|(e, x)| (earlier[e] - x).abs() <= 1e-6 * x.max(1.0)), "an earlier year changed");
    // an area with none of A's land is refused
    assert!(d.transfer_land(a, b, &sq(bb[2] + 30.0, cy, 1.0)).is_err());
}
