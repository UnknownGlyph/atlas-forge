//! Local views ("局部视图"): a named region of the map and a stretch of
//! years kept in the project — a war, a migration, a coast that changes —
//! to look at on its own, without layers and without changing the map.
//! Entering one shows that place in those years; what changed there is
//! worked out here: the years something changes, the land that changed
//! hands since the first year, and who gained and lost it.

use crate::doc::{Document, EntityId};
use crate::geom::{self, LonLat};
use crate::topo::{FaceId, FaceKind, TResult, BACKGROUND};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub id: u32,
    pub name: String,
    /// West, south, east, north (degrees). East − west is the width, so a
    /// region across the antimeridian has its east beyond 180.
    pub bounds: [f64; 4],
    /// The first and last year it is about; `None`: the place in any year.
    #[serde(default)]
    pub years: Option<(i32, i32)>,
    #[serde(default)]
    pub note: String,
}

impl Focus {
    /// Whether a point lies in the region.
    pub fn contains(&self, p: LonLat) -> bool {
        let [w, s, e, n] = self.bounds;
        p.lat >= s && p.lat <= n && (p.lon - w).rem_euclid(360.0) <= e - w
    }

    pub fn center(&self) -> LonLat {
        let [w, s, e, n] = self.bounds;
        LonLat::new(geom::wrap_lon((w + e) * 0.5), (s + n) * 0.5)
    }

    /// Its edge, closed, a point at least every `step` degrees (east along
    /// the south edge, north, west along the north edge, south).
    pub fn outline(&self, step: f64) -> Vec<LonLat> {
        let [w, s, e, n] = self.bounds;
        let along = |a: f64, b: f64| ((b - a).abs() / step.max(0.01)).ceil().max(1.0) as usize;
        let mut out = Vec::new();
        let (nx, ny) = (along(w, e), along(s, n));
        out.extend((0..nx).map(|i| LonLat::new(geom::wrap_lon(w + (e - w) * i as f64 / nx as f64), s)));
        out.extend((0..ny).map(|i| LonLat::new(geom::wrap_lon(e), s + (n - s) * i as f64 / ny as f64)));
        out.extend((0..nx).map(|i| LonLat::new(geom::wrap_lon(e - (e - w) * i as f64 / nx as f64), n)));
        out.extend((0..ny).map(|i| LonLat::new(geom::wrap_lon(w), n - (n - s) * i as f64 / ny as f64)));
        out.push(out[0]);
        out
    }

    /// The region as it should be stored: east beyond west by at most a
    /// full turn, west in [-180, 180), south below north, both in range.
    fn checked(bounds: [f64; 4]) -> TResult<[f64; 4]> {
        let [w, s, e, n] = bounds;
        if !bounds.iter().all(|v| v.is_finite()) {
            return Err("局部视图的范围无效".into());
        }
        let (s, n) = (s.clamp(-90.0, 90.0), n.clamp(-90.0, 90.0));
        let width = e - w;
        if !(width > 0.0 && width <= 360.0 && n > s) {
            return Err("局部视图的范围无效".into());
        }
        let w2 = geom::wrap_lon(w);
        Ok([w2, s, w2 + width, n])
    }
}

/// What changed in a local view between its first and last year.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusReport {
    pub from: i32,
    pub to: i32,
    /// Years after the first, up to the last, in which something in the
    /// region changes.
    pub years: Vec<i32>,
    /// Land in the region (km²) by owner (`None`: no one) in the first and
    /// the last year, the largest change first; owners whose land did not
    /// change are left out.
    pub areas: Vec<(Option<EntityId>, f64, f64)>,
    /// Land that changed hands between the first and the last year: from
    /// whom, to whom, km²; the largest first.
    pub transfers: Vec<(Option<EntityId>, Option<EntityId>, f64)>,
    /// Land that became water, and water that became land (km²).
    pub to_water: f64,
    pub to_land: f64,
}

impl Document {
    pub fn focus(&self, id: u32) -> Option<&Focus> {
        self.focuses.iter().find(|f| f.id == id)
    }

    /// Keep a new local view; an empty name gets "局部 n". Returns its id.
    pub fn add_focus(&mut self, name: &str, bounds: [f64; 4], years: Option<(i32, i32)>) -> TResult<u32> {
        let bounds = Focus::checked(bounds)?;
        let years = years.map(|(a, b)| (a.min(b), a.max(b)));
        let id = self.focuses.iter().map(|f| f.id).max().map_or(1, |m| m + 1);
        let name = match name.trim() {
            "" => {
                let taken: BTreeSet<&str> = self.focuses.iter().map(|f| f.name.as_str()).collect();
                (1..).map(|i| format!("局部 {i}")).find(|n| !taken.contains(n.as_str())).unwrap()
            }
            n => n.to_string(),
        };
        self.focuses.push(Focus { id, name, bounds, years, note: String::new() });
        Ok(id)
    }

    /// Change a local view (found by its id): name, region, years, note.
    pub fn update_focus(&mut self, f: Focus) -> TResult<()> {
        let bounds = Focus::checked(f.bounds)?;
        let name = f.name.trim().to_string();
        if name.is_empty() {
            return Err("局部视图需要名称".into());
        }
        let years = f.years.map(|(a, b)| (a.min(b), a.max(b)));
        let x = self.focuses.iter_mut().find(|x| x.id == f.id).ok_or("没有这个局部视图")?;
        *x = Focus { name, bounds, years, ..f };
        Ok(())
    }

    pub fn remove_focus(&mut self, id: u32) -> TResult<()> {
        let n = self.focuses.len();
        self.focuses.retain(|f| f.id != id);
        if self.focuses.len() == n {
            return Err("没有这个局部视图".into());
        }
        // events that happened there keep their years, without the region
        for e in &mut self.events {
            if e.focus == Some(id) {
                e.focus = None;
            }
        }
        Ok(())
    }

    /// The faces of the region: those whose middle (a point inside them)
    /// lies in it. Sorted.
    pub fn faces_in(&self, f: &Focus) -> Vec<FaceId> {
        let mut out: Vec<FaceId> = self
            .topo
            .faces
            .keys()
            .copied()
            .filter(|id| *id != BACKGROUND)
            .filter(|id| {
                let (outer, holes) = self.topo.face_planar(*id);
                geom::interior_point(&outer, &holes).is_some_and(|(p, _)| f.contains(LonLat::new(p[0], p[1])))
            })
            .collect();
        out.sort();
        out
    }

    /// Who a face shows as its owner in a year: no one on water, and no
    /// one that does not exist then.
    pub fn shown_owner(&self, f: FaceId, year: i32) -> Option<EntityId> {
        if self.face_kind_at(f, Some(year)) != Some(FaceKind::Land) {
            return None;
        }
        self.owner_at(f, Some(year)).filter(|e| self.exists_in(*e, Some(year)))
    }

    /// Of `faces`, those that are not as they were in year `a` in year `b`
    /// (land or water, or who holds them).
    pub fn changed_between(&self, faces: &[FaceId], a: i32, b: i32) -> Vec<FaceId> {
        faces
            .iter()
            .copied()
            .filter(|f| self.face_kind_at(*f, Some(a)) != self.face_kind_at(*f, Some(b)) || self.shown_owner(*f, a) != self.shown_owner(*f, b))
            .collect()
    }

    /// Years after `from`, up to `to`, in which something about `faces`
    /// changes: who holds them, land or water, or a holder that begins or
    /// ends.
    pub fn change_years_of(&self, faces: &[FaceId], from: i32, to: i32) -> Vec<i32> {
        // (a range that ends before it begins would panic in `BTreeMap::range`)
        if to <= from {
            return vec![];
        }
        let mut years: BTreeSet<i32> = BTreeSet::new();
        let mut holders: BTreeSet<EntityId> = BTreeSet::new();
        for f in faces {
            if let Some(ch) = self.owner_changes.get(f) {
                years.extend(ch.range(from + 1..=to).map(|(y, _)| *y));
                holders.extend(ch.values().flatten().copied());
            }
            holders.extend(self.owner_base.get(f).copied());
            if let Some(ch) = self.kind_changes.get(f) {
                years.extend(ch.range(from + 1..=to).map(|(y, _)| *y));
            }
        }
        for e in holders {
            if let Some(x) = self.entities.get(&e) {
                years.extend(x.founded.into_iter().chain(x.ended.map(|y| y + 1)).filter(|y| *y > from && *y <= to));
            }
        }
        years.into_iter().collect()
    }

    /// What changed in a local view over its years; `None` for one without years.
    pub fn focus_report(&self, focus: &Focus) -> Option<FocusReport> {
        let (from, to) = focus.years?;
        Some(self.change_report(&self.faces_in(focus), from, to))
    }

    /// What changed about `faces` from year `from` to year `to`.
    pub fn change_report(&self, faces: &[FaceId], from: i32, to: i32) -> FocusReport {
        let r = self.radius();
        let mut areas: HashMap<Option<EntityId>, (f64, f64)> = HashMap::new();
        let mut transfers: HashMap<(Option<EntityId>, Option<EntityId>), f64> = HashMap::new();
        let mut rep = FocusReport { from, to, years: self.change_years_of(faces, from, to), ..Default::default() };
        for f in faces {
            let km2 = self.topo.face_area(*f, r);
            let (la, lb) = (self.face_kind_at(*f, Some(from)) == Some(FaceKind::Land), self.face_kind_at(*f, Some(to)) == Some(FaceKind::Land));
            let (oa, ob) = (self.shown_owner(*f, from), self.shown_owner(*f, to));
            if la {
                areas.entry(oa).or_default().0 += km2;
            }
            if lb {
                areas.entry(ob).or_default().1 += km2;
            }
            match (la, lb) {
                (true, false) => rep.to_water += km2,
                (false, true) => rep.to_land += km2,
                (true, true) if oa != ob => *transfers.entry((oa, ob)).or_default() += km2,
                _ => {}
            }
        }
        rep.areas = areas.into_iter().map(|(e, (a, b))| (e, a, b)).filter(|(_, a, b)| (a - b).abs() > 1e-6).collect();
        rep.areas.sort_by(|p, q| (q.2 - q.1).abs().total_cmp(&(p.2 - p.1).abs()).then(p.0.cmp(&q.0)));
        rep.transfers = transfers.into_iter().map(|((a, b), km2)| (a, b, km2)).collect();
        rep.transfers.sort_by(|p, q| q.2.total_cmp(&p.2).then((p.0, p.1).cmp(&(q.0, q.1))));
        rep
    }
}
