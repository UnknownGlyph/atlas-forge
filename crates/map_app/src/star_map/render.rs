use super::{Body, Galaxy, Star, StarMap, Target};
use eframe::egui::{
    self,
    epaint::{Mesh, Vertex, WHITE_UV},
    Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, TextureId,
};
use glam::{Quat, Vec3};
use std::f32::consts::{PI, TAU};

mod lunar;
pub(super) use lunar::{aim_eclipse, event_time, local_system, moon_phase, LunarTarget};

const SYSTEM_DISTANCE: f32 = 29.0;

#[derive(Clone, Copy)]
pub(super) struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
    base: f32,
}
impl Camera {
    pub(super) fn system() -> Self {
        Self {
            yaw: -0.42,
            pitch: 0.62,
            distance: SYSTEM_DISTANCE,
            base: SYSTEM_DISTANCE,
        }
    }
    pub(super) fn galaxy() -> Self {
        Self {
            yaw: -0.28,
            pitch: 0.98,
            distance: 125.0,
            base: 125.0,
        }
    }

    pub(super) fn lunar() -> Self {
        Self {
            yaw: -0.58,
            pitch: 0.39,
            distance: 16.5,
            base: 16.5,
        }
    }

    pub(super) fn face_star(&mut self, home: &Body, days: f64) {
        let eye_direction =
            Vec3::from_array(home.orbit.position(days).map(|x| x as f32)).normalize();
        self.yaw = eye_direction.x.atan2(eye_direction.z) + 0.24;
        self.pitch = eye_direction.y.asin().clamp(-1.55, 1.55);
    }

    pub(super) fn limit_local_zoom(&mut self) {
        self.distance = self.distance.max(3.4);
    }

    pub(super) fn input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
        if response.dragged() {
            let delta = response.drag_delta();
            self.yaw = (self.yaw - delta.x / rect.width().max(160.0) * 4.8).rem_euclid(TAU);
            self.pitch = (self.pitch + delta.y / rect.height().max(160.0) * 3.6).clamp(-1.55, 1.55);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            self.distance = (self.distance * (-0.0025 * scroll).exp())
                .clamp(self.base * 0.065, self.base * 5.0);
        }
    }

    fn view(self, rect: Rect, target: Vec3) -> View {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        let forward = Vec3::new(sy * cp, sp, cy * cp);
        let right = Vec3::Y.cross(forward).normalize();
        let up = forward.cross(right);
        View {
            eye: target + forward * self.distance,
            forward,
            right,
            up,
            center: rect.center(),
            focal: rect.width().min(rect.height()) * 1.15,
        }
    }
}

struct View {
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    center: Pos2,
    focal: f32,
}
impl View {
    fn project(&self, point: Vec3) -> Option<(Pos2, f32)> {
        let q = point - self.eye;
        let z = -q.dot(self.forward);
        (z > 0.05).then(|| {
            let k = self.focal / z;
            (
                self.center + egui::vec2(q.dot(self.right) * k, -q.dot(self.up) * k),
                z,
            )
        })
    }
}

fn backdrop(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(6, 10, 20));
    let mut rng = Random(17);
    let mut points = Mesh::default();
    for _ in 0..220 {
        let p = Pos2::new(
            rect.left() + rng.unit() * rect.width(),
            rect.top() + rng.unit() * rect.height(),
        );
        let size = 0.28 + rng.unit().powi(4) * 0.75;
        let light = (28.0 + rng.unit() * 53.0) as u8;
        dot(
            &mut points,
            p,
            size,
            Color32::from_rgb(light, light + 4, light + 14),
        );
    }
    painter.add(egui::Shape::mesh(points));
}

fn orbit_position(body: &Body, days: f64) -> Vec3 {
    let [x, y, z] = body.orbit.position(days);
    let point = Vec3::new(x as f32, y as f32, z as f32);
    let distance = point.length();
    point * (screen_scale(distance) / distance.max(0.0001))
}

fn screen_scale(au: f32) -> f32 {
    2.75 * au.mul_add(2.65, 1.0).ln()
}

fn orbit_curve(p: &Painter, view: &View, body: &Body) {
    let mut line = Vec::new();
    let stroke = Stroke::new(0.85, Color32::from_rgb(55, 78, 111));
    for i in 0..=120 {
        let pos = orbit_position(body, body.orbit.days * f64::from(i) / 120.0);
        if let Some((s, _)) = view.project(pos) {
            line.push(s);
        } else if line.len() > 1 {
            p.add(egui::Shape::line(std::mem::take(&mut line), stroke));
        } else {
            line.clear();
        }
    }
    if line.len() > 1 {
        p.add(egui::Shape::line(line, stroke));
    }
}

struct Visible<'a> {
    id: Target,
    name: &'a str,
    p: Pos2,
    z: f32,
    r: f32,
    world: Vec3,
    body: Option<&'a Body>,
}

pub(super) fn system(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    state: &StarMap,
    star: &Star,
    home: &Body,
    planets: &[Body],
) -> Option<Target> {
    let painter = ui.painter_at(rect);
    backdrop(&painter, rect);
    let entries = std::iter::once((Target::Home, home)).chain(
        planets
            .iter()
            .enumerate()
            .map(|(i, p)| (Target::Planet(i), p)),
    );
    let focus = if state.follow {
        entries
            .clone()
            .find(|(t, _)| *t == state.target)
            .map_or(Vec3::ZERO, |(_, p)| orbit_position(p, state.days))
    } else {
        Vec3::ZERO
    };
    let view = state.system_camera.view(rect, focus);
    if state.orbits {
        for (_, body) in entries.clone() {
            orbit_curve(&painter, &view, body);
        }
    }
    let mut visible: Vec<_> = entries
        .filter_map(|(id, b)| {
            let world = orbit_position(b, state.days);
            let (p, z) = view.project(world)?;
            let chosen = id == state.target && state.follow;
            let r = (5.3 * b.radii_earth.powf(0.34) as f32 * SYSTEM_DISTANCE / z)
                .clamp(3.2, if chosen { 74.0 } else { 40.0 });
            let r = if id == Target::Home {
                r.max(if chosen { 48.0 } else { 11.0 })
            } else {
                r
            };
            Some(Visible {
                id,
                name: &b.name,
                p,
                z,
                r,
                world,
                body: Some(b),
            })
        })
        .collect();
    if let Some((p, z)) = view.project(Vec3::ZERO) {
        let r =
            (18.0 * (star.radius_solar as f32).powf(0.26) * SYSTEM_DISTANCE / z).clamp(8.0, 73.0);
        visible.push(Visible {
            id: Target::Star,
            name: &star.name,
            p,
            z,
            r,
            world: Vec3::ZERO,
            body: None,
        });
    }
    visible.sort_by(|a, b| b.z.total_cmp(&a.z));
    let mut occupied: Vec<Rect> = visible
        .iter()
        .map(|v| Rect::from_center_size(v.p, egui::Vec2::splat(v.r * 1.7)))
        .collect();
    for v in &visible {
        if let Some(body) = v.body {
            let light = (-v.world).normalize_or_zero();
            let direction = if body.retrograde { -1.0 } else { 1.0 };
            let tilt = Quat::from_rotation_x((body.tilt as f32).to_radians());
            let spin = if body.synchronous {
                let local = tilt.inverse() * light;
                local.x.atan2(local.z)
            } else {
                (state.days * 24.0 / body.spin_hours).rem_euclid(1.0) as f32 * TAU * direction
            };
            let rotation = tilt * Quat::from_rotation_y(spin);
            if body.rings {
                rings(&painter, &view, v, rotation, false);
            }
            let texture = (v.id == Target::Home)
                .then(|| state.surface.as_ref().map(|s| s.image.id()))
                .flatten();
            sphere(
                &painter,
                &view,
                v.p,
                v.r,
                SphereStyle {
                    rotation,
                    light,
                    texture,
                    lighting: state.lighting,
                    color: body.color,
                    eclipse: None,
                    rocky: false,
                },
            );
            if body.rings {
                rings(&painter, &view, v, rotation, true);
            }
        } else {
            let color = stellar_color(star.temperature);
            let glow = (8.0 + 4.0 * star.luminosity_solar.clamp(0.01, 250.0).ln().max(0.0)) as u8;
            for (scale, alpha) in [(2.4, glow / 3), (1.7, glow / 2), (1.28, glow)] {
                painter.circle_filled(
                    v.p,
                    v.r * scale,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
                );
            }
            painter.circle_filled(v.p, v.r, color);
            painter.circle_filled(
                v.p,
                v.r * 0.72,
                Color32::from_rgba_unmultiplied(255, 245, 225, 55),
            );
        }
        if v.id == state.target {
            painter.circle_stroke(
                v.p,
                v.r + 3.5,
                Stroke::new(1.2, Color32::from_rgb(127, 204, 251)),
            );
        }
    }
    if state.labels {
        for v in visible.iter().rev() {
            let color = if v.id == state.target {
                Color32::from_rgb(167, 221, 255)
            } else {
                Color32::from_rgb(195, 206, 225)
            };
            label(&painter, rect, v.p, v.r + 6.0, v.name, color, &mut occupied);
        }
    }
    response
        .interact_pointer_pos()
        .filter(|_| response.clicked())
        .and_then(|pointer| {
            visible
                .iter()
                .rev()
                .find(|v| pointer.distance(v.p) < v.r.max(8.0) + 4.0)
                .map(|v| v.id)
        })
}

fn stellar_color(k: f64) -> Color32 {
    let warm = Color32::from_rgb(255, 155, 96);
    let neutral = Color32::from_rgb(255, 225, 162);
    let cool = Color32::from_rgb(161, 202, 255);
    let blend = |a: Color32, b: Color32, t: f64| {
        let c = |x: u8, y: u8| (f64::from(x) * (1.0 - t) + f64::from(y) * t).round() as u8;
        Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
    };
    if k < 5772.0 {
        blend(warm, neutral, ((k - 2200.0) / 3572.0).clamp(0.0, 1.0))
    } else {
        blend(neutral, cool, ((k - 5772.0) / 6500.0).clamp(0.0, 1.0))
    }
}

struct SphereStyle {
    rotation: Quat,
    light: Vec3,
    texture: Option<TextureId>,
    lighting: bool,
    color: [u8; 3],
    eclipse: Option<lunar::Shadow>,
    rocky: bool,
}
fn sphere(painter: &Painter, view: &View, center: Pos2, radius: f32, style: SphereStyle) {
    let SphereStyle {
        rotation,
        light,
        texture,
        lighting,
        color,
        eclipse,
        rocky,
    } = style;
    let (rings, segments) = if eclipse.is_some() {
        (34, 100)
    } else if radius < 9.0 {
        (5, 18)
    } else {
        (13, 48)
    };
    let rotation = rotation.inverse();
    let vertex = |ring: usize, segment: usize| {
        let radial = ring as f32 / rings as f32;
        let angle = TAU * segment as f32 / segments as f32;
        let (sin, cos) = angle.sin_cos();
        let normal = view.right * (cos * radial)
            + view.up * (sin * radial)
            + view.forward * (1.0 - radial * radial).max(0.0).sqrt();
        let local = rotation * normal;
        let lon = local.x.atan2(local.z);
        let lat = local.y.clamp(-1.0, 1.0).asin();
        let illumination = if lighting {
            let lit = normal.dot(light).max(0.0).sqrt();
            let visible = if lit > 0.0 {
                eclipse.as_ref().map_or(1.0, |s| s.visibility(normal))
            } else {
                1.0
            };
            0.24 + 0.76 * lit * visible
        } else {
            1.0
        };
        let (rgb, uv) = if texture.is_some() {
            ([255; 3], Pos2::new(lon / TAU + 0.5, 0.5 - lat / PI))
        } else {
            let mottling = if rocky {
                lunar::lunar_albedo(local)
            } else {
                0.93 + 0.07 * (lat * 13.0 + (lon * 3.0).sin()).sin()
            };
            (color.map(|c| (f32::from(c) * mottling) as u8), WHITE_UV)
        };
        let [r, g, b] = rgb.map(|c| (f32::from(c) * illumination).round() as u8);
        Vertex {
            pos: center + egui::vec2(cos * radius * radial, -sin * radius * radial),
            uv,
            color: Color32::from_rgb(r, g, b),
        }
    };
    let mut mesh = Mesh::with_texture(texture.unwrap_or_default());
    let rows: Vec<_> = (0..=rings)
        .map(|r| (0..segments).map(|s| vertex(r, s)).collect::<Vec<_>>())
        .collect();
    let mut tri = |mut vertices: [Vertex; 3]| {
        if texture.is_some() {
            let min = vertices.iter().map(|v| v.uv.x).fold(1.0, f32::min);
            let max = vertices.iter().map(|v| v.uv.x).fold(0.0, f32::max);
            if max - min > 0.5 {
                for v in &mut vertices {
                    if v.uv.x < 0.5 {
                        v.uv.x += 1.0;
                    }
                }
            }
        }
        let start = mesh.vertices.len() as u32;
        mesh.vertices.extend(vertices);
        mesh.add_triangle(start, start + 1, start + 2);
    };
    for r in 0..rings {
        for s in 0..segments {
            let t = (s + 1) % segments;
            let [a, b, c, d] = [rows[r][s], rows[r + 1][s], rows[r + 1][t], rows[r][t]];
            tri([a, b, c]);
            if r != 0 {
                tri([a, c, d]);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn rings(painter: &Painter, view: &View, body: &Visible<'_>, rotation: Quat, front: bool) {
    let tilt = rotation * Quat::from_rotation_x(0.17);
    for scale in [1.45, 1.58, 1.78] {
        let point = |i: usize| {
            let (s, c) = (TAU * i as f32 / 64.0).sin_cos();
            tilt * Vec3::new(c, 0.0, s)
        };
        for i in 0..64 {
            let (a, b) = (point(i), point(i + 1));
            if ((a + b).dot(view.forward) > 0.0) != front {
                continue;
            }
            let pos = |q: Vec3| {
                body.p + egui::vec2(q.dot(view.right), -q.dot(view.up)) * (scale * body.r)
            };
            painter.line_segment(
                [pos(a), pos(b)],
                Stroke::new(0.85, Color32::from_rgb(157, 141, 119)),
            );
        }
    }
}

fn label(
    painter: &Painter,
    rect: Rect,
    start: Pos2,
    gap: f32,
    text: &str,
    color: Color32,
    occupied: &mut Vec<Rect>,
) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), color);
    let w = galley.size().x;
    let h = galley.size().y;
    for offset in [
        egui::vec2(gap, -h / 2.0),
        egui::vec2(-w / 2.0, gap),
        egui::vec2(-w - gap, -h / 2.0),
        egui::vec2(-w / 2.0, -h - gap),
    ] {
        let area = Rect::from_min_size(start + offset, galley.size());
        if rect.contains_rect(area) && !occupied.iter().any(|r| r.intersects(area)) {
            painter.galley(area.min, galley, color);
            occupied.push(area.expand(2.5));
            break;
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PointStar {
    p: Vec3,
    color: Color32,
    size: f32,
}

struct Random(u64);
impl Random {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 40) as f32) / 16_777_216.0
    }
    fn noise(&mut self) -> f32 {
        (self.unit() + self.unit() + self.unit() + self.unit() - 2.0) * 1.2
    }
}

fn galactic_stars(g: &Galaxy) -> Vec<PointStar> {
    let mut rng = Random(710_284);
    let n = g.arms.clamp(2, 6) as usize;
    (0..4400)
        .map(|i| {
            let (p, star) = if i < 750 {
                let r = rng.unit().powi(2) * 17.0;
                let a = rng.unit() * TAU;
                let (s, c) = a.sin_cos();
                let (sx, sz) = if g.barred { (1.08, 0.38) } else { (0.69, 0.69) };
                (
                    Vec3::new(c * r * sx, rng.noise() * 1.5, s * r * sz),
                    Color32::from_rgb(244, 199, 151),
                )
            } else {
                let t = rng.unit();
                let r = 6.0 + (g.radius_kly - 6.0) * t.powf(0.85);
                let arm = i % n;
                let angle =
                    (arm as f32 / n as f32) * TAU + 3.1 * t + (rng.noise() * (0.09 + 0.09 * t));
                let (s, c) = angle.sin_cos();
                let strength = if arm.is_multiple_of(2) { 1.0 } else { 0.66 };
                let tint = if rng.unit() < 0.18 {
                    [229, 155, 158]
                } else {
                    [132, 174, 248]
                };
                (
                    Vec3::new(
                        c * r + rng.noise() * 1.0,
                        rng.noise() * (0.24 + 0.10 * t),
                        s * r + rng.noise() * 1.0,
                    ),
                    Color32::from_rgb(tint[0], tint[1], tint[2]).gamma_multiply(strength),
                )
            };
            PointStar {
                p,
                color: star.gamma_multiply(0.33 + 0.63 * rng.unit()),
                size: 0.45 + rng.unit().powi(4) * 1.24,
            }
        })
        .collect()
}

fn dot(mesh: &mut Mesh, point: Pos2, r: f32, color: Color32) {
    let i = mesh.vertices.len() as u32;
    for offset in [
        egui::vec2(-r, 0.0),
        egui::vec2(0.0, -r),
        egui::vec2(r, 0.0),
        egui::vec2(0.0, r),
    ] {
        mesh.colored_vertex(point + offset, color);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

pub(super) fn galaxy(ui: &egui::Ui, rect: Rect, response: &egui::Response, state: &mut StarMap) {
    let painter = ui.painter_at(rect);
    backdrop(&painter, rect);
    let (_, draft) = state.draft.as_ref().unwrap();
    let g = &draft.galaxy;
    if state.points.as_ref().is_none_or(|(old, _)| {
        old.radius_kly != g.radius_kly || old.arms != g.arms || old.barred != g.barred
    }) {
        state.points = Some((g.clone(), galactic_stars(g)));
    }
    let view = state.galaxy_camera.view(rect, Vec3::ZERO);
    let mut mesh = Mesh::default();
    for star in &state.points.as_ref().unwrap().1 {
        if let Some((pos, z)) = view
            .project(star.p)
            .filter(|(p, _)| rect.expand(2.0).contains(*p))
        {
            dot(
                &mut mesh,
                pos,
                star.size * (125.0 / z).sqrt().clamp(0.55, 2.0),
                star.color,
            );
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    let a = g.home_angle.to_radians();
    if let Some((pos, _)) = view.project(Vec3::new(a.cos() * g.home_kly, 0.0, a.sin() * g.home_kly))
    {
        let accent = Color32::from_rgb(126, 221, 250);
        painter.circle_filled(pos, 3.0, accent);
        painter.circle_stroke(pos, 7.5, Stroke::new(1.3, accent));
        if state.labels {
            painter.text(
                pos + egui::vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                &draft.star.name,
                FontId::proportional(13.0),
                accent,
            );
        }
        if response.clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|p| p.distance(pos) < 13.0)
        {
            state.mode = super::Mode::System;
        }
    }
}
