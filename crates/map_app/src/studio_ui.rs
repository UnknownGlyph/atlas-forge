//! The studio arranges the same editing session around project navigation
//! and task workspaces. The professional shell keeps its own navigation.
mod finder;

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

impl Page {
    const ALL: [Self; 7] = [
        Self::Map,
        Self::Sources,
        Self::Worldview,
        Self::Visualizations,
        Self::History,
        Self::Environment,
        Self::Stars,
    ];

    fn title(self) -> &'static str {
        tr(match self {
            Self::Map => "地图",
            Self::Sources => "世界资料",
            Self::Worldview => "世界观",
            Self::Visualizations => "图表",
            Self::History => "历史",
            Self::Environment => "环境",
            Self::Stars => "星图",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    Full,
    Compact,
    Minimal,
}

impl Shelf {
    fn pick(available: f32, full: f32, compact: f32) -> Self {
        if available >= full {
            Self::Full
        } else if available >= compact {
            Self::Compact
        } else {
            Self::Minimal
        }
    }

    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Full => &["绘制", "选择", "撤销", "重做", "视角", "属性", "导出…"],
            Self::Compact => &["绘制", "选择", "视角", "更多…"],
            Self::Minimal => &["绘制", "更多…"],
        }
    }

    fn width(self, ui: &egui::Ui) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let buttons: f32 = self
            .labels()
            .iter()
            .map(|title| {
                let width = ui
                    .painter()
                    .layout_no_wrap(tr(title).to_owned(), font.clone(), INK)
                    .size()
                    .x;
                (width + 14.0).max(34.0)
            })
            .sum();
        let separators = if self == Self::Full { 26.0 } else { 0.0 };
        buttons + (self.labels().len() - 1) as f32 * 7.0 + separators + 20.0
    }
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
    finder: finder::Finder,
}

impl Studio {
    pub(crate) fn worldview_visible(&self) -> bool {
        self.foreground && self.page == Page::Worldview
    }
    pub(crate) fn visualizations_visible(&self) -> bool {
        self.foreground && self.page == Page::Visualizations
    }
    pub(crate) fn environment_visible(&self) -> bool {
        self.foreground && self.page == Page::Environment
    }
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

const INK: Color32 = Color32::from_rgb(22, 34, 53);
const MUTED: Color32 = Color32::from_rgb(75, 92, 114);
const ACCENT: Color32 = Color32::from_rgb(22, 87, 182);
const PAPER: Color32 = Color32::from_rgb(247, 249, 252);
const HAIRLINE: Color32 = Color32::from_rgb(209, 219, 233);

fn studio_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.override_text_color = Some(INK);
    v.panel_fill = PAPER;
    v.window_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(235, 241, 249);
    v.window_stroke = egui::Stroke::new(1.0, HAIRLINE);
    v.window_corner_radius = egui::CornerRadius::same(15);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(219, 233, 253);
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    for (widget, color) in [
        (&mut v.widgets.noninteractive, PAPER),
        (&mut v.widgets.inactive, Color32::from_rgb(233, 239, 247)),
        (&mut v.widgets.hovered, Color32::from_rgb(219, 231, 248)),
        (&mut v.widgets.active, Color32::from_rgb(199, 221, 250)),
        (&mut v.widgets.open, Color32::from_rgb(224, 235, 252)),
    ] {
        widget.bg_fill = color;
        widget.weak_bg_fill = color;
        widget.bg_stroke = egui::Stroke::new(1.0, HAIRLINE);
        widget.fg_stroke.color = INK;
        widget.corner_radius = egui::CornerRadius::same(9);
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
            ctx.set_visuals(studio_visuals());
        } else if let Some(visuals) = self.studio.professional_visuals.take() {
            self.studio.finder.open = false;
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
        let frame = egui::Frame::new()
            .fill(PAPER)
            .stroke(egui::Stroke::new(1.0, HAIRLINE))
            .inner_margin(egui::Margin::symmetric(15, 8));
        egui::Panel::top("studio-header")
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    if width >= 570.0 {
                        ui.add(
                            egui::Label::new(RichText::new(tr("创作工作室")).size(18.0).strong())
                                .truncate(),
                        );
                        ui.separator();
                    }
                    let name = &self.doc.meta.name;
                    let marker = if self.dirty { " •" } else { "" };
                    ui.allocate_ui_with_layout(
                        egui::vec2(
                            (width - if width >= 570.0 { 365.0 } else { 185.0 }).max(56.0),
                            34.0,
                        ),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{name}{marker}")).size(15.0).strong(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(name);
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(tr("保存")).color(Color32::WHITE))
                                    .fill(ACCENT)
                                    .min_size(egui::vec2(66.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.run(Act::Save);
                        }
                        ui.spacing_mut().interact_size.y = 32.0;
                        ui.menu_button(tr("更多…"), |ui| {
                            if ui
                                .checkbox(&mut self.prefs.studio_solid_controls, tr("减少透明度"))
                                .changed()
                            {
                                ui.close();
                            }
                            if ui.button(tr("设置")).clicked() {
                                self.run(Act::Prefs);
                                ui.close();
                            }
                            if ui.button(tr("切回专业界面")).clicked() {
                                self.switch_workspace();
                                ui.close();
                            }
                            ui.menu_button(tr("所有命令"), |ui| self.studio_commands(ui));
                        });
                    });
                });
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    let width = ui.available_width();
                    ui.spacing_mut().interact_size.y = 34.0;
                    ui.menu_button(tr("画板"), |ui| self.studio_boards(ui));
                    if ui.button(tr("查找")).on_hover_text("Ctrl+F").clicked() {
                        self.studio.finder.show();
                    }
                    ui.separator();
                    if width < 830.0 {
                        ui.menu_button(self.studio.page.title(), |ui| {
                            for page in Page::ALL {
                                let label = egui::Button::selectable(
                                    self.studio.page == page,
                                    page.title(),
                                );
                                if ui.add(label.min_size(egui::vec2(126.0, 33.0))).clicked() {
                                    self.studio_select_page(page);
                                    ui.close();
                                }
                            }
                        });
                    } else {
                        for page in Page::ALL {
                            if ui
                                .add(
                                    egui::Button::selectable(
                                        self.studio.page == page,
                                        page.title(),
                                    )
                                    .min_size(egui::vec2(59.0, 34.0)),
                                )
                                .clicked()
                            {
                                self.studio_select_page(page);
                            }
                        }
                    }
                });
            });
    }

    fn studio_select_page(&mut self, page: Page) {
        self.studio.page = page;
        self.star_map.open = page == Page::Stars;
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

    fn studio_boards(&mut self, ui: &mut egui::Ui) {
        let current = WorldBook::current(&self.doc);
        let mut open = None;
        ui.set_min_width(244.0);
        if ui
            .add(egui::Button::new(tr("新建空白画板")).min_size(egui::vec2(182.0, 36.0)))
            .clicked()
        {
            self.create_studio_paper();
            ui.close();
            return;
        }
        if ui.button(tr("打开工程…")).clicked() {
            self.run(Act::Open);
            ui.close();
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                if let Some(book) = &self.doc.book {
                    for &id in book.boards.keys() {
                        if let Some(board) = book.map(&self.doc, id) {
                            if ui
                                .add(
                                    egui::Button::selectable(current == id, &board.meta.name)
                                        .min_size(egui::vec2(210.0, 32.0)),
                                )
                                .clicked()
                            {
                                open = Some(id);
                            }
                        }
                    }
                } else {
                    ui.label(RichText::new(&self.doc.meta.name).strong());
                }
            });
        if let Some(id) = open {
            self.star_map.open = false;
            self.studio.show_map();
            if id != current {
                if let Err(error) = self.enter_book_map(id, None, true) {
                    self.set_status(error, true);
                }
            }
            ui.close();
        }
    }

    fn studio_sources(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(PAPER)
            .inner_margin(egui::Margin::same(24));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.heading(tr("世界资料"));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for (act, title) in [
                    (Act::WorldBook, "打开资料库…"),
                    (Act::Worldview, "编辑世界观…"),
                    (Act::Events, "事件时间线…"),
                    (Act::Diplomacy, "外交关系…"),
                ] {
                    if ui
                        .add_enabled(
                            self.can(act),
                            egui::Button::new(tr(title)).min_size(egui::vec2(100.0, 36.0)),
                        )
                        .clicked()
                    {
                        self.run(act);
                    }
                }
            });
            ui.add_space(12.0);
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
                    if !articles.is_empty() {
                        ui.strong(tr("文档"));
                    }
                    for (id, title) in articles {
                        if ui.link(title).clicked() {
                            self.book_context(Node::Article(id));
                        }
                    }
                    if !models.is_empty() {
                        ui.add_space(12.0);
                        ui.strong(tr("三维模型"));
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
        if self.land_move.active || self.tool != Tool::Select || self.studio.page != Page::Map {
            egui::Panel::top("studio-context").show(ui, |ui| {
                if self.studio.page == Page::Environment
                    && ui.button(tr("行星环境与气候…")).clicked()
                {
                    self.run(Act::Climate);
                }
                if self.studio.page == Page::History {
                    ui.horizontal_wrapped(|ui| {
                        for (act, title) in [
                            (Act::Events, "事件时间线…"),
                            (Act::TimelinePlayback, "时间播放与视频"),
                            (Act::Diplomacy, "外交关系…"),
                            (Act::Checkpoints, "保存点与自动保存…"),
                        ] {
                            if ui
                                .add_enabled(self.can(act), egui::Button::new(tr(title)))
                                .clicked()
                            {
                                self.run(act);
                            }
                        }
                    });
                }
                if self.land_move.active {
                    self.land_move_bar(ui);
                } else {
                    self.options_bar(ui);
                }
            });
        }
        egui::Panel::bottom("studio-status").show(ui, |ui| self.status_bar(ui));
        if self.doc.year.is_some() || self.studio.page == Page::History {
            egui::Panel::bottom("studio-timeline").show(ui, |ui| self.timeline_bar(ui));
        }
        if self.table_panel {
            egui::Panel::bottom("studio-data")
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| self.data_table(ui));
        }
        let available_width = ui.available_width();
        if self.studio.page == Page::Environment && self.doc.paper.is_none() {
            let inspector = egui::Panel::right("studio-environment");
            let inspector = if available_width < 760.0 {
                inspector.exact_size((available_width * 0.48).max(170.0))
            } else {
                inspector
                    .default_size(406.0)
                    .size_range(320.0..=590.0)
                    .resizable(true)
            };
            inspector.show(ui, |ui| {
                self.assistant_footer(ui);
                self.environment_panel(ui, true);
            });
        } else if self.studio.inspector
            || self.ai.open
            || crate::toolbar_ui::has_side_panel(self.tool)
        {
            let inspector = egui::Panel::right("studio-properties");
            let inspector = if available_width < 760.0 {
                inspector.exact_size((available_width * 0.46).max(166.0))
            } else {
                inspector
                    .resizable(true)
                    .default_size(328.0)
                    .size_range(255.0..=430.0)
            };
            inspector.show(ui, |ui| {
                self.assistant_footer(ui);
                if crate::toolbar_ui::has_side_panel(self.tool) {
                    self.tool_side_panel(ui);
                }
                if self.studio.inspector {
                    self.inspector(ui);
                }
            });
        }
        self.refresh_derived();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let r = ui.available_rect_before_wrap().intersect(ui.clip_rect());
                if r.width() < 100.0 || r.height() < 80.0 {
                    return;
                }
                let available = r.width() - 20.0;
                let style = Shelf::pick(available, Shelf::Full.width(ui), Shelf::Compact.width(ui));
                let shelf = egui::Rect::from_center_size(
                    egui::pos2(r.center().x, r.top() + 44.0),
                    egui::vec2(style.width(ui).min(available), 58.0),
                );
                let glass = (!self.prefs.studio_solid_controls && self.doc.paper.is_none())
                    .then_some(shelf);
                let layout = if self.doc.paper.is_some() {
                    Layout::Flat
                } else {
                    self.layout
                };
                match layout {
                    Layout::Flat => self.map_view_with_glass(ui, ViewKind::Flat, r, glass),
                    Layout::Globe => self.map_view_with_glass(ui, ViewKind::Globe, r, glass),
                    Layout::Split => {
                        let x = r.left() + r.width() * 0.61;
                        self.map_view_with_glass(
                            ui,
                            ViewKind::Flat,
                            egui::Rect::from_min_max(r.min, egui::pos2(x, r.max.y)),
                            glass,
                        );
                        self.map_view_with_glass(
                            ui,
                            ViewKind::Globe,
                            egui::Rect::from_min_max(egui::pos2(x, r.min.y), r.max),
                            glass,
                        );
                    }
                }
                self.studio_shelf(ui.ctx(), shelf, style, glass.is_some());
            });
    }

    fn studio_shelf(&mut self, ctx: &egui::Context, shelf: egui::Rect, style: Shelf, glass: bool) {
        egui::Area::new(egui::Id::unique("studio-floating-tools"))
            .order(egui::Order::Foreground)
            .fixed_pos(shelf.min)
            .constrain(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(shelf.size(), egui::Sense::hover());
                let fill = if glass {
                    Color32::from_white_alpha(20)
                } else {
                    PAPER
                };
                ui.painter().rect(
                    rect,
                    19.0,
                    fill,
                    egui::Stroke::new(1.0, HAIRLINE),
                    egui::StrokeKind::Inside,
                );
                let mut tools = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(egui::vec2(10.0, 9.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                tools.spacing_mut().item_spacing.x = 7.0;
                tools.spacing_mut().interact_size = egui::vec2(34.0, 34.0);
                tools.spacing_mut().button_padding.x = 7.0;
                tools.menu_button(RichText::new(tr("绘制")).strong().color(ACCENT), |ui| {
                    self.studio_create(ui)
                });
                if style != Shelf::Minimal {
                    if tools
                        .add(egui::Button::selectable(
                            self.tool == Tool::Select && !self.land_move.active,
                            tr("选择"),
                        ))
                        .clicked()
                    {
                        self.run(Act::Tool(Tool::Select));
                    }
                }
                if style == Shelf::Full {
                    tools.separator();
                    self.studio_edit_buttons(&mut tools);
                    tools.separator();
                }
                if style != Shelf::Minimal {
                    tools.menu_button(tr("视角"), |ui| self.studio_view_menu(ui));
                }
                if style == Shelf::Full {
                    self.studio_panel_export(&mut tools);
                } else {
                    tools.menu_button(tr("更多…"), |ui| {
                        if style == Shelf::Minimal {
                            if ui.button(tr("选择")).clicked() {
                                self.run(Act::Tool(Tool::Select));
                                ui.close();
                            }
                            ui.menu_button(tr("视角"), |ui| self.studio_view_menu(ui));
                        }
                        self.studio_edit_buttons(ui);
                        self.studio_panel_export(ui);
                    });
                }
            });
    }

    fn studio_edit_buttons(&mut self, ui: &mut egui::Ui) {
        for (act, title) in [(Act::Undo, "撤销"), (Act::Redo, "重做")] {
            if ui
                .add_enabled(self.can(act), egui::Button::new(tr(title)))
                .clicked()
            {
                self.run(act);
                ui.close_kind(egui::UiKind::Menu);
            }
        }
    }

    fn studio_panel_export(&mut self, ui: &mut egui::Ui) {
        if ui
            .toggle_value(&mut self.studio.inspector, tr("属性"))
            .clicked()
        {
            ui.close_kind(egui::UiKind::Menu);
        }
        if ui.button(tr("导出…")).clicked() {
            self.run(Act::Export);
            ui.close_kind(egui::UiKind::Menu);
        }
    }

    fn studio_view_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(136.0);
        for (act, title) in [
            (Act::Layout(Layout::Flat), "二维地图"),
            (Act::Layout(Layout::Globe), "三维球体"),
            (Act::Layout(Layout::Split), "二维 + 三维"),
            (Act::FitAll, "缩放到全图"),
            (Act::ZoomIn, "放大"),
            (Act::ZoomOut, "缩小"),
            (Act::Timeline, "时间轴"),
        ] {
            let button = egui::Button::selectable(self.checked(act) == Some(true), tr(title));
            if ui
                .add_enabled(self.can(act), button.min_size(egui::vec2(124.0, 30.0)))
                .clicked()
            {
                self.run(act);
                ui.close();
            }
        }
    }

    fn studio_create(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(260.0);
        for (heading, items) in [
            (
                "地形",
                &[
                    (Act::Tool(Tool::DrawLand), "绘制陆地"),
                    (Act::Tool(Tool::DrawLake), "绘制水域"),
                    (Act::Tool(Tool::River), "河流 / 线路"),
                ][..],
            ),
            (
                "地点与标志",
                &[
                    (Act::Tool(Tool::City), "添加城市"),
                    (Act::Tool(Tool::Label), "地图文字"),
                    (Act::Tool(Tool::Symbol), "地图符号"),
                    (Act::PixelBrush, "彩色像素画笔…"),
                ][..],
            ),
        ] {
            ui.label(RichText::new(tr(heading)).strong().color(MUTED));
            ui.horizontal_wrapped(|ui| {
                for &(action, label) in items {
                    if ui
                        .add_enabled(
                            self.can(action),
                            egui::Button::new(tr(label)).min_size(egui::vec2(78.0, 34.0)),
                        )
                        .clicked()
                    {
                        if self.studio.page == Page::Environment {
                            self.studio.show_map();
                        }
                        self.run(action);
                        ui.close();
                    }
                }
            });
            ui.add_space(5.0);
        }
        ui.separator();
        for (act, label) in [
            (Act::LandMove, "整体移动陆块"),
            (Act::LandScale, "整体缩放陆块"),
            (Act::ImportUnderlay, "导入参考图片…"),
            (Act::GeographicGuides, "地理参考线…"),
        ] {
            if ui
                .add_enabled(self.can(act), egui::Button::new(tr(label)))
                .clicked()
            {
                self.run(act);
                ui.close();
            }
        }
        ui.menu_button(tr("全部绘图工具"), |ui| {
            for tool in Tool::ALL {
                if ui
                    .selectable_label(self.tool == tool, tool.name())
                    .on_hover_text(tool.help())
                    .clicked()
                {
                    self.run(Act::Tool(tool));
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_keeps_core_actions_with_decreasing_space() {
        assert_eq!(Shelf::pick(500.0, 400.0, 260.0), Shelf::Full);
        assert_eq!(Shelf::pick(320.0, 400.0, 260.0), Shelf::Compact);
        assert_eq!(Shelf::pick(190.0, 400.0, 260.0), Shelf::Minimal);
        assert!(Shelf::Minimal.labels().contains(&"绘制"));
    }
}
