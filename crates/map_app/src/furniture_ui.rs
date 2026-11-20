//! Map furniture on the views: a scale bar measured at the middle of the
//! view and a north arrow pointing the way north runs there (on the flat
//! map and the globe alike), and — while the export window is open — the
//! frame of what will be exported, on the flat map.

use crate::app::App;
use crate::view::{ViewGeom, ViewKind};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use map_core::geom::LonLat;

/// The largest 1, 2 or 5 × 10ⁿ not above `t`.
fn nice(t: f64) -> f64 {
    let p = 10f64.powf(t.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|m| m * p).find(|v| *v <= t).unwrap_or(p)
}

impl App {
    /// The scale bar and north arrow, in the lower left of a view.
    pub fn draw_furniture(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if !self.furniture || self.doc.paper.is_some() {
            return;
        }
        let r = vg.rect;
        let mid = r.center();
        let Some(here) = vg.to_lonlat(mid) else { return };
        // kilometres per pixel at the middle, measured across 100 px
        let km_per_px = vg.deg_per_px(mid, 100.0).to_radians() * self.doc.radius() / 100.0;
        if !km_per_px.is_finite() || km_per_px <= 0.0 {
            return;
        }
        let len_km = nice(km_per_px * (r.width() as f64 * 0.22).min(180.0));
        let len_px = (len_km / km_per_px) as f32;
        let ink = Color32::from_gray(30);
        let paper = Color32::from_rgba_unmultiplied(255, 255, 255, 200);
        let base = pos2(r.left() + 16.0, r.bottom() - 18.0);
        // a box behind, the bar in four black and white parts, the length
        let label = if len_km >= 1.0 { format!("{len_km:.0} km") } else { format!("{len_km} km") };
        let back = Rect::from_min_max(base + vec2(-8.0, -30.0), base + vec2(len_px + 48.0, 8.0));
        painter.rect_filled(back, 4.0, paper);
        for i in 0..4 {
            let x0 = base.x + len_px * i as f32 / 4.0;
            let seg = Rect::from_min_max(pos2(x0, base.y - 5.0), pos2(x0 + len_px / 4.0, base.y));
            painter.rect_filled(seg, 0.0, if i % 2 == 0 { ink } else { Color32::WHITE });
        }
        painter.rect_stroke(Rect::from_min_max(pos2(base.x, base.y - 5.0), pos2(base.x + len_px, base.y)), 0.0, Stroke::new(1.0, ink), egui::StrokeKind::Middle);
        painter.text(base + vec2(0.0, -8.0), Align2::LEFT_BOTTOM, "0", FontId::proportional(11.0), ink);
        painter.text(base + vec2(len_px, -8.0), Align2::CENTER_BOTTOM, label, FontId::proportional(11.0), ink);
        // on a map of much of the world the scale holds near the middle only
        if r.width() as f64 * km_per_px > 8000.0 {
            painter.text(base + vec2(len_px + 40.0, -2.0), Align2::RIGHT_BOTTOM, "图中心", FontId::proportional(9.5), Color32::from_gray(90));
        }
        // north: from the middle of the view towards the pole
        let north = LonLat::new(here.lon, (here.lat + 0.5).min(89.9));
        let dir = match (vg.to_screen(here), vg.to_screen(north)) {
            (Some(a), Some(b)) if (b - a).length() > 1e-3 => (b - a).normalized(),
            _ => Vec2::new(0.0, -1.0),
        };
        let c = pos2(back.right() + 26.0, base.y - 12.0);
        painter.circle_filled(c, 15.0, paper);
        let side = Vec2::new(-dir.y, dir.x);
        let tip = c + dir * 11.0;
        let tail = c - dir * 9.0;
        painter.add(egui::Shape::convex_polygon(vec![tip, c + side * 5.0, c], ink, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![tip, c, c - side * 5.0], Color32::from_gray(150), Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![tail, c - side * 5.0, c, c + side * 5.0], Color32::from_gray(210), Stroke::new(0.6, ink)));
        painter.text(tip + dir * 7.0, Align2::CENTER_CENTER, "北", FontId::proportional(10.0), ink);
    }

    /// While the export window is open: the frame of the picture on the
    /// flat map (when the export uses the flat map's projection).
    pub fn draw_export_frame(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if !self.export.open || vg.kind != ViewKind::Flat {
            return;
        }
        let o = self.export_options();
        if o.projection != self.cam2d.proj || (o.lon0 - self.cam2d.lon0).abs() > 1e-9 {
            return;
        }
        let b = o.bounds.unwrap_or_else(|| o.projection.bounds());
        // fitted as the export fits it: the picture's shape around the region
        let (w, h) = (o.width as f64, o.height as f64);
        let scale = (w / (b[2] - b[0])).min(h / (b[3] - b[1]));
        let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
        let (hw, hh) = (w / scale * 0.5, h / scale * 0.5);
        let c = vg.rect.center();
        let at = |x: f64, y: f64| -> Pos2 {
            let s = self.cam2d.proj_to_screen([x, y]);
            pos2(c.x + s[0] as f32, c.y + s[1] as f32)
        };
        let frame = Rect::from_two_pos(at(cx - hw, cy - hh), at(cx + hw, cy + hh));
        let accent = Color32::from_rgb(230, 120, 20);
        let corners = [frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom(), frame.left_top()];
        painter.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.6, accent), 8.0, 5.0));
        let tag = format!("导出范围 {} × {}", o.width, o.height);
        let g = painter.layout_no_wrap(tag, FontId::proportional(11.5), Color32::WHITE);
        let tr = Rect::from_min_size(frame.left_top() + vec2(0.0, -g.size().y - 4.0), g.size() + vec2(8.0, 2.0));
        painter.rect_filled(tr, 3.0, accent);
        painter.galley(tr.min + vec2(4.0, 1.0), g, Color32::WHITE);
    }
}
