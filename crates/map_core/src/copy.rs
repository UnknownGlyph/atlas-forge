//! Moving and copying the drawn layout: rivers, routes, hatched areas,
//! symbols and map text. These are layout (not territory), so a move or
//! copy changes no ownership and no year.

use crate::doc::{Document, LineId};
use crate::geom::{wrap_lon, LonLat};
use crate::topo::TResult;

/// `pts` moved on the sphere so that `from` lands on `to`: a rotation
/// about the axis through both, so every distance, angle and area is kept
/// (a shape moved towards a pole keeps its true size).
pub fn moved(pts: &[LonLat], from: LonLat, to: LonLat) -> Vec<LonLat> {
    let v = |p: LonLat| {
        let (lo, la) = (p.lon.to_radians(), p.lat.to_radians());
        [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
    };
    let (a, b) = (v(from), v(to));
    let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let sin = (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();
    let cos = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if sin < 1e-12 {
        // no move (or to the exact opposite point, which has no single axis)
        return pts.to_vec();
    }
    let k = [cross[0] / sin, cross[1] / sin, cross[2] / sin];
    pts.iter()
        .map(|p| {
            let x = v(*p);
            let kx = [k[1] * x[2] - k[2] * x[1], k[2] * x[0] - k[0] * x[2], k[0] * x[1] - k[1] * x[0]];
            let kd = k[0] * x[0] + k[1] * x[1] + k[2] * x[2];
            let r: Vec<f64> = (0..3).map(|i| x[i] * cos + kx[i] * sin + k[i] * kd * (1.0 - cos)).collect();
            LonLat::new(r[1].atan2(r[0]).to_degrees(), r[2].clamp(-1.0, 1.0).asin().to_degrees())
        })
        .collect()
}

/// Where a copy goes: its first point `dlon`, `dlat` degrees away (kept
/// off the poles), the rest moved with it on the sphere.
fn copy_of(pts: &[LonLat], dlon: f64, dlat: f64) -> Vec<LonLat> {
    let Some(first) = pts.first() else { return vec![] };
    moved(pts, *first, copy_target(*first, dlon, dlat))
}

fn copy_target(first: LonLat, dlon: f64, dlat: f64) -> LonLat {
    LonLat::new(wrap_lon(first.lon + dlon), (first.lat + dlat).clamp(-89.0, 89.0))
}

impl Document {
    /// A copy follows its original's style.
    fn copy_style_link(&mut self, from: LineId, to: LineId) {
        if let Some(s) = self.style_of.get(&from).copied() {
            self.style_of.insert(to, s);
        }
    }

    /// Replace a river's or route's points.
    pub fn set_line_points(&mut self, id: LineId, pts: Vec<LonLat>) -> TResult<()> {
        if pts.len() < 2 {
            return Err("线至少需要两个点".into());
        }
        self.rivers.get_mut(&id).ok_or("线不存在")?.pts = pts;
        self.points_changed(id);
        Ok(())
    }

    /// Replace a hatched area's outline and its extra rings.
    pub fn set_area_rings(&mut self, id: LineId, ring: Vec<LonLat>, extra: Vec<Vec<LonLat>>) -> TResult<()> {
        self.set_area_ring(id, ring)?;
        self.areas.get_mut(&id).ok_or("区域不存在")?.extra = extra;
        Ok(())
    }

    /// Replace a hatched area's outline.
    pub fn set_area_ring(&mut self, id: LineId, ring: Vec<LonLat>) -> TResult<()> {
        if ring.len() < 3 {
            return Err("区域至少需要三个点".into());
        }
        self.areas.get_mut(&id).ok_or("区域不存在")?.ring = ring;
        self.points_changed(id);
        Ok(())
    }

    /// A copy of a river or route, its first point moved by `dlon`, `dlat`
    /// and the rest with it (true shape and size kept); returns its id.
    pub fn duplicate_line(&mut self, id: LineId, dlon: f64, dlat: f64) -> TResult<LineId> {
        let mut r = self.rivers.get(&id).ok_or("线不存在")?.clone();
        let first = *r.pts.first().ok_or("线没有点")?;
        let to = copy_target(first, dlon, dlat);
        r.pts = moved(&r.pts, first, to);
        let new = LineId(self.topo.alloc());
        self.rivers.insert(new, r);
        self.copy_style_link(id, new);
        // the copy's curve, moved the same way
        if let Some(mut c) = self.curves.get(&id).cloned() {
            // (its points worked out again from the moved curve: a curve
            // is drawn in the lon/lat plane, which turning the sphere bends)
            c.map(&|p| moved(&[p], first, to)[0]);
            self.set_curve(new, c)?;
        }
        Ok(new)
    }

    /// A copy of a hatched area, moved by `dlon`, `dlat`; returns its id.
    pub fn duplicate_area(&mut self, id: LineId, dlon: f64, dlat: f64) -> TResult<LineId> {
        let mut a = self.areas.get(&id).ok_or("区域不存在")?.clone();
        // every ring turns with the outline, so pieces and holes stay in place
        let first = *a.ring.first().ok_or("区域没有点")?;
        let to = copy_target(first, dlon, dlat);
        a.ring = moved(&a.ring, first, to);
        a.extra = a.extra.iter().map(|r| moved(r, first, to)).collect();
        let new = LineId(self.topo.alloc());
        self.areas.insert(new, a);
        self.copy_style_link(id, new);
        if let Some(mut c) = self.curves.get(&id).cloned() {
            // (its points worked out again from the moved curve: a curve
            // is drawn in the lon/lat plane, which turning the sphere bends)
            c.map(&|p| moved(&[p], first, to)[0]);
            self.set_curve(new, c)?;
        }
        Ok(new)
    }

    /// A copy of a symbol, moved by `dlon`, `dlat`; returns its id.
    pub fn duplicate_symbol(&mut self, id: LineId, dlon: f64, dlat: f64) -> TResult<LineId> {
        let mut s = self.symbols.get(&id).ok_or("符号不存在")?.clone();
        s.at = copy_of(&[s.at], dlon, dlat)[0];
        let new = LineId(self.topo.alloc());
        self.symbols.insert(new, s);
        self.copy_style_link(id, new);
        Ok(new)
    }

    /// A copy of a piece of map text, moved by `dlon`, `dlat`; returns its id.
    pub fn duplicate_annotation(&mut self, id: LineId, dlon: f64, dlat: f64) -> TResult<LineId> {
        let mut a = self.annotations.get(&id).ok_or("文字不存在")?.clone();
        // a curve the text follows moves with it
        a.move_to(copy_of(&[a.at], dlon, dlat)[0]);
        let new = LineId(self.topo.alloc());
        self.annotations.insert(new, a);
        self.copy_style_link(id, new);
        Ok(new)
    }
}
