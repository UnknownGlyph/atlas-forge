//! Line styles for drawn shapes and existing boundaries: smoothing (corner
//! cutting) and "natural" roughening (fractal midpoint displacement, the
//! look of real coasts). Segments between two fixed points (e.g. traced
//! along existing borders) and fixed points themselves never move, so a
//! styled shape still joins the boundaries it was snapped to.

use crate::geom::{self, LonLat};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineStyle {
    /// As drawn.
    #[default]
    Plain,
    /// Rounded corners.
    Smooth,
    /// Irregular, coast-like detail.
    Natural,
}

impl LineStyle {
    pub const ALL: [LineStyle; 3] = [LineStyle::Plain, LineStyle::Smooth, LineStyle::Natural];
    pub fn name(self) -> &'static str {
        match self {
            LineStyle::Plain => "原样",
            LineStyle::Smooth => "平滑",
            LineStyle::Natural => "自然（海岸状）",
        }
    }
}

/// Local planar frame: unwrapped longitude scaled by cos(latitude) so that
/// displacements are the same length in every direction.
struct Frame {
    k: f64,
}

impl Frame {
    fn new(pts: &[LonLat]) -> Self {
        let lat = pts.iter().map(|p| p.lat).sum::<f64>() / pts.len().max(1) as f64;
        Self { k: lat.to_radians().cos().max(0.05) }
    }
    fn to(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.k, p[1]]
    }
    fn from(&self, p: [f64; 2]) -> LonLat {
        LonLat::new(geom::wrap_lon(p[0] / self.k), p[1].clamp(-90.0, 90.0))
    }
}

/// Equal-area frame (Lambert cylindrical with its standard parallel at the
/// mean latitude): plane areas are sphere areas, so a correction that keeps
/// plane area keeps the land area the map reports.
struct AreaFrame {
    k: f64,
}

impl AreaFrame {
    fn new(pts: &[LonLat]) -> Self {
        Self { k: Frame::new(pts).k }
    }
    fn to(&self, p: [f64; 2]) -> [f64; 2] {
        // sin(lat) in degree units, so the frame is near 1:1 at the equator
        [p[0] * self.k, p[1].to_radians().sin() * DEG / self.k]
    }
    fn from(&self, p: [f64; 2]) -> LonLat {
        let s = (p[1] * self.k / DEG).clamp(-1.0, 1.0);
        LonLat::new(geom::wrap_lon(p[0] / self.k), s.asin().to_degrees())
    }
}

const DEG: f64 = 57.295_779_513_082_32;

/// Deterministic pseudo-random number in [-0.5, 0.5) from a seed.
fn noise(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

/// A seed tied to a segment's end points, so a segment keeps its shape
/// while other parts of the draft change.
fn seed_of(a: [f64; 2], b: [f64; 2]) -> u64 {
    let q = |v: f64| (v * 1e6).round() as i64 as u64;
    q(a[0]).wrapping_mul(31).wrapping_add(q(a[1])).wrapping_mul(1_000_003).wrapping_add(q(b[0]).wrapping_mul(17)).wrapping_add(q(b[1]))
}

fn displace(a: [f64; 2], b: [f64; 2], amp: f64, min_len: f64, seed: u64, depth: u32, out: &mut Vec<[f64; 2]>) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < min_len || depth > 10 {
        return;
    }
    let off = len * amp * noise(seed ^ (depth as u64).wrapping_mul(0x5851_F42D_4C95_7F2D));
    let m = [(a[0] + b[0]) * 0.5 - dy / len * off, (a[1] + b[1]) * 0.5 + dx / len * off];
    displace(a, m, amp, min_len, seed.wrapping_mul(3).wrapping_add(1), depth + 1, out);
    out.push(m);
    displace(m, b, amp, min_len, seed.wrapping_mul(3).wrapping_add(2), depth + 1, out);
}

/// Chaikin corner cutting of an open run, keeping its two end points.
fn chaikin(run: &[[f64; 2]], iterations: u32) -> Vec<[f64; 2]> {
    let mut v = run.to_vec();
    for _ in 0..iterations {
        if v.len() < 3 {
            break;
        }
        let mut n = vec![v[0]];
        for w in v.windows(2) {
            let (a, b) = (w[0], w[1]);
            n.push([0.75 * a[0] + 0.25 * b[0], 0.75 * a[1] + 0.25 * b[1]]);
            n.push([0.25 * a[0] + 0.75 * b[0], 0.25 * a[1] + 0.75 * b[1]]);
        }
        n.push(*v.last().unwrap());
        // the first and last cut points next to the kept ends are redundant
        n.remove(1);
        let l = n.len();
        n.remove(l - 2);
        v = n;
    }
    v
}

/// Apply a style. `fixed[i]` marks points that must stay; a segment whose
/// two ends are both fixed stays as it is. `closed`: the last point joins
/// the first. `amount` (0–1) is the roughness for [`LineStyle::Natural`]
/// and ignored otherwise; `min_len` (degrees) stops subdividing below that
/// length. Returns the new points and their fixed flags. If the result
/// would cross itself, the roughness is halved (up to three times) and
/// finally the input is returned unchanged.
pub fn apply(pts: &[LonLat], fixed: &[bool], closed: bool, style: LineStyle, amount: f64, min_len: f64) -> (Vec<LonLat>, Vec<bool>) {
    let n = pts.len();
    if style == LineStyle::Plain || n < 2 {
        return (pts.to_vec(), fixed.to_vec());
    }
    let mut amp = amount.clamp(0.0, 1.0) * 0.6;
    for _ in 0..4 {
        let (out, flags) = apply_once(pts, fixed, closed, style, amp, min_len);
        let planar: Vec<[f64; 2]> = geom::unwrap_path(&out, None);
        let simple = if closed { geom::ring_is_simple(&planar) } else { geom::path_is_simple(&planar) };
        if simple {
            return (out, flags);
        }
        if style == LineStyle::Smooth {
            break;
        }
        amp *= 0.5;
    }
    (pts.to_vec(), fixed.to_vec())
}

fn apply_once(pts: &[LonLat], fixed: &[bool], closed: bool, style: LineStyle, amp: f64, min_len: f64) -> (Vec<LonLat>, Vec<bool>) {
    let fr = Frame::new(pts);
    let un = geom::unwrap_path(pts, None);
    let p: Vec<[f64; 2]> = un.iter().map(|q| fr.to(*q)).collect();
    let n = p.len();
    let fx = |i: usize| fixed.get(i % n).copied().unwrap_or(false);
    let segs = if closed { n } else { n - 1 };
    let free = |i: usize| !(fx(i) && fx(i + 1));
    let mut out: Vec<([f64; 2], bool)> = Vec::new();
    match style {
        LineStyle::Natural => {
            for i in 0..segs {
                let (a, b) = (p[i], if i + 1 == n { p[0] } else { p[i + 1] });
                out.push((a, fx(i)));
                if free(i) {
                    let mut mids = Vec::new();
                    displace(a, b, amp, min_len, seed_of(un[i], un[(i + 1) % n]), 0, &mut mids);
                    out.extend(mids.into_iter().map(|m| (m, false)));
                }
            }
            if !closed {
                out.push((p[n - 1], fx(n - 1)));
            }
        }
        LineStyle::Smooth => {
            // runs of free segments are smoothed, keeping their end points
            // a closed ring with no fixed point at all is cut everywhere; a
            // single fixed point must stay, so it starts a run
            let all_free = closed && (0..n).all(|i| !fx(i));
            if all_free {
                // a closed ring with nothing fixed: cut every corner
                let mut v: Vec<[f64; 2]> = p.clone();
                for _ in 0..3 {
                    let m = v.len();
                    let mut nv = Vec::with_capacity(2 * m);
                    for i in 0..m {
                        let (a, b) = (v[i], v[(i + 1) % m]);
                        nv.push([0.75 * a[0] + 0.25 * b[0], 0.75 * a[1] + 0.25 * b[1]]);
                        nv.push([0.25 * a[0] + 0.75 * b[0], 0.25 * a[1] + 0.75 * b[1]]);
                    }
                    v = nv;
                }
                out = v.into_iter().map(|q| (q, false)).collect();
            } else {
                // rotate a closed ring to start at a fixed point
                let start = if closed { (0..n).find(|i| !free(*i) || fx(*i)).unwrap_or(0) } else { 0 };
                let idx: Vec<usize> = if closed { (0..=n).map(|k| (start + k) % n).collect() } else { (0..n).collect() };
                let mut run: Vec<[f64; 2]> = vec![p[idx[0]]];
                out.push((p[idx[0]], fx(idx[0])));
                for w in idx.windows(2) {
                    let (i, j) = (w[0], w[1]);
                    if free(i) {
                        run.push(p[j]);
                        // a run ends at a fixed point or the end
                        if fx(j) || w[1] == *idx.last().unwrap() && !closed {
                            let sm = chaikin(&run, 3);
                            out.extend(sm[1..].iter().map(|q| (*q, false)));
                            out.last_mut().unwrap().1 = fx(j);
                            run = vec![p[j]];
                        }
                    } else {
                        if run.len() > 1 {
                            let sm = chaikin(&run, 3);
                            out.extend(sm[1..sm.len() - 1].iter().map(|q| (*q, false)));
                        }
                        out.push((p[j], fx(j)));
                        run = vec![p[j]];
                    }
                }
                if run.len() > 1 {
                    let sm = chaikin(&run, 3);
                    out.extend(sm[1..].iter().map(|q| (*q, false)));
                }
                if closed {
                    out.pop(); // back at the start
                }
            }
        }
        LineStyle::Plain => unreachable!(),
    }
    let pts: Vec<LonLat> = out.iter().map(|(q, _)| fr.from(*q)).collect();
    (pts, out.iter().map(|(_, f)| *f).collect())
}

/// The outline a round brush of radius `r` leaves along a stroke, in a
/// plane (the drawing tools pass screen pixels): the union of a disc at
/// every point and a band along every segment. One outer ring per
/// separate patch; a stroke that closes a loop paints the inside too.
pub fn brush_outline(stroke: &[[f64; 2]], r: f64) -> Vec<Vec<[f64; 2]>> {
    use geo::BooleanOps;
    if stroke.is_empty() || r <= 0.0 {
        return vec![];
    }
    let poly = |pts: Vec<[f64; 2]>| geo::MultiPolygon::new(vec![geo::Polygon::new(geo::LineString::from(pts.into_iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()), vec![])]);
    let disc = |c: [f64; 2]| poly((0..48).map(|k| {
        let a = k as f64 * std::f64::consts::TAU / 48.0;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect());
    let mut parts: Vec<geo::MultiPolygon> = stroke.iter().map(|c| disc(*c)).collect();
    for w in stroke.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if len > 1e-9 {
            let n = [-(b[1] - a[1]) / len * r, (b[0] - a[0]) / len * r];
            parts.push(poly(vec![[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]]));
        }
    }
    // merge in pairs: a balanced tree keeps every union small
    while parts.len() > 1 {
        parts = parts.chunks(2).map(|c| if c.len() == 2 { c[0].union(&c[1]) } else { c[0].clone() }).collect();
    }
    parts
        .pop()
        .unwrap()
        .0
        .into_iter()
        .map(|p| {
            let mut ring: Vec<[f64; 2]> = p.exterior().coords().map(|c| [c.x, c.y]).collect();
            ring.pop(); // closed in geo
            // drop the near-collinear points of the disc joins
            let keep = crate::trace::simplify(&ring, r * 0.002);
            keep.into_iter().map(|i| ring[i]).collect::<Vec<_>>()
        })
        .filter(|r: &Vec<[f64; 2]>| r.len() >= 3)
        .collect()
}

/// Shoelace signed area.
fn signed(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>() * 0.5
}

/// Move the free points of a styled line along their normals so that it
/// encloses the same area as the original: the line keeps its character
/// but no land is gained or lost. For an open line (a border between two
/// fixed end points) the area between old and new is driven to zero and
/// the shift fades in from the ends; for a closed ring the ring's area is
/// restored, fixed points staying. None when the area cannot be matched
/// (a line with no free point, or a correction that would make it cross
/// itself): the caller then keeps the original line.
pub fn preserve_area(old: &[LonLat], new: &[LonLat], fixed_new: &[bool], closed: bool) -> Option<Vec<LonLat>> {
    if new.len() < 3 || old.len() < 2 {
        return None;
    }
    let mut all = old.to_vec();
    all.extend_from_slice(new);
    let fr = AreaFrame::new(&all);
    let start = Some(old[0].lon);
    let o: Vec<[f64; 2]> = geom::unwrap_path(old, start).into_iter().map(|q| fr.to(q)).collect();
    let mut p: Vec<[f64; 2]> = geom::unwrap_path(new, start).into_iter().map(|q| fr.to(q)).collect();
    let n = p.len();
    let target = if closed { signed(&o) } else { 0.0 };
    let area = |p: &[[f64; 2]]| -> f64 {
        if closed {
            signed(p)
        } else {
            // old forward, new backward: the area between them
            let mut ring = o.clone();
            ring.extend(p.iter().rev().skip(1).take(p.len().saturating_sub(2)));
            signed(&ring)
        }
    };
    // per point: normal and weight
    let len: Vec<f64> = std::iter::once(0.0)
        .chain(p.windows(2).scan(0.0, |acc, w| {
            *acc += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            Some(*acc)
        }))
        .collect();
    let total = len.last().copied().unwrap_or(0.0).max(1e-12);
    let weight: Vec<f64> = (0..n)
        .map(|i| {
            let fixed = fixed_new.get(i).copied().unwrap_or(false);
            if fixed || (!closed && (i == 0 || i + 1 == n)) {
                0.0
            } else if closed {
                1.0
            } else {
                (std::f64::consts::PI * len[i] / total).sin()
            }
        })
        .collect();
    // shift directions that stay steady on jagged lines: for an open line
    // the normal of its chord, for a ring a normal over a wide stencil
    let k = (n / 16).max(1);
    let normals = |p: &[[f64; 2]]| -> Vec<[f64; 2]> {
        let unit = |a: [f64; 2], b: [f64; 2]| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-12);
            [-dy / l, dx / l]
        };
        if !closed {
            let c = unit(p[0], p[n - 1]);
            return vec![c; n];
        }
        (0..n).map(|i| unit(p[(i + n - k) % n], p[(i + k) % n])).collect()
    };
    let simple = |p: &[[f64; 2]]| if closed { geom::ring_is_simple(p) } else { geom::path_is_simple(p) };
    let shifted = |p: &[[f64; 2]], c: f64| -> Vec<[f64; 2]> {
        let nm = normals(p);
        p.iter().enumerate().map(|(i, q)| [q[0] + c * weight[i] * nm[i][0], q[1] + c * weight[i] * nm[i][1]]).collect()
    };
    for _ in 0..8 {
        let a0 = area(&p) - target;
        if a0.abs() <= 1e-9 * total * total {
            break;
        }
        let probe = 1e-4 * total;
        let a1 = area(&shifted(&p, probe)) - target;
        let slope = (a1 - a0) / probe;
        if slope.abs() < 1e-15 {
            break;
        }
        // the full step, or smaller ones when it would cross itself
        let mut step = -a0 / slope;
        let mut moved = false;
        for _ in 0..6 {
            let cand = shifted(&p, step);
            if simple(&cand) {
                p = cand;
                moved = true;
                break;
            }
            step *= 0.5;
        }
        if !moved {
            break;
        }
    }
    if (area(&p) - target).abs() > 1e-6 * total * total {
        return None;
    }
    Some(p.into_iter().map(|q| fr.from(q)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<LonLat> {
        vec![LonLat::new(0.0, 0.0), LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0), LonLat::new(0.0, 10.0)]
    }

    #[test]
    fn natural_adds_detail_deterministically_and_stays_simple() {
        let sq = square();
        let fixed = vec![false; 4];
        let (a, fa) = apply(&sq, &fixed, true, LineStyle::Natural, 0.5, 0.2);
        let (b, _) = apply(&sq, &fixed, true, LineStyle::Natural, 0.5, 0.2);
        assert_eq!(a, b, "same input, same coast");
        assert!(a.len() > 100, "{}", a.len());
        assert_eq!(fa.len(), a.len());
        assert!(geom::ring_is_simple(&geom::unwrap_path(&a, None)));
        // the corners stay
        for c in &sq {
            assert!(a.contains(c));
        }
        // area stays close (displacements go both ways)
        let area = |r: &[LonLat]| geom::signed_area(&geom::unwrap_path(r, None)).abs();
        assert!((area(&a) - 100.0).abs() < 30.0, "{}", area(&a));
        // zero roughness: only collinear points are added
        let (z, _) = apply(&sq, &fixed, true, LineStyle::Natural, 0.0, 0.2);
        assert!((area(&z) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn fixed_segments_and_points_never_move() {
        let sq = square();
        // the bottom edge was traced along a coast: both ends fixed
        let fixed = vec![true, true, false, false];
        for style in [LineStyle::Natural, LineStyle::Smooth] {
            let (out, flags) = apply(&sq, &fixed, true, style, 0.6, 0.2);
            assert!(out.contains(&sq[0]) && out.contains(&sq[1]), "{style:?}");
            // nothing between the two fixed points: the traced edge is untouched
            let i0 = out.iter().position(|p| *p == sq[0]).unwrap();
            let i1 = out.iter().position(|p| *p == sq[1]).unwrap();
            assert_eq!((i0 + 1) % out.len(), i1, "{style:?}: {out:?}");
            assert!(flags[i0] && flags[i1]);
        }
    }

    #[test]
    fn smooth_rounds_corners_and_keeps_open_ends() {
        let line = vec![LonLat::new(0.0, 0.0), LonLat::new(5.0, 5.0), LonLat::new(10.0, 0.0)];
        let (s, _) = apply(&line, &[false, false, false], false, LineStyle::Smooth, 0.0, 0.1);
        assert_eq!(s[0], line[0]);
        assert_eq!(*s.last().unwrap(), line[2]);
        assert!(s.len() > 5);
        // the sharp apex is cut off
        assert!(s.iter().all(|p| p.lat < 5.0 - 1e-9));
        let (r, _) = apply(&square(), &[false; 4], true, LineStyle::Smooth, 0.0, 0.1);
        assert!(r.len() >= 32 && geom::ring_is_simple(&geom::unwrap_path(&r, None)));
        // plain: unchanged
        assert_eq!(apply(&line, &[false; 3], false, LineStyle::Plain, 0.5, 0.1).0, line);
    }

    #[test]
    fn a_brush_leaves_discs_and_bands() {
        let r = 10.0;
        let area = |ring: &[[f64; 2]]| signed(ring).abs();
        // a dab: a disc
        let dab = brush_outline(&[[0.0, 0.0]], r);
        assert_eq!(dab.len(), 1);
        let disc = std::f64::consts::PI * r * r;
        assert!((area(&dab[0]) - disc).abs() / disc < 0.01, "{}", area(&dab[0]));
        // a straight stroke of length 100: a capsule
        let line: Vec<[f64; 2]> = (0..=20).map(|i| [i as f64 * 5.0, 0.0]).collect();
        let cap = brush_outline(&line, r);
        assert_eq!(cap.len(), 1);
        let want = 2.0 * r * 100.0 + disc;
        assert!((area(&cap[0]) - want).abs() / want < 0.01, "{} vs {want}", area(&cap[0]));
        assert!(geom::ring_is_simple(&cap[0]));
        // a closed loop paints its inside: one ring, no hole
        let loop_: Vec<[f64; 2]> = (0..=40).map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 40.0;
            [100.0 * a.cos(), 100.0 * a.sin()]
        }).collect();
        let o = brush_outline(&loop_, r);
        assert_eq!(o.len(), 1);
        assert!(area(&o[0]) > std::f64::consts::PI * 100.0 * 100.0);
        assert!(brush_outline(&[], r).is_empty());
    }

    #[test]
    fn keeping_area_cancels_the_change() {
        // an open border (e.g. between two countries) and its styled version
        let line: Vec<LonLat> = (0..=10).map(|i| LonLat::new(i as f64, if i % 2 == 0 { 0.0 } else { 0.8 })).collect();
        let mut fixed = vec![false; line.len()];
        fixed[0] = true;
        *fixed.last_mut().unwrap() = true;
        // areas as on the sphere: longitude by sin(latitude)
        let ea = |r: &[LonLat]| -> Vec<[f64; 2]> { geom::unwrap_path(r, None).into_iter().map(|q| [q[0], q[1].to_radians().sin() * DEG]).collect() };
        let between = |a: &[LonLat], b: &[LonLat]| {
            let mut r = ea(a);
            r.extend(ea(b).into_iter().rev().skip(1).take(b.len() - 2));
            signed(&r).abs()
        };
        for style in [LineStyle::Smooth, LineStyle::Natural] {
            let (styled, flags) = apply(&line, &fixed, false, style, 0.8, 0.05);
            let before = between(&line, &styled);
            let kept = preserve_area(&line, &styled, &flags, false).unwrap();
            let after = between(&line, &kept);
            assert!(before > 0.05, "{style:?}: the style must move area ({before})");
            assert!(after < 1e-6, "{style:?}: {before} -> {after}");
            assert_eq!((kept[0], *kept.last().unwrap()), (line[0], *line.last().unwrap()), "ends stay");
        }
        // a closed shape: area restored, fixed points stay
        let sq = vec![LonLat::new(0.0, 0.0), LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0), LonLat::new(0.0, 10.0)];
        let (sm, flags) = apply(&sq, &[true, false, false, false], true, LineStyle::Smooth, 0.0, 0.1);
        let a = |r: &[LonLat]| signed(&ea(r)).abs();
        let target = a(&sq);
        assert!((a(&sm) - target).abs() > 1.0);
        assert!(sm.contains(&sq[0]) && flags[sm.iter().position(|p| *p == sq[0]).unwrap()], "an isolated fixed point is kept");
        let kept = preserve_area(&sq, &sm, &flags, true).unwrap();
        // a straight line has nothing free to move: no area-keeping version
        let two = [LonLat::new(0.0, 0.0), LonLat::new(10.0, 0.0)];
        assert!(preserve_area(&line, &two, &[true, true], false).is_none());
        assert!((a(&kept) - target).abs() < 1e-6, "{} vs {target}", a(&kept));
        assert_eq!(kept[0], sq[0]);
    }
}
