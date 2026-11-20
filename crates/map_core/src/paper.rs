//! Size and pixel coordinates of a rectangular drawing board.
use crate::LonLat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paper {
    pub width: u32,
    pub height: u32,
}

impl Default for Paper {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 1200,
        }
    }
}

impl Paper {
    pub fn validate(self) -> Result<(), String> {
        if !(64..=8192).contains(&self.width) || !(64..=8192).contains(&self.height) {
            return Err("画布边长应在 64–8192 像素之间".into());
        }
        Ok(())
    }

    /// Internal coordinates for reuse of the vector editor. The visible
    /// canvas and all exported positions use a linear pixel transform.
    pub fn half_extents(self) -> [f64; 2] {
        let unit = 60.0 / f64::from(self.width.max(self.height).max(1));
        [f64::from(self.width) * unit, f64::from(self.height) * unit]
    }

    pub fn bounds(self) -> [f64; 4] {
        let [x, y] = self.half_extents();
        [
            -x.to_radians(),
            -y.to_radians(),
            x.to_radians(),
            y.to_radians(),
        ]
    }

    pub fn pixel(self, point: LonLat) -> [f64; 2] {
        let [x, y] = self.half_extents();
        [
            (point.lon / x + 1.0) * f64::from(self.width) * 0.5,
            (1.0 - point.lat / y) * f64::from(self.height) * 0.5,
        ]
    }

    pub fn length(self, points: &[LonLat]) -> f64 {
        points
            .windows(2)
            .map(|pair| {
                let (a, b) = (self.pixel(pair[0]), self.pixel(pair[1]));
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .sum()
    }

    pub fn area(self, points: &[LonLat]) -> f64 {
        let ring: Vec<_> = points.iter().map(|p| self.pixel(*p)).collect();
        crate::geom::signed_area(&ring).abs()
    }
}
