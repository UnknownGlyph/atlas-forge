//! The studio arranges the same editing session around project navigation
//! and task workspaces. The professional shell keeps its own navigation.
use crate::{
    actions::{self, Act, Menu},
    app::{App, Layout},
    i18n::tr,
    view::{GlobeInertia, Tool, ViewKind},
};
use eframe::egui::{self, Color32, RichText};
use map_core::world_book::{Node, WorldBook};
use map_render::camera::{Camera2D, GlobeCamera};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Map,
    Sources,
    Worldview,
    Visualizations,
    History,
    Environment,
    Stars,
}

struct Navigation {
    flat: Camera2D,
    globe: GlobeCamera,
    spin: GlobeInertia,
    rect: egui::Rect,
    fit: bool,
    touched: bool,
    layout: Layout,
    stars: bool,
}

impl Navigation {
    fn initial(app: &App) -> Self {
        Self {
            flat: app.cam2d,
            globe: app.globe,
            spin: Default::default(),
            rect: app.last_flat_rect,
            fit: app.fit_pending,
            touched: app.nav_touched,
            layout: app.layout,
            stars: false,
        }
    }
    fn swap(&mut self, app: &mut App) {
        use std::mem::swap;
        swap(&mut self.flat, &mut app.cam2d);
        swap(&mut self.globe, &mut app.globe);
        swap(&mut self.spin, &mut app.globe_inertia);
        swap(&mut self.rect, &mut app.last_flat_rect);
        swap(&mut self.fit, &mut app.fit_pending);
        swap(&mut self.touched, &mut app.nav_touched);
        swap(&mut self.layout, &mut app.layout);
        swap(&mut self.stars, &mut app.star_map.open);
    }
}

#[derive(Default)]
pub(crate) struct Studio {
    pub foreground: bool,
    switch_pending: bool,
    professional_visuals: Option<egui::Visuals>,
    page: Page,
    navigation: Option<Navigation>,
    inspector: bool,
}

impl Studio {
    pub(crate) fn worldview_visible(&self) -> bool { self.foreground && self.page == Page::Worldview }
    pub(crate) fn visualizations_visible(&self) -> bool { self.foreground && self.page == Page::Visualizations }
    pub(crate) fn environment_visible(&self) -> bool { self.foreground && self.page == Page::Environment }
    pub(crate) fn show_worldview(&mut self) {
        self.page = Page::Worldview;
    }
    pub(crate) fn show_visualizations(&mut self) {
        self.page = Page::Visualizations;
    }
    pub(crate) fn show_environment(&mut self) {
        self.page = Page::Environment;
    }
    pub(crate) fn show_map(&mut self) {
        self.page = Page::Map;
    }
}

fn parchment_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    let ink = Color32::from_rgb(57, 41, 26);
    let line = Color32::from_rgb(173, 139, 94);
    v.override_text_color = Some(ink);
    v.panel_fill = Color32::from_rgb(243, 231, 204);
    v.window_fill = Color32::from_rgb(249, 239, 216);
    v.extreme_bg_color = Color32::from_rgb(252, 244, 224);
    v.faint_bg_color = Color32::from_rgb(235, 218, 183);
    v.window_stroke = egui::Stroke::new(1.0, line);
    v.hyperlink_color = Color32::from_rgb(117, 70, 35);
    v.selection.bg_fill = Color32::from_rgb(221, 188, 132);
    v.selection.stroke = egui::Stroke::new(1.0, ink);
    for (widget, color) in [
        (&mut v.widgets.noninteractive, Color32::from_rgb(243, 231, 204)),
        (&mut v.widgets.inactive, Color32::from_rgb(236, 220, 187)),
        (&mut v.widgets.hovered, Color32::from_rgb(224, 200, 154)),
        (&mut v.widgets.active, Color32::from_rgb(215, 185, 130)),
        (&mut v.widgets.open, Color32::from_rgb(229, 208, 169)),
    ] {
        widget.bg_fill = color;
        widget.weak_bg_fill = color;
        widget.bg_stroke = egui::Stroke::new(1.0, line);
        widget.fg_stroke.color = ink;
    }
    v
}

impl App {
    pub(crate) fn switch_workspace(&mut self) {
        self.studio.switch_pending = !self.studio.switch_pending;
    }

    pub(crate) fn apply_workspace_switch(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.studio.switch_pending) {
            return;
        }
        self.finish_pending_settings();
        let mut nav = self.studio.navigation.take().unwrap_or_else(|| {
            let mut nav = Navigation::initial(self);
            nav.fit = true;
            nav.stars = !self.studio.foreground && self.studio.page == Page::Stars;
            nav
        });
        nav.swap(self);
        self.globe_inertia.stop();
        self.studio.navigation = Some(nav);
        self.studio.foreground = !self.studio.foreground;
        if self.studio.foreground {
            self.studio.professional_visuals = Some(ctx.global_style().visuals.clone());
            self.studio.inspector = true;
            ctx.set_visuals(parchment_visuals());
        } else if let Some(visuals) = self.studio.professional_visuals.take() {
            ctx.set_visuals(visuals);
        }
    }

    pub(crate) fn reset_studio_navigation(&mut self) {
        self.studio.navigation = None;
    }

    pub(crate) fn studio_workspace(&mut self, ui: &mut egui::Ui) {
        if self.star_map.open {
            self.studio.page = Page::Stars;
        } else if self.studio.page == Page::Stars {
            self.studio.page = Page::Map;
        }
        self.studio_header(ui);
        if self.studio.page != Page::Environment {
            self.environment_ui.stop_picking();
        }
        self.studio_project(ui);
        if matches!(self.studio.page, Page::Worldview | Page::Visualizations) {
            egui::Panel::bottom("studio-study-timeline").show(ui, |ui| self.timeline_bar(ui));
        }
        match self.studio.page {
            Page::Stars => self.star_chart(ui),
            Page::Sources => self.studio_sources(ui),
            Page::Worldview => self.studio_worldview(ui),
            Page::Visualizations => self.studio_visualizations(ui),
            Page::Map | Page::History | Page::Environment => self.studio_map(ui),
        }
    }

    fn studio_header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("studio-header").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(RichText::new("Atlas Forge").color(Color32::from_rgb(96, 56, 29)));
                ui.separator();
                for (page, title) in [
                    (Page::Map, "地图"),
                    (Page::Sources, "世界资料"),
                    (Page::Worldview, "世界观"),
                    (Page::Visualizations, "图表"),
                    (Page::History, "历史"),
                    (Page::Environment, "环境"),
                    (Page::Stars, "星图"),
                ] {
                    if ui
                        .selectable_value(&mut self.studio.page, page, tr(title))
                        .changed()
                    {
                        self.star_map.open = page == Page::Stars;
                    }
                }
                ui.separator();
                if ui.button(tr("保存")).clicked() {
                    self.run(Act::Save);
                }
                if ui.button(tr("设置")).clicked() {
                    self.run(Act::Prefs);
                }
                if ui.button(tr("切回专业界面")).clicked() {
                    self.switch_workspace();
                }
                ui.menu_button(tr("所有命令"), |ui| self.studio_commands(ui));
            });
        });
    }

    fn studio_commands(&mut self, ui: &mut egui::Ui) {
        let commands = actions::table();
        for group in Menu::ALL {
            ui.menu_button(group.name(), |ui| {
                for command in commands.iter().filter(|a| a.menu == group) {
                    let enabled = self.can(command.act);
                    if ui
                        .add_enabled(enabled, egui::Button::new(command.name))
                        .on_hover_text(command.hint)
                        .clicked()
                    {
                        self.run(command.act);
                        if command.act == Act::StarChart {
                            self.studio.page = if self.star_map.open {
                                Page::Stars
                            } else {
                                Page::Map
                            };
                        }
                        ui.close();
                    }
                }
            });
        }
    }

    fn studio_project(&mut self, ui: &mut egui::Ui) {
        let current = WorldBook::current(&self.doc);
        let boards: Vec<_> = self
            .doc
            .book
            .as_ref()
            .map(|book| {
                book.boards
                    .keys()
                    .copied()
                    .filter_map(|id| book.map(&self.doc, id).map(|d| (id, d.meta.name.clone())))
                    .collect()
            })
            .unwrap_or_else(|| vec![(current, self.doc.meta.name.clone())]);
        let mut board = None;
        egui::Panel::left("studio-project")
            .default_size(235.0)
            .size_range(160.0..=348.0)
            .show(ui, |ui| {
                ui.heading(tr("项目"));
                ui.strong(tr("画板"));
                egui::ScrollArea::vertical()
                    .max_height(270.0)
                    .show(ui, |ui| {
                        for (id, name) in boards {
                            if ui.selectable_label(id == current, name).clicked() {
                                board = Some(id);
                            }
                        }
                    });
                ui.separator();
                for (act, label) in [
                    (Act::Events, "事件时间线…"),
                    (Act::TimelinePlayback, "时间播放与视频"),
                    (Act::Diplomacy, "外交关系…"),
                    (Act::Checkpoints, "保存点与自动保存…"),
                ] {
                    if ui
                        .add_enabled(self.can(act), egui::Button::new(tr(label)))
                        .clicked()
                    {
                        self.run(act);
                    }
                }
            });
        if let Some(id) = board.filter(|id| *id != current) {
            if let Err(e) = self.enter_book_map(id, None, true) {
                self.set_status(e, true);
            }
            self.studio.page = Page::Map;
        }
    }

    fn studio_sources(&mut self, ui: &mut egui::Ui) {
        let counts = self
            .doc
            .book
            .as_ref()
            .map(|b| (b.articles.len(), b.assets.len(), b.links.len()))
            .unwrap_or_default();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(tr("世界资料"));
            ui.horizontal_wrapped(|ui| {
                for (label, count) in [
                    ("文档", counts.0),
                    ("图片与附件", counts.1),
                    ("关联", counts.2),
                ] {
                    ui.group(|ui| {
                        ui.label(tr(label));
                        ui.heading(count.to_string());
                    });
                }
            });
            ui.separator();
            if ui.button(tr("世界观设定…")).clicked() {
                self.open_worldview(None);
            }
            if ui.button(tr("世界资料库…")).clicked() {
                self.run(Act::WorldBook);
            }
            if ui.button(tr("地图")).clicked() {
                self.studio.page = Page::Map;
            }
            if let Some(book) = &self.doc.book {
                let articles: Vec<_> = book
                    .articles
                    .iter()
                    .map(|(&id, a)| (id, a.name.clone()))
                    .collect();
                let models: Vec<_> = book
                    .assets
                    .iter()
                    .filter(|(_, a)| map_io::model::is_model(a))
                    .map(|(&id, a)| (id, a.name.clone()))
                    .collect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (id, title) in articles {
                        if ui.link(title).clicked() {
                            self.book_context(Node::Article(id));
                        }
                    }
                    for (id, title) in models {
                        if ui.link(title).clicked() {
                            self.open_model_asset(id);
                        }
                    }
                });
            }
        });
    }

    fn studio_map(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("studio-tools").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.menu_button(tr("工具"), |ui| {
                    for tool in Tool::ALL {
                        if ui
                            .selectable_label(self.tool == tool, tool.name())
                            .on_hover_text(tool.help())
                            .clicked()
                        {
                            self.set_tool(tool);
                            ui.close();
                        }
                    }
                });
                for (act, name) in [
                    (Act::LandMove, "整体移动陆块"),
                    (Act::LandScale, "整体缩放陆块"),
                    (Act::Export, "导出…"),
                    (Act::GeographicGuides, "地理参考线…"),
                ] {
                    if ui
                        .add_enabled(self.can(act), egui::Button::new(tr(name)))
                        .clicked()
                    {
                        self.run(act);
                    }
                }
                if self.studio.page == Page::Environment
                    && ui.button(tr("行星环境与气候…")).clicked()
                {
                    self.run(Act::Climate);
                }
                if self.studio.page == Page::History && ui.button(tr("时间播放与视频")).clicked()
                {
                    self.run(Act::TimelinePlayback);
                }
                ui.checkbox(&mut self.studio.inspector, tr("属性"));
            });
            if self.land_move.active {
                self.land_move_bar(ui);
            } else {
                self.options_bar(ui);
            }
        });
        egui::Panel::bottom("studio-status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("studio-timeline").show(ui, |ui| self.timeline_bar(ui));
        if self.table_panel {
            egui::Panel::bottom("studio-data")
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| self.data_table(ui));
        }
        if self.studio.page == Page::Environment && self.doc.paper.is_none() {
            egui::Panel::right("studio-environment")
                .default_size(406.0)
                .size_range(320.0..=590.0)
                .resizable(true)
                .show(ui, |ui| {
                    self.assistant_footer(ui);
                    self.environment_panel(ui, true);
                });
        } else if self.studio.inspector || self.ai.open {
            egui::Panel::right("studio-properties")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    self.assistant_footer(ui);
                    if self.studio.inspector {
                        self.inspector(ui);
                    }
                });
        }
        if crate::toolbar_ui::has_side_panel(self.tool) {
            egui::Panel::right("studio-tool-detail").show(ui, |ui| self.tool_side_panel(ui));
        }
        self.refresh_derived();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let r = ui.available_rect_before_wrap().intersect(ui.clip_rect());
                let layout = if self.doc.paper.is_some() {
                    Layout::Flat
                } else {
                    self.layout
                };
                match layout {
                    Layout::Flat => self.map_view(ui, ViewKind::Flat, r),
                    Layout::Globe => self.map_view(ui, ViewKind::Globe, r),
                    Layout::Split => {
                        let x = r.left() + r.width() * 0.61;
                        self.map_view(
                            ui,
                            ViewKind::Flat,
                            egui::Rect::from_min_max(r.min, egui::pos2(x - 1.0, r.max.y)),
                        );
                        self.map_view(
                            ui,
                            ViewKind::Globe,
                            egui::Rect::from_min_max(egui::pos2(x + 1.0, r.min.y), r.max),
                        );
                    }
                }
            });
    }
}
