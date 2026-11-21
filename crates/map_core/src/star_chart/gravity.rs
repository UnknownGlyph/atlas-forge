//! Mass-consistent orbital initial conditions and adaptive N-body dynamics.
//! Positions are AU, velocities AU per Earth day and masses solar masses.
use super::{Body, Orbit, StarChart, AU_KM, EARTH_RADIUS_KM, SUN_RADIUS_KM};
use crate::Document;
use glam::{DQuat, DVec3};

/// Nominal solar gravity parameter, converted from SI to AU³/day².
pub const G: f64 =
    1.327_124_4e20 * 86_400.0 * 86_400.0 / (AU_KM * 1000.0 * AU_KM * 1000.0 * AU_KM * 1000.0);
const SOLAR_MASS_KG: f64 = 1.988_41e30;
const EARTH_IN_SOLAR_MASSES: f64 = 398_600.435_507 / 132_712_440_041.279_42;
const MIN_STEP: f64 = 1e-11;
const MAX_STEP: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKey {
    Star,
    Home,
    Planet(usize),
    Companion(usize),
    Moon,
    Satellite(usize),
    GalaxyStar(usize),
}

impl BodyKey {
    fn parent(self) -> Option<Self> {
        match self {
            Self::Home | Self::Planet(_) | Self::Companion(_) => Some(Self::Star),
            Self::Moon | Self::Satellite(_) => Some(Self::Home),
            Self::Star | Self::GalaxyStar(_) => None,
        }
    }

    pub fn can_compare_without(self) -> bool {
        matches!(
            self,
            Self::Planet(_) | Self::Companion(_) | Self::Moon | Self::Satellite(_)
        )
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub key: BodyKey,
    pub name: String,
    pub mass: f64,
    pub radius: f64,
    pub position: DVec3,
    pub velocity: DVec3,
}

#[derive(Clone, Copy)]
struct Oblateness {
    index: usize,
    axis: DVec3,
    j2_radius_sq: f64,
}

#[derive(Clone)]
pub struct Gravity {
    bodies: Vec<State>,
    days: f64,
    next_step: f64,
    oblate: Option<Oblateness>,
}

#[derive(Debug, Clone, Copy)]
pub struct OrbitReading {
    pub distance_km: f64,
    pub speed_km_s: f64,
    pub eccentricity: f64,
    pub periapsis_km: f64,
    pub apoapsis_km: Option<f64>,
    pub period_days: Option<f64>,
}

#[derive(Debug)]
pub struct Perturbation {
    pub name: String,
    pub displacement_km: f64,
    pub eccentricity_change: f64,
    pub periapsis_change_km: f64,
}

impl Body {
    /// Measured masses for reference planets, Earth density as the editable
    /// mass estimate for other worlds.
    pub fn effective_mass_earth(&self) -> f64 {
        self.mass_earth.unwrap_or_else(|| match self.name.as_str() {
            "水星" => 0.0553,
            "金星" => 0.815,
            "火星" => 0.10745,
            "木星" => 317.83,
            "土星" => 95.16,
            "天王星" => 14.54,
            "海王星" => 17.15,
            _ => self.radii_earth.powi(3),
        })
    }
}

fn orbital_state(orbit: &Orbit, days: f64, parent: f64, body: f64) -> (DVec3, DVec3) {
    // Retain the authored position when switching from schematic playback.
    // The phase clock does not set the tangential speed in the physical run.
    let (position, velocity) = orbit.state(days);
    let kepler_days = orbit.kepler_period(parent, body);
    (
        DVec3::from(position),
        DVec3::from(velocity) * (orbit.days / kepler_days),
    )
}

impl Gravity {
    pub fn new(chart: &StarChart, doc: &Document, days: f64) -> Result<Self, String> {
        chart.validate()?;
        let (star, home) = chart.resolved(doc);
        let home_mass = home.effective_mass_earth() * EARTH_IN_SOLAR_MASSES;
        let home_radius = home.radii_earth * EARTH_RADIUS_KM / AU_KM;
        let moons = std::iter::once((BodyKey::Moon, &chart.moon)).chain(
            chart
                .satellites
                .iter()
                .enumerate()
                .map(|(i, m)| (BodyKey::Satellite(i), m)),
        );
        let moons: Vec<_> = moons
            .map(|(key, satellite)| {
                let mass = satellite
                    .mass_kg
                    .unwrap_or(7.342e22 * (satellite.radius_km / 1737.4).powi(3))
                    / SOLAR_MASS_KG;
                let (position, velocity) = orbital_state(&satellite.orbit, days, home_mass, mass);
                let plane = home.orbit.plane();
                State {
                    key,
                    name: satellite.name.clone(),
                    mass,
                    radius: satellite.radius_km / AU_KM,
                    position: plane * position,
                    velocity: plane * velocity,
                }
            })
            .collect();
        let cluster_mass = home_mass + moons.iter().map(|m| m.mass).sum::<f64>();
        if cluster_mass <= 0.0 || !cluster_mass.is_finite() {
            return Err("主体星球及卫星需要有正质量，才能建立局部轨道。".into());
        }
        let mut bodies = vec![State {
            key: BodyKey::Star,
            name: star.name.clone(),
            mass: star.mass_solar,
            radius: star.radius_solar * SUN_RADIUS_KM / AU_KM,
            position: DVec3::ZERO,
            velocity: DVec3::ZERO,
        }];
        let (cluster_position, cluster_velocity) =
            orbital_state(&home.orbit, days, star.mass_solar, cluster_mass);
        let position = cluster_position
            - moons.iter().map(|m| m.mass * m.position).sum::<DVec3>() / cluster_mass;
        let velocity = cluster_velocity
            - moons.iter().map(|m| m.mass * m.velocity).sum::<DVec3>() / cluster_mass;
        bodies.push(State {
            key: BodyKey::Home,
            name: home.name,
            mass: home_mass,
            radius: home_radius,
            position,
            velocity,
        });
        for (i, planet) in chart.planets.iter().enumerate() {
            let mass = planet.effective_mass_earth() * EARTH_IN_SOLAR_MASSES;
            let (position, velocity) = orbital_state(&planet.orbit, days, star.mass_solar, mass);
            bodies.push(State {
                key: BodyKey::Planet(i),
                name: planet.name.clone(),
                mass,
                radius: planet.radii_earth * EARTH_RADIUS_KM / AU_KM,
                position,
                velocity,
            });
        }
        for (i, companion) in chart.companions.iter().enumerate() {
            let source = &companion.star;
            let (position, velocity) =
                orbital_state(&companion.orbit, days, star.mass_solar, source.mass_solar);
            bodies.push(State {
                key: BodyKey::Companion(i),
                name: source.name.clone(),
                mass: source.mass_solar,
                radius: source.radius_solar * SUN_RADIUS_KM / AU_KM,
                position,
                velocity,
            });
        }
        bodies.extend(moons.into_iter().map(|mut moon| {
            moon.position += position;
            moon.velocity += velocity;
            moon
        }));
        let mut sim = Self::from_states(bodies, days)?;
        if chart.home_j2 > 0.0 {
            sim.oblate = Some(Oblateness {
                index: 1,
                axis: home.orbit.plane()
                    * DQuat::from_rotation_x(home.tilt.to_radians())
                    * DVec3::Y,
                j2_radius_sq: chart.home_j2 * home_radius * home_radius,
            });
        }
        Ok(sim)
    }

    /// Translate and boost into the system's barycentric inertial frame.
    pub fn from_states(mut bodies: Vec<State>, days: f64) -> Result<Self, String> {
        let mass: f64 = bodies.iter().map(|p| p.mass).sum();
        if !days.is_finite()
            || mass <= 0.0
            || !mass.is_finite()
            || bodies.iter().any(|p| {
                !(p.mass >= 0.0
                    && p.radius >= 0.0
                    && p.radius.is_finite()
                    && p.position.is_finite()
                    && p.velocity.is_finite())
            })
        {
            return Err("引力模拟的初始状态无效。".into());
        }
        let barycenter = bodies.iter().map(|p| p.mass * p.position).sum::<DVec3>() / mass;
        let motion = bodies.iter().map(|p| p.mass * p.velocity).sum::<DVec3>() / mass;
        for p in &mut bodies {
            p.position -= barycenter;
            p.velocity -= motion;
        }
        let sim = Self {
            bodies,
            days,
            next_step: MAX_STEP,
            oblate: None,
        };
        sim.step_limit()?;
        Ok(sim)
    }

    pub fn days(&self) -> f64 {
        self.days
    }
    pub fn states(&self) -> &[State] {
        &self.bodies
    }

    fn body(&self, key: BodyKey) -> Option<&State> {
        self.bodies.iter().find(|p| p.key == key)
    }
    pub fn position(&self, key: BodyKey) -> Option<DVec3> {
        Some(self.body(key)?.position)
    }

    fn relative(&self, key: BodyKey) -> Option<(DVec3, DVec3, f64)> {
        let body = self.body(key)?;
        let parent = self.body(key.parent()?)?;
        Some((
            body.position - parent.position,
            body.velocity - parent.velocity,
            G * (parent.mass + body.mass),
        ))
    }

    /// Instantaneous two-body elements, useful as readings of an evolving
    /// trajectory. The simulation itself still feels all forces.
    pub fn orbit_reading(&self, key: BodyKey) -> Option<OrbitReading> {
        let (r, v, mu) = self.relative(key)?;
        let distance = r.length();
        if distance <= 0.0 || mu <= 0.0 {
            return None;
        }
        let h = r.cross(v);
        let eccentricity = (v.cross(h) / mu - r / distance).length();
        let parameter_km = h.length_squared() * AU_KM / mu;
        let apoapsis = (eccentricity < 1.0).then(|| parameter_km / (1.0 - eccentricity));
        let periapsis = parameter_km / (1.0 + eccentricity);
        Some(OrbitReading {
            distance_km: distance * AU_KM,
            speed_km_s: v.length() * AU_KM / 86_400.0,
            eccentricity,
            periapsis_km: periapsis,
            apoapsis_km: apoapsis,
            period_days: apoapsis.map(|apoapsis| {
                let axis = (periapsis + apoapsis) / (2.0 * AU_KM);
                std::f64::consts::TAU * (axis.powi(3) / mu).sqrt()
            }),
        })
    }

    /// Copy the exact same initial states into a counterfactual experiment.
    /// Removing a body leaves all other pairwise positions and velocities intact.
    pub fn without(&self, key: BodyKey) -> Result<Self, String> {
        if !key.can_compare_without() || self.body(key).is_none() {
            return Err("请选择要从对照组移除的行星、伴星或卫星。".into());
        }
        let bodies: Vec<_> = self
            .bodies
            .iter()
            .filter(|p| p.key != key)
            .cloned()
            .collect();
        let mut sim = Self::from_states(bodies, self.days)?;
        sim.oblate = self.oblate; // Star and home always occupy the same slots here.
        Ok(sim)
    }

    pub fn differences(&self, reference: &Self) -> Vec<Perturbation> {
        let mut rows: Vec<_> = self
            .bodies
            .iter()
            .filter_map(|p| {
                let current = self.orbit_reading(p.key)?;
                let baseline = reference.orbit_reading(p.key)?;
                let displacement = self.relative(p.key)?.0 - reference.relative(p.key)?.0;
                Some(Perturbation {
                    name: p.name.clone(),
                    displacement_km: displacement.length() * AU_KM,
                    eccentricity_change: current.eccentricity - baseline.eccentricity,
                    periapsis_change_km: current.periapsis_km - baseline.periapsis_km,
                })
            })
            .collect();
        rows.sort_by(|a, b| b.displacement_km.total_cmp(&a.displacement_km));
        rows
    }

    fn contact(&self, i: usize, j: usize) -> String {
        format!(
            "检测到天体接触：{} 与 {}。模拟已暂停。",
            self.bodies[i].name, self.bodies[j].name
        )
    }

    fn step_limit(&self) -> Result<f64, String> {
        let mut limit = MAX_STEP;
        for (i, a) in self.bodies.iter().enumerate() {
            for (j, b) in self.bodies.iter().enumerate().skip(i + 1) {
                let distance = (a.position - b.position).length();
                if !distance.is_finite() {
                    return Err("天体坐标超出数值范围。".into());
                }
                if distance <= (a.radius + b.radius).max(1e-13) {
                    return Err(self.contact(i, j));
                }
                let speed = (a.velocity - b.velocity).length();
                let mu = G * (a.mass + b.mass);
                limit = limit.min(0.10 * distance / speed.max(1e-30));
                if mu > 0.0 {
                    limit = limit.min(0.10 * (distance.powi(3) / mu).sqrt());
                }
            }
        }
        Ok(limit)
    }

    /// Advance up to the requested span. The step budget includes rejected
    /// steps, so close encounters cannot monopolize an interactive frame.
    pub fn advance(&mut self, span: f64, max_steps: usize) -> Result<f64, String> {
        if !span.is_finite() || span < 0.0 {
            return Err("引力模拟时间步无效。".into());
        }
        if span == 0.0 || max_steps == 0 {
            return Ok(0.0);
        }
        let mut workspace = Integrator::new(&self.bodies);
        let mut elapsed = 0.0;
        for _ in 0..max_steps {
            if span - elapsed < MIN_STEP {
                break;
            }
            let dt = (span - elapsed).min(self.next_step).min(self.step_limit()?);
            if dt < MIN_STEP {
                return Err("近距离相遇所需步长过小。请检查轨道与天体质量。".into());
            }
            let error = workspace.propose(&self.bodies, self.oblate, dt);
            let factor = (0.90 * error.powf(-0.2)).clamp(0.2, 4.0);
            self.next_step = (dt * factor).min(MAX_STEP);
            if !error.is_finite() {
                self.next_step = dt * 0.2;
                continue;
            }
            if error > 1.0 {
                continue;
            }
            // Check a swept segment between the accepted endpoints. Close
            // encounters also shorten the independent dynamical step bound.
            if let Some((i, j)) = workspace.contact(&self.bodies) {
                return Err(self.contact(i, j));
            }
            for (body, state) in self.bodies.iter_mut().zip(&workspace.proposed) {
                body.position = state.r;
                body.velocity = state.v;
            }
            elapsed += dt;
            self.days += dt;
        }
        Ok(elapsed)
    }
}

#[derive(Clone, Copy, Default)]
struct Phase {
    r: DVec3,
    v: DVec3,
}

struct Integrator {
    proposed: Vec<Phase>,
    stages: [Vec<Phase>; 7],
    // Scale local error by the orbit about the designated parent, so an
    // AU-wide system does not mask the error of a low satellite.
    scales: Vec<(Option<usize>, f64)>,
}

impl Integrator {
    fn new(bodies: &[State]) -> Self {
        Self {
            proposed: vec![Phase::default(); bodies.len()],
            stages: std::array::from_fn(|_| vec![Phase::default(); bodies.len()]),
            scales: bodies
                .iter()
                .map(|p| {
                    let parent = p
                        .key
                        .parent()
                        .and_then(|key| bodies.iter().position(|q| q.key == key));
                    (parent, p.radius)
                })
                .collect(),
        }
    }

    fn propose(&mut self, bodies: &[State], oblate: Option<Oblateness>, dt: f64) -> f64 {
        // Dormand–Prince RK5(4), DOI 10.1016/0771-050X(80)90013-3.
        const A: [&[f64]; 7] = [
            &[],
            &[1.0 / 5.0],
            &[3.0 / 40.0, 9.0 / 40.0],
            &[44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0],
            &[
                19372.0 / 6561.0,
                -25360.0 / 2187.0,
                64448.0 / 6561.0,
                -212.0 / 729.0,
            ],
            &[
                9017.0 / 3168.0,
                -355.0 / 33.0,
                46732.0 / 5247.0,
                49.0 / 176.0,
                -5103.0 / 18656.0,
            ],
            &[
                35.0 / 384.0,
                0.0,
                500.0 / 1113.0,
                125.0 / 192.0,
                -2187.0 / 6784.0,
                11.0 / 84.0,
            ],
        ];
        const B4: [f64; 7] = [
            5179.0 / 57600.0,
            0.0,
            7571.0 / 16695.0,
            393.0 / 640.0,
            -92097.0 / 339200.0,
            187.0 / 2100.0,
            1.0 / 40.0,
        ];

        for (stage, weights) in A.into_iter().enumerate() {
            for (i, body) in bodies.iter().enumerate() {
                let mut phase = Phase {
                    r: body.position,
                    v: body.velocity,
                };
                for (previous, coefficient) in weights.iter().enumerate() {
                    let past = self.stages[previous][i];
                    phase.r += past.r * (dt * coefficient);
                    phase.v += past.v * (dt * coefficient);
                }
                self.proposed[i] = phase;
            }
            let rates = &mut self.stages[stage];
            for (rate, state) in rates.iter_mut().zip(&self.proposed) {
                *rate = Phase {
                    r: state.v,
                    v: DVec3::ZERO,
                };
            }
            for (i, a) in bodies.iter().enumerate() {
                for (j, b) in bodies.iter().enumerate().skip(i + 1) {
                    let separation = self.proposed[j].r - self.proposed[i].r;
                    let distance_sq = separation.length_squared();
                    if !distance_sq.is_finite() || distance_sq < 1e-26 {
                        return f64::INFINITY;
                    }
                    let mut field = separation;
                    if let Some(q) = oblate.filter(|q| i == q.index || j == q.index) {
                        let height = separation.dot(q.axis);
                        field -= (separation * (5.0 * height * height / distance_sq - 1.0)
                            - 2.0 * height * q.axis)
                            * (1.5 * q.j2_radius_sq / distance_sq);
                    }
                    let accel = field * (G / (distance_sq * distance_sq.sqrt()));
                    rates[i].v += accel * b.mass;
                    rates[j].v -= accel * a.mass;
                }
            }
        }

        let mut error = 0.0_f64;
        for (i, &(parent, radius)) in self.scales.iter().enumerate() {
            let reference = parent
                .map(|index| (bodies[index].position, bodies[index].velocity))
                .unwrap_or((DVec3::ZERO, DVec3::ZERO));
            let length = (bodies[i].position - reference.0).length().max(radius);
            let speed = (bodies[i].velocity - reference.1).length();
            let mut dr = DVec3::ZERO;
            let mut dv = DVec3::ZERO;
            for (stage, coefficient) in B4.iter().enumerate() {
                let weight = dt * (A[6].get(stage).copied().unwrap_or(0.0) - coefficient);
                dr += self.stages[stage][i].r * weight;
                dv += self.stages[stage][i].v * weight;
            }
            let e =
                (dr.length() / (1e-13 + 4e-9 * length)).max(dv.length() / (1e-13 + 4e-9 * speed));
            if !e.is_finite() {
                return f64::INFINITY;
            }
            error = error.max(e);
        }
        error
    }

    fn contact(&self, bodies: &[State]) -> Option<(usize, usize)> {
        for (i, a) in bodies.iter().enumerate() {
            for (j, b) in bodies.iter().enumerate().skip(i + 1) {
                let before = b.position - a.position;
                let offset = self.proposed[j].r - self.proposed[i].r - before;
                let t = (-before.dot(offset) / offset.length_squared().max(1e-48)).clamp(0.0, 1.0);
                if (before + offset * t).length_squared()
                    <= (a.radius + b.radius).max(1e-13).powi(2)
                {
                    return Some((i, j));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::star_chart::Companion;
    use std::f64::consts::TAU;

    #[test]
    fn a_measurable_companion_perturbs_the_home_planet() {
        let doc = Document::default();
        let chart = StarChart::default();
        let mut extended = chart.clone();
        let mut orbit = chart.home.orbit.clone();
        orbit.au = 3.0;
        orbit.days = TAU * (27.0 / (G * 1.6)).sqrt();
        orbit.phase = 90.0;
        let mut star = chart.star.clone();
        star.mass_solar = 0.6;
        extended.companions.push(Companion { star, orbit });
        let mut base = Gravity::new(&chart, &doc, 0.0).unwrap();
        let mut pair = Gravity::new(&extended, &doc, 0.0).unwrap();
        let relative =
            |s: &Gravity| s.position(BodyKey::Home).unwrap() - s.position(BodyKey::Star).unwrap();
        assert!((relative(&base) - relative(&pair)).length() < 1e-12);
        assert!((base.advance(25.0, 1024).unwrap() - 25.0).abs() < 1e-8);
        assert!((pair.advance(25.0, 1024).unwrap() - 25.0).abs() < 1e-8);
        assert!((relative(&base) - relative(&pair)).length() > 0.0001);
        let momentum = pair
            .states()
            .iter()
            .map(|s| s.mass * s.velocity)
            .sum::<DVec3>();
        assert!(momentum.length() < 1e-10);
    }

    #[test]
    fn a_collision_stops_before_committing_an_overlapping_step() {
        let body = |key, x, vx| State {
            key,
            position: DVec3::new(x, 0.0, 0.0),
            velocity: DVec3::new(vx, 0.0, 0.0),
            radius: 0.01,
            mass: 1.0,
            name: format!("{key:?}"),
        };
        let mut sim = Gravity::from_states(
            vec![
                body(BodyKey::Star, -0.2, 0.05),
                body(BodyKey::Companion(0), 0.2, -0.05),
            ],
            0.0,
        )
        .unwrap();
        let err = sim.advance(7.0, 250).unwrap_err();
        assert!(err.contains("天体接触"));
        let d = sim.states()[0].position.distance(sim.states()[1].position);
        assert!(d > 0.02);
        assert!(sim.days() > 0.0 && sim.days() < 7.0);
    }

    fn march(sim: &mut Gravity, days: f64) {
        let advanced = sim.advance(days, 40_000).unwrap();
        assert!((advanced - days).abs() < 1e-8, "{advanced}/{days}");
    }

    #[test]
    fn the_earth_moon_system_survives_two_years_at_several_initial_phases() {
        let chart = StarChart::default();
        let doc = Document::default();
        let mut min = f64::INFINITY;
        let mut max = 0.0_f64;
        for epoch in [0.0, 9.0, 21.0, 73.0] {
            let mut sim = Gravity::new(&chart, &doc, epoch).unwrap();
            let start = sim.orbit_reading(BodyKey::Moon).unwrap();
            assert!((start.eccentricity - chart.moon.orbit.eccentricity).abs() < 1e-10);
            for _ in 0..146 {
                march(&mut sim, 5.0);
                let reading = sim.orbit_reading(BodyKey::Moon).unwrap();
                min = min.min(reading.distance_km);
                max = max.max(reading.distance_km);
            }
        }
        println!("Earth–Moon envelope over 730 days per phase: {min:.0}–{max:.0} km");
        assert!(min > 345_000.0 && max < 425_000.0);
    }

    #[test]
    fn the_linked_world_uses_its_actual_mass_to_initialize_the_moon() {
        let chart = StarChart::default();
        let doc = Document::new("质量不同的虚构行星", EARTH_RADIUS_KM * 1.515);
        let mut sim = Gravity::new(&chart, &doc, 9.0).unwrap();
        let reading = sim.orbit_reading(BodyKey::Moon).unwrap();
        let moon = sim.body(BodyKey::Moon).unwrap();
        let home = sim.body(BodyKey::Home).unwrap();
        assert!((home.mass / EARTH_IN_SOLAR_MASSES - 1.515_f64.powi(3)).abs() < 1e-12);
        let old_speed = reading.speed_km_s * chart.moon.orbit.kepler_period(home.mass, moon.mass)
            / chart.moon.orbit.days;
        println!(
            "Linked world: old speed {old_speed:.3} km/s; adjusted speed {:.3} km/s",
            reading.speed_km_s
        );
        assert!(reading.speed_km_s > 1.7 * old_speed);
        assert!((reading.periapsis_km - 363_296.44).abs() < 0.02);
        for _ in 0..73 {
            march(&mut sim, 5.0);
            assert!(sim.orbit_reading(BodyKey::Moon).unwrap().distance_km > 345_000.0);
        }
    }

    #[test]
    fn extra_planet_experiment_starts_identically_and_measures_the_perturbation() {
        let doc = Document::default();
        let mut chart = StarChart::default();
        let mut extra = chart.home.clone();
        extra.name = "测试外来行星".into();
        extra.mass_earth = Some(50.0);
        extra.orbit.au = 1.8;
        extra.orbit.phase = 220.0;
        extra.orbit.ascending_node = 32.0;
        extra.orbit.periapsis_argument = 40.0;
        let key = BodyKey::Planet(chart.planets.len());
        chart.planets.push(extra);
        let mut full = Gravity::new(&chart, &doc, 0.0).unwrap();
        let mut without = full.without(key).unwrap();
        assert!(full
            .differences(&without)
            .iter()
            .all(|row| row.displacement_km < 1e-4));
        march(&mut full, 365.0);
        march(&mut without, 365.0);
        let rows = full.differences(&without);
        let home = rows.iter().find(|p| p.name == doc.meta.name).unwrap();
        let moon = rows.iter().find(|p| p.name == chart.moon.name).unwrap();
        println!(
            "365-day experiment: home displacement {:.2} km, lunar displacement {:.2} km",
            home.displacement_km, moon.displacement_km
        );
        assert!(home.displacement_km > 100.0 && moon.displacement_km > 0.1);
    }

    #[test]
    fn two_body_energy_and_angular_momentum_stay_bounded_over_one_century() {
        let body = |key, position, velocity, mass| State {
            key,
            name: format!("{key:?}"),
            mass,
            radius: 0.0,
            position,
            velocity,
        };
        let m = 3e-6;
        let star = body(BodyKey::Star, DVec3::ZERO, DVec3::ZERO, 1.0);
        let planet = body(
            BodyKey::Home,
            DVec3::new(0.45, 0.0, 0.0),
            DVec3::new(0.0, 0.0, (G * (1.0 + m) * (2.0 / 0.45 - 1.0)).sqrt()),
            m,
        );
        let mut sim = Gravity::from_states(vec![star, planet], 0.0).unwrap();
        let invariants = |sim: &Gravity| {
            let (r, v, mu) = sim.relative(BodyKey::Home).unwrap();
            (v.length_squared() * 0.5 - mu / r.length(), r.cross(v))
        };
        let (e0, h0) = invariants(&sim);
        for _ in 0..100 {
            march(&mut sim, 365.25);
        }
        let (e, h) = invariants(&sim);
        println!(
            "100-year two-body relative drift: energy {:.3e}, angular momentum {:.3e}",
            ((e - e0) / e0).abs(),
            (h - h0).length() / h0.length()
        );
        assert!(((e - e0) / e0).abs() < 8e-7);
        assert!((h - h0).length() / h0.length() < 8e-7);
    }

    #[test]
    fn an_oblate_home_changes_a_low_inclined_satellite_orbit() {
        let doc = Document::default();
        let mut chart = StarChart::default();
        chart.home.tilt = 0.0;
        let mut low = chart.moon.clone();
        low.radius_km = 0.001;
        low.mass_kg = Some(0.0);
        low.orbit.au = (EARTH_RADIUS_KM + 500.0) / AU_KM;
        low.orbit.eccentricity = 0.0;
        low.orbit.inclination = 40.0;
        chart.satellites.push(low);
        let mut sphere = Gravity::new(&chart, &doc, 0.0).unwrap();
        chart.home_j2 = 0.00108263;
        let mut oblate = Gravity::new(&chart, &doc, 0.0).unwrap();
        march(&mut sphere, 2.0);
        march(&mut oblate, 2.0);
        let key = BodyKey::Satellite(0);
        let orbital_normal = |s: &Gravity| {
            let (r, v, _) = s.relative(key).unwrap();
            r.cross(v).normalize()
        };
        assert!((orbital_normal(&sphere) - orbital_normal(&oblate)).length() > 0.05);
        assert!(oblate.orbit_reading(key).unwrap().periapsis_km > EARTH_RADIUS_KM + 420.0);
    }
}
