//! Persistent multi-level grid index over lon/lat bounding boxes.
//!
//! Each item is filed at the finest level whose cells are at least as large
//! as its bbox, so it touches at most 2×2 cells. Columns wrap around the
//! antimeridian. The index uses `imbl` maps, so it is snapshotted together
//! with the topology: every undo state carries a matching index.

use imbl::{OrdMap, OrdSet};

/// Finest level: 360 / 2^12 ≈ 0.088° cells.
const MAX_LEVEL: u8 = 12;
/// Above this many cells, a level is scanned instead of probed cell by cell.
const PROBE_LIMIT: i64 = 1024;

type Cell = (u8, i32, i32);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridIndex {
    cells: OrdMap<Cell, OrdSet<u64>>,
    /// Where each id was filed, so removal never depends on stale bboxes.
    of: OrdMap<u64, Vec<Cell>>,
}

fn cell_size(level: u8) -> f64 {
    360.0 / (1u64 << level) as f64
}

fn dims(level: u8) -> (i64, i64) {
    let cs = cell_size(level);
    ((360.0 / cs).round() as i64, (180.0 / cs).ceil().max(1.0) as i64)
}

/// Column/row ranges covered by `bb` at `level` (columns unwrapped).
fn span(level: u8, bb: &[f64; 4]) -> (i64, i64, i64, i64) {
    let cs = cell_size(level);
    let (_, rows) = dims(level);
    let x0 = ((bb[0] + 180.0) / cs).floor() as i64;
    let x1 = ((bb[2] + 180.0) / cs).floor() as i64;
    let y0 = (((bb[1] + 90.0) / cs).floor() as i64).clamp(0, rows - 1);
    let y1 = (((bb[3] + 90.0) / cs).floor() as i64).clamp(0, rows - 1);
    (x0, x1, y0, y1)
}

fn level_for(bb: &[f64; 4]) -> u8 {
    let size = (bb[2] - bb[0]).max(bb[3] - bb[1]);
    let mut level = 0;
    while level < MAX_LEVEL && cell_size(level + 1) >= size {
        level += 1;
    }
    level
}

impl GridIndex {
    fn cells_for(bb: &[f64; 4]) -> Vec<Cell> {
        if !bb.iter().all(|v| v.is_finite()) {
            return vec![(0, 0, 0)];
        }
        let level = level_for(bb);
        let (cols, _) = dims(level);
        let (x0, x1, y0, y1) = span(level, bb);
        let mut out = Vec::new();
        let xs: Vec<i64> = if x1 - x0 + 1 >= cols { (0..cols).collect() } else { (x0..=x1).map(|x| x.rem_euclid(cols)).collect() };
        for x in xs {
            for y in y0..=y1 {
                out.push((level, x as i32, y as i32));
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn insert(&mut self, id: u64, bb: [f64; 4]) {
        let cells = Self::cells_for(&bb);
        if self.of.get(&id) == Some(&cells) {
            return;
        }
        self.remove(id);
        for c in &cells {
            let mut set = self.cells.get(c).cloned().unwrap_or_default();
            set.insert(id);
            self.cells.insert(*c, set);
        }
        self.of.insert(id, cells);
    }

    pub fn remove(&mut self, id: u64) {
        let Some(cells) = self.of.remove(&id) else { return };
        for c in cells {
            if let Some(mut set) = self.cells.get(&c).cloned() {
                set.remove(&id);
                if set.is_empty() {
                    self.cells.remove(&c);
                } else {
                    self.cells.insert(c, set);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.of.len()
    }
    pub fn is_empty(&self) -> bool {
        self.of.is_empty()
    }

    /// Ids whose cells intersect `bb` (a superset of the true matches;
    /// callers do the exact test). Sorted and deduplicated.
    pub fn query(&self, bb: [f64; 4]) -> Vec<u64> {
        let mut out = Vec::new();
        if self.of.is_empty() {
            return out;
        }
        let all = !bb.iter().all(|v| v.is_finite());
        for level in 0..=MAX_LEVEL {
            let (cols, _) = dims(level);
            let lo = (level, i32::MIN, i32::MIN);
            let hi = (level, i32::MAX, i32::MAX);
            if all {
                for (_, set) in self.cells.range(lo..=hi) {
                    out.extend(set.iter().copied());
                }
                continue;
            }
            let (x0, x1, y0, y1) = span(level, &bb);
            let ncols = (x1 - x0 + 1).min(cols);
            if ncols * (y1 - y0 + 1) > PROBE_LIMIT {
                // large query at a fine level: scan the level and filter
                let wrap_all = x1 - x0 + 1 >= cols;
                for ((_, x, y), set) in self.cells.range(lo..=hi) {
                    let (x, y) = (*x as i64, *y as i64);
                    let in_x = wrap_all || (x0..=x1).any(|c| c.rem_euclid(cols) == x);
                    if in_x && y >= y0 && y <= y1 {
                        out.extend(set.iter().copied());
                    }
                }
                continue;
            }
            let xs: Vec<i64> = if x1 - x0 + 1 >= cols { (0..cols).collect() } else { (x0..=x1).map(|x| x.rem_euclid(cols)).collect() };
            for x in xs {
                let a = (level, x as i32, y0 as i32);
                let b = (level, x as i32, y1 as i32);
                for (_, set) in self.cells.range(a..=b) {
                    out.extend(set.iter().copied());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(items: &[(u64, [f64; 4])], q: [f64; 4]) -> Vec<u64> {
        let hit = |a: &[f64; 4], b: &[f64; 4]| {
            a[1] <= b[3] && b[1] <= a[3] && [-360.0, 0.0, 360.0].iter().any(|k| !(a[0] > b[2] + k || b[0] + k > a[2]))
        };
        let mut v: Vec<u64> = items.iter().filter(|(_, b)| hit(b, &q)).map(|(i, _)| *i).collect();
        v.sort();
        v
    }

    #[test]
    fn query_is_superset_of_exact_matches() {
        let mut rng = crate::sample::Rng::new(5);
        let mut idx = GridIndex::default();
        let mut items = Vec::new();
        for i in 0..3000u64 {
            let x = rng.range(-200.0, 200.0);
            let y = rng.range(-90.0, 85.0);
            let w = if i % 50 == 0 { rng.range(30.0, 300.0) } else { rng.range(0.0, 3.0) };
            let h = rng.range(0.0, 5.0);
            let bb = [x, y, x + w, (y + h).min(90.0)];
            idx.insert(i, bb);
            items.push((i, bb));
        }
        // move / remove some
        for i in (0..3000u64).step_by(7) {
            idx.remove(i);
        }
        items.retain(|(i, _)| i % 7 != 0);
        for _ in 0..300 {
            let x = rng.range(-180.0, 180.0);
            let y = rng.range(-90.0, 80.0);
            let s = if rng.next_f64() < 0.1 { rng.range(50.0, 400.0) } else { rng.range(0.0, 2.0) };
            let q = [x, y, x + s, (y + s).min(90.0)];
            let got = idx.query(q);
            for id in brute(&items, q) {
                assert!(got.binary_search(&id).is_ok(), "missing {id} for {q:?}");
            }
        }
        // removal leaves nothing behind
        for (i, _) in &items {
            idx.remove(*i);
        }
        assert!(idx.is_empty() && idx.cells.is_empty());
    }
}
