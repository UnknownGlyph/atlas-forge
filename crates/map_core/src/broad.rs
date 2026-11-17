//! Broad campaign arrows: a route drawn as a filled band that follows its
//! path, widening from a narrower start, and (with an arrow) ending in a
//! large head, the way historical maps show an advance or a migration.
//! The outline is built on screen from the projected path, for the app,
//! the exports and the legend alike.

/// The outline of a broad arrow `width` pixels wide along `path` (screen
/// pixels), with a head at its end when `head`. The band starts at 55% of
/// the width and widens to the full width; bends are mitred (limited so a
/// sharp turn does not spike). `None` when the path is too short.
pub fn broad_arrow(path: &[[f32; 2]], width: f32, head: bool) -> Option<Vec<[f32; 2]>> {
    // drop repeated points
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(path.len());
    for p in path {
        if pts.last().is_none_or(|l: &[f32; 2]| (p[0] - l[0]).hypot(p[1] - l[1]) > 0.5) {
            pts.push(*p);
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let mut cum = vec![0.0f32];
    for w in pts.windows(2) {
        cum.push(cum.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
    }
    let length = *cum.last().unwrap();
    let (head_len, head_half) = (width * 1.4 + 6.0, width + 4.0);
    // where the band ends: at the head's base, or at the path's end
    let end = if head { length - head_len } else { length };
    if end < width * 0.5 {
        return None;
    }
    let at = |s: f32| -> [f32; 2] {
        let i = cum.partition_point(|c| *c < s).clamp(1, pts.len() - 1);
        let seg = cum[i] - cum[i - 1];
        let t = if seg > 0.0 { ((s - cum[i - 1]) / seg).clamp(0.0, 1.0) } else { 0.0 };
        [pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * t, pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t]
    };
    // the band's spine: the path's points before `end`, then the point at `end`
    let mut spine: Vec<([f32; 2], f32)> = pts.iter().zip(&cum).filter(|(_, s)| **s < end - 0.5).map(|(p, s)| (*p, *s)).collect();
    spine.push((at(end), end));
    if spine.len() < 2 {
        return None;
    }
    let unit = |a: [f32; 2], b: [f32; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = dx.hypot(dy).max(1e-6);
        [dx / n, dy / n]
    };
    let n = spine.len();
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let (p, s) = spine[i];
        // the directions of the stretches before and after this point
        let before = if i == 0 { unit(spine[0].0, spine[1].0) } else { unit(spine[i - 1].0, spine[i].0) };
        let after = if i + 1 == n { before } else { unit(spine[i].0, spine[i + 1].0) };
        // the normals of the two sides of the corner, and their mitre
        let (n1, n2) = ([-before[1], before[0]], [-after[1], after[0]]);
        let m = [n1[0] + n2[0], n1[1] + n2[1]];
        let ml = m[0].hypot(m[1]);
        let m = if ml < 1e-6 { n1 } else { [m[0] / ml, m[1] / ml] };
        let scale = 1.0 / (m[0] * n1[0] + m[1] * n1[1]).max(0.5);
        let half = width * 0.5 * (0.55 + 0.45 * (s / end).clamp(0.0, 1.0)) * scale;
        left.push([p[0] + m[0] * half, p[1] + m[1] * half]);
        right.push([p[0] - m[0] * half, p[1] - m[1] * half]);
    }
    let mut outline = left;
    if head {
        let base = spine[n - 1].0;
        let tip = pts[pts.len() - 1];
        let d = unit(base, tip);
        let nh = [-d[1], d[0]];
        outline.push([base[0] + nh[0] * head_half, base[1] + nh[1] * head_half]);
        outline.push(tip);
        outline.push([base[0] - nh[0] * head_half, base[1] - nh[1] * head_half]);
    }
    outline.extend(right.into_iter().rev());
    Some(outline)
}
