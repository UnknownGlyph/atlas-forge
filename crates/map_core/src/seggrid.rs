//! Segments bucketed in a uniform lon/lat grid, for finding the segments
//! near another one quickly. The cell size follows the data (a few mean
//! segment lengths), so finely drawn borders do not crowd a cell. Each
//! segment is entered in exactly the cells it passes through; a query
//! looks at the cells its own segment passes through and their
//! neighbours, so segments that meet, or come within one cell, are found.
//! The cells are one flat sorted list (no hashing). Longitude cells wrap
//! around the globe when asked, so segments meeting across the
//! antimeridian find each other.

pub type P2 = [f64; 2];

pub struct SegGrid {
    cell: f64,
    x0: f64,
    y0: f64,
    nx: i64,
    ny: i64,
    wrap: bool,
    /// Segments of cell `c`: `items[start[c]..start[c + 1]]`.
    start: Vec<u32>,
    items: Vec<u32>,
    /// Per query: the last query that saw each segment (no duplicates).
    stamp: Vec<u32>,
    query: u32,
    scratch: Vec<i64>,
}

/// `b` with longitude continuous from `a`.
fn unwrap(a: P2, b: P2, wrap: bool) -> P2 {
    if !wrap {
        return b;
    }
    let mut d = b[0] - a[0];
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    [a[0] + d, b[1]]
}

impl SegGrid {
    /// A grid for `segs` (lon/lat; `wrap`: longitudes wrap at ±180°). The
    /// cell is never smaller than `min_cell` (the distance queries reach).
    pub fn new(segs: &[(P2, P2)], min_cell: f64, wrap: bool) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        let mut total = 0.0;
        for (a, b) in segs {
            let b = unwrap(*a, *b, wrap);
            total += (b[0] - a[0]).hypot(b[1] - a[1]);
            for p in [*a, b] {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        if segs.is_empty() {
            (lo, hi) = ([0.0; 2], [1.0; 2]);
        }
        let mean = if segs.is_empty() { 1.0 } else { total / segs.len() as f64 };
        let mut cell = (mean * 4.0).max(min_cell).max(1e-9);
        let pad = 2.0;
        let dims = |cell: f64| {
            let nx = if wrap { (360.0 / cell).ceil() as i64 } else { ((hi[0] - lo[0]) / cell).ceil() as i64 + 1 + 2 * pad as i64 };
            let ny = ((hi[1] - lo[1]) / cell).ceil() as i64 + 1 + 2 * pad as i64;
            (nx, ny)
        };
        // at most a few million cells
        while {
            let (nx, ny) = dims(cell);
            nx * ny > 4_000_000
        } {
            cell *= 2.0;
        }
        let (nx, ny) = dims(cell);
        let (x0, y0) = if wrap { (-180.0, lo[1] - pad * cell) } else { (lo[0] - pad * cell, lo[1] - pad * cell) };
        let mut g = SegGrid { cell, x0, y0, nx, ny, wrap, start: vec![], items: vec![], stamp: vec![0; segs.len()], query: 0, scratch: vec![] };
        // (cell, segment) pairs, sorted by cell
        let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(segs.len() * 2);
        let mut cells = Vec::new();
        for (i, (a, b)) in segs.iter().enumerate() {
            cells.clear();
            g.traverse(*a, *b, &mut cells);
            pairs.extend(cells.iter().map(|c| (*c as u32, i as u32)));
        }
        pairs.sort_unstable();
        pairs.dedup();
        let n = (nx * ny) as usize;
        let mut start = vec![0u32; n + 1];
        for (c, _) in &pairs {
            start[*c as usize + 1] += 1;
        }
        for c in 0..n {
            start[c + 1] += start[c];
        }
        g.items = pairs.into_iter().map(|(_, s)| s).collect();
        g.start = start;
        g
    }

    fn index(&self, x: i64, y: i64) -> Option<i64> {
        let x = if self.wrap { x.rem_euclid(self.nx) } else { x };
        (x >= 0 && x < self.nx && y >= 0 && y < self.ny).then_some(y * self.nx + x)
    }

    fn coords(&self, p: P2) -> (f64, f64) {
        ((p[0] - self.x0) / self.cell, (p[1] - self.y0) / self.cell)
    }

    /// The cells a segment passes through (grid traversal, corners included).
    fn traverse(&self, a: P2, b: P2, out: &mut Vec<i64>) {
        let b = unwrap(a, b, self.wrap);
        let (ax, ay) = self.coords(a);
        let (bx, by) = self.coords(b);
        let (mut x, mut y) = (ax.floor() as i64, ay.floor() as i64);
        let (ex, ey) = (bx.floor() as i64, by.floor() as i64);
        let (dx, dy) = (bx - ax, by - ay);
        let (sx, sy) = (dx.signum() as i64, dy.signum() as i64);
        let next = |v: f64, s: i64| if s > 0 { v.floor() + 1.0 - v } else { v - v.floor() };
        let (mut tx, mut ty) = (if dx != 0.0 { next(ax, sx) / dx.abs() } else { f64::INFINITY }, if dy != 0.0 { next(ay, sy) / dy.abs() } else { f64::INFINITY });
        let (ddx, ddy) = (if dx != 0.0 { 1.0 / dx.abs() } else { f64::INFINITY }, if dy != 0.0 { 1.0 / dy.abs() } else { f64::INFINITY });
        let limit = (ex - x).abs() + (ey - y).abs() + 2;
        for _ in 0..=limit {
            if let Some(i) = self.index(x, y) {
                out.push(i);
            }
            if x == ex && y == ey {
                break;
            }
            if tx < ty {
                x += sx;
                tx += ddx;
            } else {
                y += sy;
                ty += ddy;
            }
        }
    }

    /// Indices of the segments that may meet `a`–`b` or come within one
    /// cell of it, each once.
    pub fn near(&mut self, a: P2, b: P2, out: &mut Vec<u32>) {
        out.clear();
        self.query = self.query.wrapping_add(1);
        if self.query == 0 {
            self.stamp.fill(0);
            self.query = 1;
        }
        let mut cells = std::mem::take(&mut self.scratch);
        cells.clear();
        self.traverse(a, b, &mut cells);
        let base: Vec<i64> = cells.clone();
        cells.clear();
        for c in base {
            let (x, y) = (c % self.nx, c / self.nx);
            for ny in -1..=1 {
                for nx in -1..=1 {
                    if let Some(i) = self.index(x + nx, y + ny) {
                        cells.push(i);
                    }
                }
            }
        }
        cells.sort_unstable();
        cells.dedup();
        for &c in &cells {
            let (s, e) = (self.start[c as usize] as usize, self.start[c as usize + 1] as usize);
            for &i in &self.items[s..e] {
                if self.stamp[i as usize] != self.query {
                    self.stamp[i as usize] = self.query;
                    out.push(i);
                }
            }
        }
        self.scratch = cells;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_neighbours_including_across_the_antimeridian() {
        let segs = vec![([0.0, 0.0], [1.0, 0.0]), ([50.0, 50.0], [51.0, 50.0]), ([179.9, 10.0], [-179.9, 10.0]), ([0.0, 0.5], [0.0, -0.5])];
        let mut g = SegGrid::new(&segs, 0.01, true);
        let mut out = Vec::new();
        g.near([0.5, -1.0], [0.5, 1.0], &mut out);
        out.sort();
        assert_eq!(out, vec![0, 3]);
        // a segment just west of the antimeridian finds the one crossing it
        g.near([179.95, 9.0], [179.95, 11.0], &mut out);
        assert!(out.contains(&2) && !out.contains(&1));
        g.near([-179.95, 9.0], [-179.95, 11.0], &mut out);
        assert!(out.contains(&2));
        // a long segment collects everything along it, once
        g.near([0.5, 0.1], [50.5, 50.1], &mut out);
        let mut s = out.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), out.len());
        assert!(out.contains(&0) && out.contains(&1));
    }

    /// Against brute force: every pair of random short segments that
    /// intersect is found.
    #[test]
    fn misses_no_intersection() {
        let mut rng = crate::sample::Rng::new(4);
        let segs: Vec<(P2, P2)> = (0..3000)
            .map(|_| {
                let a = [rng.range(-179.0, 179.0), rng.range(-60.0, 60.0)];
                let b = [a[0] + rng.range(-3.0, 3.0), a[1] + rng.range(-3.0, 3.0)];
                (a, b)
            })
            .collect();
        let crosses = |p: (P2, P2), q: (P2, P2)| {
            let o = |a: P2, b: P2, c: P2| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            o(p.0, p.1, q.0) * o(p.0, p.1, q.1) < 0.0 && o(q.0, q.1, p.0) * o(q.0, q.1, p.1) < 0.0
        };
        let mut g = SegGrid::new(&segs, 1e-9, false);
        let mut out = Vec::new();
        for (i, s) in segs.iter().enumerate().take(300) {
            g.near(s.0, s.1, &mut out);
            for (j, t) in segs.iter().enumerate() {
                if i != j && crosses(*s, *t) {
                    assert!(out.contains(&(j as u32)), "{i} crosses {j}");
                }
            }
        }
    }
}
