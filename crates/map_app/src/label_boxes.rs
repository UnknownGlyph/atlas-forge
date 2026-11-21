//! Incremental occupancy for already placed screen-space labels.
use eframe::egui::Rect;
use std::collections::HashMap;

const CELL: f32 = 64.0;
const INDEX_MIN: usize = 128;
const MAX_CELLS: i64 = 48;

type Cell = (i32, i32);

#[derive(Default)]
pub(crate) struct LabelBoxes {
    rects: Vec<Rect>,
    grid: HashMap<Cell, Vec<usize>>,
    wide: Vec<usize>,
}

fn cells(rect: Rect) -> Option<impl Iterator<Item = Cell>> {
    let [x0, y0, x1, y1] =
        [rect.min.x, rect.min.y, rect.max.x, rect.max.y].map(|v| (v / CELL).floor() as i32);
    let w = i64::from(x1) - i64::from(x0) + 1;
    let h = i64::from(y1) - i64::from(y0) + 1;
    if w < 1 || h < 1 || w > MAX_CELLS || h > MAX_CELLS || w * h > MAX_CELLS {
        return None;
    }
    Some((y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y))))
}

impl LabelBoxes {
    fn index(&mut self, id: usize) {
        let rect = self.rects[id];
        if let Some(cells) = cells(rect) {
            for cell in cells {
                self.grid.entry(cell).or_default().push(id);
            }
        } else {
            self.wide.push(id);
        }
    }

    pub fn push(&mut self, rect: Rect) {
        self.rects.push(rect);
        match self.rects.len().cmp(&INDEX_MIN) {
            std::cmp::Ordering::Equal => {
                for id in 0..INDEX_MIN {
                    self.index(id);
                }
            }
            std::cmp::Ordering::Greater => self.index(self.rects.len() - 1),
            std::cmp::Ordering::Less => {}
        }
    }

    pub fn extend(&mut self, rects: impl IntoIterator<Item = Rect>) {
        for rect in rects {
            self.push(rect);
        }
    }

    pub fn intersects(&self, rect: Rect) -> bool {
        if self.rects.len() < INDEX_MIN {
            return self.rects.iter().any(|r| r.intersects(rect));
        }
        let Some(cells) = cells(rect) else {
            return self.rects.iter().any(|r| r.intersects(rect));
        };
        self.wide
            .iter()
            .copied()
            .chain(cells.flat_map(|cell| self.grid.get(&cell).into_iter().flatten().copied()))
            .any(|id| self.rects[id].intersects(rect))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn samples() -> Vec<Rect> {
        (0..64)
            .flat_map(|y| {
                (0..78).map(move |x| {
                    Rect::from_min_size(
                        pos2((x * 26) as f32 - 400.0, (y * 21) as f32 - 200.0),
                        vec2(13.0, 9.0),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn indexed_boxes_match_linear_intersections() {
        let mut rects = samples();
        rects.push(Rect::from_min_size(
            pos2(-9000.0, 1302.0),
            vec2(18000.0, 3.0),
        ));
        let mut index = LabelBoxes::default();
        index.extend(rects.iter().copied());
        let query = rects.iter().flat_map(|rect| {
            [
                *rect,
                rect.translate(vec2(13.0, 9.0)),
                rect.translate(vec2(15.0, 10.0)),
            ]
        });
        for r in query {
            assert_eq!(
                index.intersects(r),
                rects.iter().any(|other| other.intersects(r)),
                "{r:?}"
            );
        }
    }

    #[test]
    #[ignore = "wall-clock comparison, run explicitly in release"]
    fn measure_label_index() {
        use std::hint::black_box;
        use std::time::Instant;

        let rects = samples();
        let query: Vec<_> = rects
            .iter()
            .map(|r| r.translate(vec2(15.0, 10.0)))
            .collect();
        let t = Instant::now();
        let expected: Vec<_> = query
            .iter()
            .map(|&q| black_box(&rects).iter().any(|r| r.intersects(q)))
            .collect();
        let linear = t.elapsed();
        let t = Instant::now();
        let mut index = LabelBoxes::default();
        index.extend(rects.iter().copied());
        let build = t.elapsed();
        let actual: Vec<_> = query
            .iter()
            .map(|&q| black_box(&index).intersects(q))
            .collect();
        let indexed = t.elapsed();
        assert_eq!(actual, expected);
        println!(
            "{} boxes, {} queries. Linear {:?}; indexed {:?}, including {:?} to build.",
            rects.len(),
            query.len(),
            linear,
            indexed,
            build
        );
    }
}
