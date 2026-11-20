//! Editable schematic star chart. Distances are in AU, periods in Earth
//! days, radii in reference-body units. The renderer applies its own scale.
use crate::climate::{Param, Planet as Environment};
use crate::Document;
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

pub const AU_KM: f64 = 149_597_870.7;
pub const SUN_RADIUS_KM: f64 = 695_700.0;
pub const EARTH_RADIUS_KM: f64 = 6371.0088;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Star {
    pub name: String,
    pub mass_solar: f64,
    pub radius_solar: f64,
    pub luminosity_solar: f64,
    pub temperature: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Orbit {
    pub au: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub days: f64,
    /// Mean anomaly at the simulation's time origin.
    pub phase: f64,
}

impl Orbit {
    pub fn position(&self, days: f64) -> [f64; 3] {
        let e = self.eccentricity.clamp(0.0, 0.92);
        let mean = (TAU * days.rem_euclid(self.days) / self.days + self.phase.to_radians())
            .rem_euclid(TAU);
        let mut anomaly = if e > 0.8 { TAU * 0.5 } else { mean };
        for _ in 0..14 {
            anomaly -= (anomaly - e * anomaly.sin() - mean) / (1.0 - e * anomaly.cos());
        }
        let x = self.au * (anomaly.cos() - e);
        let along = self.au * (1.0 - e * e).sqrt() * anomaly.sin();
        let (sin, cos) = self.inclination.to_radians().sin_cos();
        [x, along * sin, along * cos]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub name: String,
    pub orbit: Orbit,
    pub radii_earth: f64,
    pub spin_hours: f64,
    pub tilt: f64,
    pub retrograde: bool,
    pub synchronous: bool,
    pub color: [u8; 3],
    pub rings: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Moon {
    pub name: String,
    pub radius_km: f64,
    /// The satellite's orbit about the current project world, in AU and days.
    pub orbit: Orbit,
    pub synchronous: bool,
    pub spin_days: f64,
}

impl Default for Moon {
    fn default() -> Self {
        Self {
            name: "月球".into(),
            radius_km: 1737.4,
            orbit: Orbit {
                au: 384_400.0 / AU_KM,
                eccentricity: 0.0549,
                inclination: 5.145,
                days: 27.3217,
                phase: 42.0,
            },
            synchronous: true,
            spin_days: 27.3217,
        }
    }
}

impl Moon {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn distance_km(&self) -> f64 {
        self.orbit.au * AU_KM
    }

    /// Moon relative to the project world. The moon's nominal orbital plane
    /// is inclined to the world's orbital plane around the star.
    pub fn position_km(&self, home: &Body, days: f64) -> [f64; 3] {
        let [x, y, z] = self.orbit.position(days).map(|v| v * AU_KM);
        let (sin, cos) = home.orbit.inclination.to_radians().sin_cos();
        [x, y * cos + z * sin, z * cos - y * sin]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Galaxy {
    pub name: String,
    pub radius_kly: f32,
    pub arms: u8,
    pub barred: bool,
    pub home_kly: f32,
    pub home_angle: f32,
}
impl Default for Galaxy {
    fn default() -> Self {
        Self {
            name: "银河系".into(),
            radius_kly: 50.0,
            arms: 4,
            barred: true,
            home_kly: 26.0,
            home_angle: 210.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StarChart {
    pub star: Star,
    /// Replaces the Earth slot of the Solar System template.
    pub home: Body,
    pub planets: Vec<Body>,
    pub use_environment: bool,
    pub galaxy: Galaxy,
    #[serde(default, skip_serializing_if = "Moon::is_default")]
    pub moon: Moon,
}

impl Default for StarChart {
    fn default() -> Self {
        let body = |name: &str, values: [f64; 8], color, rings, retrograde| {
            let [au, eccentricity, inclination, days, phase, radii_earth, spin_hours, tilt] =
                values;
            Body {
                name: name.into(),
                orbit: Orbit {
                    au,
                    eccentricity,
                    inclination,
                    days,
                    phase,
                },
                radii_earth,
                spin_hours,
                tilt,
                retrograde,
                synchronous: false,
                color,
                rings,
            }
        };
        Self {
            star: Star {
                name: "太阳".into(),
                mass_solar: 1.0,
                radius_solar: 1.0,
                luminosity_solar: 1.0,
                temperature: 5772.0,
            },
            home: body(
                "",
                [1.0, 0.0167, 0.0, 365.256, 66.0, 1.0, 23.934, 23.44],
                [80, 150, 207],
                false,
                false,
            ),
            planets: vec![
                body(
                    "水星",
                    [0.387, 0.2056, 7.0, 87.969, 16.0, 0.383, 1407.6, 0.03],
                    [160, 154, 145],
                    false,
                    false,
                ),
                body(
                    "金星",
                    [0.723, 0.0068, 3.4, 224.701, 113.0, 0.949, 5832.5, 2.64],
                    [232, 177, 111],
                    false,
                    true,
                ),
                body(
                    "火星",
                    [1.524, 0.0934, 1.85, 686.98, 87.0, 0.532, 24.623, 25.2],
                    [194, 98, 69],
                    false,
                    false,
                ),
                body(
                    "木星",
                    [5.203, 0.0484, 1.30, 4332.59, 49.0, 10.97, 9.925, 3.1],
                    [213, 163, 125],
                    false,
                    false,
                ),
                body(
                    "土星",
                    [9.537, 0.0542, 2.49, 10759.22, 148.0, 9.14, 10.7, 26.7],
                    [219, 190, 139],
                    true,
                    false,
                ),
                body(
                    "天王星",
                    [19.19, 0.0472, 0.77, 30688.5, 239.0, 3.98, 17.24, 82.2],
                    [130, 205, 211],
                    true,
                    true,
                ),
                body(
                    "海王星",
                    [30.07, 0.0086, 1.77, 60182.0, 313.0, 3.86, 16.11, 28.3],
                    [83, 122, 222],
                    false,
                    false,
                ),
            ],
            use_environment: true,
            galaxy: Galaxy::default(),
            moon: Moon::default(),
        }
    }
}

impl StarChart {
    pub fn validate(&self) -> Result<(), String> {
        let valid =
            |values: &[(f64, f64, f64)]| values.iter().all(|&(v, lo, hi)| (lo..=hi).contains(&v));
        if !valid(&[
            (self.star.mass_solar, 0.06, 100.0),
            (self.star.radius_solar, 0.03, 100.0),
            (self.star.luminosity_solar, 0.00001, 100000.0),
            (self.star.temperature, 2200.0, 50000.0),
        ]) {
            return Err("星图恒星参数超出范围".into());
        }
        if self.planets.len() > 24
            || std::iter::once(&self.home).chain(&self.planets).any(|p| {
                !valid(&[
                    (p.orbit.au, 0.015, 200.0),
                    (p.orbit.eccentricity, 0.0, 0.92),
                    (p.orbit.inclination, -180.0, 180.0),
                    (p.orbit.days, 0.1, 100000.0),
                    (p.orbit.phase, 0.0, 360.0),
                    (p.radii_earth, 0.05, 50.0),
                    (p.spin_hours, 0.1, 2400000.0),
                    (p.tilt, 0.0, 180.0),
                ])
            })
        {
            return Err("星图行星参数超出范围".into());
        }
        let g = &self.galaxy;
        if !(2..=6).contains(&g.arms)
            || !valid(&[
                (g.radius_kly.into(), 15.0, 150.0),
                (g.home_kly.into(), 0.0, g.radius_kly.into()),
                (g.home_angle.into(), 0.0, 360.0),
            ])
        {
            return Err("星系参数超出范围".into());
        }
        let m = &self.moon;
        if !valid(&[
            (m.orbit.au, 1_000.0 / AU_KM, 10_000_000.0 / AU_KM),
            (m.radius_km, 1.0, 30_000.0),
            (m.orbit.eccentricity, 0.0, 0.92),
            (m.orbit.inclination, -180.0, 180.0),
            (m.orbit.days, 0.1, 100_000.0),
            (m.orbit.phase, 0.0, 360.0),
            (m.spin_days, 0.01, 100_000.0),
        ]) {
            return Err("月球参数超出范围".into());
        }
        Ok(())
    }

    /// Star and planet settings for the active map. Climate settings are
    /// read live when linked so the two views describe the same orbit.
    pub fn resolved(&self, doc: &Document) -> (Star, Body) {
        let mut star = self.star.clone();
        let mut home = self.home.clone();
        home.name.clone_from(&doc.meta.name);
        if !self.use_environment {
            return (star, home);
        }
        home.radii_earth = (doc.radius() / EARTH_RADIUS_KM).clamp(0.05, 50.0);
        if let Some(planet) = doc
            .climate
            .as_ref()
            .and_then(|p| p.settings.planet.resolve().ok())
        {
            apply_environment(&mut star, &mut home, &planet);
        }
        (star, home)
    }
}

/// Fraction of a uniformly bright stellar disc which remains exposed.
/// Apparent radii and separation should be in the same angular units.
pub fn disc_light(star: f64, blocker: f64, separation: f64) -> f64 {
    use std::f64::consts::PI;
    if separation >= star + blocker || blocker <= 0.0 {
        return 1.0;
    }
    if separation <= (star - blocker).abs() {
        return 1.0 - (blocker / star).min(1.0).powi(2);
    }
    let (a, b, d) = (star * star, blocker * blocker, separation * separation);
    let left = ((d + a - b) / (2.0 * separation * star))
        .clamp(-1.0, 1.0)
        .acos();
    let right = ((d + b - a) / (2.0 * separation * blocker))
        .clamp(-1.0, 1.0)
        .acos();
    let triangle = ((star + blocker + separation)
        * (star + blocker - separation)
        * (star + separation - blocker)
        * (blocker + separation - star))
        .max(0.0)
        .sqrt()
        * 0.5;
    (1.0 - (a * left + b * right - triangle) / (PI * a)).clamp(0.0, 1.0)
}

fn apply_environment(star: &mut Star, home: &mut Body, env: &Environment) {
    use Param::*;
    star.mass_solar = env[StarMass];
    star.radius_solar = env[StarRadius];
    star.luminosity_solar = env[Lum];
    star.temperature = env[StarTemp];
    home.orbit.au = env[Distance];
    home.orbit.eccentricity = env[Ecc];
    home.orbit.days = env[Period];
    home.radii_earth = env[Radius];
    home.spin_hours = env[Spin];
    home.tilt = env[Tilt];
    home.retrograde = env.retrograde;
    home.synchronous = env.locked_rotation;
}
