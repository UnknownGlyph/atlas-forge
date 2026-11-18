//! A monthly energy balance with seasonal surface inertia and diagnostic
//! wind/moisture transport. Months are equal fractions of an orbit.
use super::*;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "[f32;6]", into = "[f32;6]")]
pub struct Weather {
    pub temp: f32,
    pub rain: f32,
    pub humidity: f32,
    pub east: f32,
    pub north: f32,
    pub ice: f32,
}
impl From<[f32; 6]> for Weather {
    fn from([temp, rain, humidity, east, north, ice]: [f32; 6]) -> Self {
        Self {
            temp,
            rain,
            humidity,
            east,
            north,
            ice,
        }
    }
}
impl From<Weather> for [f32; 6] {
    fn from(v: Weather) -> Self {
        [v.temp, v.rain, v.humidity, v.east, v.north, v.ice]
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub ground: Ground,
    pub months: [Weather; 12],
}

fn orbit(p: &Planet, m: usize) -> (f64, f64) {
    let mean = TAU * (m as f64 + 0.5) / 12.;
    let e = p[Param::Ecc];
    let mut eccentric = mean;
    for _ in 0..8 {
        eccentric -= (eccentric - e * eccentric.sin() - mean) / (1. - e * eccentric.cos());
    }
    let true_anomaly = ((1. - e * e).sqrt() * eccentric.sin()).atan2(eccentric.cos() - e);
    let declination = (p[Param::Tilt].to_radians().sin() * (true_anomaly - 1.36).sin()).asin();
    (
        declination,
        p[Param::Flux] / (1. - e * eccentric.cos()).powi(2),
    )
}
fn sunlight(lat: f64, lon: f64, dec: f64, locked: bool) -> f64 {
    let a = lat.sin() * dec.sin();
    let b = lat.cos() * dec.cos();
    if locked {
        return (a + b * lon.cos()).max(0.);
    }
    let h = (-lat.tan() * dec.tan()).clamp(-1., 1.).acos();
    ((h * a + b * h.sin()) / PI).max(0.)
}
fn neighbour(i: usize, dx: i32, dy: i32) -> usize {
    let x = (i as i32 % W as i32 + dx).rem_euclid(W as i32);
    let y = (i as i32 / W as i32 + dy).clamp(0, H as i32 - 1);
    y as usize * W + x as usize
}
fn breeze(p: &Planet, lat: f64, lon: f64, dec: f64) -> (f64, f64) {
    let rotation = (24. / p[Param::Spin]).sqrt().clamp(0.10, 4.);
    let dir = if p.retrograde { -1. } else { 1. };
    if p.locked_rotation {
        // Near-surface flow converges towards the moving substellar latitude.
        let east = -lon.sin() * dec.cos();
        let north = lat.cos() * dec.sin() - lat.sin() * dec.cos() * lon.cos();
        return (
            7. * east + dir * rotation * 2. * north,
            7. * north - dir * rotation * 2. * east,
        );
    }
    let relative = lat - dec * 0.5;
    let bands = (3. * rotation.sqrt()).clamp(1., 5.);
    (
        -5.5 * dir * (2. * bands * relative).cos() * lat.cos().sqrt(),
        -3. * (2. * bands * relative).sin(),
    )
}
pub fn calculate(p: &Planet, ground: &[Ground]) -> Vec<Cell> {
    use Param::*;
    let atmosphere = p[Pressure].powf(0.35) * p[AirDensity].powf(0.15);
    let spin_factor = if p.locked_rotation {
        1.0
    } else {
        (p[Spin] / 24.).powf(0.2).clamp(0.45, 1.8)
    };
    let circulation = (p[Transport] * atmosphere * spin_factor / p[Radius].powf(0.25)).max(0.001);
    let mix = circulation / (circulation + 1.5);
    let gas = match p.gas {
        Gas::CarbonDioxide => 22.,
        Gas::Hydrogen => 10.,
        Gas::Argon => -6.,
        Gas::Nitrogen => 0.,
    };
    let greenhouse = (32.8
        + gas
        + 3. * ((p[CO2] + 40.) / 460.).ln()
        + 1.2 * ((p[Methane] + 1.) / 2.9).ln()
        + 2.2 * (0.2 + 0.8 * p[Vapour]).ln())
        * p[Greenhouse]
        * atmosphere;
    let orbits: [_; 12] = std::array::from_fn(|m| orbit(p, m));
    let mut cells: Vec<_> = ground
        .iter()
        .map(|&g| Cell {
            ground: g,
            months: Default::default(),
        })
        .collect();
    for (i, cell) in cells.iter_mut().enumerate() {
        let pos = center(i);
        let lat = pos.lat.to_radians();
        let lon = (pos.lon - p[Substellar]).to_radians();
        let g = ground[i];
        let water = g.water(p[Ocean]);
        let mut eq = [0.; 12];
        let reflect = (p[Albedo]
            + match g.surface {
                Surface::Ice => 0.28,
                Surface::Forest => -0.055,
                Surface::Water | Surface::Lake => -0.035,
                _ if g.forest => -0.055,
                _ => 0.,
            })
        .clamp(0.01, 0.96);
        for (m, &(dec, flux)) in orbits.iter().enumerate() {
            let sunlight = sunlight(lat, lon, dec, p.locked_rotation);
            let local = flux * ((1. - mix) * sunlight + 0.25 * mix);
            let temp = (local * (1. - reflect) / 5.670374419e-8).powf(0.25) - 273.15 + greenhouse;
            let ice_feedback = ((-temp + 2.) / 20.).clamp(0., 1.)
                * (4. + 5. * water)
                * (5772. / p[StarTemp]).powf(-0.45);
            eq[m] = temp
                - ice_feedback
                - 0.0065 * (p[Gravity] / 9.80665).powf(0.6) * f64::from(g.height.max(0.));
        }
        // Periodic steady state of a first-order surface heat reservoir.
        let reservoir_days = if g.surface == Surface::Lake { 42. } else { 90. };
        let keep = (-p[Period] / 12. / (12. + reservoir_days * water + 14. * circulation))
            .exp()
            .min(0.9999999);
        let weighted = eq.iter().fold(0., |v, t| keep * v + (1. - keep) * t);
        let mut temp = weighted / (1. - keep.powi(12));
        for (m, &(dec, _)) in orbits.iter().enumerate() {
            temp = keep * temp + (1. - keep) * eq[m];
            let (mut east, mut north) = breeze(p, lat, lon, dec);
            // Deflect flow along high relief (east/north positive).
            let dx =
                f64::from(ground[neighbour(i, 1, 0)].height - ground[neighbour(i, -1, 0)].height)
                    / 3000.;
            let dy =
                f64::from(ground[neighbour(i, 0, -1)].height - ground[neighbour(i, 0, 1)].height)
                    / 3000.;
            let uphill = (east * dx + north * dy).max(0.) / (1. + dx * dx + dy * dy);
            east -= 0.6 * uphill * dx;
            north -= 0.6 * uphill * dy;
            let length = east.hypot(north).max(0.01);
            let mut supply = 0.;
            let mut weight = 0.;
            let mut barrier = 0_f32;
            let mut near = 0.;
            for k in 1..=6 {
                let dx = (-east / length * k as f64 / lat.cos().max(0.25)).round() as i32;
                let dy = (north / length * k as f64).round() as i32;
                let up = ground[neighbour(i, dx, dy)];
                let w = (-0.36 * k as f64).exp();
                supply += w * (up.water(p[Ocean]) + if up.river { 0.10 } else { 0. });
                weight += w;
                barrier = barrier.max(up.height - g.height);
                if k == 1 {
                    near = f64::from(g.height - up.height);
                }
            }
            let migration = if p.locked_rotation {
                sunlight(lat, lon, dec, true)
            } else {
                let distance = (lat - dec * 0.75).abs();
                let belt = (3. * (24. / p[Spin]).powf(0.2)).clamp(0.6, 5.);
                (0.5 + 0.5 * (2. * belt * distance).cos()) * lat.cos().max(0.)
            };
            let moisture = p[Vapour]
                * (0.25
                    + 0.32 * water
                    + 0.80 * supply / weight
                    + if g.river { 0.16 } else { 0. }
                    + if g.forest { 0.13 } else { 0. });
            let orographic = (1. + near.clamp(0., 5000.) / 1800.)
                * (-f64::from(barrier.max(0.)) / 2100.).exp().max(0.12);
            let temp_factor = ((temp - 14.) * 0.035).clamp(-3., 1.5).exp();
            let rain_day = 3.8 * moisture * (0.28 + migration) * temp_factor * orographic;
            cell.months[m] = Weather {
                temp: temp.max(-273.15) as f32,
                rain: (rain_day * p[Period] / 12.) as f32,
                humidity: (100. * (1. - (-moisture * (0.8 + migration) * orographic).exp()))
                    .clamp(0., 100.) as f32,
                east: east as f32,
                north: north as f32,
                ice: if g.surface == Surface::Ice {
                    100.
                } else {
                    ((2. - temp) * 8.).clamp(0., 100.) as f32
                },
            };
        }
    }
    cells
}
