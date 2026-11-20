//! One timeline schedule is shared by interactive playback and animation export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    RecordedChanges,
    Every(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub from: i32,
    pub to: i32,
    pub sampling: Sampling,
    pub duration: f64,
    /// Calendar pacing gives intervals with more elapsed years more time.
    pub calendar_pacing: bool,
    pub speed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub year: i32,
    pub hold_ms: u32,
}

impl Playback {
    pub fn schedule(self, changes: &[i32]) -> Result<Vec<Sample>, String> {
        if !(-999_999..=999_999).contains(&self.from)
            || !(-999_999..=999_999).contains(&self.to)
            || !(0.1..=3600.0).contains(&self.duration)
            || !(0.1..=32.0).contains(&self.speed)
        {
            return Err("时间范围、总时长或播放速度无效。".into());
        }
        let low = self.from.min(self.to);
        let high = self.from.max(self.to);
        let mut list = match self.sampling {
            Sampling::RecordedChanges => {
                let mut values = vec![low];
                values.extend(changes.iter().copied().filter(|y| (low..=high).contains(y)));
                values
            }
            Sampling::Every(step) => {
                if step == 0 {
                    return Err("采样间隔必须大于零。".into());
                }
                let count = (u32::try_from(high - low).unwrap() / step) + 1;
                if count >= 10_000 {
                    return Err("年份过多，请增大采样间隔或缩短范围。".into());
                }
                let direction = if self.from <= self.to { 1 } else { -1 };
                (0..count)
                    .map(|i| self.from + direction * (i * step) as i32)
                    .collect()
            }
        };
        list.extend([low, high]);
        list.sort_unstable();
        list.dedup();
        if self.from > self.to {
            list.reverse();
        }
        if list.len() > 10_000 {
            return Err("年份过多，请缩短播放范围。".into());
        }
        let duration_ms = (self.duration * 1000.0 / self.speed).round();
        // The export rasterizer samples at 25 fps. Reserve one sample per
        // distinct year, then distribute remaining time by cumulative weight.
        let ticks = (duration_ms / 40.0).round().max(1.0) as u32;
        let count = list.len() as u32;
        if ticks < count {
            return Err(
                "总时长太短，每个采样年份至少需要 40 毫秒。请延长时间、降低速度或减少采样。".into(),
            );
        }
        let mut weights: Vec<_> = list
            .windows(2)
            .map(|p| {
                if self.calendar_pacing {
                    p[0].abs_diff(p[1]).max(1)
                } else {
                    1
                }
            })
            .collect();
        weights.push(1);
        let sum: u64 = weights.iter().map(|&w| u64::from(w)).sum();
        let mut cumulative = 0u64;
        let mut previous = 0u64;
        Ok(list
            .into_iter()
            .zip(weights)
            .map(|(year, weight)| {
                cumulative += u64::from(weight) * u64::from(ticks - count);
                let allocated = cumulative / sum;
                let hold_ms = (1 + (allocated - previous) as u32) * 40;
                previous = allocated;
                Sample { year, hold_ms }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_changes_and_equal_steps_keep_endpoints_and_total_duration() {
        let mut p = Playback {
            from: 1900,
            to: 1960,
            sampling: Sampling::RecordedChanges,
            duration: 12.0,
            calendar_pacing: true,
            speed: 2.0,
        };
        let s = p.schedule(&[1910, 1940, 1940, 2000]).unwrap();
        assert_eq!(
            s.iter().map(|p| p.year).collect::<Vec<_>>(),
            [1900, 1910, 1940, 1960]
        );
        assert!(s[1].hold_ms > s[0].hold_ms);
        assert_eq!(s.iter().map(|p| p.hold_ms).sum::<u32>(), 6000);
        p.from = 1960;
        p.to = 1900;
        p.sampling = Sampling::Every(23);
        assert_eq!(
            p.schedule(&[])
                .unwrap()
                .iter()
                .map(|p| p.year)
                .collect::<Vec<_>>(),
            [1960, 1937, 1914, 1900]
        );
        p.duration = 0.1;
        assert!(p.schedule(&[]).is_err());
    }
}

/// Years recorded in an atlas workspace, optionally restricted to a local view.
pub fn study_years(
    doc: &crate::Document,
    focus: Option<&crate::focus::Focus>,
) -> std::collections::BTreeSet<i32> {
    use crate::{
        doc::EntityId,
        topo::FaceId,
        world_book::{Node, WorldBook},
    };
    use std::collections::BTreeSet;
    let local = focus.map(|focus| {
        let faces: BTreeSet<_> = doc.faces_in(focus).into_iter().collect();
        let mut entities = BTreeSet::new();
        for face in &faces {
            let (base, changes) = doc.face_history(*face);
            for owner in base
                .into_iter()
                .chain(changes.into_iter().filter_map(|(_, owner)| owner))
            {
                entities.insert(owner);
                entities.extend(doc.ancestors(owner));
            }
        }
        (focus, faces, entities)
    });
    let mut years = if let Some((_, faces, _)) = &local {
        doc.change_years_of(
            &faces.iter().copied().collect::<Vec<_>>(),
            i32::MIN,
            i32::MAX,
        )
        .into_iter()
        .collect()
    } else {
        doc.change_years()
    };
    for ((id, _), series) in &doc.values {
        if local
            .as_ref()
            .is_none_or(|(_, _, entities)| entities.contains(id))
        {
            years.extend(series.by_year.keys().copied());
        }
    }
    for event in &doc.events {
        if local.as_ref().is_none_or(|(f, _, _)| {
            event.focus == Some(f.id)
                || event
                    .focus
                    .and_then(|id| doc.focus(id))
                    .is_some_and(|e| f.contains(e.center()))
        }) {
            years.insert(event.year);
            years.insert(event.last());
        }
    }
    if let Some(book) = &doc.book {
        if let Some(catalog) = &book.worldview {
            if let Some((focus, faces, entities)) = &local {
                let current = WorldBook::current(doc);
                let mut records = BTreeSet::new();
                for link in book.links.values() {
                    let (id, node) = match (link.from, link.to) {
                        (Node::Record(id), node) | (node, Node::Record(id))
                            if node.board() == Some(current) =>
                        {
                            (id, book.resolve(doc, node))
                        }
                        _ => continue,
                    };
                    match node {
                        Node::Map(_) => {}
                        Node::Face(_, face) if faces.contains(&FaceId(face)) => {}
                        Node::Entity(_, entity) if entities.contains(&EntityId(entity)) => {}
                        node if book.position(doc, node).is_some_and(|p| focus.contains(p)) => {}
                        _ => continue,
                    }
                    records.insert(id);
                    years.extend(link.from_year);
                    years.extend(link.until_year.and_then(|y| y.checked_add(1)));
                }
                catalog.extend_years(&mut years, Some(&records));
            } else {
                catalog.extend_years(&mut years, None);
            }
        }
    }
    if let Some((focus, _, _)) = local {
        if let Some((from, to)) = focus.years {
            years.retain(|year| (from..=to).contains(year));
        }
    }
    years
}
