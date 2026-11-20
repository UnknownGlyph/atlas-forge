//! The tools as a strip of icons down the left edge, and the options of
//! the tool in hand as a bar under the menu. The icons are drawn here, as
//! lines and shapes (no image files); a tool's tooltip gives its name, key
//! and how it is used. Tools with a larger panel of their own (map text,
//! symbols, measuring) open it beside the strip.

use crate::app::App;
use crate::view::{DrawMode, Tool};
use eframe::egui::{self, pos2, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke};

pub const BUTTON: f32 = 30.0;

/// Draw a tool's icon inside `r` (designed on a 20 × 20 grid).
fn tool_icon(p: &egui::Painter, r: Rect, tool: Tool, c: Color32) {
    let s = r.width().min(r.height()) / 26.0;
    let at = |x: f32, y: f32| pos2(r.center().x + (x - 10.0) * s, r.center().y + (y - 10.0) * s);
    let pts = |v: &[(f32, f32)]| v.iter().map(|(x, y)| at(*x, *y)).collect::<Vec<Pos2>>();
    let line = Stroke::new(1.6 * s.max(0.8), c);
    let thin = Stroke::new(1.1 * s.max(0.8), c.gamma_multiply(0.75));
    let soft = c.gamma_multiply(0.3);
    let land = [(4.0, 9.0), (6.0, 5.0), (10.0, 4.0), (14.0, 5.5), (16.0, 9.0), (15.0, 13.0), (11.0, 16.0), (7.0, 15.0), (4.5, 12.5)];
    let rect_at = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    match tool {
        Tool::Select => {
            p.add(Shape::convex_polygon(pts(&[(5.0, 3.0), (5.0, 15.5), (14.5, 12.0)]), c, Stroke::NONE));
            p.line_segment([at(8.5, 12.0), at(11.5, 17.5)], Stroke::new(2.4 * s, c));
        }
        Tool::DrawLand => {
            p.add(Shape::convex_polygon(pts(&land), soft, line));
        }
        Tool::DrawLake => {
            // a drop of water
            p.circle_filled(at(10.0, 12.0), 4.6 * s, c);
            p.add(Shape::convex_polygon(pts(&[(10.0, 2.5), (14.0, 10.0), (6.0, 10.0)]), c, Stroke::NONE));
        }
        Tool::Split => {
            p.add(Shape::closed_line(pts(&land), thin));
            p.line_segment([at(3.0, 16.5), at(17.0, 3.5)], line);
        }
        Tool::Redraw => {
            p.add(Shape::line(pts(&[(7.0, 3.0), (6.0, 8.0), (8.0, 13.0), (7.0, 17.0)]), thin));
            p.add(Shape::line(pts(&[(14.0, 3.0), (15.0, 8.0), (13.0, 13.0), (14.0, 17.0)]), line));
            p.line_segment([at(8.5, 10.0), at(12.0, 10.0)], thin);
            p.add(Shape::line(pts(&[(10.5, 8.5), (12.0, 10.0), (10.5, 11.5)]), thin));
        }
        Tool::EditVertex => {
            let v = [(4.0, 14.5), (9.5, 5.5), (16.0, 12.0)];
            p.add(Shape::line(pts(&v), thin));
            for (x, y) in v {
                p.rect_filled(Rect::from_center_size(at(x, y), egui::vec2(4.2 * s, 4.2 * s)), 0.0, c);
            }
        }
        Tool::Assign => {
            p.line_segment([at(16.0, 3.5), at(9.5, 10.5)], Stroke::new(2.6 * s, c));
            p.add(Shape::convex_polygon(pts(&[(8.0, 9.5), (11.0, 12.5), (7.0, 16.5), (3.5, 17.0), (4.5, 13.0)]), c, Stroke::NONE));
        }
        Tool::MergeFaces => {
            p.rect_stroke(rect_at(3.0, 5.0, 17.0, 15.0), 1.0, line, egui::StrokeKind::Middle);
            for y in [5.5, 9.0, 12.5] {
                p.line_segment([at(10.0, y), at(10.0, y + 2.0)], thin);
            }
        }
        Tool::City => {
            p.circle_stroke(at(10.0, 10.0), 5.5 * s, line);
            p.circle_filled(at(10.0, 10.0), 2.3 * s, c);
        }
        Tool::River => {
            p.add(Shape::line(pts(&[(3.0, 6.0), (7.0, 5.0), (9.5, 8.0), (8.0, 11.0), (11.0, 13.5), (14.5, 12.5), (17.0, 15.5)]), Stroke::new(2.0 * s, c)));
        }
        Tool::Pen => {
            // a pen nib
            p.add(Shape::closed_line(pts(&[(10.0, 2.5), (15.0, 11.0), (12.0, 17.5), (8.0, 17.5), (5.0, 11.0)]), line));
            p.line_segment([at(10.0, 2.5), at(10.0, 9.5)], thin);
            p.circle_filled(at(10.0, 11.0), 1.6 * s, c);
        }
        Tool::Eyedropper => {
            // a dropper: a bulb and a slanted tube
            p.circle_filled(at(14.5, 5.5), 3.0 * s, c);
            p.line_segment([at(12.5, 7.5), at(5.0, 15.0)], Stroke::new(2.4 * s, c));
            p.line_segment([at(5.0, 15.0), at(3.5, 16.5)], thin);
        }
        Tool::Label => {
            p.line_segment([at(4.5, 5.0), at(15.5, 5.0)], Stroke::new(2.2 * s, c));
            p.line_segment([at(10.0, 5.0), at(10.0, 16.5)], Stroke::new(2.2 * s, c));
        }
        Tool::Symbol => {
            p.add(Shape::convex_polygon(pts(&[(10.0, 4.0), (16.5, 15.5), (3.5, 15.5)]), soft, line));
        }
        Tool::Hatch => {
            p.rect_stroke(rect_at(4.0, 4.0, 16.0, 16.0), 0.0, thin, egui::StrokeKind::Middle);
            for (a, b) in [((4.0, 10.0), (10.0, 4.0)), ((4.0, 16.0), (16.0, 4.0)), ((10.0, 16.0), (16.0, 10.0))] {
                p.line_segment([at(a.0, a.1), at(b.0, b.1)], line);
            }
        }
        Tool::Measure => {
            p.rect_stroke(rect_at(2.5, 7.0, 17.5, 13.0), 0.0, line, egui::StrokeKind::Middle);
            for (i, x) in [5.5, 8.5, 11.5, 14.5].into_iter().enumerate() {
                p.line_segment([at(x, 7.0), at(x, if i % 2 == 0 { 10.5 } else { 9.0 })], thin);
            }
        }
        Tool::Align => {
            p.rect_stroke(rect_at(3.0, 5.0, 17.0, 15.0), 0.0, thin, egui::StrokeKind::Middle);
            p.circle_stroke(at(10.0, 10.0), 2.6 * s, line);
            p.line_segment([at(10.0, 6.0), at(10.0, 14.0)], thin);
            p.line_segment([at(6.0, 10.0), at(14.0, 10.0)], thin);
        }
    }
}

/// Whether the tool has a panel of its own beside the strip.
pub fn has_side_panel(tool: Tool) -> bool {
    matches!(tool, Tool::Label | Tool::Symbol | Tool::Measure)
}

impl App {
    /// The strip of tool icons.
    pub fn tool_strip(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, t) in Tool::ALL.into_iter().enumerate() {
            // groups: looking, land and borders, territory, things on the map, aids
            if matches!(i, 1 | 6 | 8 | 15) {
                ui.add_space(3.0);
                ui.separator();
                ui.add_space(3.0);
            }
            let (r, resp) = ui.allocate_exact_size(egui::vec2(BUTTON, BUTTON), Sense::click());
            let on = self.tool == t;
            let v = ui.visuals();
            let (bg, fg) = if on {
                (v.selection.bg_fill, v.selection.stroke.color)
            } else if resp.hovered() {
                (v.widgets.hovered.bg_fill, v.widgets.hovered.fg_stroke.color)
            } else {
                (Color32::TRANSPARENT, v.widgets.inactive.fg_stroke.color)
            };
            ui.painter().rect_filled(r, 4.0, bg);
            tool_icon(ui.painter(), r, t, fg);
            let key = t.key().name();
            if resp
                .on_hover_ui(|ui| {
                    ui.set_max_width(320.0);
                    ui.label(RichText::new(format!("{}  （{key}）", t.name())).strong());
                    ui.label(t.help());
                })
                .clicked()
            {
                self.set_tool(t);
            }
        }
        // the country the land and territory tools paint with
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        let (r, resp) = ui.allocate_exact_size(egui::vec2(BUTTON, 22.0), Sense::hover());
        let brush = self.brush.and_then(|b| self.doc.entities.get(&b));
        let sw = Rect::from_center_size(r.center(), egui::vec2(20.0, 20.0));
        match brush {
            Some(e) => {
                ui.painter().rect_filled(sw, 3.0, Color32::from_rgba_unmultiplied(e.color[0], e.color[1], e.color[2], 255));
                resp.on_hover_text(crate::trf!("当前画笔国家：{}（绘制陆地与领土画笔使用；在国家列表中更换）", e.name));
            }
            None => {
                ui.painter().rect_stroke(sw, 3.0, Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
                ui.painter().line_segment([sw.left_bottom(), sw.right_top()], Stroke::new(1.0, ui.visuals().weak_text_color()));
                resp.on_hover_text(crate::i18n::tr("还没有画笔国家：在国家列表中选择一个国家并“设为画笔”，绘制陆地与领土画笔会用到它。"));
            }
        }
    }

    /// The options of the tool in hand, in one row under the menu.
    pub fn options_bar(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
            tool_icon(ui.painter(), r, tool, ui.visuals().text_color());
            ui.label(RichText::new(tool.name()).strong());
            ui.separator();
            let drawing = matches!(tool, Tool::DrawLand | Tool::DrawLake | Tool::Split | Tool::Redraw | Tool::River | Tool::Hatch);
            if tool == Tool::EditVertex {
                ui.checkbox(&mut self.vertex_dated, crate::i18n::tr("选定年份时作为领土变化")).on_hover_text(crate::i18n::tr("开启：时间轴选定年份时，拖动或删除国界顶点等于从该年起移交土地，更早的地图不变。关闭：修改所有年份共用的边界形状。未选年份时总是直接修改形状。"));
            }
            if matches!(tool, Tool::DrawLand | Tool::DrawLake) {
                ui.selectable_value(&mut self.brush_paint, false, crate::i18n::tr("轮廓")).on_hover_text(crate::i18n::tr("单击加点或按住拖动手绘出轮廓"));
                ui.selectable_value(&mut self.brush_paint, true, crate::i18n::tr("涂抹")).on_hover_text(if tool == Tool::DrawLand {
                    crate::i18n::tr("按住拖动涂抹，松开即应用（重叠方式与年份同样生效）；单击点一笔；[ ] 调整大小。笔画围成的圈内会一并涂上。")
                } else {
                    crate::i18n::tr("按住拖动把陆地涂成水域（橡皮），松开即应用；单击点一笔；[ ] 调整大小。")
                });
                if self.brush_paint {
                    ui.spacing_mut().slider_width = 90.0;
                    ui.add(egui::Slider::new(&mut self.brush_px, 3.0..=150.0).suffix(" px")).on_hover_text(crate::i18n::tr("画笔半径；[ 与 ] 调整"));
                }
                ui.separator();
            }
            if tool == Tool::Pen {
                self.pen_options(ui);
                ui.separator();
            }
            use map_core::styles::StyleKind;
            let makes = match tool {
                Tool::River if self.route_mode => Some(StyleKind::Route),
                Tool::Pen => match self.pen_target {
                    crate::curve_ui::PenTarget::Route => Some(StyleKind::Route),
                    crate::curve_ui::PenTarget::Area => Some(StyleKind::Area),
                    crate::curve_ui::PenTarget::River => None,
                },
                Tool::Hatch => Some(StyleKind::Area),
                Tool::Label => Some(StyleKind::Text),
                Tool::Symbol => Some(StyleKind::Symbol),
                _ => None,
            };
            if let Some(k) = makes {
                self.draw_style_picker(ui, k);
            }
            if tool == Tool::River {
                ui.selectable_value(&mut self.route_mode, false, crate::i18n::tr("河流"));
                ui.selectable_value(&mut self.route_mode, true, crate::i18n::tr("线路")).on_hover_text(crate::i18n::tr("道路、商路、航线、行军路线等；所有年份相同。画好后可在所选线路面板中修改。"));
                if self.route_mode {
                    ui.menu_button(crate::i18n::tr("线路样式…"), |ui| {
                        let mut st = self.route_style;
                        crate::river_ui::route_style_ui(ui, &mut st);
                        self.route_style = st;
                    });
                }
                ui.separator();
            }
            if tool == Tool::Select {
                ui.checkbox(&mut self.smart_snap, crate::i18n::tr("智能吸附")).on_hover_text(crate::i18n::tr("拖动要素时，关键点（符号与文字的位置、线的两端与中点、区域中心）吸附到其他要素的关键点，或与它们的纬度、经度对齐；显示经纬网时也对齐经纬线。按住 Alt 临时关闭"));
            }
            if matches!(tool, Tool::DrawLand | Tool::DrawLake | Tool::Split | Tool::Redraw | Tool::Hatch) {
                ui.checkbox(&mut self.trace, crate::i18n::tr("沿边界描线")).on_hover_text(crate::i18n::tr("两个草稿点都吸附在边界上时，沿已有边界连接（都在海岸上时只沿海岸）；按住 Ctrl 临时关闭"));
            }
            if drawing {
                ui.label(crate::i18n::tr("线条"));
                egui::ComboBox::from_id_salt("line-style").selected_text(self.line_style.name()).width(96.0).show_ui(ui, |ui| {
                    for s in map_core::shape::LineStyle::ALL {
                        ui.selectable_value(&mut self.line_style, s, s.name());
                    }
                });
                if self.line_style == map_core::shape::LineStyle::Natural {
                    ui.spacing_mut().slider_width = 70.0;
                    ui.add(egui::Slider::new(&mut self.roughness, 0.05..=1.0).show_value(false)).on_hover_text(crate::i18n::tr("粗糙度"));
                }
                if self.line_style != map_core::shape::LineStyle::Plain && tool != Tool::River {
                    ui.checkbox(&mut self.keep_area, crate::i18n::tr("保持面积")).on_hover_text(crate::i18n::tr("修正风格化后的线条，使围出的面积与所点的形状相同。沿边界描出的部分保持原样；预览即最终形状。"));
                }
            }
            if tool == Tool::DrawLand {
                ui.separator();
                ui.label(crate::i18n::tr("重叠时"));
                let name = |m: DrawMode| match m {
                    DrawMode::AddLand => "只添加陆地",
                    DrawMode::Expand => "扩张画笔国家",
                    DrawMode::Take => "夺取领土",
                    DrawMode::NewCountry => "画出新国家",
                    DrawMode::Province(_) => "绘制下级地区",
                };
                let hint = |m: DrawMode| match m {
                    DrawMode::AddLand => "不动已有陆地，只在水域里添加",
                    DrawMode::Expand => "只占海和无主地，别国的土地不变",
                    DrawMode::Take => "区域全归画笔国家，包括别国的土地",
                    DrawMode::NewCountry => "新建一个国家，区域全归它",
                    DrawMode::Province(_) => "只划入上级直属陆地",
                };
                if let DrawMode::Province(parent) = self.draw_mode {
                    if let Some(unit) = self.doc.entities.get(&parent) {
                        ui.strong(crate::trf!("上级 · {}", unit.name));
                    }
                }
                egui::ComboBox::from_id_salt("draw-mode").selected_text(name(self.draw_mode)).width(110.0).show_ui(ui, |ui| {
                    for m in [DrawMode::AddLand, DrawMode::Expand, DrawMode::Take, DrawMode::NewCountry] {
                        ui.selectable_value(&mut self.draw_mode, m, name(m)).on_hover_text(hint(m));
                    }
                });
            }
            if tool == Tool::DrawLake {
                ui.weak(crate::i18n::tr("跨越边界时，覆盖的陆地都变为水域"));
            }
            if tool == Tool::Assign {
                ui.checkbox(&mut self.assign_fill, crate::i18n::tr("填充")).on_hover_text(crate::i18n::tr("单击即把与之相连、归属相同（或同为无主）的一整片陆地分配给画笔国家；右键取消这一片的归属。按住 Shift 单击也是填充。"));
            }
            if matches!(tool, Tool::DrawLand | Tool::Assign) {
                ui.separator();
                match self.brush.and_then(|b| self.doc.entities.get(&b)) {
                    Some(e) => {
                        crate::panels::swatch(ui, e.color);
                        ui.label(&e.name).on_hover_text(crate::i18n::tr("当前画笔国家"));
                    }
                    None => {
                        ui.weak(crate::i18n::tr("未设画笔国家")).on_hover_text(crate::i18n::tr("在国家列表中选择一个国家并设为画笔"));
                    }
                }
            }
            if !self.draft.is_empty() {
                ui.separator();
                if ui.button(crate::i18n::tr("完成")).on_hover_text(crate::i18n::tr("回车")).clicked() {
                    self.finish_draft();
                }
                if ui.button(crate::i18n::tr("取消")).on_hover_text("Esc").clicked() {
                    self.clear_draft();
                }
            }
            // what is left of the row: how the tool is used, in short
            let help = tool.help();
            let short = help.split(['；', '。']).next().unwrap_or(help);
            ui.add(egui::Label::new(RichText::new(short).weak()).truncate()).on_hover_text(help);
        });
    }

    /// The panel of a tool that has one (map text, symbols, measuring).
    pub fn tool_side_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        match self.tool {
            Tool::Label => self.annotation_panel(ui),
            Tool::Symbol => self.symbol_panel(ui),
            Tool::Measure => self.measure_panel(ui),
            _ => {}
        }
    }
}
