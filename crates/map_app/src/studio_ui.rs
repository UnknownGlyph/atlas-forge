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
}

impl Navigation {
    fn initial(app: &App) -> Self {
        Self {
            flat: app.cam2d,
            globe: app.globe,
            spin: Default::default(),
            rect: app.last_flat_rect,
            fit: true,
            touched: false,
            layout: app.layout,
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
    }
}

#[derive(Default)]
pub(crate) struct Studio {
    pub open: bool,
    pub foreground: bool,
    raise: bool,
    page: Page,
    navigation: Option<Navigation>,
    inspector: bool,
}

impl Studio {
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

fn viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("atlas-studio-window")
}

impl App {
    pub(crate) fn switch_workspace(&mut self) {
        if self.studio.foreground {
            self.studio.foreground = false;
            self.studio.raise = false;
            self.professional_raise = true;
        } else {
            if !self.studio.open {
                self.studio.inspector = true;
            }
            self.studio.open = true;
            self.studio.foreground = true;
            self.studio.raise = true;
        }
    }

    pub(crate) fn reset_studio_navigation(&mut self) {
        self.studio.navigation = None;
    }

    pub(crate) fn studio_window(&mut self, ctx: &egui::Context) {
        if !self.studio.open {
            return;
        }
        let title = format!(
            "Atlas Forge · {} · {}{}",
            tr("创作工作室"),
            self.doc.meta.name,
            if self.dirty { " *" } else { "" }
        );
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1520.0, 915.0])
            .with_min_inner_size([870.0, 580.0]);
        let open = ctx.show_viewport_immediate(viewport(), builder, |ui, class| {
            let studio_ctx = ui.ctx().clone();
            if ui.input(|i| i.viewport().close_requested()) {
                return false;
            }
            if ui.input(|i| i.viewport().focused == Some(true))
                || class == egui::ViewportClass::EmbeddedWindow
            {
                self.studio.foreground = true;
            }
            if std::mem::take(&mut self.studio.raise)
                && class != egui::ViewportClass::EmbeddedWindow
            {
                studio_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let mut style = (**ui.style()).clone();
            style.spacing.item_spacing = egui::vec2(9.0, 6.0);
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = Color32::from_rgb(24, 34, 44);
            style.visuals.extreme_bg_color = Color32::from_rgb(13, 20, 28);
            style.visuals.selection.bg_fill = Color32::from_rgb(35, 79, 95);
            style.visuals.selection.stroke.color = Color32::from_rgb(208, 249, 253);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(37, 97, 111);
            ui.set_style(style);

            if !self.studio.foreground {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.heading(tr("创作工作室"));
                    ui.label(&self.doc.meta.name);
                    if ui.button(tr("在工作室继续")).clicked() {
                        self.switch_workspace();
                    }
                });
                return true;
            }
            let mut nav = self
                .studio
                .navigation
                .take()
                .unwrap_or_else(|| Navigation::initial(self));
            let epoch = self.workspace_camera_epoch;
            nav.swap(self);
            let classic_star = self.star_map.open;
            self.star_map.open = self.studio.page == Page::Stars;
            self.read_workspace_input(&studio_ctx);
            if (self.studio.page == Page::Stars) != self.star_map.open {
                self.studio.page = if self.star_map.open {
                    Page::Stars
                } else {
                    Page::Map
                };
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
                Page::Stars => {
                    self.star_map.open = true;
                    self.star_chart(ui);
                    if !self.star_map.open {
                        self.studio.page = Page::Map;
                    }
                }
                Page::Sources => self.studio_sources(ui),
                Page::Worldview => self.studio_worldview(ui),
                Page::Visualizations => self.studio_visualizations(ui),
                Page::Map | Page::History | Page::Environment => self.studio_map(ui),
            }
            self.show_workspace_editors(&studio_ctx);
            self.star_map.open = classic_star;
            nav.swap(self);
            self.studio.navigation = Some(nav);
            if epoch != self.workspace_camera_epoch {
                self.apply_home_view();
            }
            true
        });
        if !open {
            self.studio.open = false;
            self.studio.foreground = false;
            self.professional_raise = true;
        }
    }

    fn studio_header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("studio-header").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(RichText::new("Atlas Forge").color(Color32::from_rgb(181, 226, 233)));
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
                if ui.button(tr("AI 助手…")).clicked() {
                    self.run(Act::Assistant);
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
                    (Act::WorldBook, "世界资料"),
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
                .show(ui, |ui| self.environment_panel(ui, true));
        } else if self.studio.inspector {
            egui::Panel::right("studio-properties")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| self.inspector(ui));
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
