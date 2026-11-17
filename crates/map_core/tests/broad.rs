//! Broad campaign arrows.

use map_core::broad::broad_arrow;

/// On a straight path the band is symmetric, narrower at its start than at
/// the head's base, and the head (wider than the band) ends at the tip.
#[test]
fn a_straight_arrow() {
    let o = broad_arrow(&[[0.0, 100.0], [200.0, 100.0]], 20.0, true).unwrap();
    // two band points each side, the head's three points
    assert_eq!(o.len(), 7);
    let (start_l, start_r) = (o[0], o[6]);
    assert!((start_l[0] - 0.0).abs() < 1e-3 && (start_r[0] - 0.0).abs() < 1e-3);
    assert!(((start_l[1] - start_r[1]).abs() - 11.0).abs() < 1e-3, "starts at 55%");
    let (base_l, base_r) = (o[1], o[5]);
    assert!(((base_l[1] - base_r[1]).abs() - 20.0).abs() < 1e-3, "full width at the head");
    // the head: base at 200 - (20*1.4+6) = 166, half width 24, tip at the end
    assert!((o[2][0] - 166.0).abs() < 1e-3 && ((o[2][1] - o[4][1]).abs() - 48.0).abs() < 1e-3);
    assert_eq!(o[3], [200.0, 100.0]);
    // symmetric about the path
    for i in 0..3 {
        assert!(((o[i][1] - 100.0) + (o[6 - i][1] - 100.0)).abs() < 1e-3);
    }
    // without a head the band runs to the end, flat
    let flat = broad_arrow(&[[0.0, 100.0], [200.0, 100.0]], 20.0, false).unwrap();
    assert_eq!(flat.len(), 4);
    assert!((flat[1][0] - 200.0).abs() < 1e-3 && ((flat[1][1] - flat[2][1]).abs() - 20.0).abs() < 1e-3);
}

/// At a bend the band keeps its width (mitred), and a sharp turn is
/// limited instead of spiking.
#[test]
fn bends_are_mitred() {
    let o = broad_arrow(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], 10.0, false).unwrap();
    // left and right at the corner: about √2 times the half width out
    let (l, r) = (o[1], o[o.len() - 2]);
    let d = ((l[0] - 100.0).hypot(l[1])).max((r[0] - 100.0).hypot(r[1]));
    let half = 5.0 * (0.55 + 0.45 * 0.5);
    assert!((d - half * 2f32.sqrt()).abs() < 0.05, "{d}");
    // a near reversal: the mitre is capped at twice the half width
    let o = broad_arrow(&[[0.0, 0.0], [100.0, 0.0], [0.0, 1.0]], 10.0, false).unwrap();
    assert!(o.iter().all(|p| (p[0] - 100.0).hypot(p[1]) < 12.0 || p[0] < 99.0));
}

/// Too short for the head, or a single point: nothing.
#[test]
fn short_paths_draw_nothing() {
    assert!(broad_arrow(&[[0.0, 0.0], [20.0, 0.0]], 20.0, true).is_none());
    assert!(broad_arrow(&[[0.0, 0.0], [0.2, 0.0]], 20.0, false).is_none());
}
