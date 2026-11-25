//! Named styles in the interface: the style library and palette (top of
//! the 样式 tab), the 样式 row of a feature's panel and of the selection
//! panel, the style new features are drawn in (options bar), the
//! eyedropper tool, and copying a look from one feature to others. The
//! styles themselves are `map_core::styles`.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use map_core::area::Pattern;
use map_core::features::FeatureRef;
use map_core::styles::{AreaLook, Look, StyleId, StyleKind, Swatch, SymbolLook, TextLook};

const STYLE_EDIT: &str = "修改样式";

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// A small picture of a look, beside its name.
fn look_sample(ui: &mut egui::Ui, look: &Look) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(34.0, 16.0), Sense::hover());
    let p = ui.painter();
    let col = c32(look.color());
    match look {
        Look::Route(st) => {
            let (a, b) = (r.left_center() + egui::vec2(2.0, 0.0), r.right_center() - egui::vec2(2.0, 0.0));
            let w = if st.broad { 6.0 } else { st.width.clamp(1.0, 4.0) };
            match st.dash {
                map_core::doc::Dash::Solid => {
                    p.line_segment([a, b], Stroke::new(w, col));
                }
                map_core::doc::Dash::Dashed => {
                    p.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(w, col), 5.0, 3.0));
                }
                map_core::doc::Dash::Dotted => {
                    p.extend(egui::Shape::dotted_line(&[a, b], col, 4.0, w * 0.5));
                }
            }
            if st.arrow {
                p.add(egui::Shape::convex_polygon(vec![b + egui::vec2(2.0, 0.0), b + egui::vec2(-4.0, -4.0), b + egui::vec2(-4.0, 4.0)], col, Stroke::NONE));
            }
        }
        Look::Area(a) => {
            let rr = r.shrink(1.0);
            let p = p.with_clip_rect(rr);
            if a.tint {
                p.rect_filled(rr, 1.0, col.gamma_multiply(0.25));
            }
            let mut x = rr.left() - rr.height();
            while x < rr.right() {
                let (s, e) = (egui::pos2(x, rr.bottom()), egui::pos2(x + rr.height(), rr.top()));
                if a.pattern == Pattern::Dots {
                    p.circle_filled(egui::pos2(x + rr.height() * 0.5, rr.center().y), 1.2, col);
                } else {
                    p.line_segment([s, e], Stroke::new(1.0, col));
                }
                x += 5.0;
            }
            p.rect_stroke(rr, 1.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
        }
        Look::Text(t) => {
            let font = egui::FontId::proportional(13.0);
            let mut job = egui::text::LayoutJob::default();
            job.append("文字", 0.0, egui::TextFormat { font_id: font, color: col, italics: t.italic, ..Default::default() });
            let g = p.layout_job(job);
            p.galley(r.center() - g.size() * 0.5, g, col);
        }
        Look::Symbol(s) => {
            p.circle_filled(r.center(), (s.size * 0.8).clamp(3.0, 7.0), col);
        }
    }
    resp
}

/// A colour button with the palette beside it; true when the colour changed.
fn color_field(ui: &mut egui::Ui, color: &mut [u8; 4], swatches: &[Swatch]) -> bool {
    let mut changed = false;
    let mut c = c32(*color);
    if ui.color_edit_button_srgba(&mut c).changed() {
        *color = c.to_array();
        changed = true;
    }
    changed | swatch_menu(ui, color, swatches)
}

/// The palette as a menu; true when a colour was picked from it.
fn swatch_menu(ui: &mut egui::Ui, color: &mut [u8; 4], swatches: &[Swatch]) -> bool {
    let mut changed = false;
    if !swatches.is_empty() {
        ui.menu_button(crate::i18n::tr("色板…"), |ui| {
            for s in swatches {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, c32(s.color));
                    if ui.selectable_label(s.color == *color, &s.name).clicked() {
                        *color = s.color;
                        changed = true;
                        ui.close();
                    }
                });
            }
        });
    }
    changed
}

/// What a control did: (changed, a drag started, a drag stopped).
type Did = (bool, bool, bool);

fn track(acc: &mut Did, r: egui::Response) {
    acc.0 |= r.changed();
    acc.1 |= r.drag_started();
    acc.2 |= r.drag_stopped();
}

/// The controls of a look. Returns (changed, a drag started, it stopped).
fn look_editor(ui: &mut egui::Ui, look: &mut Look, swatches: &[Swatch]) -> Did {
    let mut acc: Did = (false, false, false);
    match look {
        Look::Route(st) => {
            acc = crate::river_ui::route_style_ui(ui, st);
            if !swatches.is_empty() {
                ui.horizontal(|ui| {
                    ui.weak(crate::i18n::tr("从色板选颜色"));
                    acc.0 |= swatch_menu(ui, &mut st.color, swatches);
                });
            }
        }
        Look::Area(a) => {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("图案"));
                for p in Pattern::ALL {
                    track(&mut acc, ui.radio_value(&mut a.pattern, p, p.name()));
                }
            });
            track(&mut acc, ui.add(egui::Slider::new(&mut a.spacing, 3.0..=40.0).suffix(" px").text(crate::i18n::tr("间距"))));
            track(&mut acc, ui.add(egui::Slider::new(&mut a.angle, -90.0..=90.0).suffix("°").text(crate::i18n::tr("角度"))));
            ui.horizontal(|ui| {
                acc.0 |= color_field(ui, &mut a.color, swatches);
                ui.label(crate::i18n::tr("颜色"));
                track(&mut acc, ui.checkbox(&mut a.tint, crate::i18n::tr("底色")));
                track(&mut acc, ui.checkbox(&mut a.dashed, crate::i18n::tr("虚线边框")));
            });
        }
        Look::Text(t) => {
            track(&mut acc, ui.add(egui::Slider::new(&mut t.size, 2.0..=400.0).logarithmic(true).text(crate::i18n::tr("字号"))));
            track(&mut acc, ui.add(egui::Slider::new(&mut t.spacing, 0.0..=3.0).text(crate::i18n::tr("字距"))));
            ui.horizontal(|ui| {
                acc.0 |= color_field(ui, &mut t.color, swatches);
                ui.label(crate::i18n::tr("颜色"));
                track(&mut acc, ui.checkbox(&mut t.italic, crate::i18n::tr("斜体")));
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("字体"));
                acc.0 |= crate::fonts_ui::font_picker(ui, "text-style", &mut t.font, &mut t.bold);
            });
        }
        Look::Symbol(sy) => {
            track(&mut acc, ui.add(egui::Slider::new(&mut sy.size, 2.0..=30.0).suffix(" px").text(crate::i18n::tr("大小"))));
            ui.horizontal(|ui| {
                acc.0 |= color_field(ui, &mut sy.color, swatches);
                ui.label(crate::i18n::tr("颜色"));
            });
        }
    }
    acc
}

/// A look a new style of a kind starts with.
fn default_look(kind: StyleKind) -> Look {
    match kind {
        StyleKind::Route => Look::Route(map_core::doc::RouteStyle::default()),
        StyleKind::Area => Look::Area(AreaLook { color: [150, 30, 30, 255], spacing: 8.0, angle: 45.0, tint: false, pattern: Pattern::Lines, dashed: true }),
        StyleKind::Text => Look::Text(TextLook { size: 16.0, color: [40, 70, 110, 255], spacing: 0.3, italic: true, font: None, bold: false }),
        StyleKind::Symbol => Look::Symbol(SymbolLook { size: 7.0, color: [60, 60, 60, 255] }),
    }
}

enum Op {
    Presets,
    New(StyleKind),
    FromSelection,
    Open(Option<StyleId>),
    Rename(StyleId, String),
    Duplicate(StyleId),
    Remove(StyleId),
    Move(StyleId, usize),
    Apply(StyleId),
    SelectUsers(StyleId),
    DrawWith(StyleKind, Option<StyleId>),
    Look(StyleId, Look),
    LookDone,
    AddSwatch([u8; 4]),
    RemoveSwatch(usize),
    SwatchColor(usize, [u8; 4]),
    SwatchesFromMap,
}

impl App {
    /// The style a feature of this kind is drawn in when made (none: its
    /// own default look).
    pub fn draw_style_of(&self, kind: StyleKind) -> Option<StyleId> {
        self.draw_style.get(&kind).copied().filter(|s| self.doc.style(*s).is_some_and(|x| x.look.kind() == kind))
    }

    /// A feature just made takes the style chosen for its kind.
    pub fn style_new(&mut self, f: FeatureRef) {
        if let Some(s) = StyleKind::of(f).and_then(|k| self.draw_style_of(k)) {
            let _ = self.doc.link_style(&[f], s);
        }
    }

    /// The style picker of the options bar, for the kind a tool makes.
    pub fn draw_style_picker(&mut self, ui: &mut egui::Ui, kind: StyleKind) {
        let styles: Vec<(StyleId, String, Look)> = self.doc.styles.iter().filter(|s| s.look.kind() == kind).map(|s| (s.id, s.name.clone(), s.look.clone())).collect();
        if styles.is_empty() {
            return;
        }
        let cur = self.draw_style_of(kind);
        let name = cur.and_then(|s| styles.iter().find(|x| x.0 == s)).map_or(crate::i18n::tr("（自身外观）"), |x| x.1.as_str());
        ui.label(crate::i18n::tr("样式"));
        egui::ComboBox::from_id_salt(("draw-style", kind)).selected_text(name).width(110.0).show_ui(ui, |ui| {
            if ui.selectable_label(cur.is_none(), crate::i18n::tr("（自身外观）")).on_hover_text(crate::i18n::tr("新画的要素不使用样式，外观可逐个设置")).clicked() {
                self.draw_style.remove(&kind);
            }
            for (id, n, look) in &styles {
                ui.horizontal(|ui| {
                    look_sample(ui, look);
                    if ui.selectable_label(cur == Some(*id), n).clicked() {
                        self.draw_style.insert(kind, *id);
                    }
                });
            }
        })
        .response
        .on_hover_text(crate::i18n::tr("新画的要素使用这个样式；之后修改样式，它们会一起改变"));
        ui.separator();
    }

    /// The 样式 row of a panel, for the given features: the style they
    /// follow, choosing one, making one, leaving it.
    pub fn style_row(&mut self, ui: &mut egui::Ui, fs: &[FeatureRef]) {
        let stylable: Vec<FeatureRef> = fs.iter().copied().filter(|f| StyleKind::of(*f).is_some() && self.doc.feature_look(*f).is_some()).collect();
        if stylable.is_empty() {
            return;
        }
        let kinds: std::collections::BTreeSet<StyleKind> = stylable.iter().filter_map(|f| StyleKind::of(*f)).collect();
        let links: std::collections::BTreeSet<Option<StyleId>> = stylable.iter().map(|f| self.doc.style_of_feature(*f)).collect();
        let current = (links.len() == 1).then(|| *links.iter().next().unwrap()).flatten();
        let choices: Vec<(StyleId, String, Look)> = self.doc.styles.iter().filter(|s| kinds.contains(&s.look.kind())).map(|s| (s.id, s.name.clone(), s.look.clone())).collect();
        let mut apply: Option<StyleId> = None;
        let (mut make, mut leave) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(crate::i18n::tr("样式")).strong());
            let text = match (current, links.len()) {
                (Some(s), _) => self.doc.style(s).map(|x| x.name.clone()).unwrap_or_default(),
                (None, 1) => crate::i18n::tr("（自身外观）").to_owned(),
                _ => crate::i18n::tr("（多种）").to_owned(),
            };
            egui::ComboBox::from_id_salt("style-row").selected_text(text).width(120.0).show_ui(ui, |ui| {
                if choices.is_empty() {
                    ui.weak(crate::i18n::tr("还没有这类样式：点“建立样式”，或在“样式”标签中添加常用样式"));
                }
                for (id, n, look) in &choices {
                    ui.horizontal(|ui| {
                        look_sample(ui, look);
                        if ui.selectable_label(current == Some(*id), n).clicked() {
                            apply = Some(*id);
                        }
                    });
                }
            });
            if ui.small_button(crate::i18n::tr("建立样式")).on_hover_text(crate::i18n::tr("以这个外观建立一个命名样式，所选同类要素都使用它")).clicked() {
                make = true;
            }
            if links.iter().any(|l| l.is_some()) && ui.small_button(crate::i18n::tr("脱离样式")).on_hover_text(crate::i18n::tr("保留现在的外观，但不再随样式改变；之后可以单独修改")).clicked() {
                leave = true;
            }
        });
        if let Some(s) = current {
            let n = self.doc.style_users(s).len();
            if n > stylable.len() {
                ui.weak(crate::trf!("修改外观会改变这个样式，使用它的全部 {n} 个要素都会随之改变；只改这里请先“脱离样式”。"));
            }
        }
        if let Some(s) = apply {
            if let Some(n) = self.edit("使用样式", |d| d.link_style(&stylable, s)) {
                let name = self.doc.style(s).map(|x| x.name.clone()).unwrap_or_default();
                self.set_status(crate::trf!("{n} 个要素使用样式“{name}”"), false);
            }
        }
        if make {
            let first = stylable[0];
            let kind = StyleKind::of(first).unwrap();
            let same: Vec<FeatureRef> = stylable.iter().copied().filter(|f| StyleKind::of(*f) == Some(kind)).collect();
            if let Some(s) = self.edit("建立样式", |d| {
                let s = d.style_from_feature(first, "")?;
                d.link_style(&same, s)?;
                Ok(s)
            }) {
                let name = self.doc.style(s).map(|x| x.name.clone()).unwrap_or_default();
                self.style_open = Some(s);
                self.set_status(crate::trf!("已建立样式“{name}”，{} 个要素使用它；在“样式”标签中可改名与修改", same.len()), false);
            }
        }
        if leave {
            self.edit("脱离样式", |d| {
                d.unlink_style(&stylable);
                Ok(())
            });
        }
    }

    /// Copy the look of the (first) selected feature, for 粘贴样式.
    pub fn copy_style(&mut self) {
        let Some(f) = self.selection().into_iter().find(|f| self.doc.feature_look(*f).is_some()) else {
            return self.set_status(crate::i18n::tr("所选要素没有可复制的样式"), true);
        };
        let look = self.doc.feature_look(f).unwrap();
        self.style_clip = Some((look, self.doc.style_of_feature(f)));
        self.set_status(crate::trf!("已复制{}的样式：选中其他同类要素后“粘贴样式”", f.kind_name()), false);
    }

    /// Give the selection the copied look (and its style, if it had one).
    pub fn paste_style(&mut self) {
        let Some((look, link)) = self.style_clip.clone() else { return };
        let fs: Vec<FeatureRef> = self.selection().into_iter().filter(|f| StyleKind::of(*f) == Some(look.kind())).collect();
        if fs.is_empty() {
            return self.set_status(crate::trf!("所选要素中没有{}，不能粘贴这个样式", look.kind().name()), true);
        }
        let link = link.filter(|s| self.doc.style(*s).is_some_and(|x| x.look == look));
        if let Some(n) = self.edit("粘贴样式", |d| {
            match link {
                Some(s) => d.link_style(&fs, s)?,
                None => {
                    for f in &fs {
                        d.set_feature_look(*f, &look)?;
                    }
                    d.unlink_style(&fs);
                    fs.len()
                }
            };
            Ok(fs.len())
        }) {
            self.set_status(crate::trf!("已把样式粘贴到 {n} 个要素"), false);
        }
    }

    /// The eyedropper: a click on a feature gives its look (and style) to
    /// the selected features of its kind.
    pub fn eyedropper_input(&mut self, resp: &egui::Response, vg: &ViewGeom) {
        if !resp.clicked() {
            return;
        }
        let Some(o) = resp.interact_pointer_pos() else { return };
        let Some(from) = self.feature_at(o, vg).filter(|f| self.doc.feature_look(*f).is_some()) else {
            return self.set_status(crate::i18n::tr("这里没有可吸取样式的线路、区域、文字或符号"), true);
        };
        let to: Vec<FeatureRef> = self.selection().into_iter().filter(|f| *f != from).collect();
        if to.is_empty() {
            return self.set_status(crate::i18n::tr("先用选择工具（V）选中要修改的要素，再用吸管单击样板"), true);
        }
        match self.edit("吸取样式", |d| d.copy_look(from, &to)) {
            Some(0) => self.set_status(crate::trf!("所选要素中没有{}，吸管只把样式给同类要素", from.kind_name()), true),
            Some(n) => {
                let via = self.doc.style_of_feature(from).and_then(|s| self.doc.style(s)).map(|s| format!("样式“{}”", s.name)).unwrap_or_else(|| "外观".into());
                self.set_status(crate::trf!("{n} 个要素换成了这个{}的{via}", from.kind_name()), false);
            }
            None => {}
        }
    }

    /// After each frame: an edit made to a linked feature's look changes
    /// its style, and every feature linked to it.
    pub fn settle_styles(&mut self, ctx: &egui::Context) {
        // a style being changed in the library: one undo step once the
        // pointer is let go (a colour picker reports no 'drag stopped')
        if self.pending.as_ref().is_some_and(|p| p.label == STYLE_EDIT) && !ctx.input(|i| i.pointer.any_down()) && ctx.memory(|m| m.focused().is_none()) {
            self.end_pending();
        }
        let changed = self.doc.settle_styles();
        if let Some(st) = changed.first().and_then(|s| self.doc.style(*s)) {
            let (name, n) = (st.name.clone(), self.doc.style_users(st.id).len());
            if n > 1 {
                self.set_status(crate::trf!("样式“{name}”已随之修改：使用它的 {n} 个要素都已更新"), false);
            }
        }
    }

    /// The style library and the palette (top of the 样式 tab).
    pub fn styles_section(&mut self, ui: &mut egui::Ui) {
        if self.style_open.is_some_and(|s| self.doc.style(s).is_none()) {
            self.style_open = None;
        }
        let mut ops: Vec<Op> = Vec::new();
        let sel = self.selection();
        let sel_kinds: std::collections::BTreeSet<StyleKind> = sel.iter().filter(|f| self.doc.feature_look(**f).is_some()).filter_map(|f| StyleKind::of(*f)).collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(crate::i18n::tr("要素样式")).strong());
            if ui.small_button(crate::i18n::tr("添加常用样式")).on_hover_text(crate::i18n::tr("进军路线、前线、铁路、商路、争议地区、占领区、海洋名称……已有的不重复添加")).clicked() {
                ops.push(Op::Presets);
            }
            if ui.add_enabled(!sel_kinds.is_empty(), egui::Button::new(crate::i18n::tr("从所选建立")).small()).on_hover_text(crate::i18n::tr("以所选要素的外观建立样式，所选同类要素都使用它")).clicked() {
                ops.push(Op::FromSelection);
            }
            ui.menu_button(crate::i18n::tr("新建…"), |ui| {
                for k in StyleKind::ALL {
                    if ui.button(k.name()).clicked() {
                        ops.push(Op::New(k));
                    }
                }
            });
        });
        if self.doc.styles.is_empty() {
            ui.weak(crate::i18n::tr("样式是有名字的外观（线路的颜色与线型、区域的图案、文字的字号……）。要素使用样式后，修改样式即可一次改变全部使用它的要素。没有使用样式的要素保持各自的外观。"));
        }
        let swatches = self.doc.swatches.clone();
        let styles = self.doc.styles.clone();
        for kind in StyleKind::ALL {
            let of_kind: Vec<&map_core::styles::NamedStyle> = styles.iter().filter(|s| s.look.kind() == kind).collect();
            if of_kind.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(kind.name()).strong());
                let draw = self.draw_style_of(kind);
                if let Some(s) = draw.and_then(|s| self.doc.style(s)) {
                    ui.weak(crate::trf!("新画的用“{}”", s.name));
                }
            });
            let place = |id: StyleId| styles.iter().position(|s| s.id == id).unwrap_or(0);
            for (i, st) in of_kind.iter().enumerate() {
                let users = self.doc.style_users(st.id).len();
                let open = self.style_open == Some(st.id);
                ui.push_id(st.id.0, |ui| {
                    ui.horizontal(|ui| {
                        look_sample(ui, &st.look);
                        match &mut self.style_rename {
                            Some((id, text, focus)) if *id == st.id => {
                                let r = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                                if *focus {
                                    r.request_focus();
                                    *focus = false;
                                } else if r.lost_focus() {
                                    ops.push(Op::Rename(st.id, text.clone()));
                                }
                            }
                            _ => {
                                let r = ui.selectable_label(open, &st.name).on_hover_text(crate::i18n::tr("单击展开修改；双击改名；右键更多操作"));
                                if r.double_clicked() {
                                    self.style_rename = Some((st.id, st.name.clone(), true));
                                } else if r.clicked() {
                                    ops.push(Op::Open((!open).then_some(st.id)));
                                }
                                r.context_menu(|ui| {
                                    if ui.button(crate::i18n::tr("改名")).clicked() {
                                        self.style_rename = Some((st.id, st.name.clone(), true));
                                    }
                                    if ui.button(crate::i18n::tr("复制样式")).clicked() {
                                        ops.push(Op::Duplicate(st.id));
                                    }
                                    if ui.button(crate::i18n::tr("选中使用它的要素")).clicked() {
                                        ops.push(Op::SelectUsers(st.id));
                                    }
                                    if ui.button(crate::i18n::tr("新画的要素用它")).clicked() {
                                        ops.push(Op::DrawWith(kind, Some(st.id)));
                                    }
                                    ui.separator();
                                    if ui.add_enabled(i > 0, egui::Button::new(crate::i18n::tr("上移"))).clicked() {
                                        ops.push(Op::Move(st.id, place(of_kind[i - 1].id)));
                                    }
                                    if ui.add_enabled(i + 1 < of_kind.len(), egui::Button::new(crate::i18n::tr("下移"))).clicked() {
                                        ops.push(Op::Move(st.id, place(of_kind[i + 1].id)));
                                    }
                                    ui.separator();
                                    if ui.button(crate::i18n::tr("删除样式（要素保留外观）")).clicked() {
                                        ops.push(Op::Remove(st.id));
                                    }
                                });
                            }
                        }
                        ui.weak(crate::trf!("{users} 个"));
                        if sel_kinds.contains(&kind) && ui.small_button(crate::i18n::tr("应用到所选")).clicked() {
                            ops.push(Op::Apply(st.id));
                        }
                    });
                    if open {
                        ui.indent("look", |ui| {
                            let mut look = st.look.clone();
                            let (changed, _, stopped) = look_editor(ui, &mut look, &swatches);
                            if changed {
                                ops.push(Op::Look(st.id, look));
                            }
                            if stopped || (changed && !ui.ctx().input(|i| i.pointer.any_down())) {
                                ops.push(Op::LookDone);
                            }
                            ui.weak(if users > 0 { crate::trf!("修改立即应用到使用它的 {users} 个要素。") } else { "还没有要素使用它：在要素面板的“样式”中选择，或用吸管。".into() });
                        });
                    }
                });
            }
        }
        // the palette
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(crate::i18n::tr("色板")).strong());
            let mut c = c32(self.new_swatch);
            if ui.color_edit_button_srgba(&mut c).changed() {
                self.new_swatch = c.to_array();
            }
            if ui.small_button(crate::i18n::tr("＋ 加入")).on_hover_text(crate::i18n::tr("把左边的颜色加入色板；样式与要素的颜色旁都可以从色板选色")).clicked() {
                ops.push(Op::AddSwatch(self.new_swatch));
            }
            if ui.small_button(crate::i18n::tr("取自地图")).on_hover_text(crate::i18n::tr("把地图上要素与样式用得最多的颜色加入色板（最多 12 种）")).clicked() {
                ops.push(Op::SwatchesFromMap);
            }
        });
        if !swatches.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (i, s) in swatches.iter().enumerate() {
                    let mut c = c32(s.color);
                    let r = ui.color_edit_button_srgba(&mut c).on_hover_text(crate::trf!("{}（右键删除）", s.name));
                    if r.changed() {
                        ops.push(Op::SwatchColor(i, c.to_array()));
                    }
                    r.context_menu(|ui| {
                        if ui.button(crate::i18n::tr("从色板删除")).clicked() {
                            ops.push(Op::RemoveSwatch(i));
                        }
                    });
                }
            });
        }
        ui.separator();
        self.run_style_ops(ops, sel);
    }

    fn run_style_ops(&mut self, ops: Vec<Op>, sel: Vec<FeatureRef>) {
        for op in ops {
            match op {
                Op::Presets => {
                    if let Some(n) = self.edit("添加常用样式", |d| Ok(d.add_preset_styles())) {
                        self.set_status(if n == 0 { "常用样式都已在样式库中".to_string() } else { crate::trf!("已添加 {n} 个常用样式") }, false);
                    }
                }
                Op::New(k) => {
                    if let Some(s) = self.edit("新建样式", |d| Ok(d.add_style("", default_look(k)))) {
                        self.style_open = Some(s);
                        let name = self.doc.style(s).map(|x| x.name.clone()).unwrap_or_default();
                        self.style_rename = Some((s, name, true));
                    }
                }
                Op::FromSelection => {
                    let Some(first) = sel.iter().copied().find(|f| self.doc.feature_look(*f).is_some()) else { continue };
                    let kind = StyleKind::of(first);
                    let same: Vec<FeatureRef> = sel.iter().copied().filter(|f| StyleKind::of(*f) == kind).collect();
                    if let Some(s) = self.edit("建立样式", |d| {
                        let s = d.style_from_feature(first, "")?;
                        d.link_style(&same, s)?;
                        Ok(s)
                    }) {
                        self.style_open = Some(s);
                        let name = self.doc.style(s).map(|x| x.name.clone()).unwrap_or_default();
                        self.style_rename = Some((s, name, true));
                    }
                }
                Op::Open(s) => self.style_open = s,
                Op::Rename(s, name) => {
                    self.style_rename = None;
                    if !name.trim().is_empty() && self.doc.style(s).is_some_and(|x| x.name != name.trim()) {
                        self.edit("样式改名", |d| d.rename_style(s, &name));
                    }
                }
                Op::Duplicate(s) => {
                    if let Some(n) = self.edit("复制样式", |d| d.duplicate_style(s)) {
                        self.style_open = Some(n);
                    }
                }
                Op::Remove(s) => {
                    if let Some(n) = self.edit("删除样式", |d| d.remove_style(s)) {
                        self.set_status(crate::trf!("样式已删除；使用它的 {n} 个要素保留现在的外观（Ctrl+Z 撤销）"), false);
                    }
                }
                Op::Move(s, to) => {
                    self.edit("调整样式顺序", |d| d.move_style(s, to));
                }
                Op::Apply(s) => {
                    if let Some(n) = self.edit("使用样式", |d| d.link_style(&sel, s)) {
                        self.set_status(crate::trf!("{n} 个要素使用了这个样式"), false);
                    }
                }
                Op::SelectUsers(s) => {
                    let fs: Vec<FeatureRef> = self.doc.style_users(s).into_iter().filter(|f| self.doc.pickable(*f)).collect();
                    let n = fs.len();
                    self.set_tool(Tool::Select);
                    self.set_selection(fs);
                    self.set_status(crate::trf!("已选择使用这个样式的 {n} 个要素"), false);
                }
                Op::DrawWith(k, s) => {
                    match s {
                        Some(s) => self.draw_style.insert(k, s),
                        None => self.draw_style.remove(&k),
                    };
                }
                // one undo step for a whole drag of a slider
                Op::Look(s, look) => {
                    if self.pending.is_none() {
                        self.begin_pending(STYLE_EDIT);
                    }
                    if self.doc.update_style(s, look).is_ok() {
                        if let Some(pe) = &mut self.pending {
                            pe.changed = true;
                        }
                    }
                }
                Op::LookDone => {
                    if self.pending.as_ref().is_some_and(|p| p.label == STYLE_EDIT) {
                        self.end_pending();
                    }
                }
                Op::AddSwatch(c) => {
                    self.edit("加入色板", |d| Ok(d.add_swatch("", c)));
                }
                Op::RemoveSwatch(i) => {
                    self.edit("从色板删除", |d| d.remove_swatch(i));
                }
                Op::SwatchColor(i, c) => {
                    if let Some(s) = self.doc.swatches.get_mut(i) {
                        s.color = c;
                        self.dirty = true;
                    }
                }
                Op::SwatchesFromMap => {
                    let used: Vec<[u8; 4]> = self.doc.colors_in_use().into_iter().take(12).map(|c| c.0).collect();
                    if let Some(n) = self.edit("从地图取色", |d| {
                        let before = d.swatches.len();
                        for c in &used {
                            d.add_swatch("", *c);
                        }
                        Ok(d.swatches.len() - before)
                    }) {
                        self.set_status(crate::trf!("色板中加入了 {n} 种地图上用到的颜色"), false);
                    }
                }
            }
        }
    }
}
