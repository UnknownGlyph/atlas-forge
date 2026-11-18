//! Map views (2D projection and globe), editing tools and overlays.
//! Both views convert the pointer to lon/lat first, so every tool works in
//! either view.

use crate::app::App;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu;
use map_core::geom::{self, LonLat};
use map_core::topo::{FaceKind, BACKGROUND};
use map_core::EntityKind;
use map_render::camera::{Camera2D, GlobeCamera};
use map_render::gpu::{FrameData, LineStyle, MapGpu, ViewUniform};
use map_render::tess::rgba;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    DrawLand,
    DrawLake,
    Split,
    /// Redraw a stretch of border: the land between old and new line changes hands.
    Redraw,
    EditVertex,
    Assign,
    City,
    River,
    MergeFaces,
    Align,
    /// Place country labels by hand.
    Label,
    /// Point symbols: peaks, battles, ports...
    Symbol,
    /// Hatched areas: disputed, occupied, claimed...
    Hatch,
    /// Measure lengths and areas (nothing is saved).
    Measure,
    /// Bézier curves: rivers, routes and hatched areas (see `curve_ui`).
    Pen,
    /// Takes a feature's look (and style) to the selected features.
    Eyedropper,
}

impl Tool {
    /// Open lines that start and end on an existing boundary.
    pub fn ends_on_boundary(self) -> bool {
        matches!(self, Tool::Split | Tool::Redraw)
    }

    pub const ALL: [Tool; 17] = [
        Tool::Select,
        Tool::DrawLand,
        Tool::DrawLake,
        Tool::Split,
        Tool::Redraw,
        Tool::EditVertex,
        Tool::Assign,
        Tool::MergeFaces,
        Tool::City,
        Tool::River,
        Tool::Pen,
        Tool::Label,
        Tool::Symbol,
        Tool::Hatch,
        Tool::Eyedropper,
        Tool::Measure,
        Tool::Align,
    ];

    /// The name without the shortcut letter.
    pub fn name(self) -> &'static str {
        let l = self.label();
        l.rsplit_once("  ").map_or(l, |x| x.0)
    }

    /// The name as written (Chinese), without its key.
    pub fn source_name(self) -> &'static str {
        let l = self.source_label();
        l.rsplit_once("  ").map_or(l, |x| x.0)
    }

    /// The key that selects the tool.
    pub fn key(self) -> egui::Key {
        use egui::Key;
        match self {
            Tool::Select => Key::V,
            Tool::DrawLand => Key::L,
            Tool::DrawLake => Key::K,
            Tool::Split => Key::S,
            Tool::Redraw => Key::G,
            Tool::EditVertex => Key::E,
            Tool::Assign => Key::B,
            Tool::MergeFaces => Key::M,
            Tool::City => Key::C,
            Tool::River => Key::R,
            Tool::Align => Key::A,
            Tool::Label => Key::T,
            Tool::Symbol => Key::Y,
            Tool::Hatch => Key::H,
            Tool::Measure => Key::D,
            Tool::Pen => Key::P,
            Tool::Eyedropper => Key::I,
        }
    }

    /// The tool's name in the interface's language.
    pub fn label(self) -> &'static str {
        crate::i18n::tr(self.source_label())
    }

    /// What the tool does, in the interface's language.
    pub fn help(self) -> &'static str {
        crate::i18n::tr(self.source_help())
    }

    /// The name as written (Chinese).
    pub fn source_label(self) -> &'static str {
        match self {
            Tool::Select => "选择 / 浏览  V",
            Tool::DrawLand => "绘制陆地  L",
            Tool::DrawLake => "绘制湖泊  K",
            Tool::Split => "切分 / 画边界  S",
            Tool::Redraw => "重画国界  G",
            Tool::EditVertex => "编辑顶点  E",
            Tool::Assign => "领土画笔  B",
            Tool::MergeFaces => "合并相邻面  M",
            Tool::City => "添加城市  C",
            Tool::River => "河流 / 线路  R",
            Tool::Align => "配准参考图  A",
            Tool::Label => "标注  T",
            Tool::Symbol => "地图符号  Y",
            Tool::Hatch => "斜线区域  H",
            Tool::Measure => "测量  D",
            Tool::Pen => "钢笔 / 曲线  P",
            Tool::Eyedropper => "吸管  I",
        }
    }

    /// What the tool does, as written (Chinese).
    pub fn source_help(self) -> &'static str {
        match self {
            Tool::Select => "单击选择国家/地区；拖动平移（球体为旋转），滚轮缩放。",
            Tool::DrawLand => "单击加点或按住拖动手绘；靠近边界/海岸自动吸附，两点都在边界上时沿边界描线（Ctrl 临时关闭）；Shift 约束 15° 方向；拖动草稿点移动、拖动中点插入、右键删除；单击起点/双击/回车完成，Backspace 撤回，Esc 取消。",
            Tool::DrawLake => "与绘制陆地相同的操作；画出的区域变为水域（可跨越边界）。",
            Tool::Split => "从边界上一点开始，单击或拖动手绘穿过面内部，落到边界上即完成 → 一个面切成两个；草稿点可拖动、右键删除。",
            Tool::Redraw => "从边界上一点出发画新的走向，落回边界即完成：新旧边界之间的土地归对侧所有（对侧是海则变为水域）。这是领土变化，选了年份时只从该年起生效，更早的地图不变。光标停在边界上时预览移交的区域。",
            Tool::EditVertex => "拖动顶点（共享边界两侧同步）；拖动边上任意位置插入新顶点；右键删除顶点。",
            Tool::Assign => "左键单击/拖动把陆地分配给“当前画笔国家”；右键取消归属；Shift+单击（或勾选“填充”）把相连的一整片分配给它。",
            Tool::MergeFaces => "依次单击两个相邻的面，删除它们之间的边界。",
            Tool::City => "单击空白处添加城市（自动归属所在国家）；单击城市选中，拖动城市移动，右键城市删除。",
            Tool::River => "单击添加折线点，双击或回车结束；工具面板中选择画河流还是线路（道路、商路、行军路线）。点会吸附到同类线的端点和线上（线路还吸附到城市），Ctrl 临时关闭；从一条线的端点开始（或在端点结束）会延长那条线。画好的线用选择工具单击选中。",
            Tool::Align => "把参考图片上的地物拖到它的真实位置，图片随之变形对齐；右键单击配准点删除。",
            Tool::Measure => "单击各点沿大圆测量距离，单击第一个点闭合并给出面积；右键撤回一个点，Esc 清除。按本星球半径计算，不会保存。",
            Tool::Hatch => "像绘制陆地一样画出区域（单击加点或拖动手绘，可吸附并沿边界描线；单击起点/双击/回车完成），用斜线标出争议、占领、势力范围等；不改变领土归属。单击区域边线选中，在国家页的面板中修改。",
            Tool::Symbol => "单击空白处放置符号（山峰、战役、首都、港口、要塞、遗址），单击选中，拖动移动，右键或 Delete 删除；可命名并设置显示的年份范围（未选年份时全部显示）。",
            Tool::Pen => "用贝塞尔曲线画河流、线路或斜线区域（在选项栏选择）：单击放尖角锚点，按住拖动放平滑锚点并拉出控制柄；画区域时单击第一个锚点闭合；双击或回车完成，右键或 Backspace 撤回上一个锚点，Esc 放弃。没有在画时，单击一条线或区域选中它，拖动其锚点与控制柄修改。",
            Tool::Eyedropper => "先用选择工具选中要修改的线路、区域、文字或符号，再用吸管单击作样板的要素：所选的同类要素换成它的外观；样板使用某个样式时，它们也使用这个样式。",
            Tool::Label => "双击空白处添加地图文字；拖动国家标注到想要的位置（所有年份相同，始终显示，导出同样生效）；单击标注选中国家，在国家属性中旋转、缩放、隐藏或恢复自动。此工具下显示全部标注（含被避让和隐藏的）。",
        }
    }
}

/// What the draw-land tool does where the shape overlaps existing land.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawMode {
    AddLand,
    Expand,
    Take,
    /// Create a new country holding the whole drawn area.
    NewCountry,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Flat,
    Globe,
}

/// Screen ⇄ lon/lat mapping of one view.
#[derive(Clone, Copy)]
pub struct ViewGeom {
    pub kind: ViewKind,
    pub rect: Rect,
    pub cam: Camera2D,
    pub globe: GlobeCamera,
}

impl ViewGeom {
    fn d(&self, p: Pos2) -> [f64; 2] {
        let c = self.rect.center();
        [(p.x - c.x) as f64, (p.y - c.y) as f64]
    }
    pub fn to_lonlat(&self, p: Pos2) -> Option<LonLat> {
        match self.kind {
            ViewKind::Flat => self.cam.screen_to_lonlat(self.d(p)),
            ViewKind::Globe => self.globe.pick(self.d(p), self.rect.width() as f64, self.rect.height() as f64),
        }
    }
    pub fn to_screen(&self, ll: LonLat) -> Option<Pos2> {
        let c = self.rect.center();
        let s = match self.kind {
            ViewKind::Flat => Some(self.cam.lonlat_to_screen(ll)),
            ViewKind::Globe => self.globe.project(ll, self.rect.width() as f64, self.rect.height() as f64),
        }?;
        Some(Pos2::new(c.x + s[0] as f32, c.y + s[1] as f32))
    }
    /// Degrees corresponding to `px` screen pixels at `p`.
    pub fn deg_per_px(&self, p: Pos2, px: f32) -> f64 {
        match (self.to_lonlat(p), self.to_lonlat(p + Vec2::new(px, 0.0))) {
            (Some(a), Some(b)) => geom::geo_dist_deg(a, b).max(1e-9),
            _ => 1.0,
        }
    }
    pub fn px_per_deg(&self) -> f64 {
        match self.kind {
            ViewKind::Flat => self.cam.zoom * std::f64::consts::PI / 180.0,
            ViewKind::Globe => {
                self.globe.px_per_rad(self.rect.width() as f64, self.rect.height() as f64) * std::f64::consts::PI / 180.0
            }
        }
    }
}

struct MapCallback(FrameData);

impl egui_wgpu::CallbackTrait for MapCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _sd: &egui_wgpu::ScreenDescriptor,
        _enc: &mut egui_wgpu::wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        if let Some(gpu) = res.get_mut::<MapGpu>() {
            gpu.prepare(device, queue, &self.0);
        }
        vec![]
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut egui_wgpu::wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        if let Some(gpu) = res.get::<MapGpu>() {
            gpu.paint(pass, self.0.view_id);
        }
    }
}

impl App {
    fn line_styles(&self, ppp: f32) -> [LineStyle; 12] {
        let s = &self.doc.style;
        let ls = |c: [u8; 4], w: f32| LineStyle { color: rgba(c), width: w * ppp };
        [
            ls(s.coast, s.coast_width),
            ls(s.border, s.border_width),
            ls(s.internal, s.internal_width),
            ls(s.river, s.river_width),
            ls([255, 120, 0, 255], 2.6),
            ls([20, 90, 220, 220], 1.8),
            ls([255, 255, 255, 90], 0.8),
            ls(s.overlay_color, s.overlay_width),
            // land changed hands in a local view
            ls([215, 35, 35, 255], 2.4),
            // the coast bands, widest first
            ls(s.glow_bands()[0], s.coast_glow_width * 2.0),
            ls(s.glow_bands()[1], s.coast_glow_width * 4.0 / 3.0),
            ls(s.glow_bands()[2], s.coast_glow_width * 2.0 / 3.0),
        ]
    }

    pub fn map_view(&mut self, ui: &mut egui::Ui, kind: ViewKind, rect: Rect) {
        let id = ui.scope_id().with(match kind {
            ViewKind::Flat => "flat",
            ViewKind::Globe => "globe",
        });
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if kind == ViewKind::Flat {
            // keep the whole world in view on resize until the user navigates
            if !self.nav_touched && (rect.size() - self.last_flat_rect.size()).length() > 1.0 {
                self.fit_pending = true;
            }
            self.last_flat_rect = rect;
        }
        if self.fit_pending && kind == ViewKind::Flat {
            self.cam2d.fit(rect.width() as f64, rect.height() as f64);
            self.nav_touched = false;
            if self.layout == crate::app::Layout::Flat || self.layout == crate::app::Layout::Split {
                self.fit_pending = false;
            }
        }
        if self.fit_pending && self.layout == crate::app::Layout::Globe {
            self.fit_pending = false;
        }
        let mut vg = ViewGeom { kind, rect, cam: self.cam2d, globe: self.globe };

        // navigation
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let f = (scroll as f64 * 0.002).exp();
                match kind {
                    ViewKind::Flat => {
                        let p = resp.hover_pos().unwrap_or(rect.center());
                        self.cam2d.zoom_at(vg.d(p), f);
                        self.nav_touched = true;
                    }
                    ViewKind::Globe => self.globe.zoom(f),
                }
            }
        }
        // a drag on the selected river, route or area moves it instead of the map
        let climate_pick = self.climate_ui.open && self.climate_ui.picking;
        let moving = !climate_pick && (self.curve_drag(&resp, &vg) || self.move_drag(&resp, &vg));
        let nav = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (resp.dragged_by(egui::PointerButton::Primary) && (self.space_pan || (matches!(self.tool, crate::view::Tool::Select) && !moving)));
        if resp.hovered() && ui.input(|i| i.key_down(egui::Key::Space)) && !ui.ctx().egui_wants_keyboard_input() {
            ui.ctx().set_cursor_icon(if self.space_pan { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        if nav {
            let d = resp.drag_delta();
            match kind {
                ViewKind::Flat => {
                    self.cam2d.pan_px([d.x as f64, d.y as f64]);
                    self.nav_touched = true;
                }
                ViewKind::Globe => self.globe.rotate_px([d.x as f64, d.y as f64], rect.width() as f64, rect.height() as f64),
            }
        }
        vg.cam = self.cam2d;
        vg.globe = self.globe;

        let hover_ll = resp.hover_pos().and_then(|p| vg.to_lonlat(p));
        if resp.hovered() {
            self.cursor = hover_ll;
        }
        // a press that began with Space held pans; the tool does not see it
        if climate_pick && !self.space_pan {
            if resp.clicked_by(egui::PointerButton::Primary) {
                if let Some(point) = hover_ll { self.climate_pick(point); }
            }
        } else if !self.space_pan {
            self.tool_input(&resp, &vg, hover_ll);
        }
        // the select tool's context menu (other tools use the right button)
        if self.tool == Tool::Select {
            resp.context_menu(|ui| self.canvas_menu(ui));
        }

        // GPU paint
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = (rect.width() * ppp, rect.height() * ppp);
        let styles = self.line_styles(ppp);
        let mut c = self.cam2d;
        c.zoom *= ppp as f64;
        let uniform = match kind {
            ViewKind::Flat => ViewUniform::for_2d(&c, w, h, &styles),
            ViewKind::Globe => ViewUniform::for_globe(&self.globe, self.scene.lon0(), w, h, &styles),
        };
        let data = FrameData {
            view_id: kind as u32,
            uniform,
            fill: self.scene.fill_chunks(),
            lines: self.scene.line_chunks(),
            colors: (self.scene.color_gen, self.scene.colors.clone()),
            overlay: self.overlay.clone(),
            glow: self.glow.clone(),
            graticule: self.graticule,
            underlays: self.underlay_frames(kind, &c, &self.globe, w, h),
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(40));
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, MapCallback(data)));
        let t_ov = std::time::Instant::now();
        self.draw_overlays(&painter, &vg, hover_ll);
        if self.prof.is_some() {
            eprintln!("OVERLAYS {} {:.2}", if vg.kind == ViewKind::Flat { "flat" } else { "globe" }, t_ov.elapsed().as_secs_f32() * 1000.0);
        }
        self.draw_selection(&painter, &vg);
        self.draw_reshape(&painter, &vg);
        self.draw_river_marks(&painter, &vg, hover_ll);
        self.draw_curve_overlay(&painter, &vg, hover_ll);
        self.draw_area_handles(&painter, &vg, hover_ll);
        self.draw_draft(&painter, &vg, hover_ll);
        self.draw_measure(&painter, &vg, hover_ll);
        self.draw_underlay_marks(&painter, &vg);
        if kind == ViewKind::Globe && self.layout == crate::app::Layout::Split {
            painter.text(rect.left_top() + Vec2::new(8.0, 6.0), egui::Align2::LEFT_TOP, "三维球体", egui::FontId::proportional(13.0), Color32::from_gray(230));
        }
    }

    fn tool_input(&mut self, resp: &egui::Response, vg: &ViewGeom, ll: Option<LonLat>) {
        use egui::PointerButton::{Primary, Secondary};
        let tol = resp.hover_pos().map(|p| vg.deg_per_px(p, 8.0)).unwrap_or(0.5);
        // hover face (for highlighting)
        self.hover_face = match (self.tool, ll) {
            (Tool::Select | Tool::Assign | Tool::MergeFaces, Some(p)) if resp.hovered() => {
                Some(self.doc.topo.face_at(p)).filter(|f| *f != BACKGROUND || self.doc.background_kind() == FaceKind::Land)
            }
            _ => None,
        };
        if self.tool == Tool::Align {
            return self.align_input(resp, vg, ll);
        }
        if self.tool == Tool::Label {
            return self.label_input(resp, vg);
        }
        if self.tool == Tool::Symbol {
            return self.symbol_input(resp, vg);
        }
        if self.tool == Tool::Measure {
            return self.measure_input(resp, vg, ll);
        }
        // anchors and handles of a curve: double-click, right-click
        if matches!(self.tool, Tool::Select | Tool::Pen | Tool::EditVertex) && (self.tool != Tool::Pen || self.pen.is_empty()) && (resp.clicked() || resp.secondary_clicked()) && self.curve_click(resp, vg) {
            return;
        }
        if self.tool == Tool::Pen {
            return self.pen_input(resp, vg);
        }
        if self.tool == Tool::Eyedropper {
            return self.eyedropper_input(resp, vg);
        }
        let Some(p) = ll else { return };
        if self.tool == Tool::EditVertex && (self.river_edit_input(resp, vg, p, tol) || self.area_edit_input(resp, vg, p, tol)) {
            return;
        }
        match self.tool {
            Tool::Select => {
                // a right click selects what is under it too, for its menu
                let picked = resp.clicked() || resp.secondary_clicked();
                // drawn features first (one or several, see `select_ui`)
                if picked && self.select_feature_click(resp, vg) {
                    return;
                }
                if picked {
                    self.clear_selection();
                }
                if picked && !self.city_select_click(p, tol) {
                    let f = self.doc.topo.face_at(p);
                    self.selected_face = (f != BACKGROUND || self.doc.background_kind() == FaceKind::Land).then_some(f);
                    self.selected = self.doc.owner.get(&f).copied();
                    if let Some(e) = self.selected {
                        self.tab = crate::panels::Tab::Countries;
                        let name = self.doc.entities[&e].name.clone();
                        self.set_status(format!("已选择 {name}"), false);
                    }
                }
            }
            Tool::Hatch => {
                // a click on an outline selects it while nothing is being drawn
                if self.draft.is_empty() && resp.clicked() && !resp.double_clicked() && !resp.triple_clicked() && self.area_select_click(p, tol) {
                    return;
                }
                self.draft_input(resp, vg, p)
            }
            Tool::DrawLand | Tool::DrawLake | Tool::River | Tool::Split | Tool::Redraw => self.draft_input(resp, vg, p),
            Tool::EditVertex if self.doc.year.is_some() && self.vertex_dated => self.dated_vertex_input(resp, vg, p, tol),
            Tool::EditVertex => {
                if resp.drag_started_by(Primary) {
                    // pick at the press position: by the time a drag is recognised
                    // the pointer may already be several pixels away
                    let origin = resp.ctx.input(|i| i.pointer.press_origin());
                    let p = origin.and_then(|o| vg.to_lonlat(o)).unwrap_or(p);
                    if let Some((v, _)) = self.doc.topo.nearest_vertex(p, tol) {
                        self.begin_pending("移动顶点");
                        self.drag_vertex = Some(v);
                    } else if let Some((e, seg, q)) = self.doc.topo.nearest_on_edge(p, tol) {
                        self.begin_pending("插入并移动顶点");
                        match self.doc.topo.insert_vertex(e, seg, q) {
                            Ok(v) => {
                                self.drag_vertex = Some(v);
                                if let Some(pe) = &mut self.pending {
                                    pe.changed = true;
                                }
                            }
                            Err(err) => self.set_status(err, true),
                        }
                    }
                }
                if resp.dragged_by(Primary) {
                    if let Some(v) = self.drag_vertex {
                        let mut trial = self.doc.clone();
                        match trial.topo.move_vertex(v, p) {
                            Ok(()) => {
                                self.doc = trial;
                                if let Some(pe) = &mut self.pending {
                                    pe.changed = true;
                                }
                                self.status_err = false;
                            }
                            Err(e) => self.set_status(format!("此位置无效：{e}"), true),
                        }
                    }
                }
                if resp.drag_stopped() && self.drag_vertex.take().is_some() {
                    self.end_pending();
                }
                if resp.clicked_by(Secondary) {
                    if let Some((v, _)) = self.doc.topo.nearest_vertex(p, tol) {
                        self.edit("删除顶点", |d| d.topo.delete_vertex(v));
                    }
                }
            }
            Tool::Assign => {
                let f = self.doc.topo.face_at(p);
                let land = self.doc.topo.faces[&f].kind == FaceKind::Land;
                // the fill bucket (option, or Shift held): the whole connected patch
                if self.assign_fill || self.press_mods.shift {
                    if resp.clicked_by(Secondary) && land {
                        if let Some(n) = self.edit("取消一片的归属", |d| d.fill(f, None)) {
                            self.set_status(format!("{n} 个地块成为无主地"), false);
                        }
                    }
                    if resp.clicked() && land {
                        let layer = self.doc.active_layer;
                        let Some(b) = self.brush.filter(|b| self.doc.entities.get(b).is_some_and(|e| e.layer == layer)) else {
                            self.set_status("请先在“国家”面板选择或新建一个国家作为画笔", true);
                            return;
                        };
                        let name = self.entity_name(Some(b));
                        if let Some(n) = self.edit("填充领土", |d| d.fill(f, Some(b))) {
                            self.set_status(if n == 0 { format!("这片土地已属于{name}") } else { format!("填充：{n} 个相连的地块归属{name}") }, false);
                        }
                    }
                    return;
                }
                if resp.clicked_by(Secondary) && land {
                    self.edit("取消归属", |d| d.assign(f, None));
                }
                let painting = resp.clicked() || resp.dragged_by(Primary);
                if painting && land {
                    let layer = self.doc.active_layer;
                    let Some(b) = self.brush.filter(|b| self.doc.entities.get(b).is_some_and(|e| e.layer == layer)) else {
                        self.set_status("请先在“国家”面板选择或新建一个国家作为画笔", true);
                        return;
                    };
                    // on a share layer the brush sets its unit's share
                    let bp = (self.share_pct * 100.0).round() as u16;
                    let needed = if self.doc.active_is_shares() { self.doc.shares(f).get(&b).copied().unwrap_or(0) != bp } else { self.doc.owner.get(&f) != Some(&b) };
                    if needed {
                        if self.pending.is_none() {
                            self.begin_pending("领土分配");
                        }
                        let r = if self.doc.active_is_shares() { self.doc.set_share(f, Some(b), bp) } else { self.doc.assign(f, Some(b)) };
                        match r {
                            Ok(()) => {
                                if let Some(pe) = &mut self.pending {
                                    pe.changed = true;
                                }
                            }
                            Err(e) => self.set_status(e, true),
                        }
                    }
                }
                if resp.clicked() || resp.drag_stopped() {
                    self.end_pending();
                }
            }
            Tool::Align | Tool::Label | Tool::Symbol | Tool::Measure | Tool::Pen | Tool::Eyedropper => {}
            Tool::MergeFaces => {
                if resp.clicked() {
                    let f = self.doc.topo.face_at(p);
                    if f == BACKGROUND {
                        return;
                    }
                    match self.merge_first {
                        None => self.merge_first = Some(f),
                        Some(a) => {
                            self.merge_first = None;
                            self.edit("合并面", |d| d.merge_faces(a, f));
                        }
                    }
                }
            }
            Tool::City => {
                if self.city_input(resp, vg, p, tol) {
                    return;
                }
                if resp.clicked() {
                    let n = self.doc.entities.values().filter(|e| e.kind == EntityKind::City).count() + 1;
                    if let Some(id) = self.edit("添加城市", |d| Ok(d.add_city(p, &format!("城市{n}")))) {
                        self.selected = Some(id);
                        self.tab = crate::panels::Tab::Countries;
                    }
                }
            }
        }
    }

    fn draw_overlays(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let rect = vg.rect;
        let style = &self.doc.style;
        // country labels: placed ones, then automatic ones avoiding each other
        let mut taken: Vec<Rect> = Vec::new();
        let t_a = std::time::Instant::now();
        // drawn features, layer by layer: those in no layer first, then
        // each layer from the bottom of the stack up, with its opacity
        for pass in self.doc.draw_passes() {
            let mut p = painter.clone();
            p.set_opacity(pass.opacity);
            self.draw_areas(&p, vg, &mut taken, pass);
            self.draw_routes(&p, vg, pass);
            self.draw_annotations(&p, vg, &mut taken, pass);
            self.draw_symbols(&p, vg, &mut taken, pass);
        }
        let t_features = t_a.elapsed().as_secs_f32() * 1000.0;
        // a local view's region, the frame of an export
        self.draw_focus(painter, vg);
        self.draw_export_frame(painter, vg);
        self.draw_partition(painter, vg);
        self.draw_roads(painter, vg);
        let t_b = std::time::Instant::now();
        self.draw_line_names(painter, vg, &mut taken);
        let t_lines = t_b.elapsed().as_secs_f32() * 1000.0;
        let t_b = std::time::Instant::now();
        self.draw_country_labels(painter, vg, &mut taken);
        if self.prof.is_some() {
            eprintln!("PARTS features {:.2} line_names {:.2} countries {:.2}", t_features, t_lines, t_b.elapsed().as_secs_f32() * 1000.0);
        }
        if style.show_cities {
            for (_, e) in self.doc.entities.iter() {
                let (EntityKind::City, Some(p)) = (e.kind, e.pos) else { continue };
                if !e.exists_in(self.doc.year) {
                    continue;
                }
                let Some(s) = vg.to_screen(p) else { continue };
                if !rect.contains(s) {
                    continue;
                }
                painter.circle(s, 3.5, Color32::from_gray(40), Stroke::new(1.2, Color32::WHITE));
                let r = Rect::from_min_size(s + Vec2::new(6.0, -8.0), Vec2::new(e.name.chars().count() as f32 * 12.0, 16.0));
                if !taken.iter().any(|t| t.intersects(r)) {
                    // dark names, or a dark theme's light ones
                    let lc = self.doc.style.label;
                    let ink = if map_core::doc::halo_for(lc) == [255, 255, 255, 255] { Color32::from_gray(25) } else { Color32::from_rgb(lc[0], lc[1], lc[2]) };
                    painter.text(s + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, &e.name, egui::FontId::proportional(12.0), ink);
                }
            }
        }
        // the scale bar and north arrow
        self.draw_furniture(painter, vg);
        let accent = Color32::from_rgb(230, 80, 20);
        let Some(h) = hover else { return };
        let Some(hs) = vg.to_screen(h) else { return };
        let tol = vg.deg_per_px(hs, 8.0);
        match self.tool {
            Tool::EditVertex => {
                // handles near the cursor
                let radius = vg.deg_per_px(hs, 160.0);
                let mut n = 0;
                for (_, e) in self.doc.topo.edges.iter() {
                    let full = self.doc.topo.edge_full(e);
                    for (i, q) in full.iter().enumerate() {
                        if geom::geo_dist_deg(*q, h) > radius {
                            continue;
                        }
                        if let Some(s) = vg.to_screen(*q) {
                            let node = i == 0 || i + 1 == full.len();
                            let c = if node { Color32::from_rgb(200, 30, 30) } else { Color32::from_rgb(30, 30, 30) };
                            painter.rect_filled(Rect::from_center_size(s, Vec2::splat(if node { 6.0 } else { 4.5 })), 0.0, c);
                            n += 1;
                        }
                    }
                    if n > 4000 {
                        break;
                    }
                }
                if let Some((_, Some((at, plan)))) = &self.dated_drag {
                    if let Some(s) = vg.to_screen(*at) {
                        self.paint_plan(painter, vg, plan, s, "松开即应用");
                    }
                }
                let target = match self.drag_vertex {
                    Some(v) => self.doc.topo.vertex_exists(v).then(|| self.doc.topo.position_of(v)),
                    None => self.doc.topo.nearest_vertex(h, tol).map(|x| x.1),
                };
                if let Some(s) = target.and_then(|t| vg.to_screen(t)) {
                    painter.circle_stroke(s, 7.0, Stroke::new(2.0, accent));
                } else if let Some((_, _, q)) = self.doc.topo.nearest_on_edge(h, tol) {
                    if let Some(s) = vg.to_screen(q) {
                        painter.circle_stroke(s, 5.0, Stroke::new(1.5, Color32::from_rgb(20, 120, 220)));
                    }
                }
            }
            _ => {}
        }
    }
}
