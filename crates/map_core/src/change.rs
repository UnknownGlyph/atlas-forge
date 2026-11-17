//! Change sets: which objects differ between two document states.
//!
//! Derived by diffing snapshots instead of being recorded by each edit
//! operation, so no operation can forget to report what it touched.
//! `imbl` skips subtrees shared by both maps, so the cost is proportional
//! to the size of the change, not of the document.

use crate::doc::{Document, EntityId, FieldId};
use crate::topo::{EdgeId, FaceId, NodeId};
use imbl::ordmap::DiffItem;
use imbl::OrdMap;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet {
    pub nodes: BTreeSet<NodeId>,
    pub edges: BTreeSet<EdgeId>,
    /// Faces whose record changed (geometry edits always bump `Face::rev`).
    pub faces: BTreeSet<FaceId>,
    /// Faces whose owner changed.
    pub owners: BTreeSet<FaceId>,
    pub entities: BTreeSet<EntityId>,
    pub fields: BTreeSet<FieldId>,
    pub values: BTreeSet<(EntityId, FieldId)>,
    pub rivers: bool,
    /// Style, theme, metadata or current year.
    pub settings: bool,
    /// Rough number of bytes held only by the *old* state (what a snapshot
    /// of it costs to keep alive), see [`ChangeSet::between`].
    pub old_bytes: usize,
}

/// imbl B-tree nodes hold up to 64 entries; a changed key forces the
/// leaf-to-root path to be copied.
const NODE_ENTRIES: usize = 64;
const PATH_DEPTH: usize = 3;

fn diff_map<K: Ord + Copy, V: PartialEq>(
    a: &OrdMap<K, V>,
    b: &OrdMap<K, V>,
    out: &mut BTreeSet<K>,
    bytes: &mut usize,
    value_bytes: impl Fn(&V) -> usize,
) {
    let mut changed = 0usize;
    for d in a.diff(b) {
        match d {
            DiffItem::Add(k, _) => {
                out.insert(*k);
            }
            DiffItem::Update { old: (k, v), .. } | DiffItem::Remove(k, v) => {
                out.insert(*k);
                *bytes += value_bytes(v);
            }
        }
        changed += 1;
    }
    if changed > 0 {
        // copied tree nodes: at most one path per changed key, bounded by the tree size
        let leaves = changed.min(a.len() / NODE_ENTRIES + 1);
        *bytes += leaves * PATH_DEPTH * NODE_ENTRIES * std::mem::size_of::<(K, V)>();
    }
}

impl ChangeSet {
    pub fn between(a: &Document, b: &Document) -> Self {
        let mut c = ChangeSet::default();
        let mut bytes = 0;
        diff_map(&a.topo.nodes, &b.topo.nodes, &mut c.nodes, &mut bytes, |_| 0);
        diff_map(&a.topo.edges, &b.topo.edges, &mut c.edges, &mut bytes, |e| e.pts.capacity() * 16);
        diff_map(&a.topo.faces, &b.topo.faces, &mut c.faces, &mut bytes, |f| {
            (f.outer.capacity() + f.holes.iter().map(|h| h.capacity()).sum::<usize>()) * 16 + f.holes.capacity() * 24
        });
        diff_map(&a.owner, &b.owner, &mut c.owners, &mut bytes, |_| 0);
        diff_map(&a.entities, &b.entities, &mut c.entities, &mut bytes, |e| e.name.len() + e.short.len() + e.code.len() + e.note.len());
        diff_map(&a.fields, &b.fields, &mut c.fields, &mut bytes, |f| f.key.len() + f.name.len() + f.expr.as_ref().map_or(0, |s| s.len()));
        diff_map(&a.values, &b.values, &mut c.values, &mut bytes, |s| s.by_year.len() * 64);
        let mut rivers = BTreeSet::new();
        diff_map(&a.rivers, &b.rivers, &mut rivers, &mut bytes, |r| r.pts.capacity() * 16);
        c.rivers = !rivers.is_empty();
        c.settings = a.style != b.style || a.theme != b.theme || a.meta != b.meta || a.year != b.year;
        c.old_bytes = bytes;
        c
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.edges.is_empty()
            && self.faces.is_empty()
            && self.owners.is_empty()
            && self.entities.is_empty()
            && self.fields.is_empty()
            && self.values.is_empty()
            && !self.rivers
            && !self.settings
    }

    /// Entities whose territory or own record changed: owners (before and
    /// after) of every changed face, plus changed entities themselves.
    pub fn affected_entities(&self, a: &Document, b: &Document) -> BTreeSet<EntityId> {
        let mut out: BTreeSet<EntityId> = self.entities.clone();
        for f in self.faces.iter().chain(self.owners.iter()) {
            out.extend(a.owner.get(f).copied());
            out.extend(b.owner.get(f).copied());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::LonLat;
    use crate::sample::grid_world;

    #[test]
    fn change_sets_are_local_and_complete() {
        let a = grid_world(10, 6, 4, 3);
        assert!(ChangeSet::between(&a, &a.clone()).is_empty());

        // move one shared vertex: exactly the two neighbouring faces change
        let mut b = a.clone();
        let (eid, e) = b.topo.edges.iter().find(|(_, e)| e.pts.len() > 2 && e.left != crate::BACKGROUND && e.right != crate::BACKGROUND).map(|(i, e)| (*i, e.clone())).unwrap();
        let p = e.pts[1];
        b.topo.move_vertex(crate::VertexRef::Pt { edge: eid, idx: 1 }, LonLat::new(p.lon + 0.01, p.lat)).unwrap();
        let c = ChangeSet::between(&a, &b);
        assert_eq!(c.edges, BTreeSet::from([eid]));
        assert_eq!(c.faces, BTreeSet::from([e.left, e.right]));
        let affected = c.affected_entities(&a, &b);
        assert_eq!(affected.len(), 2);
        assert!(c.old_bytes > 0);

        // ownership change only
        let mut d = a.clone();
        let other = d.countries()[0];
        d.assign(e.left, Some(other)).unwrap();
        let c = ChangeSet::between(&a, &d);
        assert!(c.faces.is_empty() && c.owners == BTreeSet::from([e.left]));
        assert!(c.affected_entities(&a, &d).contains(&other));

        // style only
        let mut s = a.clone();
        s.style.border_width = 3.0;
        let c = ChangeSet::between(&a, &s);
        assert!(c.settings && c.faces.is_empty() && c.entities.is_empty());
    }
}
