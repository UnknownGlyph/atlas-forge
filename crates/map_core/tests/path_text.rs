//! Names written along lines.

use map_core::path_text::along_path;

/// On a straight line, glyphs sit side by side, centred on the middle,
/// above the line, unturned; the same whichever way the line was drawn.
#[test]
fn a_straight_line_carries_its_name_in_the_middle() {
    let line = [[0.0, 100.0], [200.0, 100.0]];
    let g = along_path(&line, &[10.0, 10.0, 10.0], 5.0).unwrap();
    let xs: Vec<f32> = g.iter().map(|p| p.0[0]).collect();
    assert_eq!(xs, vec![90.0, 100.0, 110.0]);
    assert!(g.iter().all(|p| (p.0[1] - 95.0).abs() < 1e-4 && p.1.abs() < 1e-4));
    let back = along_path(&[[200.0, 100.0], [0.0, 100.0]], &[10.0, 10.0, 10.0], 5.0).unwrap();
    assert_eq!(back, g, "drawn right to left, still read left to right");
}

/// On a line rising to the right, glyphs turn counter-clockwise and sit
/// on its upper left; around a corner each glyph follows its own stretch.
#[test]
fn glyphs_turn_with_the_line() {
    let rising = [[0.0, 100.0], [100.0, 0.0]];
    let g = along_path(&rising, &[10.0, 10.0], 4.0).unwrap();
    assert!(g.iter().all(|p| (p.1 - 45.0).abs() < 1e-3));
    // above the line means up and to the left of it
    let mid = [50.0, 50.0];
    let c = [(g[0].0[0] + g[1].0[0]) * 0.5, (g[0].0[1] + g[1].0[1]) * 0.5];
    assert!(c[0] < mid[0] && c[1] < mid[1]);
    // an L: flat, then down; long glyph runs cross the corner
    let corner = [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]];
    let g = along_path(&corner, &[10.0; 8], 0.0).unwrap();
    assert!(g[0].1.abs() < 1e-3 && (g[7].1 + 90.0).abs() < 1e-3, "{} {}", g[0].1, g[7].1);
}

/// Too short a line, or nothing to write, gives nothing.
#[test]
fn short_lines_carry_no_name() {
    assert!(along_path(&[[0.0, 0.0], [25.0, 0.0]], &[10.0, 10.0, 10.0], 0.0).is_none());
    assert!(along_path(&[[0.0, 0.0], [100.0, 0.0]], &[], 0.0).is_none());
    assert!(along_path(&[[0.0, 0.0]], &[10.0], 0.0).is_none());
}
