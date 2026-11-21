//! Decorated lines: a front line's teeth on one side, a railway's cross
//! ties. The marks are laid along the line on screen (pixels), so their
//! size and spacing stay even at any zoom and in every projection; the
//! app, the exports and the legend all draw them from here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deco {
    #[default]
    None,
    /// Triangles on one side (a front line; they point at the enemy).
    Teeth,
    /// Short ties across the line (a railway).
    Ticks,
    /// Little upright peaks along the line, a little to either side in
    /// turn (a mountain range drawn along its ridge).
    Peaks,
    /// Little upright trees along the line (a belt of forest).
    Trees,
}

impl Deco {
    pub const ALL: [Deco; 5] = [Deco::None, Deco::Teeth, Deco::Ticks, Deco::Peaks, Deco::Trees];
    pub fn name(self) -> &'static str {
        match self {
            Deco::None => "无",
            Deco::Teeth => "锯齿（前线）",
            Deco::Ticks => "横线（铁路）",
            Deco::Peaks => "山峰（山脉）",
            Deco::Trees => "树木（林带）",
        }
    }
}

/// One decoration mark, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum DecoMark {
    /// A filled triangle (convex).
    Fill(Vec<[f32; 2]>),
    /// A short stroke as wide as the line.
    Stroke([[f32; 2]; 2]),
}

/// Spacing and size of the marks for a line `width` pixels wide:
/// (distance between marks, half the base or tie length, tooth height).
pub fn deco_size(deco: Deco, width: f32) -> (f32, f32, f32) {
    match deco {
        Deco::None => (0.0, 0.0, 0.0),
        Deco::Teeth => ((width * 6.0).max(12.0), (width * 1.6).max(3.0), (width * 2.2).max(5.0)),
        Deco::Ticks => ((width * 5.0).max(10.0), (width * 2.2).max(5.0), 0.0),
        Deco::Peaks => ((width * 5.0).max(11.0), (width * 2.6).max(5.5), (width * 3.4).max(8.0)),
        Deco::Trees => ((width * 4.0).max(9.0), (width * 1.8).max(4.0), (width * 3.4).max(8.0)),
    }
}

/// The marks along `path` (screen pixels, y down) for a line `width`
/// pixels wide. Teeth stand on the left of the direction the line was
/// drawn in (`flip`: on the right); marks are spaced evenly, the first half
/// a step from the start.
pub fn decorate(path: &[[f32; 2]], deco: Deco, width: f32, flip: bool) -> Vec<DecoMark> {
    decorate_from(path, deco, width, flip, 0.0)
}

/// Marks on a visible portion of a path whose origin is `offset` pixels earlier.
pub fn decorate_from(path: &[[f32; 2]], deco: Deco, width: f32, flip: bool, offset: f64) -> Vec<DecoMark> {
    if deco == Deco::None || path.len() < 2 {
        return vec![];
    }
    let mut cum = vec![0.0f32];
    for w in path.windows(2) {
        cum.push(cum.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
    }
    let length = *cum.last().unwrap();
    let at = |s: f32| -> [f32; 2] {
        let s = s.clamp(0.0, length);
        let i = cum.partition_point(|c| *c < s).clamp(1, path.len() - 1);
        let seg = cum[i] - cum[i - 1];
        let t = if seg > 0.0 { (s - cum[i - 1]) / seg } else { 0.0 };
        [path[i - 1][0] + (path[i][0] - path[i - 1][0]) * t, path[i - 1][1] + (path[i][1] - path[i - 1][1]) * t]
    };
    let (step, half, height) = deco_size(deco, width);
    let side = if flip { -1.0 } else { 1.0 };
    let mut out = Vec::new();
    let spacing = f64::from(step);
    let mut k = ((offset - spacing * 0.5) / spacing).ceil().max(0.0) as u64;
    let mut s = ((k as f64 + 0.5) * spacing - offset) as f32;
    while s + half <= length {
        k += 1;
        let (a, b) = (at(s - half), at(s + half));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = dx.hypot(dy);
        if n > 1e-3 {
            // the left of the direction of travel (y down): the tangent turned a quarter
            let nl = [dy / n * side, -dx / n * side];
            let c = at(s);
            match deco {
                Deco::Teeth => out.push(DecoMark::Fill(vec![a, b, [c[0] + nl[0] * height, c[1] + nl[1] * height]])),
                Deco::Ticks => out.push(DecoMark::Stroke([[c[0] - nl[0] * half, c[1] - nl[1] * half], [c[0] + nl[0] * half, c[1] + nl[1] * half]])),
                Deco::Peaks | Deco::Trees => {
                    // upright whatever the line's direction, to either side of it in turn
                    let off = if k % 2 == 0 { height * 0.35 } else { -height * 0.35 };
                    let (x, y) = (c[0] + nl[0] * off, c[1] + nl[1] * off);
                    let (w, h) = (half, height * 0.5);
                    if deco == Deco::Peaks {
                        let top = [x, y - h * if k % 3 == 0 { 1.2 } else { 1.0 }];
                        out.push(DecoMark::Stroke([[x - w, y + h * 0.6], top]));
                        out.push(DecoMark::Stroke([top, [x + w, y + h * 0.6]]));
                    } else {
                        out.push(DecoMark::Fill(vec![[x, y - h], [x - w, y + h * 0.35], [x + w, y + h * 0.35]]));
                        out.push(DecoMark::Stroke([[x, y + h * 0.35], [x, y + h * 0.9]]));
                    }
                }
                Deco::None => {}
            }
        }
        s += step;
    }
    out
}
