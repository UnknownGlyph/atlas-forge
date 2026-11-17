//! Land handed from one unit to another: the part of `from`'s land (in
//! the selected year) inside an area goes to `to` — and nothing else: not
//! the sea, not a lake, not a third country's land inside the area. The
//! pieces are drawn with their holes ("take" from the selected year on), as
//! the automatic partition draws its units.

use crate::doc::{Document, EntityId};
use crate::geom::{self, LonLat, P2};
use crate::overlay::{self, DrawIntent};
use crate::topo::TResult;
use geo::BooleanOps;

fn ring_ls(r: &[P2]) -> geo::LineString<f64> {
    geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>())
}

impl Document {
    /// Hand the part of `from`'s land inside `area` (a ring, lon/lat) to
    /// `to`, from the selected year on (all years when none is selected).
    /// Returns the area handed over (km²); an error when `area` holds none
    /// of `from`'s land.
    pub fn transfer_land(&mut self, from: EntityId, to: EntityId, area: &[LonLat]) -> TResult<f64> {
        if from == to {
            return Err("给出与接收的是同一个单位".into());
        }
        if !self.entities.contains_key(&to) {
            return Err("接收的单位不存在".into());
        }
        if area.len() < 3 {
            return Err("范围至少需要 3 个点".into());
        }
        let pieces = self.dissolve(from);
        if pieces.is_empty() {
            return Err("给出的单位在这一年没有领土".into());
        }
        // the area in the plane, around the longitudes of the land
        let mut a: Vec<P2> = geom::ring_to_planar(&area.iter().map(|p| p.normalized()).collect::<Vec<_>>());
        if geom::signed_area(&a) < 0.0 {
            a.reverse();
        }
        let ab = geom::bbox(&a);
        let before = self.areas_by_entity().get(&to).copied().unwrap_or(0.0);
        let mut cut_any = false;
        for p in &pieces {
            let pb = geom::bbox(&p.outer);
            // the area moved by whole turns to lie over this piece
            let shift = (((pb[0] + pb[2]) - (ab[0] + ab[2])) * 0.5 / 360.0).round() * 360.0;
            let moved: Vec<P2> = a.iter().map(|q| [q[0] + shift, q[1]]).collect();
            let land = geo::Polygon::new(ring_ls(&p.outer), p.holes.iter().map(|h| ring_ls(h)).collect());
            let cut = land.intersection(&geo::Polygon::new(ring_ls(&moved), vec![]));
            for poly in cut.0 {
                let ring = |ls: &geo::LineString<f64>| -> Vec<LonLat> {
                    let mut v: Vec<LonLat> = ls.coords().map(|c| LonLat::new(c.x, c.y)).collect();
                    v.dedup();
                    if v.len() > 1 && v.first() == v.last() {
                        v.pop();
                    }
                    v
                };
                let outer = ring(poly.exterior());
                if outer.len() < 3 || geom::signed_area(&outer.iter().map(|q| [q.lon, q.lat]).collect::<Vec<_>>()).abs() < 1e-9 {
                    continue;
                }
                let holes: Vec<Vec<LonLat>> = poly.interiors().iter().map(ring).filter(|h| h.len() >= 3).collect();
                let (next, _) = overlay::apply_with_holes(self, &outer, &holes, DrawIntent::Take(to))?;
                *self = next;
                cut_any = true;
            }
        }
        if !cut_any {
            return Err("范围内没有给出单位的土地".into());
        }
        Ok(self.areas_by_entity().get(&to).copied().unwrap_or(0.0) - before)
    }
}
