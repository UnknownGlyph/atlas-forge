//! When entities exist. A country (or province, culture...) may be founded
//! in a year and end in a year (both inclusive: "1871–1918" exists in
//! 1918); a blank end is open, so maps without years and old projects work
//! as before. With a year selected, an entity that does not exist then
//! owns no land and has no members: this is applied when the year's view
//! is worked out, never by changing the ownership records. Land whose
//! recorded owner does not exist (land that rose from the sea again after
//! its owner ended, an end set too early...) is shown as unassigned and
//! listed for the author to confirm; the fixes are explicit edits.

use crate::doc::{Document, Entity, EntityId};
use crate::symbol::{in_years, years_text};
use crate::topo::{FaceId, FaceKind, TResult, BACKGROUND};

/// Where an entity stands in a year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Exists,
    /// Before its founding year.
    NotYet,
    /// After its last year.
    Ended,
}

impl Entity {
    /// Whether it exists in `year` (`None`: no year selected, it does).
    pub fn exists_in(&self, year: Option<i32>) -> bool {
        in_years(self.founded, self.ended, year)
    }

    pub fn presence(&self, year: Option<i32>) -> Presence {
        match year {
            Some(y) if self.founded.is_some_and(|f| y < f) => Presence::NotYet,
            Some(y) if self.ended.is_some_and(|e| y > e) => Presence::Ended,
            _ => Presence::Exists,
        }
    }

    /// Its lifespan as text ("1871–1918 年", "1871 年起"...), empty when open.
    pub fn lifespan_text(&self) -> String {
        years_text(self.founded, self.ended)
    }
}

/// Land in a year recorded to an entity that does not exist then, or
/// members whose recorded parent does not exist then.
#[derive(Clone, Debug, PartialEq)]
pub struct LifespanConflict {
    pub entity: EntityId,
    pub presence: Presence,
    pub faces: Vec<FaceId>,
    pub members: Vec<EntityId>,
}

impl Document {
    /// Whether entity `e` exists in `year`.
    pub fn exists_in(&self, e: EntityId, year: Option<i32>) -> bool {
        self.entities.get(&e).is_some_and(|x| x.exists_in(year))
    }

    /// The entities that name `e` as a successor.
    pub fn predecessors(&self, e: EntityId) -> Vec<EntityId> {
        self.entities.iter().filter(|(_, x)| x.successors.contains(&e)).map(|(id, _)| *id).collect()
    }

    /// The owner a face has in `year` once lifespans count: none for water,
    /// none when the recorded owner does not exist then.
    pub fn effective_owner_at(&self, f: FaceId, year: Option<i32>) -> Option<EntityId> {
        if self.face_kind_at(f, year) != Some(FaceKind::Land) {
            return None;
        }
        self.owner_at(f, year).filter(|e| self.exists_in(*e, year))
    }

    /// Everything in `year` recorded to an entity that does not exist
    /// then: land (only land: water owns nothing anyway) and members.
    pub fn lifespan_conflicts(&self, year: i32) -> Vec<LifespanConflict> {
        let y = Some(year);
        let mut out: Vec<LifespanConflict> = Vec::new();
        let entry = |out: &mut Vec<LifespanConflict>, e: EntityId| -> usize {
            if let Some(i) = out.iter().position(|c| c.entity == e) {
                return i;
            }
            let presence = self.entities.get(&e).map_or(Presence::Ended, |x| x.presence(y));
            out.push(LifespanConflict { entity: e, presence, faces: vec![], members: vec![] });
            out.len() - 1
        };
        for f in self.topo.faces.keys().filter(|f| **f != BACKGROUND) {
            if self.face_kind_at(*f, y) != Some(FaceKind::Land) {
                continue;
            }
            if let Some(e) = self.owner_at(*f, y).filter(|e| !self.exists_in(*e, y)) {
                let i = entry(&mut out, e);
                out[i].faces.push(*f);
            }
        }
        for (id, _) in self.entities.iter().filter(|(_, x)| x.exists_in(y)) {
            if let Some(p) = self.parent_at(*id, y).filter(|p| !self.exists_in(*p, y)) {
                let i = entry(&mut out, p);
                out[i].members.push(*id);
            }
        }
        out.sort_by_key(|c| c.entity);
        out
    }

    /// Land still recorded to `e` in the year after it ended: from that
    /// year on, give it to `to` (a successor) or leave it unassigned
    /// (`None`). Earlier years are untouched; later recorded changes stay.
    /// Returns how many pieces of land changed.
    pub fn reassign_after_end(&mut self, e: EntityId, to: Option<EntityId>) -> TResult<usize> {
        if self.active_is_shares() {
            return Err("比例层的归属请逐块修改".into());
        }
        let ended = self.entities.get(&e).ok_or("实体不存在")?.ended.ok_or("这个实体没有终止年份")?;
        let y = ended + 1;
        if let Some(t) = to {
            if !self.exists_in(t, Some(y)) {
                let name = self.entities.get(&t).map_or(String::new(), |x| x.name.clone());
                return Err(format!("{name} 在 {y} 年不存续，不能接收领土"));
            }
        }
        let faces: Vec<FaceId> = self.topo.faces.keys().copied().filter(|f| self.owner_at(*f, Some(y)) == Some(e)).collect();
        for f in &faces {
            let base = self.owner_base.get(f).copied();
            let mut ch = self.owner_changes.get(f).cloned().unwrap_or_default();
            crate::doc::insert_event(&base, &mut ch, y, to);
            self.set_history(*f, base, ch);
        }
        self.refresh_time_view();
        Ok(faces.len())
    }
}

impl Document {
    /// Set an entity's successors (itself and unknown ids are left out).
    pub fn successors_set(&mut self, e: EntityId, list: Vec<EntityId>) {
        let keep: Vec<EntityId> = list.into_iter().filter(|s| *s != e && self.entities.contains_key(s)).collect();
        if let Some(x) = self.entities.get_mut(&e) {
            x.successors = keep;
        }
    }

    /// Set when an entity exists (both inclusive, `None` open); refuses an
    /// end before the founding.
    pub fn set_lifespan(&mut self, e: EntityId, founded: Option<i32>, ended: Option<i32>) -> TResult<()> {
        if let (Some(f), Some(t)) = (founded, ended) {
            if t < f {
                return Err("终止年份早于成立年份".into());
            }
        }
        let x = self.entities.get_mut(&e).ok_or("实体不存在")?;
        x.founded = founded;
        x.ended = ended;
        self.refresh_time_view();
        Ok(())
    }
}
