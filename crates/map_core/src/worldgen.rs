//! New worlds ("新世界向导"): continents, terrain, rivers and countries
//! made from a few settings, as an ordinary map that can be edited like
//! any other (faces, borders, rivers, countries, cities).
//!
//! 1. Elevation on a half-degree grid: fractal noise on the sphere (so it
//!    has no seams), bulges where the continents are to be, ridged noise
//!    for mountain ranges; the sea level set so that the requested share
//!    of the surface is land. The sea is kept along the antimeridian and
//!    near the poles, so no land crosses the map's edge or wraps a pole.
//! 2. Lakes are filled and the smallest islets dropped; cells touching only
//!    at a corner are joined, so every outline is simple.
//! 3. Countries grow from seeds spread over the land (more on larger
//!    landmasses), each step costing more across steep ground, so borders
//!    tend to follow ridges; islands with no seed go to the nearest one.
//! 4. Each country's (or landmass's) cells are traced into polygons that
//!    share their borders cell edge for cell edge, built into a shared-edge
//!    topology, then every edge is smoothed and made fractal (nodes stay).
//! 5. Rivers follow the flow over the elevation (depressions filled), the
//!    largest catchments first; capitals sit inside each country.
//!
//! The elevation is kept as the document's [`Terrain`] (hill shading).

use crate::build::{build, SourcePolygon};
use crate::doc::{Document, EntityId, EntityKind, LineFeature, LineId};
use crate::geom::LonLat;
use crate::names::{self, NameKind, NameStyle};
use crate::sample::Rng;
use crate::terrain::Terrain;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::Instant;

/// Grid cells per degree.
const N: usize = 2;
const W: usize = 360 * N;
const H: usize = 180 * N;

#[derive(Clone, Debug)]
pub struct WorldSpec {
    pub name: String,
    pub radius_km: f64,
    pub seed: u64,
    /// Share of the surface that is land (0.1 – 0.7).
    pub land: f64,
    /// How many continents (1 – 8); the rest are islands.
    pub continents: usize,
    /// How ragged coasts and borders are (0 – 1).
    pub roughness: f64,
    /// Countries to divide the land into (0: unclaimed land).
    pub countries: usize,
    pub style: NameStyle,
    /// Rivers to draw (the largest catchments).
    pub rivers: usize,
    /// A capital in each country.
    pub capitals: bool,
    /// A random history: first year, last year, how many changes (the
    /// countries above are the map before it).
    pub history: Option<(i32, i32, usize)>,
}

impl Default for WorldSpec {
    fn default() -> Self {
        WorldSpec { name: "新世界".into(), radius_km: 6371.0, seed: 1, land: 0.32, continents: 3, roughness: 0.5, countries: 24, style: NameStyle::Huaxia, rivers: 12, capitals: true, history: None }
    }
}

/// What making the world did.
#[derive(Clone, Debug, Default)]
pub struct WorldReport {
    /// Share of the surface that came out land.
    pub land: f64,
    pub landmasses: usize,
    pub countries: usize,
    pub rivers: usize,
    pub cities: usize,
    pub faces: usize,
    /// Changes of the random history that took place.
    pub changes: usize,
    pub ms: f64,
}

// ------------------------------------------------------------- noise

fn hash(x: i64, y: i64, z: i64, seed: u64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9) ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Smooth value noise in three dimensions (-1 – 1).
fn noise(p: [f64; 3], seed: u64) -> f64 {
    let f = [p[0].floor(), p[1].floor(), p[2].floor()];
    let t: [f64; 3] = std::array::from_fn(|k| {
        let x = p[k] - f[k];
        x * x * (3.0 - 2.0 * x)
    });
    let (x, y, z) = (f[0] as i64, f[1] as i64, f[2] as i64);
    let mut acc = 0.0;
    for dx in 0..2 {
        for dy in 0..2 {
            for dz in 0..2 {
                let w = (if dx == 1 { t[0] } else { 1.0 - t[0] }) * (if dy == 1 { t[1] } else { 1.0 - t[1] }) * (if dz == 1 { t[2] } else { 1.0 - t[2] });
                acc += w * hash(x + dx, y + dy, z + dz, seed);
            }
        }
    }
    acc
}

/// Fractal noise: octaves of halving weight and doubling detail (-1 – 1).
pub(crate) fn fbm(p: [f64; 3], octaves: usize, seed: u64) -> f64 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0);
    for k in 0..octaves {
        sum += amp * noise([p[0] * f, p[1] * f, p[2] * f], seed.wrapping_add(k as u64 * 101));
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

pub(crate) fn unit(lon: f64, lat: f64) -> [f64; 3] {
    let (lo, la) = (lon.to_radians(), lat.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

fn cell_lon(i: usize) -> f64 {
    -180.0 + (i as f64 + 0.5) / N as f64
}

fn cell_lat(j: usize) -> f64 {
    90.0 - (j as f64 + 0.5) / N as f64
}

/// The four neighbours of a cell (no wrapping: the edges are sea).
fn around(i: usize, j: usize) -> impl Iterator<Item = (usize, usize)> {
    [(i as i64 - 1, j as i64), (i as i64 + 1, j as i64), (i as i64, j as i64 - 1), (i as i64, j as i64 + 1)].into_iter().filter(|(x, y)| *x >= 0 && *y >= 0 && (*x as usize) < W && (*y as usize) < H).map(|(x, y)| (x as usize, y as usize))
}

/// Connected groups of cells for which `inside` holds (4-neighbourhood).
fn components(inside: &[bool]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; W * H];
    let mut out = Vec::new();
    for start in 0..W * H {
        if !inside[start] || seen[start] {
            continue;
        }
        let mut group = vec![start];
        seen[start] = true;
        let mut k = 0;
        while k < group.len() {
            let c = group[k];
            k += 1;
            for (x, y) in around(c % W, c / W) {
                let n = y * W + x;
                if inside[n] && !seen[n] {
                    seen[n] = true;
                    group.push(n);
                }
            }
        }
        out.push(group);
    }
    out
}

fn cell_area(c: usize) -> f64 {
    cell_lat(c / W).to_radians().cos()
}

// ------------------------------------------------------------- elevation

/// Elevation (arbitrary units) and the sea level giving the land share.
fn elevation(spec: &WorldSpec, rng: &mut Rng) -> (Vec<f64>, f64) {
    let seed = spec.seed;
    let n = spec.continents.clamp(1, 8);
    // continent centres away from the map's edge and the poles
    let centres: Vec<([f64; 3], f64)> = (0..n)
        .map(|_| {
            let (lon, lat) = (rng.range(-150.0, 150.0), rng.range(-55.0, 60.0));
            let r = (spec.land.clamp(0.1, 0.7) * 4.0 / n as f64).sqrt() * rng.range(0.55, 0.85);
            (unit(lon, lat), r)
        })
        .collect();
    let mut e = vec![0.0; W * H];
    for j in 0..H {
        for i in 0..W {
            let (lon, lat) = (cell_lon(i), cell_lat(j));
            let v = unit(lon, lat);
            // a warped space for coasts that are not round
            let warp = [fbm([v[0] * 2.0, v[1] * 2.0, v[2] * 2.0], 3, seed + 11), fbm([v[0] * 2.0, v[1] * 2.0, v[2] * 2.0], 3, seed + 12), fbm([v[0] * 2.0, v[1] * 2.0, v[2] * 2.0], 3, seed + 13)];
            let q = [v[0] + 0.3 * warp[0], v[1] + 0.3 * warp[1], v[2] + 0.3 * warp[2]];
            let mut h = 0.55 * fbm([q[0] * 1.8, q[1] * 1.8, q[2] * 1.8], 6, seed);
            for (c, r) in &centres {
                let cos = (v[0] * c[0] + v[1] * c[1] + v[2] * c[2]).clamp(-1.0, 1.0);
                let a = cos.acos();
                h += 0.75 * (-(a / r) * (a / r)).exp();
            }
            // mountain ranges: ridged noise, stronger inland
            let ridge = 1.0 - fbm([q[0] * 4.0, q[1] * 4.0, q[2] * 4.0], 4, seed + 31).abs();
            h += 0.22 * ridge * ridge * h.max(0.0);
            // the sea along the antimeridian and near the poles
            let edge = (lon.abs() - 165.0).max(0.0) / 8.0;
            let pole = (lat.abs() - 62.0).max(0.0) / 10.0;
            h -= 1.5 * (edge * edge + pole * pole);
            e[j * W + i] = h;
        }
    }
    // sea level: the land share by area
    let mut by: Vec<(f64, f64)> = (0..W * H).map(|c| (e[c], cell_area(c))).collect();
    by.sort_by(|a, b| b.0.total_cmp(&a.0));
    let total: f64 = by.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    let mut sea = by[0].0;
    for (h, a) in by {
        acc += a;
        sea = h;
        if acc >= total * spec.land.clamp(0.05, 0.8) {
            break;
        }
    }
    (e, sea)
}

/// The land mask, tidied: no land at the edges, no lakes, no islets, no
/// cells meeting only at a corner.
fn land_mask(e: &[f64], sea: f64) -> Vec<bool> {
    let mut land: Vec<bool> = (0..W * H).map(|c| e[c] > sea).collect();
    for (c, l) in land.iter_mut().enumerate() {
        let (lon, lat) = (cell_lon(c % W), cell_lat(c / W));
        if lon.abs() > 176.0 || lat.abs() > 80.0 {
            *l = false;
        }
    }
    for _ in 0..12 {
        // islets of a few cells go
        for g in components(&land) {
            if g.len() < 10 {
                for c in g {
                    land[c] = false;
                }
            }
        }
        // water not joined to the open sea becomes land
        let water: Vec<bool> = land.iter().map(|l| !l).collect();
        let groups = components(&water);
        if let Some(ocean) = groups.iter().map(|g| g.iter().map(|c| cell_area(*c)).sum::<f64>()).enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0) {
            for (k, g) in groups.iter().enumerate() {
                if k != ocean {
                    for c in g {
                        land[*c] = true;
                    }
                }
            }
        }
        // cells meeting only at a corner are joined
        let mut changed = false;
        for j in 0..H - 1 {
            for i in 0..W - 1 {
                let (a, b, c, d) = (j * W + i, j * W + i + 1, (j + 1) * W + i, (j + 1) * W + i + 1);
                if land[a] == land[d] && land[b] == land[c] && land[a] != land[b] {
                    for x in [a, b, c, d] {
                        land[x] = true;
                    }
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    land
}

// ------------------------------------------------------------- countries

/// A cost in a priority queue (ordered by `total_cmp`).
struct Cost(f64);
impl PartialEq for Cost {
    fn eq(&self, o: &Self) -> bool {
        self.0.total_cmp(&o.0).is_eq()
    }
}
impl Eq for Cost {}
impl PartialOrd for Cost {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cost {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// A label for every land cell: its country (0..countries), grown from
/// seeds; `None` at sea.
fn grow_countries(spec: &WorldSpec, e: &[f64], sea: f64, land: &[bool], masses: &[Vec<usize>], rng: &mut Rng) -> (Vec<Option<usize>>, usize) {
    let mut label: Vec<Option<usize>> = vec![None; W * H];
    let total: f64 = masses.iter().map(|g| g.iter().map(|c| cell_area(*c)).sum::<f64>()).sum();
    // seeds, spread out on each landmass in proportion to its size
    let mut seeds: Vec<usize> = Vec::new();
    let mut order: Vec<&Vec<usize>> = masses.iter().collect();
    order.sort_by_key(|g| Reverse(g.len()));
    for (k, g) in order.iter().enumerate() {
        let share = g.iter().map(|c| cell_area(*c)).sum::<f64>() / total;
        let mut n = (share * spec.countries as f64).round() as usize;
        if k == 0 {
            n = n.max(1);
        }
        let n = n.min(g.len() / 20).min(spec.countries.saturating_sub(seeds.len()));
        let sample: Vec<usize> = (0..400.min(g.len())).map(|_| g[(rng.next_f64() * g.len() as f64) as usize % g.len()]).collect();
        let mut mine: Vec<usize> = Vec::new();
        for _ in 0..n {
            let far = |c: usize| mine.iter().map(|s| dist(*s, c)).fold(f64::INFINITY, f64::min);
            let pick = if mine.is_empty() { sample[0] } else { *sample.iter().max_by(|a, b| far(**a).total_cmp(&far(**b))).unwrap() };
            mine.push(pick);
        }
        seeds.extend(mine);
    }
    let n = seeds.len();
    if n == 0 {
        return (label, 0);
    }
    // growth: steep ground costs more, so borders follow ridges
    let rough: Vec<f64> = (0..W * H).map(|c| if land[c] { 0.5 * (fbm(unit(cell_lon(c % W), cell_lat(c / W)).map(|x| x * 6.0), 3, spec.seed + 71) + 1.0) } else { 0.0 }).collect();
    let mut best = vec![f64::INFINITY; W * H];
    let mut heap: BinaryHeap<(Reverse<Cost>, usize, usize)> = BinaryHeap::new();
    for (k, s) in seeds.iter().enumerate() {
        best[*s] = 0.0;
        heap.push((Reverse(Cost(0.0)), *s, k));
    }
    while let Some((Reverse(Cost(c)), at, k)) = heap.pop() {
        if c > best[at] || label[at].is_some() {
            continue;
        }
        label[at] = Some(k);
        for (x, y) in around(at % W, at / W) {
            let nb = y * W + x;
            if !land[nb] || label[nb].is_some() {
                continue;
            }
            let climb = ((e[nb] - e[at]).abs() / (1.0 - sea).max(0.05)) * 40.0;
            let step = dist(at, nb) * (1.0 + rough[nb]) * (1.0 + climb);
            if c + step < best[nb] {
                best[nb] = c + step;
                heap.push((Reverse(Cost(c + step)), nb, k));
            }
        }
    }
    // landmasses no seed reached: to the nearest seed
    for g in masses {
        if g.iter().all(|c| label[*c].is_none()) {
            let mid = g[g.len() / 2];
            let k = (0..n).min_by(|a, b| dist(seeds[*a], mid).total_cmp(&dist(seeds[*b], mid))).unwrap();
            for c in g {
                label[*c] = Some(k);
            }
        }
    }
    no_corner_touches(&mut label);
    (label, n)
}

/// No country's cells may meet only at a corner (its outline would touch
/// itself there): one of the other two cells of such a square joins it.
fn no_corner_touches(label: &mut [Option<usize>]) {
    for _ in 0..64 {
        let mut changed = false;
        for j in 0..H - 1 {
            for i in 0..W - 1 {
                let (a, b, c, d) = (j * W + i, j * W + i + 1, (j + 1) * W + i, (j + 1) * W + i + 1);
                for (x, y, p, q) in [(a, d, b, c), (b, c, a, d)] {
                    let l = label[x];
                    if l.is_some() && label[y] == l && label[p] != l && label[q] != l {
                        // a land cell of the other diagonal joins it (water stays water)
                        if let Some(t) = [p, q].into_iter().find(|t| label[*t].is_some()) {
                            label[t] = l;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Distance between two cells (degrees of arc, near enough).
fn dist(a: usize, b: usize) -> f64 {
    let (la, lb) = (cell_lat(a / W), cell_lat(b / W));
    let dl = (cell_lon(a % W) - cell_lon(b % W)) * ((la + lb) * 0.5).to_radians().cos();
    dl.hypot(la - lb)
}

// ------------------------------------------------------------- outlines

/// The outlines of the cells with `label`: closed rings (lon/lat, no
/// repeated last point), region on the left (outer rings counter-clockwise).
fn trace(labels: &[Option<usize>], label: usize) -> Vec<Vec<LonLat>> {
    // grid corners: x 0..=W, y 0..=H (y down); the cell (i, j) spans x i..i+1, y j..j+1
    let has = |i: i64, j: i64| i >= 0 && j >= 0 && (i as usize) < W && (j as usize) < H && labels[j as usize * W + i as usize] == Some(label);
    let mut next: std::collections::BTreeMap<(i64, i64), Vec<(i64, i64)>> = std::collections::BTreeMap::new();
    let mut count = 0;
    for j in 0..H as i64 {
        for i in 0..W as i64 {
            if !has(i, j) {
                continue;
            }
            // counter-clockwise as seen with north up (y down on the grid)
            if !has(i, j + 1) {
                next.entry((i, j + 1)).or_default().push((i + 1, j + 1));
                count += 1;
            }
            if !has(i + 1, j) {
                next.entry((i + 1, j + 1)).or_default().push((i + 1, j));
                count += 1;
            }
            if !has(i, j - 1) {
                next.entry((i + 1, j)).or_default().push((i, j));
                count += 1;
            }
            if !has(i - 1, j) {
                next.entry((i, j)).or_default().push((i, j + 1));
                count += 1;
            }
        }
    }
    let mut rings = Vec::new();
    let mut used = 0;
    while used < count {
        let Some((&start, _)) = next.iter().find(|(_, v)| !v.is_empty()) else { break };
        let mut ring = vec![start];
        let mut at = start;
        let mut came = (0i64, 0i64);
        loop {
            let outs = next.get_mut(&at).unwrap();
            // at a corner shared by two runs of the region: turn left first
            let pick = if outs.len() == 1 {
                0
            } else {
                let turn = |d: (i64, i64)| -> i64 {
                    // cross product (y down): left turns first
                    let c = came.0 * d.1 - came.1 * d.0;
                    -c
                };
                (0..outs.len()).max_by_key(|k| turn((outs[*k].0 - at.0, outs[*k].1 - at.1))).unwrap()
            };
            let to = outs.swap_remove(pick);
            used += 1;
            came = (to.0 - at.0, to.1 - at.1);
            at = to;
            if at == start {
                break;
            }
            ring.push(at);
        }
        // every grid point is kept: a point in the middle of a straight run
        // here may be where two neighbours meet on the other side, and
        // shared borders must match point for point (straight runs are
        // thinned on the built edges, see `straightened`)
        if ring.len() >= 3 {
            rings.push(ring.into_iter().map(|(x, y)| LonLat::new(-180.0 + x as f64 / N as f64, 90.0 - y as f64 / N as f64)).collect());
        }
    }
    rings
}

/// Signed area (lon/lat degrees, counter-clockwise positive).
fn ring_area(r: &[LonLat]) -> f64 {
    let n = r.len();
    (0..n).map(|i| r[i].lon * r[(i + 1) % n].lat - r[(i + 1) % n].lon * r[i].lat).sum::<f64>() * 0.5
}

/// Rings into polygons: each outer ring with the holes inside it.
fn polygons(rings: Vec<Vec<LonLat>>) -> Vec<SourcePolygon> {
    let (outer, holes): (Vec<Vec<LonLat>>, Vec<Vec<LonLat>>) = rings.into_iter().partition(|r| ring_area(r) > 0.0);
    let mut polys: Vec<SourcePolygon> = outer.into_iter().map(|r| SourcePolygon { rings: vec![r] }).collect();
    for h in holes {
        let p = h[0];
        let inside = |r: &Vec<LonLat>| {
            let pts: Vec<[f64; 2]> = r.iter().map(|q| [q.lon, q.lat]).collect();
            crate::geom::point_in_poly([p.lon + 1e-6, p.lat + 1e-6], &pts)
        };
        // the smallest outer ring around it
        if let Some(k) = (0..polys.len()).filter(|k| inside(&polys[*k].rings[0])).min_by(|a, b| ring_area(&polys[*a].rings[0]).total_cmp(&ring_area(&polys[*b].rings[0]))) {
            polys[k].rings.push(h);
        }
    }
    polys
}

/// A line with the middle points of its straight runs left out (its ends kept).
fn straightened(pts: &[LonLat]) -> Vec<LonLat> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for k in 1..n - 1 {
        let (a, b, c) = (pts[k - 1], pts[k], pts[k + 1]);
        let cross = (b.lon - a.lon) * (c.lat - b.lat) - (b.lat - a.lat) * (c.lon - b.lon);
        if cross.abs() > 1e-9 {
            out.push(b);
        }
    }
    out.push(pts[n - 1]);
    out
}

/// A line's straight runs thinned, then smoothed `n` times (its ends kept).
pub(crate) fn smooth_line(pts: &[LonLat], n: usize) -> Vec<LonLat> {
    smooth(&straightened(pts), n)
}

/// Chaikin's corner cutting of a line (its ends kept), `n` times.
fn smooth(pts: &[LonLat], n: usize) -> Vec<LonLat> {
    let mut v: Vec<LonLat> = pts.to_vec();
    for _ in 0..n {
        if v.len() < 3 {
            break;
        }
        let mut o = vec![v[0]];
        for w in v.windows(2) {
            let (a, b) = (w[0], w[1]);
            o.push(LonLat::new(a.lon * 0.75 + b.lon * 0.25, a.lat * 0.75 + b.lat * 0.25));
            o.push(LonLat::new(a.lon * 0.25 + b.lon * 0.75, a.lat * 0.25 + b.lat * 0.75));
        }
        o.push(*v.last().unwrap());
        // the cut next to each end is dropped, so the ends keep their direction
        o.remove(1);
        let m = o.len();
        o.remove(m - 2);
        v = o;
    }
    v
}

// ------------------------------------------------------------- rivers

/// Rivers down the elevation: the land's flow towards the sea (low ground
/// first, depressions filled), the largest catchments' main streams.
fn rivers(e: &[f64], land: &[bool], count: usize) -> Vec<Vec<LonLat>> {
    if count == 0 {
        return vec![];
    }
    // priority flood from the coast inland: each cell drains to the cell it was reached from
    let mut down: Vec<usize> = vec![usize::MAX; W * H];
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; W * H];
    let mut heap: BinaryHeap<(Reverse<Cost>, usize)> = BinaryHeap::new();
    for c in 0..W * H {
        if land[c] && around(c % W, c / W).any(|(x, y)| !land[y * W + x]) {
            seen[c] = true;
            heap.push((Reverse(Cost(e[c])), c));
        }
    }
    let mut fill = e.to_vec();
    while let Some((Reverse(Cost(h)), c)) = heap.pop() {
        order.push(c);
        for (x, y) in around(c % W, c / W) {
            let n = y * W + x;
            if land[n] && !seen[n] {
                seen[n] = true;
                fill[n] = fill[n].max(h);
                down[n] = c;
                heap.push((Reverse(Cost(fill[n])), n));
            }
        }
    }
    // how much land drains through each cell
    let mut acc: Vec<f64> = (0..W * H).map(|c| if land[c] { cell_area(c) } else { 0.0 }).collect();
    for c in order.iter().rev() {
        if down[*c] != usize::MAX {
            acc[down[*c]] += acc[*c];
        }
    }
    // mouths: coast cells, the largest first
    let mut mouths: Vec<usize> = order.iter().copied().filter(|c| down[*c] == usize::MAX).collect();
    mouths.sort_by(|a, b| acc[*b].total_cmp(&acc[*a]));
    let mut ups: HashMap<usize, Vec<usize>> = HashMap::new();
    for c in &order {
        if down[*c] != usize::MAX {
            ups.entry(down[*c]).or_default().push(*c);
        }
    }
    let mut out = Vec::new();
    for m in mouths.into_iter().take(count * 3) {
        if out.len() >= count || acc[m] < 30.0 {
            break;
        }
        // up the main stream until the catchment is small
        let mut path = vec![m];
        let mut at = m;
        while let Some(u) = ups.get(&at).and_then(|v| v.iter().copied().max_by(|a, b| acc[*a].total_cmp(&acc[*b]))) {
            if acc[u] < acc[m] * 0.02 || acc[u] < 3.0 {
                break;
            }
            path.push(u);
            at = u;
        }
        if path.len() < 8 {
            continue;
        }
        path.reverse();
        let pts: Vec<LonLat> = path.iter().map(|c| LonLat::new(cell_lon(c % W), cell_lat(c / W))).collect();
        out.push(smooth(&pts, 2));
    }
    out
}

// ------------------------------------------------------------- the world

pub fn generate(spec: &WorldSpec) -> Result<(Document, WorldReport), String> {
    let t0 = Instant::now();
    let mut rng = Rng::new(spec.seed);
    let (e, sea) = elevation(spec, &mut rng);
    let land = land_mask(&e, sea);
    let masses = components(&land);
    if masses.is_empty() {
        return Err("没有生成陆地：请提高陆地比例".into());
    }
    let mut rep = WorldReport { landmasses: masses.len(), ..Default::default() };
    let all: f64 = (0..W * H).map(cell_area).sum();
    rep.land = (0..W * H).filter(|c| land[*c]).map(cell_area).sum::<f64>() / all;
    // countries, or each landmass on its own
    let (labels, n) = if spec.countries > 0 {
        grow_countries(spec, &e, sea, &land, &masses, &mut rng)
    } else {
        let mut l = vec![None; W * H];
        for (k, g) in masses.iter().enumerate() {
            for c in g {
                l[*c] = Some(k);
            }
        }
        (l, masses.len())
    };
    // polygons, built into a topology
    let mut polys: Vec<SourcePolygon> = Vec::new();
    let mut owner_of: Vec<usize> = Vec::new();
    for k in 0..n {
        for p in polygons(trace(&labels, k)) {
            polys.push(p);
            owner_of.push(k);
        }
    }
    let (topo, faces, _) = build(&polys).map_err(|b| format!("生成的轮廓无法建成拓扑：{}", b.problems.join("；")))?;
    let mut doc = Document::default();
    doc.meta.name = spec.name.clone();
    doc.meta.planet_radius_km = spec.radius_km;
    doc.topo = topo;
    // smoothed and made fractal, nodes kept
    let ids: Vec<crate::topo::EdgeId> = doc.topo.edges.keys().copied().collect();
    for id in ids {
        let full = doc.topo.edge_full(&doc.topo.edges[&id]);
        if full.len() < 3 {
            continue;
        }
        let s = smooth(&straightened(&full), 3);
        let backup = doc.topo.clone();
        if doc.topo.replace_edge_points(id, s[1..s.len() - 1].to_vec()).is_err() {
            doc.topo = backup;
        }
    }
    if spec.roughness > 0.0 {
        crate::testworld::refine_borders(&mut doc, 30.0, 0.25 + spec.roughness * 0.35, spec.seed + 5);
    }
    // countries with names and colours
    if spec.countries > 0 {
        let taken = HashSet::new();
        let names = names::unique(spec.style, NameKind::Country, n, spec.seed, &taken);
        let ents: Vec<EntityId> = (0..n).map(|k| doc.add_entity(EntityKind::Country, names.get(k).map_or("国家", |s| s.as_str()))).collect();
        for (pi, f) in faces.iter().enumerate() {
            if let Some(f) = f {
                let _ = doc.assign(*f, Some(ents[owner_of[pi]]));
            }
        }
        doc.apply_auto_colours(&crate::colouring::palette());
        rep.countries = n;
        // capitals inside each country
        if spec.capitals {
            let city_names = names::unique(spec.style, NameKind::City, n, spec.seed + 3, &names.iter().cloned().collect());
            for (k, ent) in ents.iter().enumerate() {
                let Some((at, _)) = doc.label_anchor(*ent) else { continue };
                if doc.owner.get(&doc.topo.face_at(at)) != Some(ent) {
                    continue;
                }
                let c = doc.add_city(at, city_names.get(k).map_or("首都", |s| s.as_str()));
                if let Some(x) = doc.entities.get_mut(ent) {
                    x.capital = Some(c);
                }
                rep.cities += 1;
            }
        }
    }
    // a history: borders move, countries break away and end
    if let (Some((from, to, events)), true) = (spec.history, spec.countries > 0) {
        let taken: HashSet<String> = doc.entities.values().map(|x| x.name.clone()).collect();
        let pool = names::unique(spec.style, NameKind::Country, events / 8 + 8, spec.seed + 13, &taken);
        let mut hr = crate::testworld::TestWorldReport::default();
        let mut hrng = Rng::new(spec.seed + 17);
        let (lo, hi) = (from.min(to), from.max(to));
        // (each break-away, even one that fails, takes the next name)
        let next = std::cell::Cell::new(0usize);
        crate::testworld::random_history(&mut doc, lo, hi, events, &mut hrng, &mut hr, &|n| {
            let k = next.get();
            next.set(k + 1);
            pool.get(k).cloned().unwrap_or_else(|| format!("新国{n}"))
        });
        rep.changes = hr.events_applied;
        // the new countries' colours, neighbours in any year apart
        doc.apply_auto_colours(&crate::colouring::palette());
    }
    // rivers with names
    let rs = rivers(&e, &land, spec.rivers);
    let river_names = names::unique(spec.style, NameKind::River, rs.len(), spec.seed + 7, &HashSet::new());
    for (k, pts) in rs.into_iter().enumerate() {
        let id = LineId(doc.topo.alloc());
        doc.rivers.insert(id, LineFeature { name: river_names.get(k).cloned().unwrap_or_default(), pts, route: None, legend: String::new(), show_name: true, hidden: false });
        rep.rivers += 1;
    }
    // the elevation, for hill shading
    let span_land = e.iter().copied().fold(f64::MIN, f64::max) - sea;
    let span_sea = sea - e.iter().copied().fold(f64::MAX, f64::min);
    let heights: Vec<i16> = (0..W * H)
        .map(|c| {
            let h = e[c] - sea;
            if land[c] {
                (h.max(0.0) / span_land.max(1e-9) * 4500.0 + 10.0).round() as i16
            } else {
                (h.min(0.0) / span_sea.max(1e-9) * 5000.0 - 10.0).round() as i16
            }
        })
        .collect();
    doc.terrain = Some(std::sync::Arc::new(Terrain::new(W as u32, H as u32, heights)));
    rep.faces = doc.topo.faces.len() - 1;
    rep.ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok((doc, rep))
}
