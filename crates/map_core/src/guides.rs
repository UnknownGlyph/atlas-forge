//! Fixed geographic guides for alignment and optional map annotation.
use crate::geom::{self, LonLat};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Latitude,
    Longitude,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: Axis,
    pub degrees: f64,
    pub name: String,
    pub color: [u8; 4],
    pub visible: bool,
    pub include_export: bool,
    pub snap: bool,
    /// Optional coordinate along the line at which the text sits.
    #[serde(default)]
    pub label_at: Option<f64>,
}

impl Guide {
    pub fn new(axis: Axis, degrees: f64, name: impl Into<String>) -> Self {
        Self {
            axis,
            degrees,
            name: name.into(),
            color: [207, 62, 71, 210],
            visible: true,
            include_export: false,
            snap: true,
            label_at: None,
        }
    }

    pub fn valid(&self) -> bool {
        let range = match self.axis {
            Axis::Latitude => -89.999..=89.999,
            Axis::Longitude => -180.0..=180.0,
        };
        range.contains(&self.degrees)
    }

    pub fn path(&self, meridian: f64) -> Vec<LonLat> {
        match self.axis {
            Axis::Latitude => (0..=180)
                .map(|i| {
                    let delta = (-180.0 + 2.0 * f64::from(i)).clamp(-179.99999, 179.99999);
                    LonLat::new(geom::wrap_lon(meridian + delta), self.degrees)
                })
                .collect(),
            Axis::Longitude => (-44..=44)
                .map(|i| LonLat::new(self.degrees, f64::from(i) * 2.0))
                .collect(),
        }
    }

    pub fn label_position(&self, meridian: f64) -> LonLat {
        match self.axis {
            Axis::Latitude => LonLat::new(self.label_at.unwrap_or(meridian), self.degrees),
            Axis::Longitude => LonLat::new(self.degrees, self.label_at.unwrap_or(40.0)),
        }
    }
}

pub fn snap(guides: &[Guide], p: LonLat, tolerance: f64) -> Option<LonLat> {
    let mut out = p;
    let mut dx = tolerance;
    let mut dy = tolerance;
    for guide in guides.iter().filter(|g| g.visible && g.snap && g.valid()) {
        match guide.axis {
            Axis::Longitude => {
                let d = geom::lon_delta(p.lon, guide.degrees).abs();
                if d <= dx {
                    dx = d;
                    out.lon = geom::wrap_lon(guide.degrees);
                }
            }
            Axis::Latitude => {
                let d = (p.lat - guide.degrees).abs();
                if d <= dy {
                    dy = d;
                    out.lat = guide.degrees;
                }
            }
        }
    }
    (out != p).then_some(out)
}
