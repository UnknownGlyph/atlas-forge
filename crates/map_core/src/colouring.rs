//! Automatic colours ("自动配色"): countries that border each other — in
//! any year of the map's history — get different colours from a palette,
//! the busiest countries first, each taking the colour its neighbours use
//! least (so the colours stay balanced). Only countries at the top of
//! their hierarchy are coloured, as the political map colours by them.

use crate::doc::{Document, EntityId, EntityKind};
use std::collections::{BTreeMap, BTreeSet};

/// The default palette (the colours new countries get).
pub fn palette() -> Vec<[u8; 4]> {
    crate::doc::PALETTE.to_vec()
}

impl Document {
    /// Which top-level countries border which, over every year with a
    /// change and the latest state.
    pub fn country_neighbours(&self) -> BTreeMap<EntityId, BTreeSet<EntityId>> {
        let mut years: Vec<Option<i32>> = vec![None];
        years.extend(self.change_years().into_iter().map(Some));
        let mut out: BTreeMap<EntityId, BTreeSet<EntityId>> = BTreeMap::new();
        for c in self.countries() {
            if self.entities.get(&c).is_some_and(|e| e.parent.is_none()) {
                out.entry(c).or_default();
            }
        }
        let top = |e: EntityId| self.top_ancestor(e);
        for y in years {
            for (_, e) in self.topo.edges.iter() {
                if e.left == e.right {
                    continue;
                }
                let shown = |f| self.owner_at(f, y).filter(|x| self.exists_in(*x, y)).filter(|_| self.face_kind_at(f, y) == Some(crate::topo::FaceKind::Land));
                let (Some(a), Some(b)) = (shown(e.left), shown(e.right)) else { continue };
                let (a, b) = (top(a), top(b));
                let kind = |x: EntityId| self.entities.get(&x).map(|e| e.kind);
                if a != b && kind(a) == Some(EntityKind::Country) && kind(b) == Some(EntityKind::Country) {
                    out.entry(a).or_default().insert(b);
                    out.entry(b).or_default().insert(a);
                }
            }
        }
        out
    }

    /// A colour for every top-level country, neighbours different.
    pub fn auto_colours(&self, palette: &[[u8; 4]]) -> BTreeMap<EntityId, [u8; 4]> {
        let next = self.country_neighbours();
        // the busiest first (ties by id, so the result is always the same)
        let mut order: Vec<EntityId> = next.keys().copied().collect();
        order.sort_by_key(|e| (std::cmp::Reverse(next[e].len()), *e));
        let mut used = vec![0usize; palette.len()];
        let mut got: BTreeMap<EntityId, usize> = BTreeMap::new();
        for e in order {
            let taken: BTreeSet<usize> = next[&e].iter().filter_map(|n| got.get(n)).copied().collect();
            // a free colour used least so far; failing that, the one fewest neighbours use
            let pick = (0..palette.len()).filter(|i| !taken.contains(i)).min_by_key(|i| (used[*i], *i)).unwrap_or_else(|| {
                (0..palette.len()).min_by_key(|i| (next[&e].iter().filter(|n| got.get(n) == Some(i)).count(), *i)).unwrap_or(0)
            });
            used[pick] += 1;
            got.insert(e, pick);
        }
        got.into_iter().map(|(e, i)| (e, palette[i])).collect()
    }

    /// Give every top-level country its automatic colour; returns how many changed.
    pub fn apply_auto_colours(&mut self, palette: &[[u8; 4]]) -> usize {
        let mut n = 0;
        for (e, c) in self.auto_colours(palette) {
            if let Some(x) = self.entities.get_mut(&e) {
                if x.color != c {
                    x.color = c;
                    n += 1;
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::grid_world;

    #[test]
    fn neighbours_differ_in_every_year() {
        let mut d = grid_world(8, 5, 4, 9);
        // a border change in 1900: new neighbours then
        let cs = d.countries();
        let f = d.faces_of(cs[0])[0];
        d.set_year(Some(1900));
        d.record_owner(f, Some(cs[20])).unwrap();
        d.set_year(None);
        let next = d.country_neighbours();
        assert!(next[&cs[20]].contains(&cs[0]) || next[&cs[20]].len() > 4, "the 1900 border counts");
        let pal = palette();
        let col = d.auto_colours(&pal);
        for (a, ns) in &next {
            for b in ns {
                assert_ne!(col[a], col[b], "{a:?} and {b:?} border each other");
            }
        }
        // balanced: no colour more than twice as common as the rarest used
        let mut count: BTreeMap<[u8; 4], usize> = BTreeMap::new();
        for c in col.values() {
            *count.entry(*c).or_default() += 1;
        }
        let (lo, hi) = (count.values().min().unwrap(), count.values().max().unwrap());
        assert!(*hi <= lo * 2 + 1, "{count:?}");
        // the same every time; applied
        assert_eq!(col, d.auto_colours(&pal));
        d.apply_auto_colours(&pal);
        assert!(col.iter().all(|(e, c)| d.entities[e].color == *c));
        assert_eq!(d.apply_auto_colours(&pal), 0);
    }
}
