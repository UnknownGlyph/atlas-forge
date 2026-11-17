//! Free text on the map (seas, ranges, regions...). Added, moved and
//! edited with the label tool; sized geographically, so it grows and
//! shrinks with the map like the place it names, and is exported as drawn.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use map_core::doc::{Annotation, LineId};

fn view_index(vg: &ViewGeom) -> usize {
    usize::from(vg.kind != crate::view::ViewKind::Flat)
}

/// The longest stretch of a curve that is on screen in one piece.
fn screen_path(vg: &ViewGeom, path: &[map_core::geom::LonLat]) -> Option<Vec<[f32; 2]>> {
    let mut pieces: Vec<Vec<[f32; 2]>> = vec![Vec::new()];
    let mut prev: Option<Pos2> = None;
    for q in crate::river_ui::densify_ll(path) {
        let s = vg.to_screen(q);
        match s {
            Some(s) if prev.is_none_or(|p| (p.x - s.x).abs() < vg.rect.width() * 0.5) => pieces.last_mut().unwrap().push([s.x, s.y]),
            Some(s) => pieces.push(vec![[s.x, s.y]]),
            None => pieces.push(Vec::new()),
        }
        prev = s;
    }
    let len = |p: &Vec<[f32; 2]>| p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
    pieces.into_iter().filter(|p| p.len() >= 2).max_by(|a, b| len(a).total_cmp(&len(b)))
}

fn rotate(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

impl App {
    /// Draw every annotation, recording where each is for the label tool.
    /// `taken` collects their boxes so labels drawn after keep clear.
    pub fn draw_annotations(&self, painter: &egui::Painter, vg: &ViewGeom, taken: &mut Vec<Rect>, pass: map_core::stack::DrawPass) {
        let editing = self.tool == Tool::Label;
        let zoom = (vg.px_per_deg() / Annotation::REF_PX_PER_DEG) as f32;
        let mut rects = Vec::new();
        for (id, a) in self.doc.annotations.iter().filter(|(id, _)| self.doc.group_of_id(**id) == pass.group) {
            // hidden text shows faintly only while editing text
            if a.hidden && !editing {
                continue;
            }
            let mut size = a.size * zoom;
            if size < 5.0 {
                // too small to read at this scale; still grabbable while editing
                if !editing {
                    continue;
                }
                size = 5.0;
            }
            let Some(pos) = vg.to_screen(a.at) else { continue };
            let colour = Color32::from_rgba_unmultiplied(a.color[0], a.color[1], a.color[2], a.color[3]).gamma_multiply(if a.hidden { 0.35 } else { 1.0 });
            let job = |text: &str, c: Color32| {
                let mut j = egui::text::LayoutJob::default();
                j.append(text, 0.0, egui::TextFormat { font_id: egui::FontId::new(size, self.font_family(&a.font, a.bold)), color: c, italics: a.italic, ..Default::default() });
                j
            };
            // a curved line goes glyph by glyph along its arc or its drawn curve
            let text = a.spaced();
            let along = a.on_path() || a.curved();
            let glyphs: Vec<String> = if along { text.chars().map(String::from).collect() } else { vec![text] };
            let galleys: Vec<_> = glyphs.iter().map(|g| painter.layout_job(job(g, colour))).collect();
            let widths: Vec<f32> = galleys.iter().map(|g| g.size().x).collect();
            let angle = -a.angle.to_radians();
            // each glyph's centre on screen and its turn (radians, as egui turns)
            let on_curve = a.on_path().then(|| screen_path(vg, &a.path)).flatten().and_then(|p| map_core::path_text::along_path(&p, &widths, 0.0));
            let places: Vec<(Pos2, f32)> = match on_curve {
                Some(v) => v.into_iter().map(|([x, y], turn)| (egui::pos2(x, y), -turn.to_radians())).collect(),
                None => map_core::doc::arc_layout(&widths, a.bend).into_iter().map(|([x, y], turn)| (pos + rotate(Vec2::new(x, -y), angle), angle - turn.to_radians())).collect(),
            };
            let mut aabb = Rect::NOTHING;
            for ((g, galley), (centre, ga)) in glyphs.iter().zip(galleys).zip(places) {
                let half = galley.size() * 0.5;
                let (c, s) = (ga.cos().abs(), ga.sin().abs());
                aabb = aabb.union(Rect::from_center_size(centre, Vec2::new(half.x * 2.0 * c + half.y * 2.0 * s, half.x * 2.0 * s + half.y * 2.0 * c)));
                if g.trim().is_empty() {
                    continue;
                }
                let tl = centre - rotate(half, ga);
                let halo = painter.layout_job(job(g, Color32::from_rgba_unmultiplied(255, 255, 255, 170)));
                for o in [Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0)] {
                    painter.add(egui::epaint::TextShape::new(tl + o, halo.clone(), Color32::WHITE).with_angle(ga));
                }
                painter.add(egui::epaint::TextShape::new(tl, galley, colour).with_angle(ga));
            }
            let aabb = aabb.expand(2.0);
            taken.push(aabb);
            rects.push((*id, aabb));
            if editing {
                let strong = self.selected_ann == Some(*id);
                painter.rect_stroke(aabb, 2.0, Stroke::new(if strong { 2.0 } else { 0.8 }, Color32::from_rgb(40, 110, 200)), egui::StrokeKind::Outside);
            }
        }
        // the first pass starts the list anew; later layers lie on top
        let slot = &mut self.ann_rects.borrow_mut()[view_index(vg)];
        if pass.group.is_none() {
            slot.clear();
        }
        slot.extend(rects);
        // a curve being drawn for a piece of text
        if let Some((_, pts)) = &self.ann_path {
            let s: Vec<Pos2> = pts.iter().filter_map(|p| vg.to_screen(*p)).collect();
            painter.add(egui::Shape::line(s.clone(), Stroke::new(1.5, Color32::from_rgb(40, 110, 200))));
            for p in s {
                painter.circle_filled(p, 3.5, Color32::from_rgb(40, 110, 200));
            }
        }
    }

    /// Finish the curve being drawn for a piece of text: it follows it now.
    pub fn finish_ann_path(&mut self) {
        let Some((id, pts)) = self.ann_path.take() else { return };
        if pts.len() < 2 {
            return self.set_status("曲线至少需要两个点", true);
        }
        if self.edit("文字沿曲线排列", |d| {
            d.annotations.get_mut(&id).ok_or("文字不存在")?.set_path(pts);
            Ok(())
        }).is_some() {
            self.set_status("文字已沿曲线排列：拖动文字整体移动；曲线在屏幕上太短时按原角度显示", false);
        }
    }

    /// Label tool on annotations: drag moves, click selects, right click
    /// deletes, double click on empty map adds. Returns whether it took the input.
    pub fn annotation_input(&mut self, resp: &egui::Response, vg: &ViewGeom) -> bool {
        use egui::PointerButton::{Primary, Secondary};
        let hit = |rects: &[(LineId, Rect)], at: Pos2| rects.iter().rev().find(|(_, r)| r.contains(at)).map(|(id, r)| (*id, r.center()));
        // drawing a curve for a piece of text: clicks add points, a double
        // click or Enter finishes, right click takes one back
        if self.ann_path.is_some() {
            let at = resp.interact_pointer_pos().and_then(|p| vg.to_lonlat(p));
            if resp.clicked_by(Secondary) {
                if let Some((_, pts)) = &mut self.ann_path {
                    pts.pop();
                }
            } else if resp.double_clicked() || resp.triple_clicked() {
                self.finish_ann_path();
            } else if resp.clicked() {
                if let (Some((_, pts)), Some(p)) = (&mut self.ann_path, at) {
                    pts.push(p);
                }
            }
            return true;
        }
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let found = origin.and_then(|o| hit(&self.ann_rects.borrow()[view_index(vg)], o).map(|(id, c)| (id, c - o)));
            if let Some((id, off)) = found {
                self.begin_pending("移动文字");
                self.drag_ann = Some((id, off));
                self.selected_ann = Some(id);
            }
        }
        if let Some((id, off)) = self.drag_ann {
            if resp.dragged_by(Primary) {
                if let Some(ll) = resp.interact_pointer_pos().and_then(|p| vg.to_lonlat(p + off)) {
                    if let Some(a) = self.doc.annotations.get_mut(&id) {
                        a.move_to(ll);
                        if let Some(pe) = &mut self.pending {
                            pe.changed = true;
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_ann = None;
                self.end_pending();
            }
            return true;
        }
        let at = resp.interact_pointer_pos();
        let under = at.and_then(|p| hit(&self.ann_rects.borrow()[view_index(vg)], p)).map(|x| x.0);
        if resp.clicked_by(Secondary) {
            if let Some(id) = under {
                self.selected_ann = Some(id);
                self.delete_selected_annotation();
                return true;
            }
        }
        if resp.double_clicked() && under.is_none() {
            if let Some(ll) = at.and_then(|p| vg.to_lonlat(p)) {
                // a size that reads at the current scale
                let zoom = (vg.px_per_deg() / Annotation::REF_PX_PER_DEG) as f32;
                let mut a = Annotation::new("新文字", ll);
                a.size = (16.0 / zoom.max(1e-3)).clamp(2.0, 400.0);
                let id = LineId(self.doc.topo.alloc());
                if self.edit("添加文字", |d| {
                    d.annotations.insert(id, a);
                    Ok(())
                }).is_some() {
                    self.style_new(map_core::features::FeatureRef::Text(id));
                    self.selected_ann = Some(id);
                    self.ann_focus = true;
                    self.set_status("已添加文字：在左侧面板输入内容，拖动可移动", false);
                }
                return true;
            }
        }
        if resp.clicked() {
            self.selected_ann = under;
            return under.is_some();
        }
        false
    }

    pub fn delete_selected_annotation(&mut self) {
        if let Some(id) = self.selected_ann.take() {
            if self.edit("删除文字", |d| d.annotations.remove(&id).map(|_| ()).ok_or_else(|| "文字不存在".to_string())).is_some() {
                self.set_status("已删除文字（Ctrl+Z 撤销）", false);
            }
        }
    }

    /// Tool panel for the label tool: the selected annotation's settings.
    pub fn annotation_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new("地图文字").strong());
        let Some(id) = self.selected_ann.filter(|id| self.doc.annotations.contains_key(id)) else {
            self.selected_ann = None;
            ui.weak("双击地图空白处添加文字（海洋、山脉、地区名等）；单击文字选中，拖动移动，右键删除。");
            return;
        };
        let a = self.doc.annotations[&id].clone();
        if self.ann_text.as_ref().is_none_or(|(x, _)| *x != id) {
            self.ann_text = Some((id, a.text.clone()));
        }
        let buf = &mut self.ann_text.as_mut().unwrap().1;
        let r = ui.add(egui::TextEdit::singleline(buf).hint_text("文字内容"));
        if std::mem::take(&mut self.ann_focus) {
            r.request_focus();
        }
        if r.lost_focus() && *buf != a.text && !buf.trim().is_empty() {
            let v = buf.clone();
            self.edit("修改文字", |d| {
                d.annotations.get_mut(&id).ok_or("文字不存在")?.text = v;
                Ok(())
            });
        }
        self.style_row(ui, &[map_core::features::FeatureRef::Text(id)]);
        let (mut size, mut angle, mut spacing, mut italic, mut color, mut bend) = (a.size, a.angle, a.spacing, a.italic, a.color, a.bend);
        let mut sliders = vec![ui.add(egui::Slider::new(&mut size, 2.0..=400.0).logarithmic(true).text("字号"))];
        if !a.on_path() {
            sliders.push(ui.add(egui::Slider::new(&mut angle, -90.0..=90.0).suffix("°").text("角度")));
        }
        sliders.push(ui.add(egui::Slider::new(&mut spacing, 0.0..=3.0).text("字距")));
        if !a.on_path() {
            sliders.push(ui.add(egui::Slider::new(&mut bend, -180.0..=180.0).suffix("°").text("弯曲")).on_hover_text("让文字沿弧线排列：正值向上拱起，负值向下弯；0 为直线"));
        }
        // following a drawn curve
        let drawing = self.ann_path.as_ref().map(|(_, p)| p.len());
        let (mut finish, mut cancel, mut start, mut straight) = (false, false, false, false);
        ui.horizontal(|ui| match drawing {
            Some(n) => {
                ui.label(format!("正在画曲线：{n} 个点"));
                finish = ui.button("完成").clicked();
                cancel = ui.button("取消").clicked();
            }
            None => {
                start = ui
                    .button(if a.on_path() { "重新画曲线" } else { "沿曲线排列…" })
                    .on_hover_text("在地图上单击各点画一条曲线（如山脊、海岸），双击或回车完成，右键撤回一点；文字将沿它排列")
                    .clicked();
                straight = a.on_path() && ui.button("改回直线").clicked();
            }
        });
        if finish {
            self.finish_ann_path();
        }
        if cancel {
            self.ann_path = None;
        }
        if start {
            self.ann_path = Some((id, Vec::new()));
            self.set_status("单击各点画出文字要沿的曲线，双击或回车完成，右键撤回一点，Esc 取消", false);
        }
        if straight {
            self.edit("文字改回直线", |d| {
                d.annotations.get_mut(&id).ok_or("文字不存在")?.set_path(Vec::new());
                Ok(())
            });
        }
        let mut changed = sliders.iter().any(|r| r.changed());
        let started = sliders.iter().any(|r| r.drag_started());
        let stopped = sliders.iter().any(|r| r.drag_stopped());
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut italic, "斜体").changed();
            let mut c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
            if ui.color_edit_button_srgba(&mut c).changed() {
                color = c.to_array();
                changed = true;
            }
            ui.label("颜色");
            if ui.button("删除").clicked() {
                self.selected_ann = Some(id);
                self.delete_selected_annotation();
            }
        });
        let (mut font, mut bold) = (a.font.clone(), a.bold);
        ui.horizontal(|ui| {
            ui.label("字体");
            changed |= crate::fonts_ui::font_picker(ui, "annotation", &mut font, &mut bold);
        });
        if started {
            self.begin_pending("调整文字");
        }
        if changed {
            let apply = move |d: &mut map_core::Document| -> Result<(), String> {
                let x = d.annotations.get_mut(&id).ok_or("文字不存在")?;
                (x.size, x.angle, x.spacing, x.italic, x.color, x.bend) = (size, angle, spacing, italic, color, bend);
                (x.font, x.bold) = (font, bold);
                Ok(())
            };
            if self.pending.is_some() {
                let _ = apply(&mut self.doc);
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            } else {
                self.edit("调整文字", apply);
            }
        }
        if stopped {
            self.end_pending();
        }
        ui.weak("字号按地图比例：全图宽 1600 像素时的大小，随缩放与导出一起变化。");
    }
}
