//! Writing a name along a line (a river, a road): each glyph sits on the
//! line at its own place, turned with it, a little above it, centred on
//! the middle of the line and always reading left to right. Shared by the
//! app and the exports, so a name falls in the same place in both.

/// Size of a line's name in pixels (scaled in export).
pub const NAME_PX: f32 = 12.0;

/// Glyph centres and turns (degrees, counter-clockwise as seen) for a name
/// along `path` (screen pixels, y down), given each glyph's width; `lift`
/// is how far above the line the glyph centres sit. `None` when the line
/// is too short to carry the name.
pub fn along_path(path: &[[f32; 2]], advances: &[f32], lift: f32) -> Option<Vec<([f32; 2], f32)>> {
    along_path_shifted(path, advances, lift, 0.0)
}

/// As [`along_path`], with the name moved `shift` pixels from the middle
/// of the line towards its right end (back when negative), no further
/// than the line's ends.
pub fn along_path_shifted(path: &[[f32; 2]], advances: &[f32], lift: f32, shift: f32) -> Option<Vec<([f32; 2], f32)>> {
    if path.len() < 2 || advances.is_empty() {
        return None;
    }
    // read left to right: follow the line from its left end
    let mut pts: Vec<[f32; 2]> = path.to_vec();
    if pts[pts.len() - 1][0] < pts[0][0] {
        pts.reverse();
    }
    let mut cum = vec![0.0f32];
    for w in pts.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        cum.push(cum.last().unwrap() + d);
    }
    let length = *cum.last().unwrap();
    let total: f32 = advances.iter().sum();
    if length < total * 1.1 {
        return None;
    }
    // the point at distance s along the line
    let at = |s: f32| -> [f32; 2] {
        let s = s.clamp(0.0, length);
        let i = cum.partition_point(|c| *c < s).clamp(1, pts.len() - 1);
        let seg = cum[i] - cum[i - 1];
        let t = if seg > 0.0 { (s - cum[i - 1]) / seg } else { 0.0 };
        [pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * t, pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t]
    };
    let mut s = ((length - total) * 0.5 + shift).clamp(0.0, length - total);
    let mut out = Vec::with_capacity(advances.len());
    for w in advances {
        let mid = s + w * 0.5;
        // the direction across the glyph's width, smoother than at a point
        let (a, b) = (at(mid - w * 0.5), at(mid + w * 0.5));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (tx, ty) = (dx / n, dy / n);
        let p = at(mid);
        // above the line: the tangent turned a quarter towards the top
        out.push(([p[0] + ty * lift, p[1] - tx * lift], (-ty).atan2(tx).to_degrees()));
        s += w;
    }
    Some(out)
}
