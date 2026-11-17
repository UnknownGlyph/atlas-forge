//! Smart snapping while features are moved: a key point of what is moved
//! (where a symbol or text sits, the ends and middle of a line, the middle
//! of an area) lands on a key point of another feature when it comes near
//! one; otherwise it lines up with another key point's latitude or
//! longitude, or with a graticule line. The guides say what it snapped to.

use crate::doc::Document;
use crate::features::FeatureRef;
use crate::geom::{self, LonLat};

/// What a move snapped to, for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide {
    /// A key point of another feature.
    Point(LonLat),
    /// A parallel, and the point that set it.
    Parallel(f64, Option<LonLat>),
    /// A meridian, and the point that set it.
    Meridian(f64, Option<LonLat>),
}

/// The shift to add to a move (degrees of longitude and latitude), and why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub dlon: f64,
    pub dlat: f64,
    pub guides: Vec<Guide>,
}

impl Document {
    /// A feature's key points, for snapping.
    pub fn key_points(&self, f: FeatureRef) -> Vec<LonLat> {
        match f {
            FeatureRef::Text(_) | FeatureRef::Symbol(_) | FeatureRef::Area(_) => self.feature_anchor(f).into_iter().collect(),
            FeatureRef::Route(id) | FeatureRef::River(id) => match self.rivers.get(&id) {
                Some(r) if r.pts.len() >= 2 => vec![r.pts[0], crate::doc::path_middle(&r.pts), r.pts[r.pts.len() - 1]],
                Some(r) => r.pts.clone(),
                None => vec![],
            },
        }
    }

    /// Key points of every shown feature except `except`.
    pub fn snap_targets(&self, except: &[FeatureRef]) -> Vec<LonLat> {
        self.features().into_iter().filter(|f| !f.hidden && !except.contains(&f.what)).flat_map(|f| self.key_points(f.what)).collect()
    }
}

/// How to shift `keys` (the moved selection's key points) so that one of
/// them lands on one of `targets` within `tol` degrees; failing that, the
/// smallest shifts that line one up with a target's latitude and one with
/// a target's longitude (or a graticule line every `grid` degrees), each
/// within `tol` (east-west measured along the parallel).
pub fn snap(keys: &[LonLat], targets: &[LonLat], grid: Option<f64>, tol: f64) -> Snap {
    // a point on a point
    let mut best: Option<(f64, LonLat, LonLat)> = None;
    for k in keys {
        for t in targets {
            let d = geom::geo_dist_deg(*k, *t);
            if d < tol && best.is_none_or(|b| d < b.0) {
                best = Some((d, *k, *t));
            }
        }
    }
    if let Some((_, k, t)) = best {
        return Snap { dlon: geom::lon_delta(k.lon, t.lon), dlat: t.lat - k.lat, guides: vec![Guide::Point(t)] };
    }
    let mut out = Snap::default();
    // latitudes: of targets, and of the graticule
    let mut lat: Option<(f64, f64, Option<LonLat>)> = None;
    let mut lon: Option<(f64, f64, Option<LonLat>)> = None;
    for k in keys {
        let c = k.lat.to_radians().cos().max(0.05);
        let lats = targets.iter().map(|t| (t.lat, Some(*t))).chain(grid.map(|g| ((k.lat / g).round() * g, None)));
        for (y, from) in lats {
            let d = (y - k.lat).abs();
            if d < tol && lat.is_none_or(|b| d < b.0) {
                lat = Some((d, y - k.lat, from));
            }
        }
        let lons = targets.iter().map(|t| (t.lon, Some(*t))).chain(grid.map(|g| ((k.lon / g).round() * g, None)));
        for (x, from) in lons {
            let dl = geom::lon_delta(k.lon, x);
            let d = dl.abs() * c;
            if d < tol && lon.is_none_or(|b| d < b.0) {
                lon = Some((d, dl, from));
            }
        }
    }
    if let Some((_, dy, from)) = lat {
        out.dlat = dy;
        let y = keys.iter().map(|k| k.lat + dy).find(|y| from.is_none_or(|t| (t.lat - y).abs() < 1e-9)).unwrap_or(0.0);
        out.guides.push(Guide::Parallel(from.map_or(y, |t| t.lat), from));
    }
    if let Some((_, dx, from)) = lon {
        out.dlon = dx;
        let x = keys.iter().map(|k| geom::wrap_lon(k.lon + dx)).find(|x| from.is_none_or(|t| geom::lon_delta(t.lon, *x).abs() < 1e-9)).unwrap_or(0.0);
        out.guides.push(Guide::Meridian(from.map_or(x, |t| t.lon), from));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(lon: f64, lat: f64) -> LonLat {
        LonLat::new(lon, lat)
    }

    #[test]
    fn a_point_onto_a_point_before_lining_up() {
        let s = snap(&[p(10.0, 20.0)], &[p(10.3, 20.2), p(50.0, 20.1)], Some(10.0), 0.5);
        assert!((s.dlon - 0.3).abs() < 1e-9 && (s.dlat - 0.2).abs() < 1e-9);
        assert_eq!(s.guides, vec![Guide::Point(p(10.3, 20.2))]);
    }

    #[test]
    fn lining_up_with_a_latitude_and_the_graticule() {
        // far from the target, but at nearly its latitude; near a meridian of the 10° grid
        let s = snap(&[p(29.7, 21.0)], &[p(80.0, 21.3)], Some(10.0), 0.5);
        assert!((s.dlat - 0.3).abs() < 1e-9 && (s.dlon - 0.3).abs() < 1e-9, "{s:?}");
        assert_eq!(s.guides, vec![Guide::Parallel(21.3, Some(p(80.0, 21.3))), Guide::Meridian(30.0, None)]);
        // nothing near: no shift
        let s = snap(&[p(25.0, 25.0)], &[p(80.0, 21.0)], Some(10.0), 0.5);
        assert_eq!(s, Snap::default());
        // across the antimeridian
        let s = snap(&[p(179.8, 5.0)], &[p(-179.9, 40.0)], None, 0.5);
        assert!((s.dlon - 0.3).abs() < 1e-9 && s.dlat == 0.0, "{s:?}");
    }
}
