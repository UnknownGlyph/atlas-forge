//! The measuring tool: click points to measure along great circles, click
//! the first point again to close the shape and get its area too. Nothing
//! is saved; Esc or the panel's button clears the measurement.

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Vec2};
use map_core::doc::format_num;
use map_core::geom::LonLat;
use map_core::measure::great_circle_path;

const INK: Color32 = Color32::from_rgb(20, 20, 20);
const HALO: Color32 = Color32::from_rgb(255, 210, 60);

impl App {
    pub fn measure_input(&mut self, resp: &egui::Response, vg: &ViewGeom, ll: Option<LonLat>) {
        if resp.clicked_by(egui::PointerButton::Secondary) {
            self.measure_closed = false;
            self.measure_pts.pop();
            return;
        }
        if !resp.clicked() {
            return;
        }
        let Some(p) = ll else { return };
        if self.measure_closed {
            // a new measurement
            self.measure_pts.clear();
            self.measure_closed = false;
        }
        // a click on the first point closes the shape
        let at = resp.interact_pointer_pos();
        let on_first = self.measure_pts.len() >= 3 && at.zip(self.measure_pts.first().and_then(|f| vg.to_screen(*f))).is_some_and(|(a, f)| a.distance(f) < 10.0);
        if on_first {
            self.measure_closed = true;
        } else {
            self.measure_pts.push(p);
        }
    }

    /// The measured path (and the stretch to the cursor while measuring),
    /// with the running total by the cursor or the last point.
    pub fn draw_measure(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        if self.tool != crate::view::Tool::Measure || self.measure_pts.is_empty() {
            return;
        }
        let mut pts = self.measure_pts.clone();
        if self.measure_closed {
            pts.push(pts[0]);
        }
        let live = (!self.measure_closed).then_some(hover).flatten();
        let line = |a: LonLat, b: LonLat, w: f32, col: Color32| {
            if self.doc.paper.is_some() {
                if let (Some(a),Some(b)) = (vg.to_screen(a),vg.to_screen(b)) {
                    painter.line_segment([a,b], Stroke::new(w,col));
                }
                return;
            }
            let s: Vec<Option<Pos2>> = great_circle_path(a, b, 1.0).into_iter().map(|q| vg.to_screen(q)).collect();
            for seg in s.windows(2) {
                if let (Some(x), Some(y)) = (seg[0], seg[1]) {
                    if (x.x - y.x).abs() < vg.rect.width() * 0.5 {
                        painter.line_segment([x, y], Stroke::new(w, col));
                    }
                }
            }
        };
        for w in pts.windows(2) {
            line(w[0], w[1], 4.0, HALO);
            line(w[0], w[1], 1.6, INK);
        }
        if let (Some(last), Some(h)) = (pts.last(), live) {
            line(*last, h, 1.2, INK.gamma_multiply(0.6));
        }
        for p in &self.measure_pts {
            if let Some(s) = vg.to_screen(*p) {
                painter.circle(s, 3.5, HALO, Stroke::new(1.2, INK));
            }
        }
        let (text, at) = match live {
            Some(h) => {
                let mut all = pts.clone();
                all.push(h);
                let segment = [*pts.last().unwrap(), h];
                let (total,step,unit) = if let Some(paper) = self.doc.paper {
                    (paper.length(&all),paper.length(&segment),"px")
                } else {
                    (self.doc.path_length_km(&all),self.doc.path_length_km(&segment),"km")
                };
                (format!("{} {unit}（本段 {} {unit}）", format_num(total.round()), format_num(step.round())), vg.to_screen(h))
            }
            None => (self.measure_text(), self.measure_pts.last().and_then(|p| vg.to_screen(*p))),
        };
        if let Some(at) = at {
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), INK);
            let r = egui::Rect::from_min_size(at + Vec2::new(12.0, 8.0), galley.size()).expand(3.0);
            painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(255, 248, 220, 235));
            painter.galley(r.min + Vec2::splat(3.0), galley, INK);
        }
    }

    /// The finished measurement as text: total length, and area when closed.
    fn measure_text(&self) -> String {
        let mut pts = self.measure_pts.clone();
        if self.measure_closed {
            pts.push(pts[0]);
        }
        if let Some(paper) = self.doc.paper {
            let mut text = format!("{} px",format_num(paper.length(&pts).round()));
            if self.measure_closed {
                text += &format!("　面积 {} px²",format_num(paper.area(&self.measure_pts).round()));
            }
            return text;
        }
        let mut t = format!("{} km", format_num(self.doc.path_length_km(&pts).round()));
        if self.measure_closed {
            t += &format!("　面积 {} km²", format_num(self.doc.ring_area_km2(&self.measure_pts).round()));
        }
        t
    }

    pub fn measure_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new("测量").strong());
        if self.measure_pts.len() < 2 {
            ui.weak(if self.doc.paper.is_some() {
                "在画布上依次点击各点测量像素距离。点击起点闭合后计算像素面积。"
            } else { "在地图上单击各点：沿大圆测量距离；单击第一个点闭合后同时给出面积。右键撤回一个点，Esc 清除。按本星球半径计算，测量不会保存。" });
            return;
        }
        let label = if self.measure_closed { "周长" } else { "长度" };
        ui.label(format!("{label}：{}", self.measure_text().replace("　面积", "\n面积：")));
        if self.doc.paper.is_none() {
            ui.weak(format!("{} 个点 · 星球半径 {} km", self.measure_pts.len(), format_num(self.doc.radius().round())));
            if !self.measure_closed {
                ui.label(crate::roads_ui::travel_lines(self.doc.path_length_km(&self.measure_pts))).on_hover_text("按每日行程估算：步行 30 km、车队 25 km、骑马 60 km、驿马换马 200 km；一天按十小时计");
            }
        }
        if ui.button("清除").clicked() {
            self.measure_pts.clear();
            self.measure_closed = false;
        }
    }
}
