//! Drawing rivers and routes onto existing lines and cities.

use map_core::doc::{Document, LineFeature, LineId};
use map_core::geom::LonLat;
use map_core::lines::LineSnap;

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

fn line(d: &mut Document, pts: Vec<LonLat>, route: bool) -> LineId {
    let id = LineId(d.topo.alloc());
    d.rivers.insert(id, LineFeature { name: "l".into(), pts, route: route.then(Default::default), legend: String::new(), show_name: false, hidden: false });
    id
}

/// Ends win over cities, cities over points on a line; rivers and routes
/// only snap to their own kind; nothing beyond the tolerance.
#[test]
fn snapping_prefers_ends_then_cities_then_lines() {
    let mut d = Document::default();
    let river = line(&mut d, vec![ll(0.0, 0.0), ll(10.0, 0.0)], false);
    let road = line(&mut d, vec![ll(0.0, 5.0), ll(10.0, 5.0)], true);
    d.add_city(ll(5.0, 5.2), "城");
    // a river point near the river's end, and near its middle
    assert_eq!(d.line_snap(ll(10.3, 0.2), 0.5, false), Some((ll(10.0, 0.0), LineSnap::End(river, true))));
    let (q, s) = d.line_snap(ll(5.0, 0.3), 0.5, false).unwrap();
    assert_eq!(s, LineSnap::On(river));
    assert!((q.lat - 0.0).abs() < 1e-3 && (q.lon - 5.0).abs() < 1e-3);
    // a river does not snap to the road, a road not to the river
    assert!(d.line_snap(ll(5.0, 5.1), 0.5, false).is_none());
    assert!(d.line_snap(ll(5.0, 0.1), 0.5, true).is_none());
    // a road snaps to the city before the road under it
    assert_eq!(d.line_snap(ll(5.0, 5.1), 0.5, true), Some((ll(5.0, 5.2), LineSnap::City)));
    assert_eq!(d.line_snap(ll(0.1, 5.1), 0.5, true), Some((ll(0.0, 5.0), LineSnap::End(road, false))));
    assert!(d.line_snap(ll(5.0, 2.5), 0.5, true).is_none());
}

/// A line continues from either end; the end is found only exactly.
#[test]
fn lines_continue_from_either_end() {
    let mut d = Document::default();
    let id = line(&mut d, vec![ll(0.0, 0.0), ll(10.0, 0.0)], true);
    assert_eq!(d.line_end_exact(ll(10.0, 0.0), true), Some((id, true)));
    assert_eq!(d.line_end_exact(ll(0.0, 0.0), true), Some((id, false)));
    assert_eq!(d.line_end_exact(ll(0.0, 0.0), false), None, "a river does not continue a route");
    assert_eq!(d.line_end_exact(ll(0.01, 0.0), true), None);
    d.extend_line(id, true, &[ll(10.0, 0.0), ll(12.0, 1.0), ll(14.0, 1.0)]).unwrap();
    assert_eq!(d.rivers[&id].pts, vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(12.0, 1.0), ll(14.0, 1.0)]);
    d.extend_line(id, false, &[ll(0.0, 0.0), ll(-3.0, 2.0)]).unwrap();
    assert_eq!(d.rivers[&id].pts, vec![ll(-3.0, 2.0), ll(0.0, 0.0), ll(10.0, 0.0), ll(12.0, 1.0), ll(14.0, 1.0)]);
    assert!(d.extend_line(id, true, &[ll(14.0, 1.0)]).is_err());
}
