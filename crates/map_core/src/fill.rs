//! The fill bucket: a click gives a whole patch of land to a country — the
//! land connected to the clicked face across shared borders that shows
//! the same owner (or no one), as the map shows it in the selected year
//! and partition layer. One step to undo.

use crate::doc::{Document, EntityId};
use crate::topo::{FaceId, FaceKind, TResult, BACKGROUND};
use std::collections::{BTreeSet, HashMap, VecDeque};

impl Document {
    /// The land connected to `f` that shows the same owner as it (no one
    /// included), sorted; empty when `f` is not land.
    pub fn same_owner_patch(&self, f: FaceId) -> Vec<FaceId> {
        let land = |x: &FaceId| *x != BACKGROUND && self.topo.faces.get(x).is_some_and(|face| face.kind == FaceKind::Land);
        if !land(&f) {
            return vec![];
        }
        let who = self.owner.get(&f).copied();
        let mut next: HashMap<FaceId, Vec<FaceId>> = HashMap::new();
        for (_, e) in self.topo.edges.iter() {
            if e.left != e.right {
                next.entry(e.left).or_default().push(e.right);
                next.entry(e.right).or_default().push(e.left);
            }
        }
        let mut seen: BTreeSet<FaceId> = BTreeSet::from([f]);
        let mut todo = VecDeque::from([f]);
        while let Some(x) = todo.pop_front() {
            for y in next.get(&x).into_iter().flatten() {
                if !seen.contains(y) && land(y) && self.owner.get(y).copied() == who {
                    seen.insert(*y);
                    todo.push_back(*y);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Give the patch around `f` to `who` (from the selected year on, like
    /// the territory brush); returns how many faces changed hands.
    pub fn fill(&mut self, f: FaceId, who: Option<EntityId>) -> TResult<usize> {
        let patch = self.same_owner_patch(f);
        if patch.is_empty() {
            return Err("只能填充陆地".into());
        }
        if self.owner.get(&f).copied() == who {
            return Ok(0);
        }
        for x in &patch {
            if self.active_is_shares() {
                self.set_share(*x, who, crate::doc::FULL_SHARE)?;
            } else {
                self.assign(*x, who)?;
            }
        }
        Ok(patch.len())
    }
}
