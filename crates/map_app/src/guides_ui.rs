use crate::{app::App, view::ViewGeom};
use eframe::egui::{self, Color32, Stroke};
use map_core::guides::{Axis, Guide};

impl App {
    pub(crate) fn geographic_guides_window(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.guide_draft else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut erase = None;
        egui::Window::new(crate::i18n::tr("地理参考线"))
            .open(&mut open)
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(crate::i18n::tr("新建经线")).clicked() {
                        draft.push(Guide::new(Axis::Longitude, self.cam2d.lon0, "参考经线"));
                    }
                    if ui.button(crate::i18n::tr("新建纬线")).clicked() {
                        draft.push(Guide::new(Axis::Latitude, 0.0, "参考纬线"));
                    }
                    if ui.button(crate::i18n::tr("添加北回归线")).clicked() {
                        draft.push(Guide::new(Axis::Latitude, 23.436, "北回归线"));
                    }
                });
                ui.label(crate::i18n::tr("参考线固定在经纬度上。可以分别控制显示、绘制时吸附和导出。"));
                egui::ScrollArea::vertical()
                    .max_height(390.0)
                    .show(ui, |ui| {
                        for (i, line) in draft.iter_mut().enumerate() {
                            ui.push_id(i, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(if line.axis == Axis::Latitude {
                                        crate::i18n::tr("纬线")
                                    } else {
                                        crate::i18n::tr("经线")
                                    });
                                    let limit = if line.axis == Axis::Latitude {
                                        89.999
                                    } else {
                                        180.0
                                    };
                                    ui.add(
                                        egui::DragValue::new(&mut line.degrees)
                                            .range(-limit..=limit)
                                            .speed(0.1)
                                            .suffix("°"),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(&mut line.name)
                                            .desired_width(145.0)
                                            .hint_text(crate::i18n::tr("名称")),
                                    );
                                    let [r, g, b, a] = line.color;
                                    let mut color = Color32::from_rgba_unmultiplied(r, g, b, a);
                                    if egui::color_picker::color_edit_button_srgba(
                                        ui,
                                        &mut color,
                                        egui::color_picker::Alpha::OnlyBlend,
                                    )
                                    .changed()
                                    {
                                        line.color = color.to_srgba_unmultiplied();
                                    }
                                    if ui.button(crate::i18n::tr("移除")).clicked() {
                                        erase = Some(i);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut line.visible, crate::i18n::tr("显示"));
                                    ui.checkbox(&mut line.snap, crate::i18n::tr("吸附"));
                                    ui.checkbox(&mut line.include_export, crate::i18n::tr("导出"));
                                });
                                ui.separator();
                            });
                        }
                    });
                if let Some(i) = erase {
                    draft.remove(i);
                }
                apply = ui
                    .add_enabled(
                        draft.iter().all(Guide::valid),
                        egui::Button::new(crate::i18n::tr("应用参考线")),
                    )
                    .clicked();
            });
        if apply {
            let lines = self.guide_draft.take().unwrap();
            self.edit("编辑地理参考线", |doc| {
                doc.guides = lines;
                Ok(())
            });
        } else if !open {
            self.guide_draft = None;
        }
    }

    pub(crate) fn draw_geographic_guides(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if self.doc.paper.is_some() {
            return;
        }
        let meridian = if vg.kind == crate::view::ViewKind::Flat {
            vg.cam.lon0
        } else {
            vg.globe.center_lon
        };
        for line in self.doc.guides.iter().filter(|g| g.visible && g.valid()) {
            let [r, g, b, a] = line.color;
            let color = Color32::from_rgba_unmultiplied(r, g, b, a);
            let mut previous: Option<egui::Pos2> = None;
            for point in line.path(meridian) {
                let p = vg.to_screen(point);
                if let (Some(last), Some(at)) = (previous, p) {
                    if (last.x - at.x).abs() < vg.rect.width() * 0.66 {
                        painter.line_segment([last, at], Stroke::new(1.1, color));
                    }
                }
                previous = p;
            }
            if !line.name.is_empty() {
                if let Some(at) = vg
                    .to_screen(line.label_position(meridian))
                    .filter(|p| vg.rect.contains(*p))
                {
                    painter.text(
                        at - egui::vec2(0.0, 3.0),
                        egui::Align2::CENTER_BOTTOM,
                        &line.name,
                        egui::FontId::proportional(11.0),
                        color,
                    );
                }
            }
        }
    }
}
