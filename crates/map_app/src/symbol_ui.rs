//! Map symbols on screen: drawn from the shared marks, placed and edited
//! with the symbol tool (click empty map to add, click to select, drag to
//! move, right click or Delete to remove), and the panel for the selected one.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Shape, Stroke, Vec2};
use map_core::doc::LineId;
use map_core::symbol::{marks, MapSymbol, Mark, SymbolKind};

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Paint a symbol's marks; `fade` dims it (not shown in this year, drawn
/// faintly while editing).
pub fn paint_marks(painter: &egui::Painter, ms: &[Mark], fade: f32) {
    let pos = |q: &[f32; 2]| egui::pos2(q[0], q[1]);
    for m in ms {
        match m {
            Mark::Fill { outline, pieces, color } => {
                let col = c32(*color).gamma_multiply(fade);
                let mut mesh = egui::Mesh::default();
                for p in pieces {
                    let base = mesh.vertices.len() as u32;
                    for q in p {
                        mesh.colored_vertex(pos(q), col);
                    }
                    for i in 1..(p.len() as u32).saturating_sub(1) {
                        mesh.add_triangle(base, base + i, base + i + 1);
                    }
                }
                painter.add(Shape::mesh(mesh));
                // a thin edge in the same colour smooths the mesh's outline
                painter.add(Shape::closed_line(outline.iter().map(pos).collect(), Stroke::new(1.0, col)));
            }
            Mark::Stroke { pts, closed, width, color } => {
                let col = c32(*color).gamma_multiply(fade);
                let v: Vec<Pos2> = pts.iter().map(pos).collect();
                if *closed {
                    painter.add(Shape::closed_line(v, Stroke::new(*width, col)));
                } else if let (Some(a), Some(b)) = (v.first().copied(), v.last().copied()) {
                    // round ends, as in the export
                    painter.circle_filled(a, width * 0.5, col);
                    painter.circle_filled(b, width * 0.5, col);
                    painter.add(Shape::line(v, Stroke::new(*width, col)));
                }
            }
        }
    }
}

/// Start and end year of something shown only in some years: a tick box
/// and a value for each end (unticked: open), a button for "only the year
/// being viewed", and the range as text. Returns the value widgets, so a
/// drag on them can be one undo step.
pub fn years_ui(ui: &mut egui::Ui, from: &mut Option<i32>, to: &mut Option<i32>, viewing: Option<i32>) -> Vec<egui::Response> {
    let near = viewing.or(*from).or(*to).unwrap_or(1900);
    let (mut has_from, mut has_to) = (from.is_some(), to.is_some());
    let (mut f, mut t) = (from.unwrap_or(near), to.unwrap_or(near));
    let rf = ui.horizontal(|ui| {
        ui.checkbox(&mut has_from, crate::i18n::tr("起始年"));
        ui.add_enabled(has_from, egui::DragValue::new(&mut f).speed(0.5))
    });
    let rt = ui.horizontal(|ui| {
        ui.checkbox(&mut has_to, crate::i18n::tr("终止年"));
        ui.add_enabled(has_to, egui::DragValue::new(&mut t).speed(0.5))
    });
    *from = has_from.then_some(f);
    *to = has_to.then_some(t.max(if has_from { f } else { i32::MIN }));
    if let Some(y) = viewing {
        if ui.small_button(crate::trf!("只在 {y} 年显示")).clicked() {
            (*from, *to) = (Some(y), Some(y));
        }
    }
    let years = map_core::symbol::years_text(*from, *to);
    ui.weak(if years.is_empty() { "所有年份都显示。".to_string() } else { crate::trf!("显示于 {years}；未选定年份时全部显示。") });
    vec![rf.inner, rt.inner]
}

impl App {
    /// Draw the symbols shown in the current year (all of them, the others
    /// faint, with the symbol tool), recording where each is. `taken`
    /// collects their boxes so labels drawn after keep clear.
    pub fn draw_symbols(&self, painter: &egui::Painter, vg: &ViewGeom, taken: &mut crate::label_boxes::LabelBoxes, layer: &crate::draw_layers::LayerFeatures) {
        let editing = self.tool == Tool::Symbol;
        let detail = self.map_detail(vg);
        let mut rects = Vec::new();
        for id in &layer.symbols {
            let s = &self.doc.symbols[id];
            let shown = s.shown_in(self.doc.year);
            if !shown && !editing {
                continue;
            }
            if !editing && self.selected_sym != Some(*id) && !detail.visible(1)
                && matches!(s.kind, SymbolKind::Peak | SymbolKind::Volcano | SymbolKind::Pass)
            {
                continue;
            }
            let Some(pos) = vg.to_screen(s.at) else { continue };
            let fade = if shown { 1.0 } else { 0.35 };
            let rect = if let Some(art) = &s.pixels {
                let [w, h] = s.drawn_size(1.0);
                let rect = Rect::from_center_size(pos, egui::vec2(w, h));
                self.pixel_tools.paint(painter, art, rect, fade);
                rect
            } else {
                let rect = Rect::from_center_size(pos, Vec2::splat(s.size * 2.0));
                if s.kind == SymbolKind::Glyph {
                // a character symbol: its character with a halo, centred on its point
                let font = egui::FontId::proportional(map_core::symbol::glyph_px(s.size));
                let galley = painter.layout_no_wrap(s.glyph.clone(), font.clone(), c32(s.color).gamma_multiply(fade));
                let tl = pos - galley.size() * 0.5;
                let halo = painter.layout_no_wrap(s.glyph.clone(), font, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
                for o in [Vec2::new(-1.5, 0.0), Vec2::new(1.5, 0.0), Vec2::new(0.0, -1.5), Vec2::new(0.0, 1.5)] {
                    painter.galley(tl + o, halo.clone(), Color32::WHITE);
                }
                painter.galley(tl, galley, Color32::WHITE);
            } else {
                paint_marks(painter, &marks(s.kind, [pos.x, pos.y], s.size, s.color), fade);
            }
                rect
            };
            let mut r = rect.expand(2.0);
            if !s.name.is_empty() {
                let font = egui::FontId::proportional((s.size * 1.5).max(9.0));
                let galley = painter.layout_no_wrap(s.name.clone(), font.clone(), c32(s.color).gamma_multiply(fade));
                let extent = match s.name_place {
                    map_core::symbol::NamePlace::Left | map_core::symbol::NamePlace::Right => rect.width(),
                    _ => rect.height(),
                } * 0.5;
                let [x, y] = s.name_place.top_left([pos.x, pos.y], extent, galley.size().x, galley.size().y, 3.0);
                let tl = egui::pos2(x, y);
                let halo = painter.layout_no_wrap(s.name.clone(), font, Color32::from_rgba_unmultiplied(255, 255, 255, 190));
                for o in [Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0)] {
                    painter.galley(tl + o, halo.clone(), Color32::WHITE);
                }
                r = r.union(Rect::from_min_size(tl, galley.size()));
                painter.galley(tl, galley, Color32::WHITE);
            }
            taken.push(r);
            rects.push((*id, r));
            if editing {
                let strong = self.selected_sym == Some(*id);
                painter.rect_stroke(r, 2.0, Stroke::new(if strong { 2.0 } else { 0.8 }, Color32::from_rgb(40, 110, 200)), egui::StrokeKind::Outside);
            }
        }
        let slot = &mut self.sym_rects.borrow_mut()[vg.kind.index()];
        if layer.pass.group.is_none() {
            slot.clear();
        }
        slot.extend(rects);
    }

    /// The symbol tool.
    pub fn symbol_input(&mut self, resp: &egui::Response, vg: &ViewGeom) {
        use egui::PointerButton::{Primary, Secondary};
        let hit = |rects: &[(LineId, Rect)], at: Pos2, doc: &map_core::Document| rects
            .iter().rev().find(|(id, r)| r.contains(at) && doc.pickable(map_core::features::FeatureRef::Symbol(*id)))
            .map(|(id, _)| *id);
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let found = origin.and_then(|o| hit(&self.sym_rects.borrow()[vg.kind.index()], o, &self.doc).map(|id| (id, o)));
            // keep the grab offset from the symbol's own point, not its box
            if let Some((id, off)) = found.and_then(|(id, o)| self.doc.symbols.get(&id).and_then(|s| vg.to_screen(s.at)).map(|c| (id, c - o))) {
                self.begin_pending("移动符号");
                self.drag_sym = Some((id, off));
                self.selected_sym = Some(id);
            }
        }
        if let Some((id, off)) = self.drag_sym {
            if resp.dragged_by(Primary) {
                if let Some(ll) = resp.interact_pointer_pos().and_then(|p| vg.to_lonlat(p + off)) {
                    if let Some(s) = self.doc.symbols.get_mut(&id) {
                        s.at = ll;
                        if let Some(pe) = &mut self.pending {
                            pe.changed = true;
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_sym = None;
                self.end_pending();
            }
            return;
        }
        let at = resp.interact_pointer_pos();
        let under = at.and_then(|p| hit(&self.sym_rects.borrow()[vg.kind.index()], p, &self.doc));
        if resp.clicked_by(Secondary) {
            if let Some(id) = under {
                self.selected_sym = Some(id);
                self.delete_selected_symbol();
            }
            return;
        }
        if resp.clicked() {
            if under.is_some() {
                self.selected_sym = under;
                return;
            }
            let Some(ll) = at.and_then(|p| vg.to_lonlat(p)) else { return };
            let kind = self.symbol_kind;
            let stamp = self.pixel_tools.stamp.as_ref().filter(|_| self.pixel_tools.placing).cloned();
            let mut symbol = MapSymbol::new(kind, ll);
            symbol.pixels = stamp;
            if symbol.pixels.is_some() { symbol.size = 12.0; }
            let name = symbol.kind_name();
            let id = LineId(self.doc.topo.alloc());
            if self.edit("添加符号", |d| {
                d.symbols.insert(id, symbol);
                Ok(())
            }).is_some() {
                self.style_new(map_core::features::FeatureRef::Symbol(id));
                self.selected_sym = Some(id);
                self.set_status(crate::trf!("已添加{}符号：可在左侧面板命名、设置年份范围", name), false);
            }
        }
    }

    pub fn delete_selected_symbol(&mut self) {
        if let Some(id) = self.selected_sym.take() {
            if self.doc.locked_in_effect(map_core::features::FeatureRef::Symbol(id)) {
                self.selected_sym = Some(id);
                self.set_status(crate::i18n::tr("请先解锁这个符号。"), true);
                return;
            }
            if self.edit("删除符号", |d| (d.delete_features(&[map_core::features::FeatureRef::Symbol(id)]) > 0).then_some(()).ok_or_else(|| "符号不存在".to_string())).is_some() {
                self.set_status(crate::i18n::tr("已删除符号（Ctrl+Z 撤销）"), false);
            }
        }
    }

    /// Tool panel for the symbol tool: the kind new symbols get, and the
    /// selected symbol's name, kind, look and years.
    pub fn symbol_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("新符号")).strong());
        ui.horizontal_wrapped(|ui| {
            for k in SymbolKind::ALL {
                if ui.selectable_label(!self.pixel_tools.placing && self.symbol_kind == k, crate::i18n::tr(k.name())).clicked() {
                    self.symbol_kind = k;
                    self.pixel_tools.placing = false;
                }
            }
        });
        if ui.button(crate::i18n::tr("彩色像素画笔…")).clicked() {
            self.open_pixel_editor(None);
        }
        if self.pixel_tools.stamp.is_some() {
            ui.toggle_value(&mut self.pixel_tools.placing, crate::i18n::tr("放置像素标志"));
        }
        ui.weak(crate::i18n::tr("单击地图空白处放置；单击符号选中，拖动移动，右键或 Delete 删除。"));
        let Some(id) = self.selected_sym.filter(|id| self.doc.symbols.contains_key(id)) else {
            self.selected_sym = None;
            return;
        };
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("所选符号")).strong());
        if self.doc.locked_in_effect(map_core::features::FeatureRef::Symbol(id)) {
            ui.weak(crate::i18n::tr("请先解锁这个符号。"));
            return;
        }
        self.style_row(ui, &[map_core::features::FeatureRef::Symbol(id)]);
        let old = self.doc.symbols[&id].clone();
        let mut v = old.clone();
        if self.sym_name.as_ref().is_none_or(|(x, _)| *x != id) {
            self.sym_name = Some((id, old.name.clone()));
        }
        let buf = &mut self.sym_name.as_mut().unwrap().1;
        let r = ui.add(egui::TextEdit::singleline(buf).hint_text(crate::i18n::tr("名称（可留空）")));
        let mut label = "修改符号";
        if r.lost_focus() && *buf != old.name {
            v.name = buf.trim().to_string();
            label = "命名符号";
        }
        if v.pixels.is_some() {
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::i18n::tr("编辑像素图…")).clicked() {
                    self.open_pixel_editor(Some(id));
                }
                if ui.button(crate::i18n::tr("用作新标志")).clicked() {
                    self.pixel_tools.stamp = v.pixels.clone();
                    self.pixel_tools.placing = true;
                }
                if ui.button(crate::i18n::tr("移除像素图")).clicked() {
                    v.pixels = None;
                }
            });
        } else if ui.button(crate::i18n::tr("绘制标志像素图…")).clicked() {
            self.open_pixel_editor(Some(id));
        }
        egui::ComboBox::from_label("类型").selected_text(crate::i18n::tr(v.kind_name())).show_ui(ui, |ui| {
            for k in SymbolKind::ALL {
                if ui.selectable_value(&mut v.kind, k, k.name()).changed() {
                    v.pixels = None;
                    v.color = if old.color == old.kind.default_color() { k.default_color() } else { old.color };
                    if k == SymbolKind::Glyph && v.glyph.is_empty() {
                        v.glyph = "◆".into();
                    }
                }
            }
        });
        if v.pixels.is_none() && v.kind == SymbolKind::Glyph {
            // follow the symbol's character unless it is being typed (it may
            // have just been set by choosing this kind)
            if self.sym_glyph.as_ref().is_none_or(|(x, t, typing)| *x != id || (!typing && *t != v.glyph)) {
                self.sym_glyph = Some((id, v.glyph.clone(), false));
            }
            let (_, buf, typing) = self.sym_glyph.as_mut().unwrap();
            let r = ui
                .horizontal(|ui| {
                    ui.label(crate::i18n::tr("字符"));
                    ui.add(egui::TextEdit::singleline(buf).desired_width(60.0))
                })
                .inner
                .on_hover_text(crate::i18n::tr("任意字符：字母、汉字（如“寺”“关”）或符号（如 ★ ◆ ✠）"));
            *typing = r.has_focus();
            if r.lost_focus() && !buf.trim().is_empty() && buf.trim() != old.glyph {
                v.glyph = buf.trim().to_string();
            }
        }
        if let Some(t) = crate::area_ui::legend_field(ui, &mut self.sym_legend, id, &old.legend) {
            v.legend = t;
        }
        if !v.name.is_empty() {
            // wraps: the tool panel is narrow
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::tr("名称位置"));
                for p in map_core::symbol::NamePlace::ALL {
                    ui.radio_value(&mut v.name_place, p, p.name());
                }
            });
        }
        let rs = ui.add(egui::Slider::new(&mut v.size, 3.0..=if v.pixels.is_some() { 96.0 } else { 30.0 }).suffix(" px").text(crate::i18n::tr("大小")));
        ui.horizontal(|ui| {
            let mut c = c32(v.color);
            if ui.color_edit_button_srgba(&mut c).changed() {
                v.color = c.to_array();
            }
            ui.label(crate::i18n::tr("颜色"));
            if ui.button(crate::i18n::tr("删除")).clicked() {
                self.selected_sym = Some(id);
                self.delete_selected_symbol();
            }
        });
        let mut drags = vec![rs];
        drags.extend(years_ui(ui, &mut v.from, &mut v.to, self.doc.year));
        let started = drags.iter().any(|r| r.drag_started());
        let stopped = drags.iter().any(|r| r.drag_stopped());
        if started {
            self.begin_pending("调整符号");
        }
        if v != old {
            if self.pending.is_some() {
                self.doc.symbols.insert(id, v);
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            } else {
                self.edit(label, |d| {
                    d.symbols.get(&id).ok_or("符号不存在")?;
                    d.symbols.insert(id, v);
                    Ok(())
                });
            }
        }
        if stopped {
            self.end_pending();
        }
    }
}
