//! Newtonian mutual gravitation in AU, Earth days and solar masses.
use super::{Body, StarChart, AU_KM, EARTH_RADIUS_KM, SUN_RADIUS_KM};
use crate::Document;
use glam::DVec3;

/// Nominal solar gravity parameter, converted from SI to AU³/day².
pub const G: f64 =
    1.327_124_4e20 * 86_400.0 * 86_400.0 / (AU_KM * 1000.0 * AU_KM * 1000.0 * AU_KM * 1000.0);
const SOLAR_MASS_KG: f64 = 1.988_41e30;
const EARTH_MASS_KG: f64 = 5.972_2e24;

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

#[derive(Clone, Debug)]
pub struct State {
    pub key: BodyKey,
    pub name: String,
    pub mass: f64,
    pub radius: f64,
    pub position: DVec3,
    pub velocity: DVec3,
}

pub struct Gravity {
    bodies: Vec<State>,
    days: f64,
}

impl Body {
    /// Measured mass for the reference planets. A custom object gets a
    /// radius-based starting value until the author specifies a mass.
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

impl Gravity {
    pub fn new(chart: &StarChart, doc: &Document, days: f64) -> Result<Self, String> {
        chart.validate()?;
        let (star, home) = chart.resolved(doc);
        let mut bodies = vec![State {
            key: BodyKey::Star,
            name: star.name.clone(),
            mass: star.mass_solar,
            radius: star.radius_solar * SUN_RADIUS_KM / AU_KM,
            position: DVec3::ZERO,
            velocity: DVec3::ZERO,
        }];
        for (key, planet) in std::iter::once((BodyKey::Home, &home)).chain(
            chart
                .planets
                .iter()
                .enumerate()
                .map(|(i, p)| (BodyKey::Planet(i), p)),
        ) {
            let (pos, vel) = planet.orbit.state(days);
            bodies.push(State {
                key,
                name: planet.name.clone(),
                mass: planet.effective_mass_earth() * EARTH_MASS_KG / SOLAR_MASS_KG,
                radius: planet.radii_earth * EARTH_RADIUS_KM / AU_KM,
                position: DVec3::from(pos),
                velocity: DVec3::from(vel),
            });
        }
        for (i, companion) in chart.companions.iter().enumerate() {
            let (pos, vel) = companion.orbit.state(days);
            let s = &companion.star;
            bodies.push(State {
                key: BodyKey::Companion(i),
                name: s.name.clone(),
                mass: s.mass_solar,
                radius: s.radius_solar * SUN_RADIUS_KM / AU_KM,
                position: DVec3::from(pos),
                velocity: DVec3::from(vel),
            });
        }
        let (home_position, home_velocity) = (bodies[1].position, bodies[1].velocity);
        let tilt = glam::DQuat::from_rotation_x(-home.orbit.inclination.to_radians());
        for (key, satellite) in std::iter::once((BodyKey::Moon, &chart.moon)).chain(
            chart
                .satellites
                .iter()
                .enumerate()
                .map(|(i, m)| (BodyKey::Satellite(i), m)),
        ) {
            let (pos, vel) = satellite.orbit.state(days);
            let kg = satellite
                .mass_kg
                .unwrap_or(7.342e22 * (satellite.radius_km / 1737.4).powi(3));
            bodies.push(State {
                key,
                name: satellite.name.clone(),
                mass: kg / SOLAR_MASS_KG,
                radius: satellite.radius_km / AU_KM,
                position: home_position + tilt * DVec3::from(pos),
                velocity: home_velocity + tilt * DVec3::from(vel),
            });
        }
        Self::from_states(bodies, days)
    }

    /// Re-center the initial state on its barycenter and zero the total
    /// momentum. Relative positions and velocities are unchanged.
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
        let sim = Self { bodies, days };
        sim.forces(&sim.positions())?;
        Ok(sim)
    }

    pub fn days(&self) -> f64 {
        self.days
    }
    pub fn states(&self) -> &[State] {
        &self.bodies
    }
    pub fn position(&self, key: BodyKey) -> Option<DVec3> {
        self.bodies
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.position)
    }

    fn positions(&self) -> Vec<DVec3> {
        self.bodies.iter().map(|p| p.position).collect()
    }

    fn forces(&self, positions: &[DVec3]) -> Result<(Vec<DVec3>, f64), String> {
        let mut accelerations = vec![DVec3::ZERO; self.bodies.len()];
        let mut step = 3.0_f64;
        for (i, a) in self.bodies.iter().enumerate() {
            for (j, b) in self.bodies.iter().enumerate().skip(i + 1) {
                let delta = positions[j] - positions[i];
                let d = delta.length();
                if !d.is_finite() {
                    return Err("天体坐标超出数值范围。".into());
                }
                if d <= a.radius + b.radius || d < 1e-12 {
                    return Err(format!(
                        "检测到天体接触：{} 与 {}。模拟已暂停。",
                        a.name, b.name
                    ));
                }
                let distance_cubed = d * d * d;
                let force = G * delta / distance_cubed;
                accelerations[i] += force * b.mass;
                accelerations[j] -= force * a.mass;
                let motion = (a.velocity - b.velocity).length();
                step = step.min(0.04 * d / motion.max(1e-30));
                if a.mass + b.mass > 0.0 {
                    step = step.min(0.04 * (distance_cubed / (G * (a.mass + b.mass))).sqrt());
                }
            }
        }
        Ok((accelerations, step))
    }

    /// Velocity Verlet with a local step bound. Return actual elapsed days
    /// so the display can slow down rather than jumping through encounters.
    pub fn advance(&mut self, span: f64, max_steps: usize) -> Result<f64, String> {
        if !span.is_finite() || span < 0.0 {
            return Err("引力模拟时间步无效。".into());
        }
        let mut done = 0.0;
        for _ in 0..max_steps {
            if span - done <= 1e-12 {
                break;
            }
            let (first, limit) = self.forces(&self.positions())?;
            let dt = (span - done).min(limit);
            if dt <= 1e-10 {
                return Err("天体过于接近，无法在稳定的时间步内继续模拟。".into());
            }
            let proposed = self
                .bodies
                .iter()
                .zip(&first)
                .map(|(p, a)| p.position + p.velocity * dt + a * (0.5 * dt * dt))
                .collect::<Vec<_>>();
            let (second, _) = self.forces(&proposed)?;
            for (i, p) in self.bodies.iter_mut().enumerate() {
                p.position = proposed[i];
                p.velocity += (first[i] + second[i]) * (0.5 * dt);
            }
            done += dt;
            self.days += dt;
        }
        Ok(done)
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
        let mut base = Gravity::new(&chart,&doc,0.0).unwrap();
        let mut pair = Gravity::new(&extended,&doc,0.0).unwrap();
        let relative = |s:&Gravity| s.position(BodyKey::Home).unwrap()-s.position(BodyKey::Star).unwrap();
        assert!((relative(&base)-relative(&pair)).length() < 1e-12);
        assert!((base.advance(25.0,1024).unwrap()-25.0).abs() < 1e-8);
        assert!((pair.advance(25.0,1024).unwrap()-25.0).abs() < 1e-8);
        assert!((relative(&base)-relative(&pair)).length() > 0.0001);
        let momentum = pair.states().iter().map(|s|s.mass*s.velocity).sum::<DVec3>();
        assert!(momentum.length()<1e-10);
    }

    #[test]
    fn a_collision_stops_before_committing_an_overlapping_step() {
        let body = |key, x, vx| State { key, position:DVec3::new(x,0.0,0.0),
            velocity:DVec3::new(vx,0.0,0.0), radius:0.01,mass:1.0,name:format!("{key:?}") };
        let mut sim = Gravity::from_states(vec![body(BodyKey::Star,-0.2,0.05),body(BodyKey::Companion(0),0.2,-0.05)],0.0).unwrap();
        let err = sim.advance(7.0,250).unwrap_err();
        assert!(err.contains("天体接触"));
        let d = sim.states()[0].position.distance(sim.states()[1].position);
        assert!(d > 0.02);
        assert!(sim.days()>0.0 && sim.days()<7.0);
    }
}
