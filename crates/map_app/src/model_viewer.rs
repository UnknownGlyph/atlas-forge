use crate::{app::App, i18n::tr};
use eframe::egui::{
    self,
    epaint::{Mesh, Vertex, WHITE_UV},
    Color32, Painter, Pos2, Rect, Vec2,
};
use glam::{Quat, Vec3};
use map_core::world_book::{BookId, WorldBook};
use map_io::model::Model;
use std::sync::{mpsc, Arc};

struct Camera {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.55,
            pitch: -0.26,
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

pub(crate) struct Viewer {
    id: BookId,
    source: map_core::underlay::Blob,
    name: String,
    camera: Camera,
    wireframe: bool,
    load: Option<mpsc::Receiver<Result<Arc<Model>, String>>>,
    result: Option<Result<Arc<Model>, String>>,
    frame: Option<((Rect, f32, f32, f32, Vec2, bool, bool), Arc<Mesh>)>,
}

/// Project the entire normalized mesh with the given camera. The same
/// triangles are usable for a body in a star view or in the asset inspector.
pub(crate) fn project_mesh(
    model: &Model,
    rotation: Quat,
    project: impl Fn(Vec3) -> Option<(Pos2, f32)>,
    interactive: bool,
    wireframe: bool,
) -> Mesh {
    let pts: Vec<_> = model
        .positions
        .iter()
        .map(|&v| {
            let rotated = rotation * v;
            (rotated, project(rotated))
        })
        .collect();
    // Drop preview resolution only while the camera is moving. The settled
    // frame draws the complete mesh and is then cached until the view changes.
    let stride = if interactive {
        model.triangles.len().div_ceil(32_000).max(1)
    } else {
        1
    };
    let mut order: Vec<_> = model
        .triangles
        .iter()
        .enumerate()
        .step_by(stride)
        .filter_map(|(i, face)| {
            let [a, b, c] = face.vertices.map(|v| pts[v as usize].1);
            let z = (a?.1 + b?.1 + c?.1) / 3.0;
            Some((i, z))
        })
        .collect();
    order.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    let mut mesh = Mesh::default();
    let light = Vec3::new(-0.38, 0.74, 0.55).normalize();
    for (i, _) in order {
        let face = &model.triangles[i];
        let [a, b, c] = face.vertices.map(|v| pts[v as usize]);
        let normal = (b.0 - a.0).cross(c.0 - a.0).normalize_or_zero();
        let brightness = 0.37 + 0.63 * normal.dot(light).abs();
        let [r, g, b_, alpha] = face.color;
        let [r, g, b_] = [r, g, b_].map(|k| (f32::from(k) * brightness).round() as u8);
        let p = [a.1.unwrap().0, b.1.unwrap().0, c.1.unwrap().0];
        if wireframe {
            for (start, end) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                let d = end - start;
                if d.length_sq() < 0.01 {
                    continue;
                }
                let edge = egui::vec2(-d.y, d.x).normalized() * 0.34;
                let first = mesh.vertices.len() as u32;
                let color = Color32::from_rgb(113, 194, 213);
                mesh.vertices
                    .extend(
                        [start - edge, end - edge, end + edge, start + edge].map(|pos| Vertex {
                            pos,
                            uv: WHITE_UV,
                            color,
                        }),
                    );
                mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| first + i));
            }
        } else {
            let first = mesh.vertices.len() as u32;
            let color = Color32::from_rgba_unmultiplied(r, g, b_, alpha);
            mesh.vertices.extend(p.map(|pos| Vertex {
                pos,
                uv: WHITE_UV,
                color,
            }));
            mesh.indices.extend([first, first + 1, first + 2]);
        }
    }
    mesh
}

impl Viewer {
    fn frame(&mut self, ui: &mut egui::Ui, model: &Arc<Model>) {
        let size = ui.available_size().max(egui::vec2(160.0, 160.0));
        let (rect, input) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let moving = input.dragged();
        if input.dragged_by(egui::PointerButton::Primary) {
            self.camera.yaw =
                (self.camera.yaw + input.drag_delta().x * 0.009).rem_euclid(std::f32::consts::TAU);
            self.camera.pitch =
                (self.camera.pitch + input.drag_delta().y * 0.009).clamp(-1.56, 1.56);
        } else if moving {
            self.camera.pan += input.drag_delta();
        }
        if input.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            self.camera.zoom = (self.camera.zoom * (0.0023 * scroll).exp()).clamp(0.18, 5.0);
        }
        let key = (
            rect,
            self.camera.yaw,
            self.camera.pitch,
            self.camera.zoom,
            self.camera.pan,
            self.wireframe,
            moving,
        );
        if self.frame.as_ref().is_none_or(|(last, _)| *last != key) {
            let rotation =
                Quat::from_rotation_x(self.camera.pitch) * Quat::from_rotation_y(self.camera.yaw);
            let center = rect.center() + self.camera.pan;
            let focal = rect.width().min(rect.height()) * 1.2 * self.camera.zoom;
            let mesh = project_mesh(
                model,
                rotation,
                |p| {
                    let z = 3.4 - p.z;
                    Some((center + egui::vec2(p.x, -p.y) * (focal / z), z))
                },
                moving,
                self.wireframe,
            );
            self.frame = Some((key, Arc::new(mesh)));
        }
        paint_frame(&ui.painter_at(rect), rect);
        if let Some((_, mesh)) = &self.frame {
            ui.painter_at(rect).add(egui::Shape::Mesh(mesh.clone()));
        }
    }
}

fn paint_frame(painter: &Painter, rect: Rect) {
    let ink = Color32::from_rgb(36, 50, 64);
    painter.rect_filled(rect, 5.0, Color32::from_rgb(18, 25, 34));
    let spacing = 32.0;
    for i in 0..=(rect.height() / spacing) as usize {
        let y = rect.top() + i as f32 * spacing;
        painter.hline(rect.x_range(), y, (0.7, ink));
    }
    for i in 0..=(rect.width() / spacing) as usize {
        let x = rect.left() + i as f32 * spacing;
        painter.vline(x, rect.y_range(), (0.7, ink));
    }
}

impl App {
    pub(crate) fn open_model_asset(&mut self, id: BookId) {
        let Some(asset) = self
            .doc
            .book
            .as_ref()
            .and_then(|book| book.assets.get(&id))
            .cloned()
        else {
            self.set_status(tr("附件已不存在"), true);
            return;
        };
        if !map_io::model::is_model(&asset) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let name = asset.name.clone();
        let source = asset.bytes.clone();
        std::thread::spawn(move || {
            let _ = tx.send(map_io::model::decode(&asset).map(Arc::new));
        });
        self.model_viewer = Some(Viewer {
            id,
            source,
            name,
            camera: Default::default(),
            wireframe: false,
            load: Some(rx),
            result: None,
            frame: None,
        });
    }

    pub(crate) fn import_model_asset(&mut self, from: map_core::world_book::Node) {
        let Some(path) = self
            .file_dialog()
            .add_filter("GLB / glTF / OBJ", &["glb", "gltf", "obj"])
            .pick_file()
        else {
            return;
        };
        let (asset, model) = match map_io::model::import(&path) {
            Ok(pair) => pair,
            Err(err) => {
                self.set_status(err, true);
                return;
            }
        };
        let name = asset.name.clone();
        let source = asset.bytes.clone();
        let id = self.edit(tr("导入 3D 模型"), |doc| {
            WorldBook::register_anchor(doc, from);
            let book = WorldBook::edit(doc);
            let id = book.add_asset(asset);
            book.add_link(map_core::world_book::Link::new(
                from,
                map_core::world_book::Node::Asset(id),
                false,
            ))?;
            Ok(id)
        });
        if let Some(id) = id {
            self.model_viewer = Some(Viewer {
                id,
                source,
                name,
                camera: Default::default(),
                wireframe: false,
                load: None,
                result: Some(Ok(Arc::new(model))),
                frame: None,
            });
        }
    }

    pub(crate) fn model_viewer_window(&mut self, ctx: &egui::Context) {
        let Some(viewer) = &mut self.model_viewer else {
            return;
        };
        if !self
            .doc
            .book
            .as_ref()
            .and_then(|book| book.assets.get(&viewer.id))
            .is_some_and(|asset| Arc::ptr_eq(&asset.bytes.0, &viewer.source.0))
        {
            self.model_viewer = None;
            return;
        }
        if let Some(job) = &viewer.load {
            match job.try_recv() {
                Ok(result) => {
                    viewer.result = Some(result);
                    viewer.load = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    viewer.result = Some(Err(tr("未能载入模型。").into()));
                    viewer.load = None;
                }
            }
        }
        let viewport = egui::ViewportId::from_hash_of("atlas-model-viewer");
        let window = egui::ViewportBuilder::default()
            .with_title(format!("{} · {}", tr("3D 模型"), viewer.name))
            .with_inner_size([850.0, 660.0])
            .with_min_inner_size([380.0, 320.0]);
        let open = ctx.show_viewport_immediate(viewport, window, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return false;
            }
            egui::Panel::top("model-viewer-tools").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("重置视角")).clicked() {
                        viewer.camera = Camera::default();
                    }
                    ui.checkbox(&mut viewer.wireframe, tr("线框"));
                    ui.weak(tr("左键旋转 · 右键平移 · 滚轮缩放"));
                })
            });
            match viewer.result.clone() {
                Some(Ok(model)) => viewer.frame(ui, &model),
                Some(Err(error)) => {
                    ui.colored_label(Color32::LIGHT_RED, crate::i18n::message(error));
                }
                None => {
                    ui.spinner();
                }
            }
            true
        });
        if !open {
            self.model_viewer = None;
        }
    }
}
