//! Painting land or water with a round brush. The stroke is collected on
//! screen; on release its outline (discs and bands, `shape::brush_outline`)
//! becomes a drawn shape and goes the same way as one drawn point by point:
//! the overlap modes, the selected year, the background rebuild and undo
//! all apply.

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, Pos2, Stroke};
use map_core::geom::LonLat;

impl App {
    /// Brush input for the land and lake tools.
    pub fn brush_input(&mut self, resp: &egui::Response, vg: &ViewGeom, pos: Pos2) {
        use egui::PointerButton::Primary;
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            self.brush_stroke = vec![origin];
        }
        if resp.dragged_by(Primary) && !self.brush_stroke.is_empty() {
            let step = (self.brush_px * 0.25).max(1.0);
            if self.brush_stroke.last().is_none_or(|l| l.distance(pos) >= step) {
                self.brush_stroke.push(pos);
            }
        }
        if resp.drag_stopped() && !self.brush_stroke.is_empty() {
            self.brush_stroke.push(pos);
            self.commit_brush(vg);
        } else if resp.clicked_by(Primary) {
            self.brush_stroke = vec![pos];
            self.commit_brush(vg);
        }
    }

    fn commit_brush(&mut self, vg: &ViewGeom) {
        let stroke = std::mem::take(&mut self.brush_stroke);
        if self.bg_edit.is_some() {
            return self.set_status(crate::i18n::tr("正在处理上一次绘制，请稍候"), true);
        }
        let pts: Vec<[f64; 2]> = stroke.iter().map(|p| [p.x as f64, p.y as f64]).collect();
        // one stroke is one connected patch; keep the largest if it is not
        let Some(ring) = map_core::shape::brush_outline(&pts, self.brush_px as f64).into_iter().max_by_key(|r| r.len()) else { return };
        let ll: Option<Vec<LonLat>> = ring.iter().map(|p| vg.to_lonlat(Pos2::new(p[0] as f32, p[1] as f32))).collect();
        let Some(ll) = ll else {
            return self.set_status(crate::i18n::tr("画笔超出了地图范围，请在地图内涂抹"), true);
        };
        if self.brush_color_mode {
            let color = self.brush_color.to_srgba_unmultiplied();
            if let Some(id) = self.edit("绘制彩色笔触", |doc| {
                let id = map_core::doc::LineId(doc.topo.alloc());
                let mut area = map_core::area::AreaMark::new(ll);
                area.color = color;
                area.pattern = map_core::area::Pattern::Solid;
                area.from = doc.year;
                doc.areas.insert(id, area);
                Ok(id)
            }) {
                self.selected_area = Some(id);
            }
            return;
        }
        self.clear_draft();
        self.draft_snap = vec![false; ll.len()];
        self.draft = ll;
        self.last_draft_view = Some(*vg);
        self.finish_draft();
    }

    /// The stroke being painted and the brush under the cursor.
    pub fn draw_brush(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let fill = if self.brush_color_mode { self.brush_color } else { self.preview_fill() };
        let r = self.brush_px;
        for w in self.brush_stroke.windows(2) {
            painter.line_segment([w[0], w[1]], Stroke::new(r * 2.0, fill));
        }
        for p in &self.brush_stroke {
            painter.circle_filled(*p, r, fill);
        }
        if let Some(s) = hover.and_then(|h| vg.to_screen(h)) {
            painter.circle_stroke(s, r, Stroke::new(1.5, Color32::from_rgb(230, 80, 20)));
        }
    }
}
