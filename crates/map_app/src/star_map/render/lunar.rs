//! Local planet and satellite. Sky directions have no local depth or
//! scale; shadow geometry uses the actual dimensions in kilometres.
use super::{dot, label, paint_model, sphere, stellar_color, Camera, Random, SphereStyle, View};
use crate::star_map::StarMap;
use eframe::egui::{self, epaint::Mesh, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke};
use glam::{DVec3, Quat, Vec3};
use map_core::star_chart::{
    disc_light, Body, BodyKey, Moon, Star, AU_KM, EARTH_RADIUS_KM, SUN_RADIUS_KM,
};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::star_map) enum LunarTarget {
    #[default]
    Home,
    Moon,
    Satellite(usize),
}

impl LunarTarget {
    pub(in crate::star_map) fn key(self) -> BodyKey {
        match self {
            Self::Home => BodyKey::Home,
            Self::Moon => BodyKey::Moon,
            Self::Satellite(i) => BodyKey::Satellite(i),
        }
    }
}

struct LocalState {
    light: DVec3,
    moon: DVec3,
    star_r: f64,
    planet_r: f64,
    moon_r: f64,
}

impl LocalState {
    fn new(star: &Star, home: &Body, moon: &Moon, days: f64) -> Self {
        Self {
            light: star_position(home, days),
            moon: DVec3::from_array(moon.position_km(home, days)),
            star_r: star.radius_solar * SUN_RADIUS_KM,
            planet_r: home.radii_earth * EARTH_RADIUS_KM,
            moon_r: moon.radius_km,
        }
    }

    fn shadow(&self, target: LunarTarget) -> Shadow {
        let (center, radius, occ, occ_r) = match target {
            LunarTarget::Home => (DVec3::ZERO, self.planet_r, self.moon, self.moon_r),
            LunarTarget::Moon | LunarTarget::Satellite(_) => {
                (self.moon, self.moon_r, DVec3::ZERO, self.planet_r)
            }
        };
        Shadow {
            center,
            radius,
            occ,
            occ_r,
            light: self.light,
            light_r: self.star_r,
        }
    }
}

fn star_position(home: &Body, days: f64) -> DVec3 {
    DVec3::from_array(home.orbit.position(days)) * -AU_KM
}

#[derive(Clone, Copy)]
pub(super) struct Shadow {
    center: DVec3,
    radius: f64,
    occ: DVec3,
    occ_r: f64,
    light: DVec3,
    light_r: f64,
}

impl Shadow {
    /// Positive outside the outer shadow cone, negative on intersection.
    fn margin(&self) -> f64 {
        let ray = self.occ - self.light;
        let axis = ray.normalize_or_zero();
        let off = self.center - self.occ;
        let behind = off.dot(axis);
        if behind <= 0.0 {
            return self.radius + self.occ_r;
        }
        let penumbra = self.occ_r + (self.light_r + self.occ_r) * behind / ray.length().max(1.0);
        (off - axis * behind).length() - penumbra - self.radius
    }

    pub(super) fn visibility(&self, normal: Vec3) -> f32 {
        let point = self.center + normal.as_dvec3() * self.radius;
        let (sun, occ) = (self.light - point, self.occ - point);
        let (sd, od) = (sun.length(), occ.length());
        if od >= sd || od <= self.occ_r || sun.dot(occ) <= 0.0 {
            return 1.0;
        }
        let sr = (self.light_r / sd).clamp(0.0, 1.0).asin();
        let r = (self.occ_r / od).clamp(0.0, 1.0).asin();
        let separation = (sun.dot(occ) / (sd * od)).clamp(-1.0, 1.0).acos();
        disc_light(sr, r, separation) as f32
    }

    fn day_side_anchor(&self) -> DVec3 {
        let dir = (self.light - self.occ).normalize_or_zero();
        let relative = self.occ - self.center;
        let lateral = relative - relative.dot(dir) * dir;
        let span = (lateral / self.radius).clamp_length_max(0.99);
        span + dir * (1.0 - span.length_squared()).sqrt()
    }
}

pub(in crate::star_map) fn aim_eclipse(
    camera: &mut Camera,
    star: &Star,
    home: &Body,
    moon: &Moon,
    days: f64,
    target: LunarTarget,
) {
    let shadow = LocalState::new(star, home, moon, days).shadow(target);
    let sun = (shadow.light - shadow.center).normalize();
    let anchor = shadow.day_side_anchor();
    let mut direction = (anchor + sun * 0.18).normalize();
    // A view along the light ray lets the foreground sphere obscure the
    // shadow. Move sideways toward the shadow, including for grazing eclipses.
    let minimum_angle = 0.42_f64;
    if direction.dot(sun) > minimum_angle.cos() {
        let sideways = (anchor - sun * anchor.dot(sun))
            .try_normalize()
            .unwrap_or_else(|| sun.cross(DVec3::Y).try_normalize().unwrap_or(DVec3::X));
        direction = sun * minimum_angle.cos() + sideways * minimum_angle.sin();
    }
    camera.yaw = direction.x.atan2(direction.z) as f32;
    camera.pitch = (direction.y.clamp(-1.0, 1.0).asin() as f32).clamp(-1.55, 1.55);
    camera.distance = 11.0;
}

pub(in crate::star_map) fn moon_phase(home: &Body, moon: &Moon, days: f64) -> f64 {
    let moon_at = DVec3::from_array(moon.position_km(home, days));
    phase(moon_at, star_position(home, days))
}

fn phase(moon_at: DVec3, star_at: DVec3) -> f64 {
    let to_star = (star_at - moon_at).normalize_or_zero();
    (1.0 - moon_at.normalize_or_zero().dot(to_star)) * 0.5
}

pub(in crate::star_map) fn displayed_phase(state: &StarMap, home: &Body, moon: &Moon) -> f64 {
    if let Some(driver) = &state.driver {
        let sim = &driver.simulation;
        if let (Some(h), Some(m), Some(s)) = (
            sim.position(BodyKey::Home),
            sim.position(BodyKey::Moon),
            sim.position(BodyKey::Star),
        ) {
            return phase(m - h, s - h);
        }
    }
    moon_phase(home, moon, state.days)
}

/// The next eclipse of the requested body along the current authored orbit.
/// Sample densely enough for the default template then refine each local
/// minimum so brief eclipses between samples are also found.
pub(in crate::star_map) fn event_time(
    star: &Star,
    home: &Body,
    moon: &Moon,
    start: f64,
    target: LunarTarget,
) -> Option<f64> {
    if matches!(target, LunarTarget::Satellite(_)) {
        return None;
    }
    let period = moon.orbit.days.min(home.orbit.days);
    let step = period / 54.0;
    let limit = (home.orbit.days * 2.0)
        .max(moon.orbit.days * 4.0)
        .min(step * 7000.0);
    let margin = |time| {
        LocalState::new(star, home, moon, time)
            .shadow(target)
            .margin()
    };
    let start = start + step * 0.25;
    let mut before = margin(start);
    let mut previous = margin(start + step);
    for i in 2..=(limit / step).ceil() as u32 {
        let time = start + f64::from(i) * step;
        let now = margin(time);
        if previous <= before && previous <= now && (previous < before || previous < now) {
            let (mut a, mut b) = (time - 2.0 * step, time);
            for _ in 0..21 {
                let c = (2.0 * a + b) / 3.0;
                let d = (a + 2.0 * b) / 3.0;
                if margin(c) < margin(d) {
                    b = d;
                } else {
                    a = c;
                }
            }
            let best = (a + b) * 0.5;
            if margin(best) < 0.0 {
                return Some(best);
            }
        }
        before = previous;
        previous = now;
    }
    None
}

fn sky_at(view: &View, direction: Vec3) -> Option<Pos2> {
    let depth = -direction.dot(view.forward);
    if depth <= 0.03 {
        return None;
    }
    Some(
        view.center
            + egui::vec2(direction.dot(view.right), -direction.dot(view.up)) * (view.focal / depth),
    )
}

fn glow(p: &Painter, where_: Pos2, radius: f32, color: Color32) {
    for (scale, alpha) in [(2.4, 11), (1.7, 23), (1.21, 45)] {
        p.circle_filled(
            where_,
            radius * scale,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
        );
    }
    p.circle_filled(where_, radius, color);
}

fn sky(p: &Painter, rect: Rect, view: &View, star: &Star, sun_direction: Vec3, labels: bool) {
    p.rect_filled(rect, 0.0, Color32::from_rgb(5, 9, 19));
    // A fixed celestial sphere. Camera zoom and following a local body move
    // only the foreground, so these marks never acquire parallax.
    let mut rng = Random(0x87ac_78fd);
    let mut pins = Mesh::default();
    for i in 0..930 {
        let yaw = rng.unit() * TAU;
        let y = 2.0 * rng.unit() - 1.0;
        let orbit = (1.0 - y * y).sqrt();
        let direction = Vec3::new(yaw.sin() * orbit, y, yaw.cos() * orbit);
        let size = 0.35 + rng.unit().powi(4) * 1.4;
        let shade = 50 + (rng.unit() * 113.0) as u8;
        let Some(pos) = sky_at(view, direction).filter(|q| rect.expand(12.0).contains(*q)) else {
            continue;
        };
        if i < 38 {
            // A few unresolved distant galaxies, drawn in the same sky pass.
            let hue = Color32::from_rgba_unmultiplied(132, 153, 219, 70);
            p.line_segment(
                [pos - egui::vec2(8.0, 2.5), pos + egui::vec2(8.0, 2.5)],
                Stroke::new(3.5, hue),
            );
            dot(&mut pins, pos, 2.5, Color32::from_rgb(183, 191, 242));
        } else {
            dot(
                &mut pins,
                pos,
                size,
                Color32::from_rgb(shade, shade + 8, shade + 18),
            );
        }
    }
    p.add(egui::Shape::mesh(pins));
    let sun_color = stellar_color(star.temperature);
    if let Some(pos) = sky_at(view, sun_direction).filter(|q| rect.shrink(16.0).contains(*q)) {
        glow(p, pos, 13.0, sun_color);
        if labels {
            p.text(
                pos + egui::vec2(19.0, 0.0),
                Align2::LEFT_CENTER,
                &star.name,
                FontId::proportional(13.0),
                sun_color,
            );
        }
    } else {
        // Retain a direction cue when the star itself lies outside the field.
        let mut axis = egui::vec2(sun_direction.dot(view.right), -sun_direction.dot(view.up));
        if axis.length_sq() < 0.00001 {
            axis = egui::vec2(1.0, 0.0);
        }
        let edge = rect.shrink(40.0);
        let t = ((edge.width() * 0.5) / axis.x.abs().max(0.00001))
            .min((edge.height() * 0.5) / axis.y.abs().max(0.00001));
        let pos = edge.center() + axis * t;
        let normalized = axis.normalized();
        let side = egui::vec2(-normalized.y, normalized.x);
        p.add(egui::Shape::convex_polygon(
            vec![
                pos + normalized * 10.0,
                pos - normalized * 4.0 + side * 5.0,
                pos - normalized * 4.0 - side * 5.0,
            ],
            sun_color,
            Stroke::NONE,
        ));
        if labels {
            p.text(
                pos - normalized * 28.0,
                Align2::CENTER_CENTER,
                &star.name,
                FontId::proportional(12.0),
                sun_color,
            );
        }
    }
}

fn satellite_radius(planet_r: f64, satellite_r: f64, home_display: f32) -> f32 {
    ((satellite_r / planet_r).powf(0.78) as f32 * home_display).clamp(0.063, 2.1)
}

fn satellite_display(
    physical: DVec3, planet_r: f64, satellite_r: f64, reference_km: f64,
    home_display: f32, linear: bool,
) -> Vec3 {
    // The illustration exaggerates radii. Compress the clearance between
    // surfaces, so a close pass can look close without drawing false contact.
    let height = (physical.length() - planet_r - satellite_r).max(0.0);
    let reference_height = (reference_km - planet_r - satellite_r).max(1.0);
    let scale = if linear {
        height / reference_height
    } else {
        let unit = (planet_r * 0.45).max(250.0);
        (height / unit).ln_1p() / (reference_height / unit).ln_1p()
    };
    let contact = f64::from(home_display + satellite_radius(planet_r, satellite_r, home_display));
    (physical.normalize_or_zero() * (contact + (5.6 - contact).max(0.5) * scale)).as_vec3()
}

fn spin(tilt: f64, turns: f64, locked_to: Option<Vec3>) -> Quat {
    let tilt = Quat::from_rotation_x((tilt as f32).to_radians());
    let heading = if let Some(direction) = locked_to {
        let local = tilt.inverse() * direction;
        local.x.atan2(local.z)
    } else {
        turns.rem_euclid(1.0) as f32 * TAU
    };
    tilt * Quat::from_rotation_y(heading)
}

pub(in crate::star_map) fn local_system(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    state: &StarMap,
    star: &Star,
    home: &Body,
    moon: &Moon,
    satellites: &[Moon],
) -> Option<LunarTarget> {
    let p = ui.painter_at(rect);
    let mut local = LocalState::new(star, home, moon, state.days);
    let home_at = state.position(BodyKey::Home, || {
        DVec3::from(home.orbit.position(state.days))
    });
    local.light = (state.position(BodyKey::Star, || DVec3::ZERO) - home_at) * AU_KM;
    let satellite_km = |target: LunarTarget, moon: &Moon| {
        (state.position(target.key(), || {
            home_at + DVec3::from(moon.position_km(home, state.days)) / AU_KM
        }) - home_at)
            * AU_KM
    };
    local.moon = satellite_km(LunarTarget::Moon, moon);
    let natural_only = satellites.is_empty();
    let home_r = ((local.planet_r / EARTH_RADIUS_KM).powf(0.40) as f32 * 1.28).clamp(0.25, 3.2);
    let moons: Vec<_> = std::iter::once((LunarTarget::Moon, moon))
        .chain(
            satellites
                .iter()
                .enumerate()
                .map(|(i, m)| (LunarTarget::Satellite(i), m)),
        )
        .map(|(target, m)| {
            let physical = satellite_km(target, m);
            (
                target,
                m,
                physical,
                satellite_display(physical, local.planet_r, m.radius_km, moon.distance_km(), home_r, natural_only),
            )
        })
        .collect();
    let camera_center = if state.lunar_follow {
        moons
            .iter()
            .find(|(t, _, _, _)| *t == state.lunar_target)
            .map_or(Vec3::ZERO, |m| m.3)
    } else {
        Vec3::ZERO
    };
    let view = state.lunar_camera.view(rect, camera_center);
    let sun_direction = local.light.normalize_or_zero().as_vec3();
    sky(&p, rect, &view, star, sun_direction, state.labels);
    if state.orbits && state.driver.is_none() {
        for (_, satellite, _, _) in &moons {
            let stroke = Stroke::new(0.9, Color32::from_rgb(66, 96, 139));
            let mut path = Vec::new();
            for i in 0..=144 {
                let moon_km = DVec3::from_array(
                    satellite.position_km(home, f64::from(i) * satellite.orbit.days / 144.0),
                );
                if let Some((at, _)) = view.project(satellite_display(
                    moon_km,
                    local.planet_r,
                    satellite.radius_km,
                    moon.distance_km(),
                    home_r,
                    natural_only,
                )) {
                    path.push(at);
                } else if path.len() > 1 {
                    p.add(egui::Shape::line(std::mem::take(&mut path), stroke));
                } else {
                    path.clear();
                }
            }
            if path.len() > 1 {
                p.add(egui::Shape::line(path, stroke));
            }
        }
    }
    let texture = state.surface.as_ref().map(|s| s.image.id());
    let mut objects = std::iter::once((
        LunarTarget::Home,
        &home.name,
        Vec3::ZERO,
        home_r,
        home.color,
        texture,
        home.model,
        None,
    ))
    .chain(moons.iter().map(|&(target, satellite, _, pos)| {
        let radius = satellite_radius(local.planet_r, satellite.radius_km, home_r);
        let artificial = satellite.mass_kg.is_some_and(|m| m < 1e10);
        (
            target,
            &satellite.name,
            pos,
            radius,
            if artificial {
                [169, 192, 229]
            } else {
                [197, 197, 197]
            },
            None,
            satellite.model,
            Some(satellite),
        )
    }))
    .filter_map(
        |(target, name, pos, display_radius, color, texture, model, satellite)| {
            let (center, depth) = view.project(pos)?;
            let radius =
                (view.focal * display_radius / depth).min(rect.width().max(rect.height()) * 1.8);
            Some((
                target, name, center, depth, radius, color, texture, model, satellite, pos,
            ))
        },
    )
    .collect::<Vec<_>>();
    objects.sort_by(|a, b| b.3.total_cmp(&a.3));
    let mut occupied = objects
        .iter()
        .map(|o| Rect::from_center_size(o.2, egui::Vec2::splat(1.8 * o.4)))
        .collect();
    for (target, name, center, depth, radius, color, texture, model, satellite, pos) in &objects {
        let physical = moons
            .iter()
            .find(|(t, _, _, _)| t == target)
            .map_or(DVec3::ZERO, |m| m.2);
        let radius_km = satellite.map_or(local.planet_r, |s| s.radius_km);
        let shadow = std::iter::once((LunarTarget::Home, DVec3::ZERO, local.planet_r))
            .chain(moons.iter().map(|&(t, m, p, _)| (t, p, m.radius_km)))
            .filter(|(t, _, _)| t != target)
            .map(|(_, occ, occ_r)| Shadow {
                center: physical,
                radius: radius_km,
                occ,
                occ_r,
                light: local.light,
                light_r: local.star_r,
            })
            .min_by(|a, b| a.margin().total_cmp(&b.margin()))
            .unwrap_or_else(|| local.shadow(*target));
        let light = (shadow.light - shadow.center).normalize_or_zero().as_vec3();
        let eclipse = (state.lighting && shadow.margin() < 0.0).then_some(shadow);
        let is_moon = satellite.is_some();
        let rotation = if let Some(satellite) = satellite {
            spin(
                0.0,
                state.days / satellite.spin_days,
                satellite.synchronous.then_some(-pos.normalize_or_zero()),
            )
        } else {
            let dir = if home.retrograde { -1.0 } else { 1.0 };
            spin(
                home.tilt,
                dir * state.days * 24.0 / home.spin_hours,
                home.synchronous.then_some(sun_direction),
            )
        };
        if !paint_model(
            state,
            &p,
            &view,
            *model,
            *pos,
            *radius * *depth / view.focal,
            rotation,
            state.playing || response.dragged(),
        ) {
            sphere(
                &p,
                &view,
                *center,
                *radius,
                SphereStyle {
                    rotation,
                    light,
                    texture: *texture,
                    lighting: state.lighting,
                    color: *color,
                    eclipse,
                    rocky: is_moon,
                },
            );
        }
        let stroke = if *target == state.lunar_target {
            Stroke::new(1.25, Color32::from_rgb(122, 203, 243))
        } else {
            Stroke::new(0.85, Color32::from_rgb(65, 80, 113))
        };
        p.circle_stroke(*center, *radius + 1.5, stroke);
        if state.labels {
            label(
                &p,
                rect,
                *center,
                *radius + 7.0,
                name,
                Color32::from_rgb(197, 218, 243),
                &mut occupied,
            );
        }
    }
    let at = response
        .interact_pointer_pos()
        .filter(|_| response.clicked())?;
    objects
        .iter()
        .rev()
        .find(|o| at.distance(o.2) <= o.4 + 4.0)
        .map(|o| o.0)
}

pub(super) fn lunar_albedo(normal: Vec3) -> f32 {
    let [x, y, z] = normal.to_array();
    let mut brightness = 0.89 + 0.025 * (x * 59.0 + z * 37.0).sin() * (y * 57.0 - z * 29.0).cos();
    for (direction, extent) in [
        (Vec3::new(0.46, 0.46, 0.76), 0.23),
        (Vec3::new(-0.59, 0.17, 0.79), 0.19),
        (Vec3::new(0.73, -0.38, -0.57), 0.21),
    ] {
        let dist = 1.0 - normal.dot(direction.normalize());
        brightness -= 0.20 * (1.0 - dist / extent).clamp(0.0, 1.0);
    }
    for (direction, extent) in [
        (Vec3::new(-0.27, -0.62, 0.74), 0.018),
        (Vec3::new(0.50, 0.21, 0.84), 0.024),
        (Vec3::new(-0.22, 0.37, -0.90), 0.019),
    ] {
        let distance = (1.0 - normal.dot(direction.normalize())) / extent;
        brightness +=
            0.10 * (1.0 - (distance - 1.0).abs() * 2.6).max(0.0) - 0.08 * (1.0 - distance).max(0.0);
    }
    brightness.clamp(0.56, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::star_chart::StarChart;

    #[test]
    fn distant_sky_is_independent_of_local_zoom_and_follow_target() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0));
        let mut camera = Camera::lunar();
        let first = camera.view(rect, Vec3::ZERO);
        let direction = (-first.forward + first.right * 0.15).normalize();
        camera.distance *= 2.5;
        let following = camera.view(rect, Vec3::new(4.0, 0.0, 2.0));
        assert_eq!(sky_at(&first, direction), sky_at(&following, direction));
    }

    #[test]
    fn moon_shadows_distinctly_at_conjunctions() {
        let chart = StarChart::default();
        for target in [LunarTarget::Home, LunarTarget::Moon] {
            let t = event_time(&chart.star, &chart.home, &chart.moon, 0.0, target)
                .expect("eclipse with the default orbit");
            let s = LocalState::new(&chart.star, &chart.home, &chart.moon, t).shadow(target);
            assert!(s.margin() < 0.0);
            let mut camera = Camera::lunar();
            aim_eclipse(
                &mut camera,
                &chart.star,
                &chart.home,
                &chart.moon,
                t,
                target,
            );
            let sight = camera.view(
                Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0)),
                Vec3::ZERO,
            );
            assert!(
                sight.forward.as_dvec3().dot(s.day_side_anchor()) > 0.25,
                "shadow outside the view for {target:?} on day {t}"
            );
            let phase = moon_phase(&chart.home, &chart.moon, t);
            assert!(if target == LunarTarget::Home {
                phase < 0.02
            } else {
                phase > 0.98
            });
            let real_light = (s.light - s.center).normalize();
            let blocked = if target == LunarTarget::Moon {
                s.visibility(real_light.as_vec3())
            } else {
                // Nearest point to the moon's shadow axis on the day side.
                let towards_occ = (s.occ - s.center).normalize();
                let lateral = (s.occ - s.center) - real_light * (s.occ - s.center).dot(real_light);
                let p = lateral / s.radius;
                let n = if p.length_squared() < 1.0 {
                    p + real_light * (1.0 - p.length_squared()).sqrt()
                } else {
                    towards_occ.cross(real_light).cross(-real_light).normalize()
                };
                s.visibility(n.as_vec3())
            };
            assert!(blocked < 0.95, "visible light fraction {blocked}");
        }
    }
}
