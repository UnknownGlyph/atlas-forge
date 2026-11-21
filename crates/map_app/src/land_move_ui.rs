use crate::{
    app::App,
    view::{ViewGeom, ViewKind},
};
use eframe::egui::{self, Color32, PointerButton, Stroke};
use std::collections::BTreeSet;
use map_core::{
    geom::{self, LonLat},
    topo::{land_group_center, GeoOffset, GlobeRotation, LandBlock, LandTransform},
    FaceId,
};

#[derive(Default)]
pub(crate) struct LandMove {
    pub active: bool,
    seeds: Vec<FaceId>,
    coast: Vec<Vec<LonLat>>,
    faces: BTreeSet<FaceId>,
    anchor: Option<LonLat>,
    longitude_only: bool,
    multi_pick: bool,
    drag: Option<(ViewKind, LonLat)>,
    offset: GeoOffset,
    generation: u64,
    scaling: bool,
    pivot: Option<LonLat>,
    percent: f64,
}

impl LandMove {
    pub(crate) fn mode_name(&self) -> &'static str {
        crate::i18n::tr(if self.scaling { "整体缩放陆块" } else { "整体移动陆块" })
    }

    fn reset_input(&mut self) {
        self.offset = GeoOffset::default();
        self.percent = 100.0;
        self.anchor = None;
    }

    fn transform(&self) -> Result<LandTransform, String> {
        if let (true, Some(pivot)) = (self.scaling, self.pivot) {
            Ok(LandTransform::Scale { pivot, factor: self.percent / 100.0 })
        } else if self.longitude_only {
            Ok(LandTransform::Translate(GeoOffset { lon: self.offset.lon, lat: 0.0 }))
        } else if let Some(anchor) = self.anchor.or(self.pivot) {
            GlobeRotation::between(anchor, self.offset.apply(anchor)).map(LandTransform::Rotate)
        } else {
            Ok(LandTransform::Translate(GeoOffset::default()))
        }
    }

}

impl App {
    pub(crate) fn start_land_move(&mut self) {
        self.start_land_transform(false);
    }

    pub(crate) fn start_land_scale(&mut self) {
        self.start_land_transform(true);
    }

    fn start_land_transform(&mut self, scaling: bool) {
        if self.tool != crate::view::Tool::Select {
            self.set_tool(crate::view::Tool::Select);
        }
        self.land_move.active = true;
        self.land_move.scaling = scaling;
        self.land_move.drag = None;
        self.land_move.reset_input();
        self.hover_face = None;
        self.set_status(
            crate::i18n::tr("单击选择陆块。Shift+单击添加或移除。拖动所选陆块可整体变换。"),
            false,
        );
    }

    fn update_land_selection(&mut self, seeds: Vec<FaceId>, block: LandBlock) {
        let coast = self.doc.topo.block_coastlines(&block);
        self.land_move.pivot = land_group_center(coast.iter().flatten().copied());
        self.land_move.seeds = seeds;
        self.land_move.faces = block.faces;
        self.land_move.coast = coast;
        self.land_move.generation = self.history.generation();
    }

    fn pick_land_move(&mut self, at: LonLat, toggle: bool) -> bool {
        let face = self.doc.topo.face_at(at);
        let component = match self.doc.topo.land_block(&[face]) {
            Ok(component) => component,
            Err(message) => {
                self.set_status(message, true);
                return false;
            }
        };
        if !toggle {
            self.update_land_selection(vec![face], component);
            return true;
        }
        let mut seeds = self.land_move.seeds.clone();
        if self.land_move.faces.contains(&face) {
            seeds.retain(|seed| !component.faces.contains(seed));
        } else {
            seeds.push(face);
        }
        if seeds.is_empty() {
            self.clear_land_selection();
        } else if let Ok(block) = self.doc.topo.land_block(&seeds) {
            self.update_land_selection(seeds, block);
        }
        true
    }

    fn clear_land_selection(&mut self) {
        self.land_move = LandMove {
            active: true,
            scaling: self.land_move.scaling,
            longitude_only: self.land_move.longitude_only,
            multi_pick: self.land_move.multi_pick,
            percent: 100.0,
            ..Default::default()
        };
    }

    fn apply_land_move(&mut self) {
        if self.land_move.seeds.is_empty() {
            return;
        }
        if self.land_move.generation != self.history.generation() {
            self.clear_land_selection();
            self.set_status(crate::i18n::tr("地图在选取后发生了变化，请重新选择陆块。"), true);
            return;
        }
        let transform = match self.land_move.transform() {
            Ok(transform) if !transform.is_identity() => transform,
            Ok(_) => return,
            Err(error) => {
                self.set_status(error, true);
                return;
            }
        };
        let seeds = self.land_move.seeds.clone();
        let label = self.land_move.mode_name();
        if let Some(block) = self.edit(label, |doc| {
            doc.transform_land(&seeds, transform)
        }) {
            self.land_move.reset_input();
            self.update_land_selection(seeds, block);
            let status = match transform {
                LandTransform::Translate(_) | LandTransform::Rotate(_) => {
                    crate::trf!("已整体移动 {} 个地块。", self.land_move.faces.len())
                }
                LandTransform::Scale { factor, .. } => {
                    crate::trf!("已将 {} 个地块整体缩放至 {:.2}%。", self.land_move.faces.len(), factor * 100.0)
                }
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
            self.clear_land_selection();
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::trf!("已选 {} 块大陆 / 岛屿", self.land_move.seeds.len()));
            ui.checkbox(&mut self.land_move.multi_pick, crate::i18n::tr("多选陆块"))
                .on_hover_text(crate::i18n::tr("Shift+单击增减陆块"));
            if ui.add_enabled(!self.land_move.seeds.is_empty(), egui::Button::new(crate::i18n::tr("清除选择"))).clicked() {
                self.clear_land_selection();
            }
            if self.land_move.scaling {
                ui.strong(crate::i18n::tr("整体缩放陆块"));
                ui.label(format!(
                    "{}: {}",
                    crate::i18n::tr("地块"),
                    self.land_move.faces.len()
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
            ui.label(crate::trf!("{} 个地块", self.land_move.faces.len()));
            ui.checkbox(&mut self.land_move.longitude_only, crate::i18n::tr("仅沿经度移动"));
            ui.add(
                egui::DragValue::new(&mut self.land_move.offset.lon)
                    .range(-180.0..=180.0)
                    .speed(0.1)
                    .prefix(crate::i18n::tr("经度 Δ "))
                    .suffix("°"),
            );
            ui.add_enabled(
                !self.land_move.longitude_only,
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
                let add = self.land_move.multi_pick || resp.ctx.input(|input| input.modifiers.shift);
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
                    && self.land_move.faces.contains(&face);
                if already_selected || self.pick_land_move(at, false) {
                    self.land_move.reset_input();
                    self.land_move.drag = Some((vg.kind, at));
                    self.land_move.anchor = Some(at);
                }
            }
        }
        if let Some((_, from)) = self.land_move.drag.filter(|(kind, _)| *kind == vg.kind) {
            let at = resp
                .interact_pointer_pos()
                .filter(|&p| vg.rect.contains(p))
                .and_then(|p| vg.to_lonlat(p));
            if let Some(at) = at {
                if let (true, Some(pivot)) = (self.land_move.scaling, self.land_move.pivot) {
                    let distance = |p: LonLat| geom::lon_delta(pivot.lon, p.lon).hypot(p.lat - pivot.lat);
                    let from_distance = distance(from);
                    if from_distance > 1e-7 {
                        self.land_move.percent = (100.0 * distance(at) / from_distance)
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
        let Ok(transform) = self.land_move.transform() else { return };
        if let Some(pivot) = self.land_move.pivot.and_then(|point| vg.to_screen(transform.apply(point))) {
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
