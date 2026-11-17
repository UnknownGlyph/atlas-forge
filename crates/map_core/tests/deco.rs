//! Front-line teeth and railway ties along a line.

use map_core::deco::{decorate, deco_size, Deco, DecoMark};

/// Along a straight line drawn left to right: teeth evenly spaced, each a
/// triangle with its base on the line and its point above it (the left of
/// the direction of travel), below when flipped; none when undecorated.
#[test]
fn teeth_stand_on_one_side() {
    let line = [[0.0, 100.0], [300.0, 100.0]];
    let (step, half, height) = deco_size(Deco::Teeth, 2.0);
    let marks = decorate(&line, Deco::Teeth, 2.0, false);
    assert_eq!(marks.len(), ((300.0 - step * 0.5 - half) / step).floor() as usize + 1);
    for (i, m) in marks.iter().enumerate() {
        let DecoMark::Fill(t) = m else { panic!("a tooth is a fill") };
        assert!((t[0][1] - 100.0).abs() < 1e-4 && (t[1][1] - 100.0).abs() < 1e-4, "base on the line");
        assert!((t[2][1] - (100.0 - height)).abs() < 1e-4, "point above");
        assert!((t[2][0] - (step * 0.5 + step * i as f32)).abs() < 1e-3, "evenly spaced");
    }
    let flipped = decorate(&line, Deco::Teeth, 2.0, true);
    assert!(flipped.iter().all(|m| matches!(m, DecoMark::Fill(t) if t[2][1] > 100.0)));
    // drawn the other way, the left side is below
    let back = decorate(&[[300.0, 100.0], [0.0, 100.0]], Deco::Teeth, 2.0, false);
    assert!(back.iter().all(|m| matches!(m, DecoMark::Fill(t) if t[2][1] > 100.0)));
    assert!(decorate(&line, Deco::None, 2.0, false).is_empty());
}

/// Ties cross the line at right angles, centred on it, also round a corner.
#[test]
fn ties_cross_the_line() {
    let corner = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]];
    let (_, half, _) = deco_size(Deco::Ticks, 2.0);
    let marks = decorate(&corner, Deco::Ticks, 2.0, false);
    assert!(marks.len() >= 15);
    for m in &marks {
        let DecoMark::Stroke([a, b]) = m else { panic!("a tie is a stroke") };
        let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        // centred on one of the two legs, and as long as asked
        assert!(mid[1].abs() < 1e-3 || (mid[0] - 100.0).abs() < 1.5, "{mid:?}");
        assert!(((b[0] - a[0]).hypot(b[1] - a[1]) - 2.0 * half).abs() < 1e-3);
    }
    // on the flat leg the ties are vertical
    let DecoMark::Stroke([a, b]) = &marks[0] else { unreachable!() };
    assert!((a[0] - b[0]).abs() < 1e-4);
}

/// Peaks and trees along a line: upright pictures, to either side of the
/// line in turn, as many as fit; a peak is two strokes meeting at its top,
/// a tree a filled crown over a trunk.
#[test]
fn peaks_and_trees_along_a_line() {
    use map_core::deco::{decorate, deco_size, Deco, DecoMark};
    // a line going up the screen: "upright" must not follow it
    let line = [[50.0f32, 300.0], [50.0, 0.0]];
    let (step, _, _) = deco_size(Deco::Peaks, 2.0);
    let peaks = decorate(&line, Deco::Peaks, 2.0, false);
    assert_eq!(peaks.len() % 2, 0);
    assert!(peaks.len() / 2 >= (300.0 / step) as usize - 1);
    for pair in peaks.chunks(2) {
        let (DecoMark::Stroke(a), DecoMark::Stroke(b)) = (&pair[0], &pair[1]) else { panic!("strokes") };
        assert!(a[1][1] < a[0][1] && a[1] == b[0], "the top above the feet");
    }
    // to either side of the line in turn
    let side = |m: &DecoMark| match m {
        DecoMark::Stroke(s) => s[1][0] - 50.0,
        DecoMark::Fill(p) => p[0][0] - 50.0,
    };
    assert!(side(&peaks[0]) * side(&peaks[2]) < 0.0);
    let trees = decorate(&line, Deco::Trees, 2.0, false);
    assert!(matches!(trees[0], DecoMark::Fill(_)) && matches!(trees[1], DecoMark::Stroke(_)));
}
