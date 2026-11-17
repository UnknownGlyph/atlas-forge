//! Drawing rivers and routes onto what is already there: a new line snaps
//! to the ends of lines of its own kind (to continue them), to cities
//! (routes), and onto lines of its own kind (a tributary joining its
//! river, a road branching off another). A line started or ended exactly
//! on another's end continues that line instead of making a new one.

use crate::doc::{Document, EntityKind, LineId};
use crate::geom::{self, LonLat};
use crate::topo::TResult;

/// What a point snapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSnap {
    /// The first or last point of a line (`true`: the last).
    End(LineId, bool),
    City,
    /// A point on a line.
    On(LineId),
}

/// Two points closer than this (degrees) are the same point.
const SAME: f64 = 1e-7;

impl Document {
    /// Where a point drawn at `p` for a river (`route` false) or a route
    /// snaps within `tol` degrees: a line end first, then (routes) a city,
    /// then the nearest point on a line of the same kind.
    pub fn line_snap(&self, p: LonLat, tol: f64, route: bool) -> Option<(LonLat, LineSnap)> {
        let same_kind = |r: &&crate::doc::LineFeature| r.route.is_some() == route;
        let mut best: Option<(f64, LonLat, LineSnap)> = None;
        for (id, r) in self.rivers.iter().filter(|(id, r)| same_kind(r) && !r.hidden && !self.group_hides(**id)) {
            for (last, q) in [(false, r.pts.first()), (true, r.pts.last())] {
                let Some(q) = q else { continue };
                let d = geom::geo_dist_deg(p, *q);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, *q, LineSnap::End(*id, last)));
                }
            }
        }
        if best.is_some() {
            return best.map(|b| (b.1, b.2));
        }
        if route {
            for e in self.entities.values().filter(|e| e.kind == EntityKind::City) {
                let Some(q) = e.pos else { continue };
                let d = geom::geo_dist_deg(p, q);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q, LineSnap::City));
                }
            }
            if best.is_some() {
                return best.map(|b| (b.1, b.2));
            }
        }
        for (id, r) in self.rivers.iter().filter(|(id, r)| same_kind(r) && !r.hidden && !self.group_hides(**id)) {
            for w in r.pts.windows(2) {
                let (q, d, _) = geom::closest_on_segment(p, w[0], w[1]);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q, LineSnap::On(*id)));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// The line of the given kind that `p` is exactly an end of, and which
    /// end (`true`: the last point).
    pub fn line_end_exact(&self, p: LonLat, route: bool) -> Option<(LineId, bool)> {
        self.rivers.iter().filter(|(_, r)| r.route.is_some() == route).find_map(|(id, r)| {
            if r.pts.last().is_some_and(|q| geom::geo_dist_deg(p, *q) < SAME) {
                Some((*id, true))
            } else if r.pts.first().is_some_and(|q| geom::geo_dist_deg(p, *q) < SAME) {
                Some((*id, false))
            } else {
                None
            }
        })
    }

    /// Continue a line from one of its ends with `pts`, which start at
    /// that end (`last`: its last point, else its first).
    pub fn extend_line(&mut self, id: LineId, last: bool, pts: &[LonLat]) -> TResult<()> {
        let r = self.rivers.get_mut(&id).ok_or("线不存在")?;
        if pts.len() < 2 {
            return Err("延长至少需要一个新点".into());
        }
        if last {
            r.pts.extend_from_slice(&pts[1..]);
        } else {
            let mut v: Vec<LonLat> = pts.iter().rev().copied().collect();
            v.extend_from_slice(&r.pts[1..]);
            r.pts = v;
        }
        self.points_changed(id);
        Ok(())
    }
}
