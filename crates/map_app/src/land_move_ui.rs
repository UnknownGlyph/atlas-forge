use crate::{
    app::App,
    view::{ViewGeom, ViewKind},
};
use eframe::egui::{self, Color32, PointerButton, Stroke};
use map_core::{
    geom::{self, LonLat},
    topo::{GeoOffset, LandTransform},
    FaceId,
};

#[derive(Default)]
pub(crate) struct LandMove {
    pub active: bool,
    seeds: Vec<FaceId>,
    coast: Vec<Vec<LonLat>>,
    faces: usize,
    drag: Option<(ViewKind, LonLat)>,
    offset: GeoOffset,
    generation: u64,
    scaling: bool,
    pivot: Option<LonLat>,
    percent: f64,
}

impl LandMove {
    fn reset_input(&mut self) {
        self.offset = GeoOffset::default();
        self.percent = 100.0;
    }

    fn transform(&self) -> LandTransform {
        if let (true, Some(pivot)) = (self.scaling, self.pivot) {
            LandTransform::Scale {
                pivot,
                factor: self.percent / 100.0,
            }
        } else {
            LandTransform::Translate(self.offset)
        }
    }
}

impl App {
    pub(crate) fn start_land_move(&mut self) {
        self.set_tool(crate::view::Tool::Select);
        self.land_move.active = true;
        self.hover_face = None;
        self.set_status(
            crate::i18n::tr("点选陆块或按住左键拖动。Shift+单击可加入其他陆块，Esc 退出。"),
            false,
        );
    }

    pub(crate) fn start_land_scale(&mut self) {
        self.set_tool(crate::view::Tool::Select);
        self.land_move = LandMove {
            active: true,
            scaling: true,
            percent: 100.0,
            ..Default::default()
        };
        self.hover_face = None;
        self.set_status(
            crate::i18n::tr("选中独立陆块。拖动或输入比例可围绕中心等比缩放。"),
            false,
        );
    }

    fn pick_land_move(&mut self, at: LonLat, add: bool) -> bool {
        let face = self.doc.topo.face_at(at);
        let mut seeds = if add && !self.land_move.scaling {
            self.land_move.seeds.clone()
        } else {
            Vec::new()
        };
        seeds.push(face);
        match self.doc.topo.land_block(&seeds) {
            Ok(block) => {
                self.land_move.seeds = seeds;
                self.land_move.faces = block.faces.len();
                self.land_move.coast = self.doc.topo.block_coastlines(&block);
                if self.land_move.scaling {
                    let points: Vec<_> = self.land_move.coast.iter().flatten().copied().collect();
                    self.land_move.pivot = map_core::arrange::centre_of(&points).or(Some(at));
                }
                self.land_move.generation = self.history.generation();
                true
            }
            Err(message) => {
                self.set_status(message, true);
                false
            }
        }
    }

    fn apply_land_move(&mut self) {
        let transform = self.land_move.transform();
        if transform.is_identity() || self.land_move.seeds.is_empty() {
            return;
        }
        if self.land_move.generation != self.history.generation() {
            self.land_move = LandMove {
                active: true,
                scaling: self.land_move.scaling,
                percent: 100.0,
                ..Default::default()
            };
            self.set_status(
                crate::i18n::tr("地图在选取后发生了变化，请重新选择陆块。"),
                true,
            );
            return;
        }
        let seeds = self.land_move.seeds.clone();
        let label = crate::i18n::tr(if self.land_move.scaling {
            "整体缩放陆块"
        } else {
            "整体移动陆块"
        });
        if let Some(count) = self.edit(label, |doc| {
            let block = match transform {
                LandTransform::Translate(offset) => doc.translate_land(&seeds, offset)?,
                LandTransform::Scale { pivot, factor } => {
                    doc.scale_land(seeds[0], pivot, factor)?
                }
            };
            Ok(block.faces.len())
        }) {
            self.land_move.reset_input();
            self.land_move.generation = self.history.generation();
            if let Ok(block) = self.doc.topo.land_block(&seeds) {
                self.land_move.coast = self.doc.topo.block_coastlines(&block);
                self.land_move.faces = block.faces.len();
            }
            let status = match transform {
                LandTransform::Translate(offset) => format!(
                    "已移动 {count} 个地块。经度偏移 {:+.4}°，纬度偏移 {:+.4}°。",
                    offset.lon, offset.lat
                ),
                LandTransform::Scale { factor, .. } => format!(
                    "{} {count} · {:.2}%",
                    crate::i18n::tr("整体缩放陆块"),
                    factor * 100.0
                ),
            };
            self.set_status(status, false);
        } else {
            self.land_move.reset_input();
        }
    }

    pub(crate) fn land_move_bar(&mut self, ui: &mut egui::Ui) {
        if !self.land_move.active {
            return;
        }
        if !self.land_move.seeds.is_empty()
            && self.land_move.generation != self.history.generation()
        {
            self.land_move = LandMove {
                active: true,
                scaling: self.land_move.scaling,
                percent: 100.0,
                ..Default::default()
            };
        }
        ui.horizontal_wrapped(|ui| {
            if self.land_move.scaling {
                ui.strong(crate::i18n::tr("整体缩放陆块"));
                ui.label(format!(
                    "{}: {}",
                    crate::i18n::tr("地块"),
                    self.land_move.faces
                ));
                ui.add_enabled(
                    self.land_move.pivot.is_some(),
                    egui::DragValue::new(&mut self.land_move.percent)
                        .range(1.0..=2000.0)
                        .speed(0.5)
                        .suffix("%"),
                );
                if ui
                    .add_enabled(
                        self.land_move.pivot.is_some(),
                        egui::Button::new(crate::i18n::tr("应用缩放")),
                    )
                    .clicked()
                {
                    self.apply_land_move();
                }
                if ui.button(crate::i18n::tr("退出")).clicked() {
                    self.land_move = LandMove::default();
                }
                return;
            }
            ui.strong(crate::i18n::tr("整体移动陆块"));
            ui.label(crate::trf!("{} 个地块", self.land_move.faces));
            ui.add(
                egui::DragValue::new(&mut self.land_move.offset.lon)
                    .range(-180.0..=180.0)
                    .speed(0.1)
                    .prefix(crate::i18n::tr("经度 Δ "))
                    .suffix("°"),
            );
            ui.add(
                egui::DragValue::new(&mut self.land_move.offset.lat)
                    .range(-180.0..=180.0)
                    .speed(0.1)
                    .prefix(crate::i18n::tr("纬度 Δ "))
                    .suffix("°"),
            );
            if ui
                .add_enabled(
                    !self.land_move.seeds.is_empty(),
                    egui::Button::new(crate::i18n::tr("应用偏移")),
                )
                .clicked()
            {
                self.apply_land_move();
            }
            if ui.button(crate::i18n::tr("退出")).clicked() {
                self.land_move = LandMove::default();
            }
        });
    }

    pub(crate) fn land_move_input(
        &mut self,
        resp: &egui::Response,
        vg: &ViewGeom,
        hover: Option<LonLat>,
    ) {
        if resp.clicked_by(PointerButton::Primary) {
            if let Some(at) = hover {
                let add = resp.ctx.input(|input| input.modifiers.shift);
                self.land_move.reset_input();
                self.pick_land_move(at, add);
            }
        }
        if resp.drag_started_by(PointerButton::Primary) {
            let at = resp
                .ctx
                .input(|i| i.pointer.press_origin())
                .and_then(|p| vg.to_lonlat(p));
            if let Some(at) = at {
                let face = self.doc.topo.face_at(at);
                let already_selected = self.land_move.generation == self.history.generation()
                    && self
                        .doc
                        .topo
                        .land_block(&self.land_move.seeds)
                        .is_ok_and(|block| block.faces.contains(&face));
                if already_selected || self.pick_land_move(at, false) {
                    self.land_move.drag = Some((vg.kind, at));
                    self.land_move.reset_input();
                }
            }
        }
        if let Some((kind, from)) = self.land_move.drag.filter(|(kind, _)| *kind == vg.kind) {
            let _ = kind;
            let at = resp
                .interact_pointer_pos()
                .filter(|&p| vg.rect.contains(p))
                .and_then(|p| vg.to_lonlat(p));
            if let Some(at) = at {
                if let (true, Some(pivot)) = (self.land_move.scaling, self.land_move.pivot) {
                    let from_distance = geom::geo_dist_deg(pivot, from);
                    if from_distance > 1e-7 {
                        self.land_move.percent = (100.0 * geom::geo_dist_deg(pivot, at)
                            / from_distance)
                            .clamp(1.0, 2000.0);
                    }
                } else {
                    let mut offset = GeoOffset::between(from, at);
                    if resp.ctx.input(|i| i.modifiers.shift) {
                        if offset.lon.abs() >= offset.lat.abs() {
                            offset.lat = 0.0;
                        } else {
                            offset.lon = 0.0;
                        }
                    }
                    self.land_move.offset = offset;
                }
            }
            if resp.drag_stopped() {
                self.land_move.drag = None;
                if at.is_some() {
                    self.apply_land_move();
                } else {
                    self.land_move.reset_input();
                    self.set_status(
                        crate::i18n::tr("松手点不在当前地图的可见范围内，操作已取消。"),
                        true,
                    );
                }
            }
        }
        if resp.hovered() {
            resp.ctx.set_cursor_icon(if self.land_move.drag.is_some() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Move
            });
        }
    }

    pub(crate) fn draw_land_move(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if !self.land_move.active || self.land_move.generation != self.history.generation() {
            return;
        }
        let ink = Stroke::new(2.0, Color32::from_rgb(18, 148, 190));
        let transform = self.land_move.transform();
        if let Some(pivot) = self.land_move.pivot.and_then(|point| vg.to_screen(point)) {
            painter.circle_stroke(pivot, 4.0, ink);
        }
        for ring in &self.land_move.coast {
            let stride = (ring.len() / 3500).max(1);
            let points = ring.iter().step_by(stride).chain(ring.first());
            let mut previous: Option<egui::Pos2> = None;
            for &point in points {
                let p = vg.to_screen(transform.apply(point));
                if let (Some(a), Some(b)) = (previous, p) {
                    if (a.x - b.x).abs() < vg.rect.width() * 0.66 {
                        painter.line_segment([a, b], ink);
                    }
                }
                previous = p;
            }
        }
    }
}
