//! The studio arranges the same editing session around project navigation
//! and task workspaces. The professional shell keeps its own navigation.
mod finder;

use crate::{
    actions::{self, Act, Menu},
    app::{App, Layout},
    i18n::tr,
    view::{DrawMode, GlobeInertia, Tool, ViewKind},
};
use eframe::egui::{self, Color32, RichText};
use map_core::doc::EntityKind;
use map_core::shape::LineStyle;
use map_core::world_book::{Node, WorldBook};
use map_core::EntityId;
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

    fn hint(self) -> &'static str {
        tr(match self {
            Self::Map => "绘制陆地、国家、河流与城市",
            Self::Sources => "文档、事件、附件与独立画板",
            Self::Worldview => "自定义实体类型、属性与关系",
            Self::Visualizations => "用地图与设定数据制作图表",
            Self::History => "按年份查看和编辑，管理事件与外交",
            Self::Environment => "地形、水文、气候与生态图层",
            Self::Stars => "恒星系、地月系与银河系",
        })
    }
}

/// Width of an icon button on the floating shelf.
const TOOL_BUTTON: f32 = 34.0;

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
            Self::Full => &["绘制", "撤销", "重做", "视角", "属性", "导出…"],
            Self::Compact => &["绘制", "视角", "更多…"],
            Self::Minimal => &["绘制", "更多…"],
        }
    }

    /// The tools kept one click away, as icons, beside the drawing menu.
    fn tools(self) -> &'static [Tool] {
        match self {
            Self::Full => &[
                Tool::Select,
                Tool::DrawLand,
                Tool::Assign,
                Tool::River,
                Tool::City,
                Tool::Label,
            ],
            Self::Compact => &[Tool::Select, Tool::DrawLand, Tool::Assign],
            Self::Minimal => &[],
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
                    .layout_no_wrap(tr(title).to_owned(), font.clone(), LIGHT.ink)
                    .size()
                    .x;
                (width + 14.0).max(34.0)
            })
            .sum();
        let icons = self.tools().len() as f32 * TOOL_BUTTON;
        let items = self.labels().len() + self.tools().len();
        let separators = if self == Self::Full { 26.0 } else { 0.0 };
        buttons + icons + (items - 1) as f32 * 7.0 + separators + 20.0
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
    /// The start card of an empty map was put away for this session.
    welcome_dismissed: bool,
    /// The appearance whose visuals are installed (None: not in the studio).
    applied_dark: Option<bool>,
    /// Show the professional editor's full tool options instead of the short ones.
    advanced: bool,
    /// The tool settings of the side not in front (see [`Presets`]).
    presets: Presets,
    /// The program's icon, decoded the first time the studio shows it.
    icon: Option<egui::TextureHandle>,
}

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/atlas-forge-64.png");

/// Tool settings the two interfaces keep apart. The studio starts from
/// presets meant for sketching a world; the professional editor keeps its
/// own values, swapped back in when it returns to the front.
#[derive(Clone, Copy)]
struct Presets {
    line_style: LineStyle,
    roughness: f32,
    keep_area: bool,
    trace: bool,
    smart_snap: bool,
    draw_mode: DrawMode,
    assign_fill: bool,
    brush_paint: bool,
}

impl Default for Presets {
    fn default() -> Self {
        Self {
            line_style: LineStyle::Natural,
            roughness: 0.45,
            keep_area: true,
            trace: true,
            smart_snap: true,
            draw_mode: DrawMode::AddLand,
            assign_fill: false,
            brush_paint: false,
        }
    }
}

impl Presets {
    fn capture(app: &App) -> Self {
        Self {
            line_style: app.line_style,
            roughness: app.roughness,
            keep_area: app.keep_area,
            trace: app.trace,
            smart_snap: app.smart_snap,
            draw_mode: app.draw_mode,
            assign_fill: app.assign_fill,
            brush_paint: app.brush_paint,
        }
    }

    fn restore(self, app: &mut App) {
        app.line_style = self.line_style;
        app.roughness = self.roughness;
        app.keep_area = self.keep_area;
        app.trace = self.trace;
        app.smart_snap = self.smart_snap;
        app.draw_mode = self.draw_mode;
        app.assign_fill = self.assign_fill;
        app.brush_paint = self.brush_paint;
    }
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

/// The studio's colours in its light and dark appearance.
struct Palette {
    ink: Color32,
    muted: Color32,
    /// Accent for text, icons and outlines.
    accent: Color32,
    /// Accent for filled buttons, with `on_accent` text.
    accent_fill: Color32,
    on_accent: Color32,
    paper: Color32,
    card: Color32,
    hairline: Color32,
    faint: Color32,
    inactive: Color32,
    hovered: Color32,
    active: Color32,
    open: Color32,
    selected: Color32,
    field: Color32,
    shadow: Color32,
    /// The backdrop behind the map views, top and bottom.
    canvas: [Color32; 2],
    dots: Color32,
}

const LIGHT: Palette = Palette {
    ink: Color32::from_rgb(22, 34, 53),
    muted: Color32::from_rgb(75, 92, 114),
    accent: Color32::from_rgb(22, 87, 182),
    accent_fill: Color32::from_rgb(22, 87, 182),
    on_accent: Color32::WHITE,
    paper: Color32::from_rgb(247, 249, 252),
    card: Color32::from_rgb(255, 255, 255),
    hairline: Color32::from_rgb(209, 219, 233),
    faint: Color32::from_rgb(235, 241, 249),
    inactive: Color32::from_rgb(233, 239, 247),
    hovered: Color32::from_rgb(219, 231, 248),
    active: Color32::from_rgb(199, 221, 250),
    open: Color32::from_rgb(224, 235, 252),
    selected: Color32::from_rgb(219, 233, 253),
    field: Color32::from_rgb(255, 255, 255),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 24),
    canvas: [Color32::from_rgb(234, 239, 246), Color32::from_rgb(220, 227, 237)],
    dots: Color32::from_rgb(201, 211, 225),
};

const DARK: Palette = Palette {
    ink: Color32::from_rgb(226, 232, 241),
    muted: Color32::from_rgb(148, 161, 181),
    accent: Color32::from_rgb(110, 166, 255),
    accent_fill: Color32::from_rgb(46, 108, 214),
    on_accent: Color32::WHITE,
    paper: Color32::from_rgb(23, 27, 35),
    card: Color32::from_rgb(31, 36, 46),
    hairline: Color32::from_rgb(54, 62, 77),
    faint: Color32::from_rgb(36, 42, 54),
    inactive: Color32::from_rgb(40, 47, 60),
    hovered: Color32::from_rgb(52, 62, 80),
    active: Color32::from_rgb(64, 80, 108),
    open: Color32::from_rgb(47, 57, 74),
    selected: Color32::from_rgb(38, 62, 104),
    field: Color32::from_rgb(18, 21, 28),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 96),
    canvas: [Color32::from_rgb(27, 31, 40), Color32::from_rgb(14, 17, 23)],
    dots: Color32::from_rgb(45, 52, 66),
};

impl Palette {
    fn of(dark: bool) -> &'static Palette {
        if dark {
            &DARK
        } else {
            &LIGHT
        }
    }

    fn ui(ui: &egui::Ui) -> &'static Palette {
        Self::of(ui.visuals().dark_mode)
    }
}

/// The backdrop colour the glass material starts from, as 0–1 sRGB.
pub(crate) fn glass_backdrop(dark: bool) -> [f32; 3] {
    let [top, bottom] = Palette::of(dark).canvas;
    let mix = |a: u8, b: u8| (a as f32 + b as f32) * 0.5 / 255.0;
    [mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b())]
}

fn studio_visuals(dark: bool) -> egui::Visuals {
    let p = Palette::of(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = Some(p.ink);
    v.panel_fill = p.paper;
    v.window_fill = p.card;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.faint;
    v.code_bg_color = p.faint;
    v.window_stroke = egui::Stroke::new(1.0, p.hairline);
    v.window_corner_radius = egui::CornerRadius::same(15);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selected;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    for (widget, color) in [
        (&mut v.widgets.noninteractive, p.paper),
        (&mut v.widgets.inactive, p.inactive),
        (&mut v.widgets.hovered, p.hovered),
        (&mut v.widgets.active, p.active),
        (&mut v.widgets.open, p.open),
    ] {
        widget.bg_fill = color;
        widget.weak_bg_fill = color;
        widget.bg_stroke = egui::Stroke::new(1.0, p.hairline);
        widget.fg_stroke.color = p.ink;
        widget.corner_radius = egui::CornerRadius::same(9);
    }
    v
}

/// A soft shadow under a card or board.
fn soft_shadow(painter: &egui::Painter, rect: egui::Rect, radius: f32, color: Color32) {
    for (grow, drop, share) in [(12.0, 7.0, 0.18), (6.0, 4.0, 0.32), (2.0, 1.5, 0.5)] {
        painter.rect_filled(
            rect.expand(grow).translate(egui::vec2(0.0, drop)),
            radius + grow,
            color.gamma_multiply(share),
        );
    }
}

/// A floating card: shadow, fill and hairline.
fn card(painter: &egui::Painter, rect: egui::Rect, radius: f32, p: &Palette) {
    soft_shadow(painter, rect, radius, p.shadow);
    painter.rect(
        rect,
        radius,
        p.card,
        egui::Stroke::new(1.0, p.hairline),
        egui::StrokeKind::Inside,
    );
}

/// A sun (light appearance) or a moon (dark appearance), drawn.
fn paint_theme_icon(painter: &egui::Painter, center: egui::Pos2, dark: bool, ink: Color32, back: Color32) {
    if dark {
        painter.circle_filled(center, 7.0, ink);
        painter.circle_filled(center + egui::vec2(3.6, -2.8), 6.0, back);
    } else {
        painter.circle_filled(center, 4.2, ink);
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let dir = egui::vec2(a.cos(), a.sin());
            painter.line_segment(
                [center + dir * 6.5, center + dir * 9.0],
                egui::Stroke::new(1.6, ink),
            );
        }
    }
}

/// A row of the map-style menu: a small swatch of the look and its name.
fn style_row(ui: &mut egui::Ui, name: &str, style: &map_core::doc::Style, on: bool) -> egui::Response {
    let p = Palette::ui(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(220.0, 34.0), egui::Sense::click());
    let fill = if on {
        p.selected
    } else if response.hovered() {
        p.hovered
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, fill);
    let rgba = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let swatch = egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 6.0), egui::vec2(38.0, 22.0));
    painter.rect_filled(swatch, 5.0, rgba(style.ocean));
    let land = egui::Rect::from_min_max(swatch.min + egui::vec2(7.0, 5.0), swatch.max - egui::vec2(9.0, 4.0));
    painter.rect_filled(land, 4.0, rgba(style.land));
    painter.line_segment(
        [egui::pos2(land.center().x, land.top()), egui::pos2(land.center().x, land.bottom())],
        egui::Stroke::new(1.2, rgba(style.border)),
    );
    painter.rect_stroke(swatch, 5.0, egui::Stroke::new(1.0, p.hairline), egui::StrokeKind::Inside);
    painter.text(
        egui::pos2(swatch.right() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(14.0),
        if on { p.accent } else { p.ink },
    );
    response
}

/// The tools with a short set of options in the studio.
fn simple_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::DrawLand | Tool::DrawLake | Tool::Assign | Tool::River | Tool::City
    )
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
        // each interface keeps its own tool settings
        let other = self.studio.presets;
        self.studio.presets = Presets::capture(self);
        other.restore(self);
        self.studio.foreground = !self.studio.foreground;
        if self.studio.foreground {
            self.studio.professional_visuals = Some(ctx.global_style().visuals.clone());
            let dark = self.studio_dark(ctx);
            ctx.set_visuals(studio_visuals(dark));
            self.studio.applied_dark = Some(dark);
        } else if let Some(visuals) = self.studio.professional_visuals.take() {
            self.studio.finder.open = false;
            self.studio.applied_dark = None;
            ctx.set_visuals(visuals);
        }
    }

    /// The program's icon as a texture.
    fn studio_icon(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        if self.studio.icon.is_none() {
            let image = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            self.studio.icon = Some(ctx.load_texture("atlas-forge-icon", pixels, egui::TextureOptions::LINEAR));
        }
        self.studio.icon.as_ref().map(|texture| texture.id())
    }

    /// Whether the studio shows its dark appearance now.
    fn studio_dark(&self, ctx: &egui::Context) -> bool {
        match self.prefs.studio_theme.as_str() {
            "dark" => true,
            "system" => ctx.system_theme() == Some(egui::Theme::Dark),
            _ => false,
        }
    }

    /// Follow a change of the appearance setting (or of the system's).
    fn studio_sync_theme(&mut self, ctx: &egui::Context) {
        let dark = self.studio_dark(ctx);
        if self.studio.applied_dark != Some(dark) {
            ctx.set_visuals(studio_visuals(dark));
            self.studio.applied_dark = Some(dark);
        }
    }

    /// Paint the studio's backdrop behind a map view; false outside the
    /// studio, where the view keeps its own.
    pub(crate) fn paint_studio_canvas(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        board: Option<egui::Rect>,
    ) -> bool {
        if !self.studio.foreground {
            return false;
        }
        let p = Palette::of(self.studio.applied_dark.unwrap_or(false));
        let [top, bottom] = p.canvas;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), top);
        mesh.colored_vertex(rect.right_top(), top);
        mesh.colored_vertex(rect.left_bottom(), bottom);
        mesh.colored_vertex(rect.right_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(2, 1, 3);
        painter.add(egui::Shape::mesh(mesh));
        // a quiet dot grid, like a drawing table
        let step = 26.0;
        let columns = (rect.width() / step) as usize;
        let rows = (rect.height() / step) as usize;
        if columns * rows <= 6000 {
            let origin = rect.left_top() + egui::vec2(step * 0.5, step * 0.5);
            painter.extend((0..=rows).flat_map(|row| {
                (0..=columns).filter_map(move |column| {
                    let at = origin + egui::vec2(column as f32 * step, row as f32 * step);
                    rect.contains(at).then(|| egui::Shape::rect_filled(
                        egui::Rect::from_center_size(at, egui::vec2(1.6, 1.6)), 0.0, p.dots,
                    ))
                })
            }));
        }
        if let Some(board) = board {
            soft_shadow(painter, board, 2.0, p.shadow);
        }
        true
    }

    /// Text drawn over a map view in the studio.
    pub(crate) fn studio_canvas_ink(&self) -> Option<Color32> {
        self.studio
            .foreground
            .then(|| Palette::of(self.studio.applied_dark.unwrap_or(false)).muted)
    }

    pub(crate) fn reset_studio_navigation(&mut self) {
        self.studio.navigation = None;
    }

    pub(crate) fn studio_workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.studio_sync_theme(&ctx);
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
        let p = Palette::ui(ui);
        let frame = egui::Frame::new()
            .fill(p.paper)
            .stroke(egui::Stroke::new(1.0, p.hairline))
            .inner_margin(egui::Margin::symmetric(15, 8));
        egui::Panel::top("studio-header")
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    if width >= 570.0 {
                        if let Some(icon) = self.studio_icon(ui.ctx()) {
                            ui.image((icon, egui::vec2(28.0, 28.0)));
                        }
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
                            (width - if width >= 570.0 { 495.0 } else { 285.0 }).max(56.0),
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
                        // A filled button while there is something to save.
                        let save = if self.dirty {
                            egui::Button::new(RichText::new(tr("保存")).color(p.on_accent))
                                .fill(p.accent_fill)
                        } else {
                            egui::Button::new(RichText::new(tr("已保存")).color(p.muted))
                        };
                        if ui
                            .add(save.min_size(egui::vec2(66.0, 32.0)))
                            .on_hover_text("Ctrl+S")
                            .clicked()
                        {
                            self.run(Act::Save);
                        }
                        ui.spacing_mut().interact_size.y = 32.0;
                        ui.menu_button(tr("更多…"), |ui| {
                            ui.set_min_width(210.0);
                            let all = actions::table();
                            for act in [Act::History, Act::Checkpoints, Act::Export] {
                                self.studio_action(ui, &all, act);
                            }
                            ui.separator();
                            ui.label(RichText::new(tr("外观")).color(p.muted));
                            ui.horizontal(|ui| {
                                for (code, title) in
                                    [("light", "浅色"), ("dark", "深色"), ("system", "跟随系统")]
                                {
                                    let on = self.prefs.studio_theme == code;
                                    if ui.selectable_label(on, tr(title)).clicked() {
                                        self.prefs.studio_theme = code.into();
                                    }
                                }
                            });
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
                        ui.menu_button(tr("帮助"), |ui| self.studio_help_menu(ui));
                        // one click between the light and the dark appearance
                        let dark = ui.visuals().dark_mode;
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(34.0, 32.0), egui::Sense::click());
                        let fill = if response.hovered() { p.hovered } else { p.inactive };
                        ui.painter().rect_filled(rect, 9.0, fill);
                        paint_theme_icon(ui.painter(), rect.center(), dark, p.ink, fill);
                        if response
                            .on_hover_text(if dark { tr("切换到浅色外观") } else { tr("切换到深色外观") })
                            .clicked()
                        {
                            self.prefs.studio_theme = if dark { "light" } else { "dark" }.into();
                        }
                    });
                });
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    let width = ui.available_width();
                    ui.spacing_mut().interact_size.y = 34.0;
                    ui.menu_button(tr("画板"), |ui| self.studio_boards(ui));
                    if ui
                        .button(tr("查找"))
                        .on_hover_text(tr("按名称查找画板、文档、国家、地区或城市（Ctrl+F）"))
                        .clicked()
                    {
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
                                if ui
                                    .add(label.min_size(egui::vec2(126.0, 33.0)))
                                    .on_hover_text(page.hint())
                                    .clicked()
                                {
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
                                .on_hover_text(page.hint())
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
            .fill(Palette::ui(ui).paper)
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
                } else if simple_tool(self.tool) && !self.studio.advanced {
                    self.studio_simple_options(ui);
                } else if self.tool != Tool::Select {
                    self.studio_tool_context(ui);
                    self.options_bar(ui);
                } else if self.studio.advanced {
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
                if self.studio.page == Page::Map {
                    self.studio_welcome(ui.ctx(), r);
                    self.studio_selection_card(ui.ctx(), r);
                }
            });
    }

    /// Above the tool's own options: the way back to selecting, and for the
    /// land and territory tools the country they paint with.
    fn studio_tool_context(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .button(tr("返回选择"))
                .on_hover_text(tr("也可以按 V，或点浮动工具条上的箭头。"))
                .clicked()
            {
                self.run(Act::Tool(Tool::Select));
            }
            if matches!(self.tool, Tool::DrawLand | Tool::Assign) {
                ui.separator();
                self.studio_brush_menu(ui);
            }
            if simple_tool(self.tool) {
                ui.separator();
                ui.toggle_value(&mut self.studio.advanced, tr("全部选项"))
                    .on_hover_text(tr("关闭后只显示常用选项"));
            }
        });
    }

    /// The common options of the drawing tools, as a short row of choices.
    /// Everything else stays in “全部选项”, the professional editor's bar.
    fn studio_simple_options(&mut self, ui: &mut egui::Ui) {
        let p = Palette::ui(ui);
        let tool = self.tool;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
            crate::toolbar_ui::tool_icon(ui.painter(), icon, tool, p.accent);
            ui.label(RichText::new(tool.name()).strong());
            if ui
                .button(tr("返回选择"))
                .on_hover_text(tr("也可以按 V，或点浮动工具条上的箭头。"))
                .clicked()
            {
                self.run(Act::Tool(Tool::Select));
            }
            ui.separator();
            if matches!(tool, Tool::DrawLand | Tool::DrawLake) {
                ui.selectable_value(&mut self.brush_paint, false, tr("轮廓"))
                    .on_hover_text(tr("单击加点或按住拖动手绘出轮廓"));
                ui.selectable_value(&mut self.brush_paint, true, tr("涂抹"))
                    .on_hover_text(tr("按住拖动涂抹，松开即应用；[ ] 调整大小。"));
                if self.brush_paint {
                    ui.spacing_mut().slider_width = 90.0;
                    ui.add(egui::Slider::new(&mut self.brush_px, 3.0..=150.0).suffix(" px"))
                        .on_hover_text(tr("画笔半径；[ 与 ] 调整"));
                }
                ui.separator();
            }
            if tool == Tool::River {
                ui.selectable_value(&mut self.route_mode, false, tr("河流"));
                ui.selectable_value(&mut self.route_mode, true, tr("线路"))
                    .on_hover_text(tr("道路、商路、航线、行军路线等；所有年份相同。画好后可在所选线路面板中修改。"));
                ui.separator();
            }
            let lines = (matches!(tool, Tool::DrawLand | Tool::DrawLake) && !self.brush_paint)
                || tool == Tool::River;
            if lines {
                ui.label(RichText::new(tr("笔触")).color(p.muted));
                for (style, hint) in [
                    (LineStyle::Natural, "海岸般的自然起伏"),
                    (LineStyle::Smooth, "圆润的转角"),
                    (LineStyle::Plain, "按点的位置，不做处理"),
                ] {
                    ui.selectable_value(&mut self.line_style, style, tr(style.name()))
                        .on_hover_text(tr(hint));
                }
                if self.line_style == LineStyle::Natural {
                    ui.spacing_mut().slider_width = 70.0;
                    ui.add(egui::Slider::new(&mut self.roughness, 0.05..=1.0).show_value(false))
                        .on_hover_text(tr("粗糙度"));
                }
                ui.separator();
            }
            if tool == Tool::DrawLand {
                ui.label(RichText::new(tr("新陆地")).color(p.muted));
                if let DrawMode::Province(parent) = self.draw_mode {
                    if let Some(unit) = self.doc.entities.get(&parent) {
                        ui.strong(crate::trf!("上级 · {}", unit.name));
                    }
                }
                for (mode, title, hint) in [
                    (DrawMode::AddLand, "无主陆地", "不动已有陆地，只在水域里添加"),
                    (DrawMode::Expand, "归画笔国家", "只占海和无主地，别国的土地不变"),
                    (DrawMode::NewCountry, "新建国家", "新建一个国家，区域全归它"),
                ] {
                    ui.selectable_value(&mut self.draw_mode, mode, tr(title)).on_hover_text(tr(hint));
                }
                ui.separator();
            }
            if matches!(tool, Tool::DrawLand | Tool::Assign) {
                self.studio_brush_menu(ui);
                ui.separator();
            }
            if tool == Tool::Assign {
                ui.checkbox(&mut self.assign_fill, tr("单击填满整片"))
                    .on_hover_text(tr("单击即把与之相连、归属相同（或同为无主）的一整片陆地分配给画笔国家；按住 Shift 单击也是填充。"));
                ui.separator();
            }
            if matches!(tool, Tool::DrawLand | Tool::DrawLake) && !self.brush_paint {
                ui.checkbox(&mut self.trace, tr("沿边界描线"))
                    .on_hover_text(tr("两个草稿点都吸附在边界上时，沿已有边界连接；按住 Ctrl 临时关闭"));
            }
            if !self.draft.is_empty() {
                if ui
                    .add(
                        egui::Button::new(RichText::new(tr("完成")).color(p.on_accent))
                            .fill(p.accent_fill),
                    )
                    .on_hover_text("Enter")
                    .clicked()
                {
                    self.finish_draft();
                }
                if ui.button(tr("取消")).on_hover_text("Esc").clicked() {
                    self.clear_draft();
                }
            }
            ui.toggle_value(&mut self.studio.advanced, tr("全部选项"))
                .on_hover_text(tr("显示专业界面中这个工具的全部选项"));
            let help = tool.help();
            let short = help.split(['；', '。']).next().unwrap_or(help);
            ui.add(egui::Label::new(RichText::new(short).color(p.muted)).truncate())
                .on_hover_text(help);
        });
    }

    /// The brush country, picked from the top-level units of the active layer.
    fn studio_brush_menu(&mut self, ui: &mut egui::Ui) {
        let current = self
            .brush
            .and_then(|id| self.doc.entities.get(&id))
            .map(|entity| (entity.name.clone(), entity.color));
        if let Some((_, color)) = &current {
            crate::panels::swatch(ui, *color);
        }
        let title = match &current {
            Some((name, _)) => crate::trf!("画笔国家：{}", name),
            None => tr("选择画笔国家").to_owned(),
        };
        ui.menu_button(title, |ui| {
            ui.set_min_width(230.0);
            let layer = self.doc.active_layer;
            let year = self.doc.year;
            let mut rows: Vec<(EntityId, String, [u8; 4])> = self
                .doc
                .entities
                .iter()
                .filter(|(_, e)| {
                    e.layer == layer
                        && e.kind == EntityKind::Country
                        && e.parent.is_none()
                        && e.exists_in(year)
                })
                .map(|(&id, e)| (id, e.name.clone(), e.color))
                .collect();
            rows.sort_by(|a, b| a.1.cmp(&b.1));
            if ui
                .button(tr("新建国家并设为画笔"))
                .on_hover_text(tr("新建一个没有领土的国家，之后绘制或涂抹的土地归它。"))
                .clicked()
            {
                let n = self.doc.countries().len() + 1;
                let name = if layer == map_core::doc::POLITICAL {
                    format!("新国家{n}")
                } else {
                    format!("新单位{n}")
                };
                if let Some(id) = self.edit("新建国家", |d| Ok(d.add_entity(EntityKind::Country, &name))) {
                    self.brush = Some(id);
                    self.selected = Some(id);
                }
                ui.close();
                return;
            }
            if rows.is_empty() {
                ui.weak(tr("还没有国家。"));
                return;
            }
            ui.separator();
            let mut chosen = None;
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (id, name, color) in &rows {
                        ui.horizontal(|ui| {
                            crate::panels::swatch(ui, *color);
                            if ui
                                .selectable_label(self.brush == Some(*id), name.as_str())
                                .clicked()
                            {
                                chosen = Some(*id);
                            }
                        });
                    }
                });
            if let Some(id) = chosen {
                self.brush = Some(id);
                ui.close();
            }
        });
    }

    /// A start card on a map with no land yet.
    fn studio_welcome(&mut self, ctx: &egui::Context, map: egui::Rect) {
        if self.studio.welcome_dismissed
            || self.doc.paper.is_some()
            || self.doc.topo.faces.len() > 1
            || self.tool != Tool::Select
            || !self.draft.is_empty()
            || map.width() < 400.0
            || map.height() < 360.0
        {
            return;
        }
        let size = egui::vec2(372.0, 310.0);
        let at = map.center() - size * 0.5;
        let icon = self.studio_icon(ctx);
        egui::Area::new(egui::Id::unique("studio-welcome"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain(false)
            .show(ctx, |ui| {
                let p = Palette::ui(ui);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                card(ui.painter(), rect, 18.0, p);
                let mut card = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink(18.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                card.spacing_mut().item_spacing.y = 7.0;
                card.horizontal(|ui| {
                    if let Some(icon) = icon {
                        ui.image((icon, egui::vec2(34.0, 34.0)));
                    }
                    ui.label(RichText::new(tr("开始创作")).size(20.0).strong());
                });
                card.label(
                    RichText::new(tr("这张地图还是空白的。选一种方式开始，每一步都可以撤销。"))
                        .color(p.muted),
                );
                card.add_space(4.0);
                let mut chosen = None;
                for (act, title) in [
                    (Act::Tool(Tool::DrawLand), "绘制第一块大陆"),
                    (Act::NewWorldWizard, "用新世界向导生成…"),
                    (Act::NewTestWorld, "打开伊瑟尔三国示例"),
                    (Act::Tutorial, "跟着创作教程学习…"),
                ] {
                    let first = act == Act::Tool(Tool::DrawLand);
                    let button = if first {
                        egui::Button::new(RichText::new(tr(title)).color(p.on_accent).strong())
                            .fill(p.accent_fill)
                    } else {
                        egui::Button::new(tr(title))
                    };
                    if card
                        .add_enabled(self.can(act), button.min_size(egui::vec2(336.0, 36.0)))
                        .clicked()
                    {
                        chosen = Some(act);
                    }
                }
                if card.link(tr("暂不需要")).clicked() {
                    self.studio.welcome_dismissed = true;
                }
                if let Some(act) = chosen {
                    self.run(act);
                }
            });
    }

    /// What is selected, with the commands used most on it.
    fn studio_selection_card(&mut self, ctx: &egui::Context, map: egui::Rect) {
        if self.tool != Tool::Select || self.land_move.active {
            return;
        }
        let Some(id) = self.selected else { return };
        let Some(entity) = self.doc.entities.get(&id) else {
            return;
        };
        let name = entity.name.clone();
        let color = entity.color;
        let territory = entity.pos.is_none();
        let kind = match entity.kind {
            EntityKind::City => "城市",
            EntityKind::Region => "地区",
            EntityKind::Country if entity.parent.is_some() => "下级地区",
            EntityKind::Country => "国家",
        };
        let size = egui::vec2(440.0_f32.min(map.width() - 24.0), 76.0);
        if size.x < 260.0 || map.height() < 220.0 {
            return;
        }
        let at = egui::pos2(map.center().x - size.x * 0.5, map.bottom() - size.y - 14.0);
        egui::Area::new(egui::Id::unique("studio-selection"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain(false)
            .show(ctx, |ui| {
                let p = Palette::ui(ui);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                card(ui.painter(), rect, 16.0, p);
                let mut card = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(egui::vec2(12.0, 8.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                card.horizontal(|ui| {
                    crate::panels::swatch(ui, color);
                    ui.label(RichText::new(tr(kind)).color(p.muted));
                    ui.add(egui::Label::new(RichText::new(name.as_str()).strong()).truncate());
                });
                let mut act = None;
                card.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.can(Act::ZoomSelection),
                            egui::Button::new(tr("缩放到所选")),
                        )
                        .on_hover_text("F")
                        .clicked()
                    {
                        act = Some(Act::ZoomSelection);
                    }
                    ui.toggle_value(&mut self.studio.inspector, tr("属性"));
                    if territory
                        && ui
                            .add_enabled(
                                self.brush != Some(id),
                                egui::Button::new(tr("设为画笔国家")),
                            )
                            .on_hover_text(tr("之后用“领土画笔”或绘制陆地时归属这个国家。"))
                            .clicked()
                    {
                        self.brush = Some(id);
                    }
                    if ui.button(tr("取消选择")).clicked() {
                        act = Some(Act::Deselect);
                    }
                });
                if let Some(act) = act {
                    self.run(act);
                }
            });
    }

    fn studio_shelf(&mut self, ctx: &egui::Context, shelf: egui::Rect, style: Shelf, glass: bool) {
        egui::Area::new(egui::Id::unique("studio-floating-tools"))
            .order(egui::Order::Foreground)
            .fixed_pos(shelf.min)
            .constrain(false)
            .show(ctx, |ui| {
                let p = Palette::ui(ui);
                let (rect, _) = ui.allocate_exact_size(shelf.size(), egui::Sense::hover());
                soft_shadow(ui.painter(), rect, 19.0, p.shadow);
                let fill = if glass {
                    // the glass material is painted under it by the map view
                    if ui.visuals().dark_mode {
                        Color32::from_black_alpha(26)
                    } else {
                        Color32::from_white_alpha(20)
                    }
                } else {
                    p.card
                };
                ui.painter().rect(
                    rect,
                    19.0,
                    fill,
                    egui::Stroke::new(1.0, p.hairline),
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
                tools.menu_button(RichText::new(tr("绘制")).strong().color(p.accent), |ui| {
                    self.studio_create(ui)
                });
                for &tool in style.tools() {
                    self.studio_tool_button(&mut tools, tool);
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

    /// One tool as an icon; its name, key and use in the tooltip.
    fn studio_tool_button(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(TOOL_BUTTON, TOOL_BUTTON), egui::Sense::click());
        let p = Palette::ui(ui);
        let on = self.tool == tool && !self.land_move.active;
        let fill = if on {
            p.selected
        } else if response.hovered() {
            p.hovered
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 9.0, fill);
        crate::toolbar_ui::tool_icon(ui.painter(), rect, tool, if on { p.accent } else { p.ink });
        let key = tool.key().name();
        let response = response.on_hover_ui(|ui| {
            ui.set_max_width(300.0);
            ui.label(RichText::new(format!("{}  （{key}）", tool.name())).strong());
            ui.label(tool.help());
        });
        if response.clicked() {
            self.run(Act::Tool(tool));
        }
    }

    fn studio_edit_buttons(&mut self, ui: &mut egui::Ui) {
        for (act, title, keys) in [(Act::Undo, "撤销", "Ctrl+Z"), (Act::Redo, "重做", "Ctrl+Y")] {
            let mut response = ui.add_enabled(self.can(act), egui::Button::new(tr(title)));
            if response.hovered() {
                // what the step would undo or redo
                let all = actions::table();
                if let Some(action) = all.iter().find(|a| a.act == act) {
                    let hint = format!("{} · {keys}", self.action_label(action));
                    response = response.on_hover_text(hint);
                }
            }
            if response.clicked() {
                self.run(act);
                ui.close_kind(egui::UiKind::Menu);
            }
        }
    }

    fn studio_panel_export(&mut self, ui: &mut egui::Ui) {
        if ui
            .toggle_value(&mut self.studio.inspector, tr("属性"))
            .on_hover_text(tr("显示或隐藏右侧的属性面板"))
            .clicked()
        {
            ui.close_kind(egui::UiKind::Menu);
        }
        if ui.button(tr("导出…")).on_hover_text("Ctrl+E").clicked() {
            self.run(Act::Export);
            ui.close_kind(egui::UiKind::Menu);
        }
    }

    /// An entry of the action table, with its shortcut and state.
    fn studio_action(&mut self, ui: &mut egui::Ui, all: &[actions::Action], act: Act) {
        if let Some(action) = all.iter().find(|a| a.act == act) {
            if self.action_button(ui, action) {
                ui.close();
            }
        }
    }

    fn studio_view_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(230.0);
        let all = actions::table();
        for act in [
            Act::Layout(Layout::Flat),
            Act::Layout(Layout::Globe),
            Act::Layout(Layout::Split),
        ] {
            self.studio_action(ui, &all, act);
        }
        ui.separator();
        for act in [Act::FitAll, Act::ZoomSelection, Act::ZoomIn, Act::ZoomOut] {
            self.studio_action(ui, &all, act);
        }
        ui.separator();
        for act in [
            Act::Timeline,
            Act::Graticule,
            Act::Furniture,
            Act::ProgressiveDetail,
        ] {
            self.studio_action(ui, &all, act);
        }
        ui.separator();
        // map looks: a whole set of document style values, one undo step
        ui.label(RichText::new(tr("地图风格")).color(Palette::ui(ui).muted))
            .on_hover_text(tr("改变这份地图的海洋、陆地、边界与文字颜色，可以撤销；专业界面看到的是同一份地图。"));
        let current = map_core::basemap::current(&self.doc.style).map(|theme| theme.name);
        for theme in &map_core::basemap::THEMES {
            if style_row(ui, tr(theme.name), &theme.style(), current == Some(theme.name))
                .on_hover_text(tr(theme.about))
                .clicked()
            {
                let all = map_io::fonts::families();
                let font = theme
                    .fonts
                    .iter()
                    .find(|f| all.iter().any(|x| x.family == **f))
                    .map(|f| f.to_string());
                self.edit(&format!("地图主题：{}", theme.name), |d| {
                    theme.apply(&mut d.style, font);
                    Ok(())
                });
                ui.close();
            }
        }
    }

    fn studio_help_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(230.0);
        let all = actions::table();
        for act in [Act::Tutorial, Act::Shortcuts, Act::Palette] {
            self.studio_action(ui, &all, act);
        }
        ui.separator();
        for act in [Act::NewTestWorld, Act::NewWorldWizard] {
            self.studio_action(ui, &all, act);
        }
        if ui
            .button(tr("打开远洋五国练习工程"))
            .on_hover_text(tr("上一版教程的成品工程，随程序附带。"))
            .clicked()
        {
            self.run(Act::TutorialExample);
            ui.close();
        }
    }

    /// Every drawing and editing tool, by task, as tiles with their keys.
    fn studio_create(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(3.0 * TILE_WIDTH + 2.0 * 8.0);
        let groups: [(&str, &[Entry]); 4] = [
            (
                "地形与水系",
                &[
                    Entry::Tool(Tool::DrawLand),
                    Entry::Tool(Tool::DrawLake),
                    Entry::Tool(Tool::River),
                    Entry::Tool(Tool::Pen),
                ],
            ),
            (
                "国家与边界",
                &[
                    Entry::Tool(Tool::Assign),
                    Entry::Tool(Tool::Split),
                    Entry::Tool(Tool::Redraw),
                    Entry::Tool(Tool::EditVertex),
                    Entry::Tool(Tool::MergeFaces),
                ],
            ),
            (
                "地点与标注",
                &[
                    Entry::Tool(Tool::City),
                    Entry::Tool(Tool::Label),
                    Entry::Tool(Tool::Symbol),
                    Entry::Tool(Tool::Hatch),
                    Entry::Command(Act::PixelBrush, Glyph::Pixels, "彩色像素画笔…"),
                ],
            ),
            (
                "整体与参考",
                &[
                    Entry::Command(Act::LandMove, Glyph::Move, "整体移动陆块"),
                    Entry::Command(Act::LandScale, Glyph::Scale, "整体缩放陆块"),
                    Entry::Command(Act::ImportUnderlay, Glyph::Picture, "导入参考图片…"),
                    Entry::Tool(Tool::Align),
                    Entry::Command(Act::GeographicGuides, Glyph::Guide, "地理参考线…"),
                    Entry::Tool(Tool::Measure),
                    Entry::Tool(Tool::Eyedropper),
                ],
            ),
        ];
        let mut chosen = None;
        for (heading, entries) in groups {
            ui.label(RichText::new(tr(heading)).strong().color(Palette::ui(ui).muted));
            for row in entries.chunks(3) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for &entry in row {
                        let (act, glyph, label, key, on) = match entry {
                            Entry::Tool(tool) => (
                                Act::Tool(tool),
                                Glyph::Tool(tool),
                                tool.name(),
                                tool.key().name(),
                                self.tool == tool && !self.land_move.active,
                            ),
                            Entry::Command(act, glyph, label) => (act, glyph, tr(label), "", false),
                        };
                        let response = tile(ui, glyph, label, key, on, self.can(act));
                        let response = match entry {
                            Entry::Tool(tool) => response.on_hover_text(tool.help()),
                            Entry::Command(..) => response,
                        };
                        if response.clicked() && self.can(act) {
                            chosen = Some(act);
                        }
                    }
                });
            }
            ui.add_space(6.0);
        }
        if let Some(act) = chosen {
            if self.studio.page == Page::Environment {
                self.studio.show_map();
            }
            self.run(act);
            ui.close();
        }
    }
}

const TILE_WIDTH: f32 = 150.0;

/// An entry of the drawing menu: a tool, or a command with its own glyph.
#[derive(Clone, Copy)]
enum Entry {
    Tool(Tool),
    Command(Act, Glyph, &'static str),
}

/// Pictures for the commands that are not tools (tools use their icons).
#[derive(Clone, Copy)]
enum Glyph {
    Tool(Tool),
    Pixels,
    Move,
    Scale,
    Picture,
    Guide,
}

fn draw_glyph(painter: &egui::Painter, rect: egui::Rect, glyph: Glyph, ink: Color32) {
    let size = rect.width().min(rect.height());
    let c = rect.center();
    let line = egui::Stroke::new(1.6, ink);
    let thin = egui::Stroke::new(1.1, ink.gamma_multiply(0.7));
    match glyph {
        Glyph::Tool(tool) => crate::toolbar_ui::tool_icon(painter, rect, tool, ink),
        Glyph::Pixels => {
            let cell = size / 5.0;
            let origin = c - egui::vec2(cell * 1.5, cell * 1.5);
            for row in 0..3 {
                for col in 0..3 {
                    let min = origin + egui::vec2(col as f32 * cell, row as f32 * cell);
                    let color = if (row + col) % 2 == 0 {
                        ink
                    } else {
                        ink.gamma_multiply(0.35)
                    };
                    painter.rect_filled(
                        egui::Rect::from_min_size(min, egui::vec2(cell - 1.0, cell - 1.0)),
                        1.0,
                        color,
                    );
                }
            }
        }
        Glyph::Move => {
            let r = size * 0.34;
            painter.line_segment([c - egui::vec2(r, 0.0), c + egui::vec2(r, 0.0)], line);
            painter.line_segment([c - egui::vec2(0.0, r), c + egui::vec2(0.0, r)], line);
            for dir in [
                egui::vec2(1.0, 0.0),
                egui::vec2(-1.0, 0.0),
                egui::vec2(0.0, 1.0),
                egui::vec2(0.0, -1.0),
            ] {
                let tip = c + dir * r;
                let back = tip - dir * (r * 0.42);
                let side = egui::vec2(-dir.y, dir.x) * (r * 0.32);
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, back + side, back - side],
                    ink,
                    egui::Stroke::NONE,
                ));
            }
        }
        Glyph::Scale => {
            let big = egui::Rect::from_center_size(c, egui::vec2(size * 0.64, size * 0.64));
            let small = egui::Rect::from_min_size(
                egui::pos2(big.left(), big.bottom() - size * 0.3),
                egui::vec2(size * 0.3, size * 0.3),
            );
            painter.rect_stroke(big, 2.0, thin, egui::StrokeKind::Middle);
            painter.rect_filled(small, 1.0, ink);
            painter.line_segment([small.right_top(), big.right_top()], line);
        }
        Glyph::Picture => {
            let frame = egui::Rect::from_center_size(c, egui::vec2(size * 0.72, size * 0.56));
            painter.rect_stroke(frame, 2.0, line, egui::StrokeKind::Middle);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(frame.left() + size * 0.08, frame.bottom() - size * 0.06),
                    egui::pos2(frame.left() + size * 0.3, frame.top() + size * 0.2),
                    egui::pos2(frame.left() + size * 0.5, frame.bottom() - size * 0.06),
                ],
                ink.gamma_multiply(0.45),
                egui::Stroke::NONE,
            ));
            painter.circle_filled(
                egui::pos2(frame.right() - size * 0.16, frame.top() + size * 0.15),
                size * 0.06,
                ink,
            );
        }
        Glyph::Guide => {
            let y = c.y;
            let mut x = rect.left() + size * 0.12;
            while x < rect.right() - size * 0.12 {
                painter.line_segment(
                    [egui::pos2(x, y), egui::pos2((x + size * 0.1).min(rect.right()), y)],
                    line,
                );
                x += size * 0.18;
            }
            painter.line_segment(
                [egui::pos2(c.x, rect.top() + size * 0.2), egui::pos2(c.x, rect.bottom() - size * 0.2)],
                thin,
            );
        }
    }
}

/// A menu tile: icon, name and key; highlighted while its tool is in hand.
fn tile(
    ui: &mut egui::Ui,
    glyph: Glyph,
    label: &str,
    key: &str,
    on: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(TILE_WIDTH, 38.0), sense);
    let p = Palette::ui(ui);
    let fill = if on {
        p.selected
    } else if enabled && response.hovered() {
        p.hovered
    } else {
        p.faint
    };
    let ink = if !enabled {
        p.muted.gamma_multiply(0.55)
    } else if on {
        p.accent
    } else {
        p.ink
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 9.0, fill);
    if on {
        painter.rect_stroke(rect, 9.0, egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
    }
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::vec2(26.0, 26.0),
    );
    draw_glyph(&painter, icon, glyph, ink);
    let room = rect.width() - 40.0 - if key.is_empty() { 6.0 } else { 24.0 };
    let mut size = 14.0;
    let width = painter
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(size), ink)
        .size()
        .x;
    if width > room {
        size = (size * room / width).max(11.0);
    }
    painter.text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(size),
        ink,
    );
    if !key.is_empty() {
        painter.text(
            egui::pos2(rect.right() - 9.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            key,
            egui::FontId::proportional(11.5),
            p.muted,
        );
    }
    response
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
        assert!(Shelf::Full.tools().contains(&Tool::Select));
        assert!(Shelf::Compact.tools().contains(&Tool::Select));
    }

    #[test]
    fn the_drawing_menu_offers_every_tool_but_select() {
        // keep in step with `studio_create`
        let listed = [
            Tool::DrawLand,
            Tool::DrawLake,
            Tool::River,
            Tool::Pen,
            Tool::Assign,
            Tool::Split,
            Tool::Redraw,
            Tool::EditVertex,
            Tool::MergeFaces,
            Tool::City,
            Tool::Label,
            Tool::Symbol,
            Tool::Hatch,
            Tool::Align,
            Tool::Measure,
            Tool::Eyedropper,
        ];
        for tool in Tool::ALL.into_iter().filter(|t| *t != Tool::Select) {
            assert!(listed.contains(&tool), "{tool:?}");
        }
    }
}
