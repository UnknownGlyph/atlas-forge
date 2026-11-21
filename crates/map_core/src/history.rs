//! Snapshot-based undo/redo. `Document` uses persistent maps, so a snapshot
//! is an O(1) clone that shares structure with the live document.
//!
//! What a snapshot really costs is the data only *it* still references
//! (old versions of changed objects plus copied tree paths). That is
//! estimated per step with [`ChangeSet`], and the oldest steps are dropped
//! once the total exceeds a memory budget.

use crate::change::ChangeSet;
use crate::doc::Document;
use crate::world_book::WorldBook;
use std::collections::VecDeque;

struct Entry {
    label: String,
    doc: Document,
    bytes: usize,
}

pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    /// Maximum number of undo steps.
    pub limit: usize,
    /// Memory budget for undo snapshots, in bytes (estimate).
    pub budget_bytes: usize,
    /// Steps always kept, even over budget.
    pub min_keep: usize,
    undo_bytes: usize,
    generation: u64,
}

impl Default for History {
    fn default() -> Self {
        Self { undo: VecDeque::new(), redo: vec![], limit: 500, budget_bytes: 512 << 20, min_keep: 20, undo_bytes: 0, generation: 0 }
    }
}

impl History {
    /// Run an edit; on success it becomes undoable, on error the document is
    /// restored exactly.
    pub fn apply<T>(
        &mut self,
        doc: &mut Document,
        label: &str,
        f: impl FnOnce(&mut Document) -> Result<T, String>,
    ) -> Result<T, String> {
        let before = doc.clone();
        match f(doc) {
            Ok(v) => {
                self.push(label, before, doc);
                Ok(v)
            }
            Err(e) => {
                *doc = before;
                Err(e)
            }
        }
    }

    /// Record `before` as an undo step leading to `after` (e.g. at the end of
    /// an interactive drag).
    pub fn push(&mut self, label: &str, before: Document, after: &Document) {
        self.generation += 1;
        let bytes = ChangeSet::between(&before, after).old_bytes + WorldBook::retained_bytes(&before, after);
        self.undo.push_back(Entry { label: label.to_string(), doc: before, bytes });
        self.undo_bytes += bytes;
        self.redo.clear();
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() > self.limit || (self.undo_bytes > self.budget_bytes && self.undo.len() > self.min_keep) {
            let Some(e) = self.undo.pop_front() else { break };
            self.undo_bytes -= e.bytes;
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.undo.pop_back()?;
        self.undo_bytes -= e.bytes;
        self.generation += 1;
        let cur = std::mem::replace(doc, e.doc);
        self.redo.push(Entry { label: e.label.clone(), doc: cur, bytes: e.bytes });
        Some(e.label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.redo.pop()?;
        self.generation += 1;
        let cur = std::mem::replace(doc, e.doc);
        self.undo_bytes += e.bytes;
        self.undo.push_back(Entry { label: e.label.clone(), doc: cur, bytes: e.bytes });
        self.enforce_limits();
        Some(e.label)
    }

    /// Changes whenever the history changes (push, undo, redo, clear).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
    /// Estimated memory held by undo snapshots.
    pub fn memory_bytes(&self) -> usize {
        self.undo_bytes
    }
    pub fn clear(&mut self) {
        self.generation += 1;
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
    }
    /// The steps that can be undone, oldest first.
    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo.iter().map(|e| e.label.as_str()).collect()
    }
    /// The steps that can be redone, the next one first.
    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo.iter().rev().map(|e| e.label.as_str()).collect()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::LonLat;
    use crate::topo::FaceKind;

    #[test]
    fn undo_redo_and_rollback() {
        let mut d = Document::default();
        let mut h = History::default();
        let sq = [LonLat::new(0.0, 0.0), LonLat::new(4.0, 0.0), LonLat::new(4.0, 4.0), LonLat::new(0.0, 4.0)];
        h.apply(&mut d, "add", |d| d.add_ring_face(&sq, FaceKind::Land)).unwrap();
        let after_add = d.clone();
        // failing edit leaves the document untouched and is not recorded
        assert!(h.apply(&mut d, "bad", |d| d.add_ring_face(&sq, FaceKind::Land)).is_err());
        assert_eq!(d, after_add);
        h.undo(&mut d);
        assert_eq!(d.topo.faces.len(), 1);
        h.redo(&mut d);
        assert_eq!(d, after_add);
    }

    #[test]
    fn memory_budget_drops_oldest_steps() {
        // one long coastline edited repeatedly: each step retains a copy of it
        let mut d = Document::default();
        let ring: Vec<LonLat> = (0..20_000)
            .map(|i| {
                let t = i as f64 / 20_000.0 * std::f64::consts::TAU;
                LonLat::new(30.0 * t.cos(), 30.0 * t.sin())
            })
            .collect();
        let mut h = History { budget_bytes: 2 << 20, min_keep: 3, ..Default::default() };
        h.apply(&mut d, "land", |d| d.add_ring_face(&ring, FaceKind::Land)).unwrap();
        for k in 0..30 {
            let e = *d.topo.edges.keys().next().unwrap();
            h.apply(&mut d, "move", |d| {
                let p = d.topo.edges[&e].pts[k];
                d.topo.move_vertex(crate::VertexRef::Pt { edge: e, idx: k }, LonLat::new(p.lon * 1.0001, p.lat))
            })
            .unwrap();
        }
        // ~320 KB retained per step: a 2 MB budget keeps only a handful
        assert!(h.len() >= 3 && h.len() < 10, "{} steps kept, {} bytes", h.len(), h.memory_bytes());
        assert!(h.memory_bytes() <= 2 << 20);
        while h.undo(&mut d).is_some() {}
        d.topo.validate().unwrap();
    }
}
