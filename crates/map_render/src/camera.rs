//! 2D and globe cameras. CPU-side transforms mirror the shader exactly so
//! picking, labels and overlays line up with GPU output.

use glam::{DMat4, DVec3, DVec4};
use map_core::geom::{self, LonLat};
use map_core::projection::{self, Projection};

#[derive(Clone, Copy, Debug)]
pub struct Camera2D {
    pub proj: Projection,
    /// Central meridian (degrees). Changing it moves the seam.
    pub lon0: f64,
    /// View centre in projected units.
    pub center: [f64; 2],
    /// Pixels per projected unit.
    pub zoom: f64,
}

impl Camera2D {
    pub fn new(proj: Projection) -> Self {
        Self { proj, lon0: 0.0, center: [0.0, 0.0], zoom: 150.0 }
    }

    pub fn fit(&mut self, w: f64, h: f64) {
        let b = self.proj.bounds();
        self.center = [0.0, 0.0];
        self.zoom = (w / (b[2] - b[0])).min(h / (b[3] - b[1])) * 0.95;
    }

    /// Screen offset from viewport centre (px, y down) → projected units.
    pub fn screen_to_proj(&self, d: [f64; 2]) -> [f64; 2] {
        [self.center[0] + d[0] / self.zoom, self.center[1] - d[1] / self.zoom]
    }

    pub fn proj_to_screen(&self, p: [f64; 2]) -> [f64; 2] {
        [(p[0] - self.center[0]) * self.zoom, -(p[1] - self.center[1]) * self.zoom]
    }

    pub fn screen_to_lonlat(&self, d: [f64; 2]) -> Option<LonLat> {
        let p = self.screen_to_proj(d);
        let ll = self.proj.inverse_deg(p[0], p[1])?;
        Some(LonLat::new(geom::wrap_lon(ll[0] + self.lon0), ll[1]))
    }

    pub fn lonlat_to_screen(&self, ll: LonLat) -> [f64; 2] {
        let rel = geom::wrap_lon(ll.lon - self.lon0);
        self.proj_to_screen(self.proj.forward_deg(rel, ll.lat))
    }

    /// Zoom by `factor` keeping the point under the cursor fixed.
    pub fn zoom_at(&mut self, d: [f64; 2], factor: f64) {
        let before = self.screen_to_proj(d);
        self.zoom = (self.zoom * factor).clamp(10.0, 5.0e7);
        let after = self.screen_to_proj(d);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    /// Put `ll` at the view's centre, keeping the zoom.
    pub fn centre_on(&mut self, ll: LonLat) {
        self.center = self.proj.forward_deg(geom::wrap_lon(ll.lon - self.lon0), ll.lat);
    }

    pub fn pan_px(&mut self, d: [f64; 2]) {
        self.center[0] -= d[0] / self.zoom;
        self.center[1] += d[1] / self.zoom;
    }
}

/// Right-handed view matrix.
fn look_at_rh(eye: DVec3, center: DVec3, up: DVec3) -> DMat4 {
    let f = (center - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    DMat4::from_cols(
        DVec4::new(s.x, u.x, -f.x, 0.0),
        DVec4::new(s.y, u.y, -f.y, 0.0),
        DVec4::new(s.z, u.z, -f.z, 0.0),
        DVec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Right-handed perspective with wgpu's [0, 1] depth range.
fn perspective_rh(fov_y: f64, aspect: f64, near: f64, far: f64) -> DMat4 {
    let h = 1.0 / (fov_y * 0.5).tan();
    let r = far / (near - far);
    DMat4::from_cols(
        DVec4::new(h / aspect, 0.0, 0.0, 0.0),
        DVec4::new(0.0, h, 0.0, 0.0),
        DVec4::new(0.0, 0.0, r, -1.0),
        DVec4::new(0.0, 0.0, r * near, 0.0),
    )
}

#[derive(Clone, Copy, Debug)]
pub struct GlobeCamera {
    pub center_lon: f64,
    pub center_lat: f64,
    /// Camera distance from the globe centre in planet radii.
    pub dist: f64,
    pub fov_y: f64,
}

impl Default for GlobeCamera {
    fn default() -> Self {
        Self { center_lon: 0.0, center_lat: 20.0, dist: 3.2, fov_y: 35f64.to_radians() }
    }
}

impl GlobeCamera {
    /// Vertical field of view such that `fov_y` covers the *smaller* viewport
    /// dimension, so the globe fits narrow panels too.
    pub fn fov_for(&self, w: f64, h: f64) -> f64 {
        if w >= h {
            self.fov_y
        } else {
            2.0 * ((self.fov_y * 0.5).tan() * h / w).atan()
        }
    }

    /// Screen pixels per radian of arc at the globe centre.
    pub fn px_per_rad(&self, w: f64, h: f64) -> f64 {
        h / (2.0 * (self.fov_for(w, h) * 0.5).tan() * (self.dist - 1.0))
    }

    /// Radius of the globe's outline on the screen (px).
    pub fn disc_px(&self, w: f64, h: f64) -> f64 {
        let focal = h / (2.0 * (self.fov_for(w, h) * 0.5).tan());
        focal / (self.dist * self.dist - 1.0).max(1e-9).sqrt()
    }

    pub fn eye(&self) -> DVec3 {
        DVec3::from_array(projection::sphere_pos(self.center_lon, self.center_lat)) * self.dist
    }

    pub fn view_proj(&self, w: f64, h: f64) -> DMat4 {
        let (l, p) = (self.center_lon.to_radians(), self.center_lat.to_radians());
        let up = DVec3::new(-p.sin() * l.sin(), p.cos(), -p.sin() * l.cos());
        perspective_rh(self.fov_for(w, h), (w / h).max(1e-3), 0.01, 100.0) * look_at_rh(self.eye(), DVec3::ZERO, up)
    }

    /// Screen offset from viewport centre (px, y down) → surface point.
    pub fn pick(&self, d: [f64; 2], w: f64, h: f64) -> Option<LonLat> {
        let inv = self.view_proj(w, h).inverse();
        let ndc = [d[0] / (w * 0.5), -d[1] / (h * 0.5)];
        let near = inv * DVec4::new(ndc[0], ndc[1], 0.0, 1.0);
        let far = inv * DVec4::new(ndc[0], ndc[1], 1.0, 1.0);
        let o = near.truncate() / near.w;
        let dir = (far.truncate() / far.w - o).normalize();
        // ray-sphere (unit)
        let b = o.dot(dir);
        let c = o.dot(o) - 1.0;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let t = -b - disc.sqrt();
        let hit = o + dir * t;
        let ll = projection::sphere_to_lonlat(hit.to_array());
        Some(LonLat::new(ll[0], ll[1]))
    }

    /// Surface point → screen offset (px, y down), None if behind the horizon.
    pub fn project(&self, ll: LonLat, w: f64, h: f64) -> Option<[f64; 2]> {
        let p = DVec3::from_array(projection::sphere_pos(ll.lon, ll.lat));
        if p.dot(self.eye()) < 1.0 {
            return None;
        }
        let c = self.view_proj(w, h) * p.extend(1.0);
        Some([c.x / c.w * w * 0.5, -c.y / c.w * h * 0.5])
    }

    pub fn rotate_px(&mut self, d: [f64; 2], w: f64, h: f64) {
        let rad_per_px = 1.0 / self.px_per_rad(w, h);
        let deg = rad_per_px.to_degrees();
        self.center_lon = geom::wrap_lon(self.center_lon - d[0] * deg / self.center_lat.to_radians().cos().max(0.2));
        self.center_lat = (self.center_lat + d[1] * deg).clamp(-89.0, 89.0);
    }

    pub fn zoom(&mut self, factor: f64) {
        self.dist = (1.0 + (self.dist - 1.0) / factor).clamp(1.02, 12.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn globe_pick_project_roundtrip() {
        let cam = GlobeCamera { center_lon: 30.0, center_lat: 10.0, ..Default::default() };
        let (w, h) = (800.0, 600.0);
        let c = cam.pick([0.0, 0.0], w, h).unwrap();
        assert!((c.lon - 30.0).abs() < 1e-6 && (c.lat - 10.0).abs() < 1e-6);
        let s = cam.project(LonLat::new(35.0, 12.0), w, h).unwrap();
        let back = cam.pick(s, w, h).unwrap();
        assert!((back.lon - 35.0).abs() < 1e-6 && (back.lat - 12.0).abs() < 1e-6);
        assert!(cam.project(LonLat::new(-150.0, -10.0), w, h).is_none());
    }
    #[test]
    fn camera2d_roundtrip() {
        let mut cam = Camera2D::new(Projection::EqualEarth);
        cam.lon0 = 150.0;
        cam.zoom_at([10.0, 5.0], 3.0);
        let ll = LonLat::new(-170.0, 40.0);
        let s = cam.lonlat_to_screen(ll);
        let back = cam.screen_to_lonlat(s).unwrap();
        assert!((back.lon - ll.lon).abs() < 1e-6 && (back.lat - ll.lat).abs() < 1e-6);
    }
}
