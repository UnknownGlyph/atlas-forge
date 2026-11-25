//! Application state, menus, file operations and frame orchestration.

use crate::actions::Act;
use crate::panels::{ExportState, Tab};
use crate::view::{Tool, ViewKind};
use eframe::egui;
use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::history::History;
use map_core::projection::Projection;
use map_core::theme::{self, ThemeResult};
use map_core::topo::FaceId;
use map_core::{EntityId, FieldId};
use map_render::camera::{Camera2D, GlobeCamera};
use map_render::gpu::MapGpu;
use map_render::tess::{LineInstance, SceneCache, UpdateStats};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Raw handles of the main window, kept for parenting native dialogs.
struct ParentWindow {
    window: raw_window_handle::RawWindowHandle,
    display: raw_window_handle::RawDisplayHandle,
}

impl ParentWindow {
    fn of(frame: &eframe::Frame) -> Option<Self> {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
        Some(Self { window: frame.window_handle().ok()?.as_raw(), display: frame.display_handle().ok()?.as_raw() })
    }
}

// The main window outlives every dialog, which is the only place these are used.
impl raw_window_handle::HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.window) })
    }
}
impl raw_window_handle::HasDisplayHandle for ParentWindow {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.display) })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Flat,
    Globe,
    Split,
}

/// An interactive edit in progress (vertex drag, brush stroke): the state
/// before it started is pushed to history when it ends.
pub struct PendingEdit {
    pub label: &'static str,
    pub before: Document,
    pub changed: bool,
}

type TestWorld = Result<(Document, Option<map_core::testworld::TestWorldReport>), String>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Selection {
    entity: Option<EntityId>,
    face: Option<FaceId>,
    merge: Option<FaceId>,
}

#[derive(Default)]
struct Highlight {
    selection: Selection,
    faces: HashSet<FaceId>,
}

impl Highlight {
    fn update(&mut self, doc: &Document, selection: Selection, ownership_changed: bool) -> bool {
        if selection == self.selection && !(selection.entity.is_some() && ownership_changed) {
            return false;
        }
        self.selection = selection;
        self.faces.clear();
        if let Some(entity) = selection.entity {
            self.faces.extend(doc.faces_of_tree(entity));
        } else {
            self.faces.extend(selection.face);
        }
        self.faces.extend(selection.merge);
        true
    }
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    #[test]
    fn reuses_selection_until_ownership_or_selection_changes() {
        let mut doc = map_core::sample::demo_world();
        let (&face, &entity) = doc.owner.iter().next().unwrap();
        let mut highlight = Highlight::default();
        let selection = Selection { entity: Some(entity), ..Selection::default() };
        assert!(highlight.update(&doc, selection, false));
        assert!(highlight.faces.contains(&face));
        assert!(!highlight.update(&doc, selection, false));
        doc.assign(face, None).unwrap();
        assert!(highlight.update(&doc, selection, true));
        assert!(!highlight.faces.contains(&face));
        let face_selection = Selection { face: Some(face), ..Selection::default() };
        assert!(highlight.update(&doc, face_selection, false));
        assert_eq!(highlight.faces, HashSet::from([face]));
        assert!(!highlight.update(&doc, face_selection, true));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct OverlayKey {
    geometry: u64,
    focus: Option<(u32, u64)>,
}

pub struct App {
    pub doc: Document,
    pub history: History,
    pub path: Option<PathBuf>,
    pub dirty: bool,

    pub scene: SceneCache,
    pub last_stats: UpdateStats,
    pub theme: ThemeResult,
    theme_src: Option<Document>,
    /// Thematic classes shared by all years and the document state they
    /// were worked out from (the year aside).
    pub fixed_scale: Option<theme::FixedScale>,
    fixed_src: Option<Document>,
    pub overlay: (u64, Arc<Vec<LineInstance>>),
    pub hover_overlay: (u64, Arc<Vec<LineInstance>>),
    hover_key: Option<(FaceId, u64)>,
    highlight: Highlight,
    /// The coast bands, and the line generation they were made from.
    pub glow: (u64, Arc<Vec<LineInstance>>),
    glow_src: Option<(u64, bool)>,
    overlay_key: Option<OverlayKey>,
    /// Boundary lines of the overlay layer and the document state they were
    /// built from (topology, layers, ownership, entities, year, style).
    layer_lines: Vec<LineInstance>,
    layer_src: Option<(Document, f64)>,
    pub labels: Vec<(EntityId, map_core::labels::LabelPath)>,
    pub(crate) label_cache: map_core::labels::LabelCache,
    pub(crate) detail_cache: crate::map_detail::DetailCache,
    pub(crate) draw_layers: std::cell::RefCell<crate::draw_layers::DrawLayers>,
    pub(crate) lifespan_cache: crate::lifespan_ui::LifespanCache,

    pub tool: Tool,
    pub draft: Vec<LonLat>,
    /// Per draft point: snapped onto an existing boundary.
    pub draft_snap: Vec<bool>,
    pub draft_drag: Option<crate::draw_ui::DraftDrag>,
    /// Trace along existing boundaries between snapped draft points.
    pub trace: bool,
    /// Last traced preview: (from, to) and the path.
    pub trace_preview: Option<((LonLat, LonLat), Option<Vec<LonLat>>)>,
    /// A drawn shape being applied in the background.
    pub bg_edit: Option<crate::draw_ui::BgEdit>,
    /// Line style for drawn shapes, and its roughness (0-1).
    pub line_style: map_core::shape::LineStyle,
    pub roughness: f32,
    /// Styled draft for the current draft, style and scale.
    pub styled_cache: Option<(crate::draw_ui::StyleKey, Vec<LonLat>)>,
    /// Styled lines keep the area they enclose (drawn shapes, reshaped borders).
    pub keep_area: bool,
    /// Land and lake tools paint with a round brush (radius in screen px)
    /// instead of placing points; the stroke being painted, on screen.
    pub brush_paint: bool,
    /// The river tool draws routes (styled lines) instead of rivers.
    pub route_mode: bool,
    pub route_style: map_core::doc::RouteStyle,
    pub brush_px: f32,
    pub brush_color_mode: bool,
    pub brush_color: egui::Color32,
    pub brush_stroke: Vec<egui::Pos2>,
    /// The selected river, a river point being dragged, and the name being typed.
    pub selected_river: Option<map_core::doc::LineId>,
    /// A city being dragged with the city tool.
    pub drag_city: Option<EntityId>,
    /// Where each country label was drawn last frame (for the label tool),
    /// and a label being dragged with its offset from the pointer.
    /// Indexed by view (flat, globe): both draw labels in split view.
    pub label_rects: std::cell::RefCell<[Vec<(EntityId, egui::Rect)>; 2]>,
    /// The glyph atlas's size when the country names were last drawn.
    pub label_atlas: std::cell::Cell<[usize; 2]>,
    /// The local view being looked at (`focus_ui`), what it shows, when
    /// its playing last stepped, and its name being typed.
    pub focus: Option<u32>,
    pub focus_view: Option<crate::focus_ui::FocusView>,
    pub focus_play: Option<f64>,
    pub(crate) playback: crate::playback_ui::TimelinePlayback,
    pub(crate) study_year_cache: std::cell::RefCell<Option<crate::timeline_ui::StudyYearCache>>,
    pub(crate) studio: crate::studio_ui::Studio,
    pub(crate) replay: crate::replay_ui::Replay,
    pub focus_name: Option<String>,
    /// The territory brush fills whole connected patches.
    pub assign_fill: bool,
    /// The "select by attribute" window and its query, while open.
    pub query: Option<map_core::features::FeatureQuery>,
    /// Smart snapping while moving features, and what the current move snapped to.
    pub smart_snap: bool,
    pub snap_guides: Vec<map_core::snap::Guide>,
    /// The name generator: open, its style and kind, and the batch shown.
    pub names_open: bool,
    /// Roads and travel (its window and plan).
    pub roads: crate::roads_ui::Roads,
    /// The AI assistant (its window, task and draft).
    pub ai: crate::ai_ui::Assistant,
    pub(crate) land_move: crate::land_move_ui::LandMove,
    pub(crate) guide_draft: Option<Vec<map_core::guides::Guide>>,
    pub plugins: crate::plugin_ui::PluginUi,
    pub book_ui: crate::book_ui::BookUi,
    pub(crate) worldview_ui: crate::worldview_ui::WorldviewUi,
    pub(crate) environment_ui: crate::environment_ui::EnvironmentUi,
    pub(crate) visualization_ui: crate::visualization_ui::VisualizationUi,
    pub(crate) model_viewer: Option<crate::model_viewer::Viewer>,
    pub climate_ui: crate::climate_ui::ClimateUi,
    /// The layout view (the export page with its furniture to drag).
    pub page: crate::pageview_ui::PageView,
    pub name_style: map_core::names::NameStyle,
    pub name_kind: map_core::names::NameKind,
    pub name_seed: u64,
    /// The event timeline window, the event picked, and its text being typed.
    pub events_open: bool,
    /// The new-world wizard.
    pub wizard: crate::wizard_ui::Wizard,
    /// The automatic partition of a country, and its plan.
    pub partition: crate::partition_ui::Partition,
    pub event_sel: Option<u32>,
    pub event_text: Option<(u32, String, String)>,
    pub drag_label: Option<(EntityId, egui::Vec2)>,
    /// Free text: the selected one, one being dragged, where each was drawn
    /// (per view), the text being typed and whether to focus its field.
    pub selected_ann: Option<map_core::doc::LineId>,
    pub drag_ann: Option<(map_core::doc::LineId, egui::Vec2)>,
    pub ann_rects: std::cell::RefCell<[Vec<(map_core::doc::LineId, egui::Rect)>; 2]>,
    pub ann_text: Option<(map_core::doc::LineId, String)>,
    pub ann_focus: bool,
    /// A curve being drawn for a piece of map text to follow.
    pub ann_path: Option<(map_core::doc::LineId, Vec<map_core::geom::LonLat>)>,
    /// Symbol tool: the kind new symbols get, the selected one, the one
    /// being dragged (with the grab offset), where each is on screen per
    /// view, and the name being typed.
    pub symbol_kind: map_core::symbol::SymbolKind,
    pub(crate) pixel_tools: crate::pixel_ui::PixelTools,
    pub selected_sym: Option<map_core::doc::LineId>,
    pub drag_sym: Option<(map_core::doc::LineId, egui::Vec2)>,
    pub sym_rects: std::cell::RefCell<[Vec<(map_core::doc::LineId, egui::Rect)>; 2]>,
    pub sym_name: Option<(map_core::doc::LineId, String)>,
    /// The character and the legend text being typed for the selected symbol.
    /// (symbol, text, being typed)
    pub sym_glyph: Option<(map_core::doc::LineId, String, bool)>,
    pub sym_legend: Option<(map_core::doc::LineId, String)>,
    /// The selected hatched area and the name being typed for it.
    pub selected_area: Option<map_core::doc::LineId>,
    pub area_name: Option<(map_core::doc::LineId, String)>,
    /// The area outline point being dragged with the vertex tool.
    pub drag_area_pt: Option<(map_core::doc::LineId, usize)>,
    /// The legend text being typed for the selected route, and for the
    /// selected area (each its own, so neither panel resets the other).
    pub route_legend: Option<(map_core::doc::LineId, String)>,
    /// A selected river, route or area being dragged whole.
    pub move_drag: Option<crate::select_ui::MoveDrag>,
    /// The drawn features picked with the select tool (see `select_ui`).
    pub picked: std::collections::BTreeSet<map_core::features::FeatureRef>,
    /// The selection box being dragged: the view it is in, and its
    /// corners in screen points.
    pub marquee: Option<(usize, egui::Pos2, egui::Pos2)>,
    /// What was last copied (the system clipboard holds it as text too).
    pub clip: Option<map_core::arrange::Clip>,
    /// The selection panel's angle (degrees) and scale (percent).
    pub xf_rotate: f64,
    pub xf_scale: f64,
    /// A copy (false) or cut (true) asked for from a menu, done where the clipboard is at hand.
    pub pending_copy: Option<bool>,
    /// The modifier keys when the pointer button last went down (a quick
    /// Shift-click may release both in one frame).
    pub press_mods: egui::Modifiers,
    /// The measuring tool's points, and whether the shape is closed.
    pub measure_pts: Vec<map_core::geom::LonLat>,
    pub measure_closed: bool,
    /// The 要素 tab's filter.
    pub feature_filter: String,
    pub(crate) feature_catalog: map_core::features::FeatureCatalog,
    pub area_legend: Option<(map_core::doc::LineId, String)>,
    /// The other area chosen for combining with the selected one.
    pub area_partner: Option<map_core::doc::LineId>,
    pub drag_river: Option<(map_core::doc::LineId, usize)>,
    pub river_name: Option<(map_core::doc::LineId, String)>,
    /// With a year selected, vertex drags are dated territory changes.
    pub vertex_dated: bool,
    /// A dated vertex drag in progress: where from, and its plan.
    pub dated_drag: Option<(map_core::redraw::DragFrom, Option<(LonLat, Result<map_core::redraw::Plan, String>)>)>,
    /// Redraw-border preview for a candidate line (draft + snapped cursor).
    pub redraw_preview: Option<(Vec<LonLat>, Result<map_core::redraw::Plan, String>)>,
    /// A border reshape waiting for the user to apply or cancel it.
    pub reshape: Option<crate::reshape_ui::Reshape>,
    pub selected: Option<EntityId>,
    pub selected_face: Option<FaceId>,
    pub hover_face: Option<FaceId>,
    pub merge_first: Option<FaceId>,
    pub brush: Option<EntityId>,
    pub pending: Option<PendingEdit>,
    /// Direct (non-`edit`) changes to settings, coalesced into one undo step.
    settings_pending: Option<Document>,
    pub drag_vertex: Option<map_core::VertexRef>,
    pub cursor: Option<LonLat>,

    pub layout: Layout,
    pub cam2d: Camera2D,
    pub globe: GlobeCamera,
    pub globe_inertia: crate::view::GlobeInertia,
    pub star_map: crate::star_map::StarMap,
    pub about_open: bool,
    pub(crate) tutorial: crate::tutorial_ui::Tutorial,
    pub(crate) diplomacy_ui: crate::diplomacy_ui::DiplomacyUi,
    pub fit_pending: bool,
    /// The user has panned/zoomed the 2D view since the last fit.
    pub nav_touched: bool,
    pub last_draft_view: Option<crate::view::ViewGeom>,
    pub graticule: bool,
    /// The scale bar and north arrow on the views.
    pub furniture: bool,

    pub tab: Tab,
    pub export: ExportState,
    pub cell_edit: HashMap<(EntityId, FieldId), String>,
    pub text_edit: HashMap<(EntityId, u8), String>,
    pub layer_name_edit: Option<(map_core::doc::LayerId, String)>,
    pub new_field: (String, String, map_core::FieldKind, String),
    pub status: String,
    pub status_err: bool,
    pub confirm_close: bool,
    /// The action waiting for an answer about unsaved changes.
    pub confirm_replace: Option<crate::actions::Act>,
    /// The command palette, while open.
    pub palette: Option<crate::actions::Palette>,
    pub shortcuts_open: bool,
    allow_close: bool,

    pub font: Option<Arc<Vec<u8>>>,
    pub table_panel: bool,
    pub draw_mode: crate::view::DrawMode,
    pub ul: crate::underlay_ui::UnderlayState,
    /// Year the timeline returns to when switched back on.
    pub timeline_last_year: i32,
    pub batch: Option<crate::panels::BatchRun>,
    /// Share the brush gives its unit on a share layer (percent).
    pub share_pct: f32,
    /// Data table: areas per year, kept while the map is unchanged.
    pub stats_cache: Option<(Document, std::collections::HashMap<EntityId, f64>, std::collections::HashMap<i32, std::collections::HashMap<EntityId, f64>>)>,
    /// Shared-border import dialog: (repair mode, snap distance in metres).
    import_dialog: Option<(bool, f64)>,
    /// Text of the last import report window.
    import_report: Option<Vec<String>>,
    parent: Option<ParentWindow>,
    pub autosave: crate::autosave::Autosave,
    pub checkpoints: crate::checkpoint_ui::Panel,
    /// The large test world being made in the background.
    test_world: Option<std::thread::JoinHandle<TestWorld>>,
    pub last_flat_rect: egui::Rect,
    pub(crate) frame_ms: f32,
    pub(crate) ui_ms: f32,
    /// What is remembered between runs, as last stored, and where.
    pub prefs: map_io::prefs::Prefs,
    pub prefs_applied: map_io::prefs::Prefs,
    pub(crate) prefs_writer: crate::prefs_writer::PrefsWriter,
    pub prefs_path: PathBuf,
    pub prefs_open: bool,
    pub welcome_open: bool,
    /// The history panel, and the history generation it last showed.
    pub history_open: bool,
    pub history_seen: u64,
    /// The primary button went down with Space held: this press pans the
    /// map whatever the tool.
    pub space_pan: bool,
    /// The layer of drawn features in hand: what is drawn or pasted goes
    /// into it (see `layers_ui`).
    pub active_group: Option<map_core::stack::GroupId>,
    /// A layer being renamed: which, the text, and whether to focus the field.
    pub group_rename: Option<(map_core::stack::GroupId, String, bool)>,
    /// The feature list shows only the layer in hand.
    pub layer_only: bool,
    /// The pen's anchors so far, whether its last anchor's handles are
    /// being pulled out, and what it draws (see `curve_ui`).
    pub pen: Vec<map_core::curve::CurveNode>,
    pub pen_pull: bool,
    pub pen_target: crate::curve_ui::PenTarget,
    /// An anchor or handle being dragged, and the curve last worked on.
    pub curve_drag: Option<crate::curve_ui::CurveDrag>,
    pub curve_focus: Option<map_core::doc::LineId>,
    /// The style each kind of new feature is drawn in, a look copied for
    /// 粘贴样式 (with the style it followed), the style open in the
    /// library, one being renamed, and the colour about to join the palette
    /// (see `styles_ui`).
    pub draw_style: std::collections::BTreeMap<map_core::styles::StyleKind, map_core::styles::StyleId>,
    pub style_clip: Option<(map_core::styles::Look, Option<map_core::styles::StyleId>)>,
    pub style_open: Option<map_core::styles::StyleId>,
    pub style_rename: Option<(map_core::styles::StyleId, String, bool)>,
    pub new_swatch: [u8; 4],
    /// Fonts the map uses that egui has (by `fonts_ui::font_key`), and
    /// those this computer does not have.
    pub fonts_loaded: std::collections::BTreeMap<String, Arc<egui::FontData>>,
    pub(crate) interface_font: Option<Arc<egui::FontData>>,
    pub(crate) font_locale: crate::fonts_ui::LocaleFont,
    pub fonts_missing: std::collections::BTreeSet<String>,
    /// The window title last sent (sending one asks for a repaint, so it
    /// is only sent when it changes; the window then rests while idle).
    title: String,
    /// With ATLAS_PROFILE set: smoothed ms per part of the frame (derived
    /// data, panels, map views, the rest), shown in the status bar.
    pub(crate) prof: Option<[f32; 4]>,
    pub(crate) performance: crate::perf_ui::Performance,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, font: Option<Arc<Vec<u8>>>, open: Option<PathBuf>) -> Self {
        let performance = crate::perf_ui::Performance::new(cc);
        let _startup = crate::perf_ui::span("启动", "初始化编辑器");
        let max_tex = cc.wgpu_render_state.as_ref().map_or(8192, |rs| rs.device.limits().max_texture_dimension_2d);
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let gpu = MapGpu::new(&rs.device, rs.target_format, crate::MSAA);
            rs.renderer.write().callback_resources.insert(gpu);
        }
        let prefs_path = map_io::prefs::default_path();
        let prefs = map_io::prefs::load(&prefs_path);
        let language = crate::i18n::Lang::from_code(&prefs.language);
        crate::i18n::set(language);
        let font_locale = crate::fonts_ui::LocaleFont::for_language(language);
        let interface_font = font.as_ref().map(|bytes| Arc::new(egui::FontData::from_owned((**bytes).clone())));
        cc.egui_ctx.set_fonts(crate::fonts_ui::definitions(interface_font.as_ref(), font_locale));
        cc.egui_ctx.set_zoom_factor(prefs.ui_scale);
        // Ctrl + plus and minus zoom the map; the interface's size is a preference
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = Self {
            doc: map_core::sample::demo_world(),
            history: History::default(),
            path: None,
            dirty: false,
            scene: SceneCache::default(),
            last_stats: UpdateStats::default(),
            theme: ThemeResult::default(),
            theme_src: None,
            fixed_scale: None,
            fixed_src: None,
            overlay: (0, Arc::new(vec![])),
            hover_overlay: (0, Arc::new(vec![])),
            hover_key: None,
            highlight: Highlight::default(),
            glow: (0, Arc::new(vec![])),
            glow_src: None,
            overlay_key: None,
            layer_lines: vec![],
            layer_src: None,
            labels: vec![],
            label_cache: Default::default(),
            detail_cache: Default::default(),
            draw_layers: Default::default(),
            lifespan_cache: Default::default(),
            tool: Tool::Select,
            draft: vec![],
            draft_snap: vec![],
            draft_drag: None,
            trace: true,
            trace_preview: None,
            bg_edit: None,
            line_style: map_core::shape::LineStyle::Plain,
            roughness: 0.5,
            styled_cache: None,
            keep_area: true,
            brush_paint: false,
            route_mode: false,
            route_style: Default::default(),
            brush_px: 18.0,
            brush_color_mode: false,
            brush_color: egui::Color32::from_rgba_unmultiplied(165, 79, 35, 220),
            brush_stroke: Vec::new(),
            selected_river: None,
            drag_city: None,
            label_rects: Default::default(),
            label_atlas: Default::default(),
            focus: None,
            focus_view: None,
            focus_play: None,
            playback: Default::default(),
            study_year_cache: Default::default(),
            studio: Default::default(),
            replay: Default::default(),
            focus_name: None,
            assign_fill: false,
            query: None,
            smart_snap: true,
            snap_guides: Vec::new(),
            names_open: false,
            roads: Default::default(),
            ai: Default::default(),
            land_move: Default::default(),
            guide_draft: None,
            plugins: Default::default(),
            book_ui: Default::default(),
            worldview_ui: Default::default(),
            environment_ui: Default::default(),
            visualization_ui: Default::default(),
            model_viewer: None,
            climate_ui: Default::default(),
            page: Default::default(),
            events_open: false,
            wizard: Default::default(),
            partition: Default::default(),
            event_sel: None,
            event_text: None,
            name_style: map_core::names::NameStyle::Huaxia,
            name_kind: map_core::names::NameKind::Country,
            name_seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs()),
            drag_label: None,
            selected_ann: None,
            drag_ann: None,
            ann_rects: Default::default(),
            ann_text: None,
            ann_focus: false,
            ann_path: None,
            symbol_kind: map_core::symbol::SymbolKind::Peak,
            pixel_tools: Default::default(),
            selected_sym: None,
            drag_sym: None,
            sym_rects: Default::default(),
            sym_name: None,
            sym_glyph: None,
            sym_legend: None,
            selected_area: None,
            area_name: None,
            drag_area_pt: None,
            route_legend: None,
            picked: Default::default(),
            marquee: None,
            clip: None,
            xf_rotate: 15.0,
            xf_scale: 100.0,
            pending_copy: None,
            press_mods: egui::Modifiers::NONE,
            move_drag: None,
            measure_pts: Vec::new(),
            measure_closed: false,
            feature_filter: String::new(),
            feature_catalog: Default::default(),
            area_legend: None,
            area_partner: None,
            drag_river: None,
            river_name: None,
            vertex_dated: true,
            dated_drag: None,
            redraw_preview: None,
            reshape: None,
            selected: None,
            selected_face: None,
            hover_face: None,
            merge_first: None,
            brush: None,
            pending: None,
            settings_pending: None,
            drag_vertex: None,
            cursor: None,
            layout: crate::shell_ui::layout_named(&prefs.layout),
            cam2d: Camera2D::new(Projection::EqualEarth),
            globe: GlobeCamera::default(),
            globe_inertia: Default::default(),
            star_map: Default::default(),
            about_open: false,
            tutorial: Default::default(),
            diplomacy_ui: Default::default(),
            fit_pending: true,
            nav_touched: false,
            last_draft_view: None,
            graticule: prefs.graticule,
            furniture: prefs.furniture,
            tab: Tab::Countries,
            export: ExportState::default(),
            cell_edit: HashMap::new(),
            text_edit: HashMap::new(),
            layer_name_edit: None,
            new_field: (String::new(), String::new(), map_core::FieldKind::Number, String::new()),
            status: "欢迎使用 Atlas Forge。已载入示例世界，可在“文件”菜单新建空白世界。".into(),
            status_err: false,
            confirm_close: false,
            confirm_replace: None,
            palette: None,
            shortcuts_open: false,
            allow_close: false,
            font,
            table_panel: false,
            draw_mode: crate::view::DrawMode::AddLand,
            ul: crate::underlay_ui::UnderlayState::new(max_tex),
            timeline_last_year: 1900,
            batch: None,
            share_pct: 100.0,
            stats_cache: None,
            import_dialog: None,
            import_report: None,
            parent: None,
            autosave: crate::autosave::Autosave::start(&prefs),
            checkpoints: Default::default(),
            test_world: None,
            last_flat_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0)),
            frame_ms: 0.0,
            ui_ms: 0.0,
            title: String::new(),
            prof: std::env::var_os("ATLAS_PROFILE").map(|_| [0.0; 4]),
            performance,
            prefs_applied: prefs.clone(),
            prefs_writer: Default::default(),
            prefs,
            prefs_path,
            prefs_open: false,
            welcome_open: false,
            history_open: false,
            history_seen: 0,
            space_pan: false,
            active_group: None,
            group_rename: None,
            layer_only: false,
            pen: Vec::new(),
            pen_pull: false,
            pen_target: crate::curve_ui::PenTarget::River,
            curve_drag: None,
            curve_focus: None,
            draw_style: Default::default(),
            style_clip: None,
            style_open: None,
            style_rename: None,
            new_swatch: [180, 60, 40, 255],
            fonts_loaded: Default::default(),
            interface_font,
            font_locale,
            fonts_missing: Default::default(),
        };
        app.history.limit = app.prefs.undo_limit;
        let _ = app.autosave.configure(&app.prefs);
        // a file given on the command line, else the last one if asked for,
        // else the sample world behind the start window
        let last = app.prefs.recent.first().filter(|p| app.prefs.reopen_last && p.exists()).cloned();
        match open.or(last) {
            Some(p) => app.open_path(p),
            // work to recover comes first
            None => app.welcome_open = app.prefs.welcome && app.autosave.offers.is_empty(),
        }
        app
    }

    /// Native file dialog parented to the main window, so it is modal and
    /// cannot open behind it.
    pub fn file_dialog(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(directory) = self.prefs.save_directory.as_ref().filter(|dir| dir.is_dir()) {
            d = d.set_directory(directory);
        }
        match &self.parent {
            Some(p) => d.set_parent(p),
            None => d,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>, err: bool) {
        self.status = crate::i18n::message(msg.into());
        self.status_err = err;
    }

    /// Run an undoable edit and report the outcome in the status bar.
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document) -> Result<T, String>) -> Option<T> {
        let _measurement = crate::perf_ui::span("编辑", "工程修改与撤销记录");
        // with a layer in hand, what the edit adds goes into it
        let had = self.active_group.map(|_| self.doc.feature_ids());
        match self.history.apply(&mut self.doc, label, f) {
            Ok(v) => {
                if let Some(had) = &had {
                    self.adopt_new_features(had);
                }
                self.dirty = true;
                self.set_status(crate::trf!("{label}：完成"), false);
                Some(v)
            }
            Err(e) => {
                self.set_status(crate::trf!("{label}失败：{e}"), true);
                None
            }
        }
    }

    pub fn begin_pending(&mut self, label: &'static str) {
        self.pending = Some(PendingEdit { label, before: self.doc.clone(), changed: false });
    }

    pub fn end_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            if p.changed {
                if self.active_group.is_some() {
                    let had = p.before.feature_ids();
                    self.adopt_new_features(&had);
                }
                self.history.push(p.label, p.before, &self.doc);
                self.dirty = true;
            }
        }
    }

    pub(crate) fn finish_pending_settings(&mut self) {
        if let Some(before) = self.settings_pending.take() {
            self.history.push("修改样式/设置", before, &self.doc);
        }
    }

    fn reset_view_state(&mut self) {
        self.pixel_tools.close_target();
        self.feature_catalog = Default::default();
        self.history.clear();
        self.book_ui=Default::default();
        self.worldview_ui=Default::default();
        self.reset_board_view();
    }

    pub(crate) fn apply_home_view(&mut self) {
        let home = self.doc.home_view.filter(|view| view.valid() && self.doc.paper.is_none());
        let projection = if self.doc.paper.is_some() {
            Projection::Equirectangular
        } else {
            home.map_or(Projection::EqualEarth, |view| view.projection)
        };
        self.cam2d = Camera2D::new(projection);
        self.globe = GlobeCamera::default();
        if let Some(view) = home {
            self.cam2d.lon0 = view.central_meridian;
            self.globe.center_lon = view.central_meridian;
            self.globe.center_lat = view.globe_latitude;
        }
        self.globe_inertia.stop();
        self.fit_pending = true;
        self.nav_touched = false;
    }

    pub(crate) fn reset_board_view(&mut self) {
        self.stop_timeline_playback();
        self.draw_layers = Default::default();
        self.lifespan_cache = Default::default();
        self.reset_studio_navigation();
        self.replay = Default::default();
        self.diplomacy_ui = Default::default();
        self.tab = if self.doc.paper.is_some() { crate::panels::Tab::Features } else { crate::panels::Tab::Countries };
        self.globe_inertia.stop();
        self.star_map = Default::default();
        self.climate_ui = Default::default();
        self.environment_ui.leave_board();
        self.visualization_ui = Default::default();
        self.ai.document_changed();
        self.land_move = Default::default();
        self.guide_draft = None;
        self.plugins = Default::default();
        self.bg_edit = None;
        self.event_sel = None;
        self.event_text = None;
        self.selected_river = None;
        self.selected_area = None;
        self.selected_sym = None;
        self.selected_ann = None;
        self.hover_face = None;
        self.draw_style.clear();
        self.style_open = None;
        self.style_rename = None;
        self.curve_focus = None;
        self.curve_drag = None;
        self.active_group = None;
        self.group_rename = None;
        self.picked.clear();
        self.marquee = None;
        self.clear_draft();
        self.selected = None;
        self.selected_face = None;
        self.merge_first = None;
        self.brush = None;
        self.pending = None;
        self.settings_pending = None;
        self.ul = crate::underlay_ui::UnderlayState::new(self.ul.max_dim);
        self.cell_edit.clear();
        self.text_edit.clear();
        self.layer_name_edit = None;
        self.partition = Default::default();
        self.draw_mode = crate::view::DrawMode::AddLand;
        self.exit_focus();
        self.apply_home_view();
    }

    /// Open the test world once it is made.
    fn poll_test_world(&mut self, ctx: &egui::Context) {
        if !self.test_world.as_ref().is_some_and(|h| h.is_finished()) {
            if self.test_world.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            return;
        }
        match self.test_world.take().unwrap().join() {
            Ok(Ok((doc, Some(r)))) => {
                self.new_doc(doc);
                self.set_status(
                    crate::trf!(
                        "测试世界：{} 个面（起初 {}），{}{} 次变化（{} 次局部重建），建国 {}、终止 {}，用时 {:.1} s",
                        r.faces_end,
                        r.faces_start,
                        r.refine.as_ref().map(|x| format!("{} 个顶点，", x.vertices_after)).unwrap_or_default(),
                        r.events_applied,
                        r.local,
                        r.founded,
                        r.ended,
                        r.total_ms / 1e3
                    ),
                    false,
                );
            }
            Ok(Ok((doc, None))) => {
                self.new_doc(doc);
                self.set_status(crate::i18n::tr("已将伊瑟尔三国工程载入为测试副本。"), false);
            }
            Ok(Err(error)) => self.set_status(error, true),
            Err(_) => self.set_status(crate::i18n::tr("生成测试世界失败"), true),
        }
    }

    pub fn new_doc(&mut self, doc: Document) {
        self.doc = doc;
        self.path = None;
        self.dirty = false;
        self.reset_view_state();
        self.autosave.open_project(None,self.history.generation());
    }

    pub fn open_path(&mut self, p: PathBuf) {
        let _measurement = crate::perf_ui::span("工程", "载入工程与重置视图");
        let t = Instant::now();
        match map_io::project::load(&p) {
            Ok(doc) => {
                self.doc = doc;
                self.dirty = false;
                self.reset_view_state();
                self.autosave.open_project(Some(&p),self.history.generation());
                self.set_status(crate::trf!("已打开 {}（{:.0} ms）", p.display(), t.elapsed().as_secs_f64() * 1e3), false);
                self.note_recent(&p);
                self.path = Some(p);
            }
            Err(e) => {
                if !p.exists() {
                    self.prefs.forget_recent(&p);
                }
                self.set_status(crate::trf!("打开失败：{e}"), true)
            }
        }
    }

    pub fn open_dialog(&mut self) {
        if let Some(p) = self.file_dialog().add_filter(crate::i18n::tr("Atlas 工程"), &["atlas"]).pick_file() {
            self.open_path(p);
        }
    }

    pub fn save(&mut self, save_as: bool) -> bool {
        self.end_pending();
        self.finish_pending_settings();
        let path = match (&self.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = format!("{}.atlas", self.doc.meta.name);
                match self.file_dialog().add_filter(crate::i18n::tr("Atlas 工程"), &["atlas"]).set_file_name(name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let t = Instant::now();
        let _measurement = crate::perf_ui::span("工程", "写入工程与备份");
        // the file being replaced kept as .bak1 (and two older ones)
        match map_io::project::save_snapshot(&path, &self.doc, 3) {
            Ok(bytes) => {
                let size = bytes.len();
                self.set_status(crate::trf!("已保存 {}（{:.1} KB，{:.0} ms）", path.display(), size as f64 / 1024.0, t.elapsed().as_secs_f64() * 1e3), false);
                self.note_recent(&path);
                if let Err(e) = self.autosave.saved_project(&path, self.history.generation(), bytes) {
                    self.set_status(crate::trf!("已保存工程，保存点迁移仍待重试：{e}"),true);
                }
                self.path = Some(path);
                self.dirty = false;
                true
            }
            Err(e) => {
                self.set_status(crate::trf!("保存失败：{e}"), true);
                false
            }
        }
    }

    pub fn undo(&mut self) {
        self.end_pending();
        let viewing = self.doc.year;
        let board=map_core::world_book::WorldBook::current(&self.doc);
        if let Some(l) = self.history.undo(&mut self.doc) {
            self.visualization_ui.reload(&self.doc);
            self.environment_ui.reload();
            self.worldview_ui.reload(&self.doc);
            self.dirty = true;
            let note = self.keep_book_history_view(board,viewing);
            self.after_history_jump();
            self.set_status(crate::trf!("撤销：{l}{note}"), false);
        }
    }
    pub fn redo(&mut self) {
        let viewing = self.doc.year;
        let board=map_core::world_book::WorldBook::current(&self.doc);
        if let Some(l) = self.history.redo(&mut self.doc) {
            self.visualization_ui.reload(&self.doc);
            self.environment_ui.reload();
            self.worldview_ui.reload(&self.doc);
            self.dirty = true;
            let note = self.keep_book_history_view(board,viewing);
            self.after_history_jump();
            self.set_status(crate::trf!("重做：{l}{note}"), false);
        }
    }

    pub(crate) fn keep_book_history_view(&mut self,board:u64,viewing:Option<i32>)->String{
        use map_core::world_book::WorldBook;
        if WorldBook::current(&self.doc)==board {return self.keep_viewed_year(viewing);}
        let _=WorldBook::switch(&mut self.doc,board,Some(viewing));
        self.restore_book_view(WorldBook::current(&self.doc));
        String::new()
    }

    /// Undo/redo restore a snapshot taken in the year the edit was made;
    /// stay on the year being viewed and say which year the edit was in.
    pub(crate) fn keep_viewed_year(&mut self, viewing: Option<i32>) -> String {
        let edited = self.doc.year;
        if edited == viewing {
            return String::new();
        }
        self.doc.set_year(viewing);
        match edited {
            Some(y) => format!("（该修改在 {y} 年，可在时间轴跳转查看）"),
            None => "（该修改针对所有年份）".into(),
        }
    }
    pub(crate) fn after_history_jump(&mut self) {
        self.cell_edit.clear();
        self.text_edit.clear();
        self.layer_name_edit = None;
        if self.selected.is_some_and(|e| !self.doc.entities.contains_key(&e)) {
            self.selected = None;
        }
        if self.selected_face.is_some_and(|f| !self.doc.topo.faces.contains_key(&f)) {
            self.selected_face = None;
        }
    }

    /// Work the shared thematic classes out again when anything but the
    /// year changed (they cover every year, so a new year changes nothing).
    fn refresh_fixed_scale(&mut self) {
        let d = &self.doc;
        let wanted = d.theme.mode == theme::ThemeMode::Choropleth && d.theme.scale == theme::ClassScale::AllYears;
        if !wanted {
            self.fixed_scale = None;
            self.fixed_src = None;
            return;
        }
        let stale = self.fixed_src.as_ref().is_none_or(|s| {
            !(s.owner_base.ptr_eq(&d.owner_base)
                && s.owner_changes.ptr_eq(&d.owner_changes)
                && s.kind_changes.ptr_eq(&d.kind_changes)
                && s.entities.ptr_eq(&d.entities)
                && s.values.ptr_eq(&d.values)
                && s.fields.ptr_eq(&d.fields)
                && s.layers.ptr_eq(&d.layers)
                && s.active_layer == d.active_layer
                && s.topo.edges.ptr_eq(&d.topo.edges)
                && s.theme == d.theme)
        });
        if stale {
            self.fixed_scale = theme::fixed_scale(d);
            self.fixed_src = Some(d.clone());
        }
    }

    /// Derived data (meshes, colours, overlays, labels) — each part is only
    /// rebuilt when its inputs changed.
    pub(crate) fn refresh_derived(&mut self) {
        let d = &self.doc;
        let ownership_changed = self.theme_src.as_ref().is_none_or(|s| {
            !s.owner.ptr_eq(&d.owner) || !s.entities.ptr_eq(&d.entities)
        });
        let theme_stale = ownership_changed || self.theme_src.as_ref().is_some_and(|s| {
            !(s.values.ptr_eq(&d.values)
                && s.fields.ptr_eq(&d.fields)
                && s.topo.faces.ptr_eq(&d.topo.faces)
                && s.layers.ptr_eq(&d.layers)
                && s.active_layer == d.active_layer
                && s.theme == d.theme
                && s.style == d.style
                && map_core::visualization::same_map_inputs(s, d)
                && s.year == d.year)
        });
        self.last_stats = self.scene.update_geometry(&self.doc, self.cam2d.lon0);
        let exists = |face: &FaceId| d.topo.faces.contains_key(face);
        let selection = Selection {
            entity: self.selected,
            face: self.selected_face.filter(exists),
            merge: self.merge_first.filter(exists),
        };
        let selection_changed = self.highlight.update(d, selection, ownership_changed);
        if theme_stale {
            self.refresh_fixed_scale();
            self.theme = theme::evaluate_with(&self.doc, self.fixed_scale.as_ref());
            self.theme_src = Some(self.doc.clone());
        }
        let lon0 = self.cam2d.lon0;
        // a local view: what it fades and outlines
        self.update_focus_view();
        let layer_stale = self.layer_src.as_ref().is_none_or(|(s, l0)| {
            let d = &self.doc;
            !(s.topo.edges.ptr_eq(&d.topo.edges)
                && s.topo.nodes.ptr_eq(&d.topo.nodes)
                && s.topo.faces.ptr_eq(&d.topo.faces)
                && s.layers.ptr_eq(&d.layers)
                && s.owner.ptr_eq(&d.owner)
                && s.entities.ptr_eq(&d.entities)
                && s.year == d.year
                && s.active_layer == d.active_layer
                && s.style.overlay_layer == d.style.overlay_layer
                && *l0 == lon0)
        });
        if layer_stale {
            self.layer_lines = match self.doc.overlay_layer() {
                Some(l) => map_render::tess::edge_lines(&self.doc, &self.doc.layer_border_edges(l), lon0, map_render::tess::class::LAYER_OVERLAY),
                None => vec![],
            };
            self.layer_src = Some((self.doc.clone(), lon0));
        }
        let key = OverlayKey {
            geometry: self.scene.geometry_gen,
            focus: self.focus_view.as_ref().zip(self.focus).map(|(view, id)| (id, view.gen)),
        };
        let focus_changed = self.overlay_key.is_none_or(|previous| previous.focus != key.focus);
        let highlight = &self.highlight.faces;
        if theme_stale || self.last_stats.fill_rebuilt || selection_changed || focus_changed {
            let faded = self.focus_colors(&self.theme.face_colors);
            self.scene.update_colors(self.doc.style.ocean, faded.as_ref().unwrap_or(&self.theme.face_colors), highlight);
        }
        if layer_stale || selection_changed || self.overlay_key != Some(key) {
            // the other layer's boundaries go under the selection outline
            let mut lines = self.layer_lines.clone();
            // so does the land a local view's years changed
            if let Some(v) = &self.focus_view {
                lines.extend(map_render::tess::outline_lines(&self.doc, &v.changed, lon0, map_render::tess::class::FOCUS));
            }
            lines.extend(map_render::tess::outline_lines(&self.doc, highlight, lon0, map_render::tess::class::SELECT));
            self.overlay = (self.overlay.0 + 1, Arc::new(lines));
            self.overlay_key = Some(key);
        }
        let hover_key = self.hover_face
            .filter(|face| !highlight.contains(face) && self.doc.topo.faces.contains_key(face))
            .map(|face| (face, key.geometry));
        if hover_key != self.hover_key {
            let lines = hover_key.map_or_else(Vec::new, |(face, _)| {
                map_render::tess::outline_lines(&self.doc, &HashSet::from([face]), lon0, map_render::tess::class::HOVER)
            });
            self.hover_overlay = (self.hover_overlay.0 + 1, Arc::new(lines));
            self.hover_key = hover_key;
        }
        self.ul.refresh_terrain(&self.doc);
        // the coast bands of a map theme, from the coast lines
        let glow_key = (self.scene.line_gen, self.doc.style.coast_glow_width > 0.0);
        if self.glow_src != Some(glow_key) {
            let v = if glow_key.1 { map_render::tess::glow_lines(&self.scene.chunks().lines) } else { vec![] };
            self.glow = (self.glow.0 + 1, Arc::new(v));
            self.glow_src = Some(glow_key);
        }
        // label anchors: only countries touched since the last update are
        // re-dissolved, so this is cheap enough to follow a drag live
        if self.label_cache.update(&self.doc) > 0 {
            self.labels = self.label_cache.sorted();
        }
        self.detail_cache.update(&self.doc);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        // everything that is an action (see `actions`)
        self.action_shortcuts(ctx);
        if self.star_map.open {
            if !ctx.egui_wants_keyboard_input() && self.palette.is_none() && ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.star_map.open = false;
            }
            return;
        }
        self.clipboard_events(ctx);
        if let Some(cut) = self.pending_copy.take() {
            self.copy_selection(ctx, cut);
        }
        // the keys of a drawing in progress
        if ctx.egui_wants_keyboard_input() || self.palette.is_some() {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.land_move = Default::default();
            self.clear_draft();
            self.merge_first = None;
            self.measure_pts.clear();
            self.measure_closed = false;
            self.ann_path = None;
        }
        // [ and ] size the brush
        if self.brush_paint {
            let (smaller, larger) = ctx.input(|i| (i.key_pressed(Key::OpenBracket), i.key_pressed(Key::CloseBracket)));
            if smaller || larger {
                self.brush_px = (self.brush_px * if larger { 1.25 } else { 0.8 }).clamp(3.0, 150.0);
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Backspace)) {
            self.pop_draft();
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) {
            if self.ann_path.is_some() {
                self.finish_ann_path();
            } else {
                self.finish_draft();
            }
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.land_move = Default::default();
        if self.tool != t {
            self.end_pending();
            self.ann_path = None;
            self.clear_draft();
            self.merge_first = None;
            self.tool = t;
            if t != Tool::DrawLand && matches!(self.draw_mode, crate::view::DrawMode::Province(_)) {
                self.draw_mode = crate::view::DrawMode::AddLand;
            }
            self.set_status(t.help(), false);
        }
    }

    /// Start making the test world in the background.
    pub fn new_test_world(&mut self, detailed: bool) {
        if detailed {
            self.test_world = Some(crate::perf_ui::spawn("生成高密度测试世界", || {
                let (world, report) = map_core::testworld::history_world(&map_core::testworld::TestWorldSpec::detailed());
                Ok((world, Some(report)))
            }));
            self.set_status(crate::i18n::tr("正在生成精细测试世界…"), false);
            return;
        }
        let configured = self.prefs.test_project.as_ref().filter(|path| path.is_file()).cloned();
        let path = configured.or_else(map_io::project::ithaer_test_project).or_else(|| {
            let path = self.file_dialog().set_title(crate::i18n::tr("选择伊瑟尔三国测试工程"))
                .add_filter("Atlas", &["atlas"]).pick_file()?;
            self.prefs.test_project = Some(path.clone());
            Some(path)
        });
        if let Some(path) = path {
            self.test_world = Some(crate::perf_ui::spawn("载入伊瑟尔测试工程", move || map_io::project::load(&path).map(|doc| (doc, None))));
            self.set_status(crate::i18n::tr("正在读取伊瑟尔三国工程…"), false);
        }
    }

    pub fn making_test_world(&self) -> bool {
        self.test_world.is_some()
    }

    /// Ask how to import polygons as a new world.
    pub fn open_import_dialog(&mut self) {
        self.import_dialog = Some((true, 100.0));
    }

    /// Merge neighbouring faces whose history is the same in every year
    /// and layer (one undo step; no year's map changes).
    pub fn tidy_faces(&mut self) {
        let faces = self.doc.topo.faces.len();
        let mut tidy = self.doc.clone();
        match tidy.tidy_faces() {
            0 => self.set_status(crate::i18n::tr("整理地块：没有可以合并的地块"), false),
            n => {
                let before = std::mem::replace(&mut self.doc, tidy);
                self.history.push("整理地块", before, &self.doc);
                self.dirty = true;
                self.selected_face = None;
                self.set_status(crate::trf!("整理地块：合并 {n} 处，地块 {} → {}；各年份地图不变", faces - 1, self.doc.topo.faces.len() - 1), false);
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            self.action_menus(ui);
            if ui.small_button(crate::i18n::tr("设置")).clicked() { self.run(Act::Prefs); }
            if ui.small_button(crate::i18n::tr("创作工作室")).clicked() { self.run(Act::Studio); }
            ui.separator();
            self.book_bar(ui);
            if let Some(paper) = self.doc.paper {
                ui.label(crate::trf!("平面画布 · {} × {} px", paper.width, paper.height));
                return;
            }
            if ui.selectable_label(self.star_map.open, crate::i18n::tr("星图")).clicked() {
                self.run(Act::StarChart);
            }
            if self.star_map.open { return; }
            ui.separator();
            ui.label(crate::i18n::tr("投影："));
            egui::ComboBox::from_id_salt("proj").selected_text(crate::i18n::tr(self.cam2d.proj.name())).show_ui(ui, |ui| {
                for p in Projection::ALL {
                    if ui.selectable_value(&mut self.cam2d.proj, p, crate::i18n::tr(p.name())).changed() {
                        self.fit_pending = true;
                    }
                }
            });
            ui.label(crate::i18n::tr("中央经线"));
            let mut meridian = self.cam2d.lon0;
            let changed = ui.add(
                egui::DragValue::new(&mut meridian)
                    .range(-180.0..=180.0)
                    .speed(1.0)
                    .suffix("°"),
            ).on_hover_text(crate::i18n::tr("点击输入经度。正值为东经，负值为西经。")).changed();
            let recenter = ui.small_button(crate::i18n::tr("居中")).on_hover_text(crate::i18n::tr("将这条经线放到二维和球体视图正中央，保留当前缩放。")).clicked();
            if changed || recenter {
                self.center_on_meridian(meridian);
            }
            ui.menu_button(crate::i18n::tr("视角"), |ui| {
                if ui.button(crate::i18n::tr("0° 经线居中")).clicked() {
                    self.center_on_meridian(0.0);
                    ui.close();
                }
                if ui.button(crate::i18n::tr("将当前中央经线记为打开视角")).clicked() {
                    let old = self.doc.home_view;
                    let view = map_core::doc::HomeView {
                        projection: self.cam2d.proj,
                        central_meridian: self.cam2d.lon0,
                        globe_latitude: self.globe.center_lat.clamp(-89.0, 89.0),
                        flat_extent: old.and_then(|view| view.flat_extent),
                    };
                    self.edit("设置打开视角", |doc| {
                        doc.home_view = Some(view);
                        Ok(())
                    });
                    ui.close();
                }
            });
        });
    }

    fn center_on_meridian(&mut self, meridian: f64) {
        if !meridian.is_finite() {
            return;
        }
        let longitude = meridian.clamp(-180.0, 180.0);
        self.cam2d.lon0 = longitude;
        self.cam2d.center[0] = 0.0;
        self.globe.center_lon = longitude;
        self.globe_inertia.stop();
        self.fit_pending = false;
        self.nav_touched = true;
    }

    /// Replace the world with imported polygons whose neighbours share
    /// borders. Undoable: Ctrl+Z restores the previous world.
    fn import_windows(&mut self, ctx: &egui::Context) {
        if let Some((mut repair, mut snap)) = self.import_dialog {
            let mut go = false;
            let mut open = true;
            egui::Window::new(crate::i18n::tr("导入 GeoJSON 为新世界")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.radio_value(&mut repair, false, crate::i18n::tr("严格模式：边界必须逐坐标一致，任何不一致都报告并停止"));
                ui.radio_value(&mut repair, true, crate::i18n::tr("修复模式：吸附、补交点、去除重叠、填补细缝，并报告每处改动"));
                ui.add_enabled_ui(repair, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("吸附距离"));
                        ui.add(egui::DragValue::new(&mut snap).range(0.1..=10000.0).speed(1.0).suffix(" m"));
                    });
                    ui.weak(crate::i18n::tr("小于吸附距离的顶点合并、贴到相邻边界上；宽度不超过 3 倍吸附距离、被陆地围住的缝隙并入相邻国家。"));
                });
                if ui.button(crate::i18n::tr("选择文件…")).clicked() {
                    go = true;
                }
            });
            self.import_dialog = if open && !go { Some((repair, snap)) } else { None };
            if go {
                self.import_geojson_topology(repair, snap);
            }
        }
        if let Some(lines) = &self.import_report {
            let mut open = true;
            egui::Window::new(crate::i18n::tr("导入报告")).open(&mut open).default_width(520.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for l in lines {
                        ui.label(l);
                    }
                });
            });
            if !open {
                self.import_report = None;
            }
        }
    }

    /// Replace the world with imported polygons whose neighbours share
    /// borders. Undoable: Ctrl+Z restores the previous world.
    fn import_geojson_topology(&mut self, repair: bool, snap_m: f64) {
        let Some((p, text)) = self.pick_vector_file() else { return };
        let mode = if repair {
            map_io::geojson::ImportMode::Repair(map_core::repair::RepairOptions { snap_m, ..Default::default() })
        } else {
            map_io::geojson::ImportMode::Strict
        };
        let t = Instant::now();
        match map_io::geojson::import_topology_with(&text, &mode) {
            Ok((mut doc, rep)) => {
                doc.meta.name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(doc.meta.name);
                self.edit("导入为新世界", move |d| {
                    *d = doc;
                    Ok(())
                });
                self.selected = None;
                self.selected_face = None;
                self.fit_pending = true;
                let mut lines = vec![format!(
                    "{}：{} 个国家，{} 个面，{} 条边，{} 个湖泊，{} 条河流，{} 个字段，跳过 {} 个无效环，用时 {:.0} ms。可 Ctrl+Z 撤销。",
                    if repair { "修复模式" } else { "严格模式" },
                    rep.countries,
                    rep.faces,
                    rep.edges,
                    rep.lakes,
                    rep.rivers,
                    rep.fields,
                    rep.skipped_rings,
                    t.elapsed().as_secs_f64() * 1e3
                )];
                let name = |f: usize| rep.feature_names.get(f).cloned().unwrap_or_else(|| format!("要素{}", f + 1));
                if let Some(r) = &rep.repair {
                    lines.push(format!("吸附：{} 个顶点，最大位移 {:.1} m；插入 {} 个交点/T 形连接点。", r.snapped_vertices, r.max_shift_m, r.inserted_vertices));
                    lines.push(format!("重叠：{} 处（先出现的要素保留重叠部分）；被完全覆盖而消失的要素：{} 个。", r.overlaps.len(), r.swallowed.len()));
                    for o in r.overlaps.iter().take(50) {
                        lines.push(format!("  · {} 保留了与 {} 重叠的 {:.3} km²（约 {:.3}°, {:.3}°）", name(o.kept_by), name(o.removed_from), o.area_km2, o.at.lon, o.at.lat));
                    }
                    for f in &r.swallowed {
                        lines.push(format!("  · {} 被完全覆盖，未导入", name(*f)));
                    }
                    lines.push(format!("填补细缝：{} 处，共 {:.3} km²。", r.slivers_filled, r.sliver_area_km2));
                    lines.push("面积变化最大的要素：".into());
                    for (f, b, a) in r.area_changes.iter().take(10) {
                        let pct = if *b > 0.0 { (a - b) / b * 100.0 } else { 0.0 };
                        lines.push(format!("  · {}：{:.1} → {:.1} km²（{:+.4}%）", name(*f), b, a, pct));
                    }
                }
                self.set_status(lines[0].clone(), false);
                self.import_report = Some(lines);
            }
            Err(rep) => {
                let first = rep.problems.iter().take(3).cloned().collect::<Vec<_>>().join("；");
                let hint = if repair { "" } else { "。可改用修复模式" };
                self.set_status(crate::trf!("共享边界导入失败（{}，{} 个问题）：{first}{hint}", if repair { "修复模式" } else { "严格模式" }, rep.problems.len()), true);
                let mut lines = vec![format!("导入失败，{} 个问题：", rep.problems.len())];
                lines.extend(rep.problems.iter().take(50).map(|p| format!("  · {p}")));
                self.import_report = Some(lines);
            }
        }
    }

    /// Ask for a GeoJSON or Shapefile and read it as GeoJSON text (a
    /// shapefile is converted; a failure is reported in the status line).
    fn pick_vector_file(&mut self) -> Option<(PathBuf, String)> {
        let p = self.file_dialog().add_filter("GeoJSON / Shapefile", &["geojson", "json", "shp"]).add_filter("Shapefile", &["shp"]).add_filter("GeoJSON", &["geojson", "json"]).pick_file()?;
        let shp = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp"));
        let text = if shp {
            match map_io::shapefile::read(&p) {
                Ok(r) if r.features == 0 => {
                    self.set_status(crate::trf!("Shapefile 中没有面或线要素（跳过 {} 个点或空要素）", r.skipped), true);
                    return None;
                }
                Ok(r) => r.geojson,
                Err(e) => {
                    self.set_status(crate::trf!("读取 Shapefile 失败：{e}"), true);
                    return None;
                }
            }
        } else {
            match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    self.set_status(crate::trf!("读取失败：{e}"), true);
                    return None;
                }
            }
        };
        Some((p, text))
    }

    pub(crate) fn import_geojson(&mut self) {
        let Some((p, text)) = self.pick_vector_file() else { return };
        let what = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp")) { "Shapefile" } else { "GeoJSON" };
        if let Some(rep) = self.edit(&format!("导入 {what}"), |d| map_io::geojson::import(d, &text)) {
            let mut msg = format!("{what}：新增 {} 个面、{} 个国家、{} 条河流，失败 {} 个", rep.faces_added, rep.countries_added, rep.rivers_added, rep.failed);
            if let Some(m) = rep.messages.first() {
                msg += &format!("（例如 {m}）");
            }
            self.set_status(msg, rep.failed > 0);
            self.fit_pending = true;
        }
    }

    pub(crate) fn import_csv(&mut self) {
        let Some(p) = self.file_dialog().add_filter("CSV", &["csv", "txt"]).pick_file() else { return };
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => return self.set_status(crate::trf!("读取失败：{e}"), true),
        };
        if let Some(rep) = self.edit("导入 CSV", |d| map_io::table::import(d, &text, false)) {
            self.set_status(
                crate::trf!(
                    "CSV：{} 行，写入 {} 个值，新建字段 {:?}，未匹配 {} 个{}",
                    rep.rows,
                    rep.values_set,
                    rep.fields_created,
                    rep.unmatched.len(),
                    if rep.unmatched.is_empty() { String::new() } else { format!("（{}…）", rep.unmatched.iter().take(5).cloned().collect::<Vec<_>>().join(", ")) }
                ),
                !rep.unmatched.is_empty(),
            );
            self.cell_edit.clear();
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if self.confirm_close {
            egui::Window::new(crate::i18n::tr("未保存的修改")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(crate::i18n::tr("当前世界有未保存的修改，是否保存？"));
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("保存")).clicked() && self.save(false) {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button(crate::i18n::tr("不保存")).clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button(crate::i18n::tr("取消")).clicked() {
                        self.confirm_close = false;
                    }
                });
            });
        }
    }
}

impl App {
    /// Style, theme, world settings, year and entity colour/note widgets
    /// mutate the document directly. Record such changes as a single undo
    /// step once the interaction is over (pointer released, no text focus).
    fn coalesce_settings(&mut self, ctx: &egui::Context, frame_start: &Document, gen_start: u64) {
        use map_core::world_book::WorldBook;
        if self.history.generation() != gen_start
            || WorldBook::current(&self.doc) != WorldBook::current(frame_start)
        {
            // this frame already went through `edit()` / undo / redo
            self.settings_pending = None;
            return;
        }
        let d = &self.doc;
        let changed = d.style != frame_start.style
            || d.theme != frame_start.theme
            || d.meta != frame_start.meta
            || d.underlays != frame_start.underlays
            || d.swatches != frame_start.swatches
            || !d.entities.ptr_eq(&frame_start.entities);
        if changed {
            self.dirty = true;
            if self.settings_pending.is_none() {
                self.settings_pending = Some(frame_start.clone());
            }
        }
        let idle = !ctx.input(|i| i.pointer.any_down()) && ctx.memory(|m| m.focused().is_none());
        if idle {
            self.finish_pending_settings();
        }
    }
}

impl App {
    pub(crate) fn read_workspace_input(&mut self,ctx:&egui::Context) {
        if self.pixel_tools.open || self.studio_find_input(ctx) {
            return;
        }
        self.shortcuts(ctx);
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::PointerButton { pressed: true, modifiers, .. } = e {
                    self.press_mods = *modifiers;
                }
            }
        });
        // Space held as the button goes down: that press pans the map, in
        // any tool, until it is let go (the frame of the release included)
        let (space, pressed, down, released) = ctx.input(|i| (i.key_down(egui::Key::Space), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released()));
        if pressed {
            self.space_pan = space && !ctx.egui_wants_keyboard_input();
        } else if !down && !released {
            self.space_pan = false;
        }
    }
    pub(crate) fn show_workspace_editors(&mut self,ctx:&egui::Context) {
        self.export_window(ctx);
        self.palette_window(ctx);
        self.shortcuts_window(ctx);
        self.about_window(ctx);
        self.tutorial_window(ctx);
        self.replay_window(ctx);
        self.prefs_window(ctx);
        self.welcome_window(ctx);
        self.history_window(ctx);
        self.checkpoints_window(ctx);
        self.focus_window(ctx);
        self.query_window(ctx);
        self.names_window(ctx);
        self.roads_window(ctx);
        self.geographic_guides_window(ctx);
        self.plugin_window(ctx);
        self.book_window(ctx);
        self.worldview_window(ctx);
        self.diplomacy_window(ctx);
        self.climate_window(ctx);
        self.environment_window(ctx);
        self.visualization_window(ctx);
        self.page_window(ctx);
        self.events_window(ctx);
        self.wizard_window(ctx);
        self.partition_window(ctx);
        self.playback_window(ctx);
        self.confirm_replace_window(ctx);
        self.reshape_window(ctx);
        self.recovery_window(ctx);
        self.import_windows(ctx);
        self.performance_window(ctx);
        self.pixel_window(ctx);
        self.studio_find_window(ctx);
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.cancel_video_on_exit();
        if let Some(mut batch) = self.batch.take() {
            use std::sync::atomic::Ordering;
            batch.cancel.store(true, Ordering::Relaxed);
            if let Some(worker) = batch.handle.take() {
                crate::shutdown::defer(move || { let _ = worker.join(); });
            }
        }
        self.autosave.defer_finish();
        self.performance.defer_finish();
        if self.prefs_writer.writing() {
            let mut writer = std::mem::take(&mut self.prefs_writer);
            let path = self.prefs_path.clone();
            crate::shutdown::defer(move || {
                if let Err(error) = writer.finish(&path) {
                    eprintln!("Atlas Forge preferences: {error}");
                }
            });
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        self.performance.mark_key(input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        if self.parent.is_none() {
            self.parent = ParentWindow::of(frame);
        }
        let ctx = ui.ctx().clone();
        let trace_frame = self.performance.begin_frame();
        if self.doc.paper.is_some() {
            self.cam2d.proj = Projection::Equirectangular;
            self.cam2d.lon0 = 0.0;
        }
        // `stable_dt` leaves out the time the window rested between repaints
        self.frame_ms = self.frame_ms * 0.9 + ctx.input(|i| i.stable_dt).min(1.0) * 1000.0 * 0.1;
        self.read_workspace_input(&ctx);
        self.apply_workspace_switch(&ctx);
        let mut lap = Instant::now();
        let input_ms = (lap - t0).as_secs_f32() * 1000.0;
        let mut laps = [0.0f32; 4];
        let mut mark = |i: usize, lap: &mut Instant| {
            laps[i] += lap.elapsed().as_secs_f32() * 1000.0;
            *lap = Instant::now();
        };
        if !self.studio.foreground && !self.star_map.open {
            self.refresh_derived();
        }
        mark(0, &mut lap);
        self.poll_underlays(&ctx);
        self.climate_tick(&ctx);
        self.environment_tick(&ctx);
        self.poll_bg_edit(&ctx);
        self.poll_test_world(&ctx);
        // where the pointer is on the map; set again by the view under it
        self.cursor = None;
        // what was selected may be gone (undo, deletion)
        let gone: Vec<_> = self.picked.iter().copied().filter(|f| !self.doc.feature_exists(*f)).collect();
        for f in gone {
            self.picked.remove(&f);
        }
        let frame_start = self.doc.clone();
        let gen_start = self.history.generation();
        let title = format!("Atlas Forge — {}{}", self.doc.meta.name, if self.dirty { " *" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        if self.studio.foreground {
            self.studio_workspace(ui);
        } else {
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        if self.star_map.open {
            self.star_chart(ui);
        } else {
            egui::Panel::top("options").show(ui, |ui| {
                if self.land_move.active { self.land_move_bar(ui); }
                else { self.options_bar(ui); }
            });
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
            egui::Panel::bottom("timeline").show(ui, |ui| self.timeline_bar(ui));
            if self.table_panel {
                egui::Panel::bottom("datatable").resizable(true).default_size(260.0).show(ui, |ui| self.data_table(ui));
            }
            egui::Panel::left("tools").resizable(false).exact_size(crate::toolbar_ui::BUTTON + 16.0).show(ui, |ui| self.tool_strip(ui));
            if crate::toolbar_ui::has_side_panel(self.tool) {
                egui::Panel::left("tool-panel").resizable(false).show(ui, |ui| self.tool_side_panel(ui));
            }
            egui::Panel::right("inspector").default_size(360.0).show(ui, |ui| {
                self.assistant_footer(ui);
                self.inspector(ui);
            });
            mark(1, &mut lap);
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                // The panel's clip rect is the visible area; the layout rect can be
                // larger, and input must not leak over the side panels.
                let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
                match if self.doc.paper.is_some() { Layout::Flat } else { self.layout } {
                    Layout::Flat => self.map_view(ui, ViewKind::Flat, rect),
                    Layout::Globe => self.map_view(ui, ViewKind::Globe, rect),
                    Layout::Split => {
                        let mid = rect.left() + rect.width() * 0.55;
                        let (a, b) = (egui::Rect::from_min_max(rect.min, egui::pos2(mid - 1.0, rect.max.y)), egui::Rect::from_min_max(egui::pos2(mid + 1.0, rect.min.y), rect.max));
                        self.map_view(ui, ViewKind::Flat, a);
                        self.map_view(ui, ViewKind::Globe, b);
                    }
                }
            });
        }
        }
        mark(2, &mut lap);
        self.show_workspace_editors(&ctx);
        self.poll_assistant(&ctx);
        self.apply_workspace_switch(&ctx);
        self.model_viewer_window(&ctx);
        self.focus_play_tick(&ctx);
        self.playback_tick(&ctx);
        self.sync_prefs(&ctx);
        if self.doc.year != frame_start.year {
            self.doc.refresh_time_view();
        }
        // a linked feature's look edited in its panel: its style, and all its features
        self.settle_styles(&ctx);
        // fonts the map has started to use
        self.sync_fonts(&ctx);
        self.coalesce_settings(&ctx, &frame_start, gen_start);
        self.handle_close(&ctx);
        if self.allow_close || (!self.dirty && ctx.input(|i| i.viewport().close_requested())) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let busy = self.pending.is_some() || self.settings_pending.is_some();
        crate::crash::remember(&self.doc, &self.path, self.history.generation(), self.dirty);
        // for testing the crash handling: ATLAS_DEBUG_PANIC makes the
        // interface panic as soon as there is unsaved work
        if self.dirty && std::env::var_os("ATLAS_DEBUG_PANIC").is_some() {
            panic!("ATLAS_DEBUG_PANIC: deliberate crash with unsaved work");
        }
        let closing = self.confirm_close || self.allow_close || ctx.input(|i| i.viewport().close_requested());
        if !closing {
            if let Some(e) = self.autosave.tick(&ctx, &self.doc, self.path.as_deref(), self.history.generation(), self.dirty, busy) {
                self.set_status(crate::trf!("自动保存失败：{e}"), true);
            }
        }
        mark(3, &mut lap);
        if let Some(p) = &mut self.prof {
            for (a, b) in p.iter_mut().zip(laps) {
                *a = *a * 0.9 + b * 0.1;
            }
            // each frame's own parts too, for measuring (stderr)
            eprintln!("FRAME ui {:.2} derived {:.2} panels {:.2} views {:.2} rest {:.2} dt {:.2}", t0.elapsed().as_secs_f32() * 1000.0, laps[0], laps[1], laps[2], laps[3], ctx.input(|i| i.unstable_dt) * 1000.0);
        }
        let ui_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.ui_ms = self.ui_ms * 0.9 + ui_ms * 0.1;
        if let Some(number) = trace_frame {
            // A release can arrive in the same input batch as drag movement.
            let ui_drag = ctx.dragged_id().is_some() || ctx.drag_stopped_id().is_some();
            let (input, interval_ms, focused, viewport) = ctx.input(|i| {
                let mut input = map_perf::Input {
                    dragging: ui_drag || i.pointer.is_decidedly_dragging(),
                    ..Default::default()
                };
                for event in &i.events {
                    match event {
                        egui::Event::MouseWheel { .. } | egui::Event::Zoom(_) => input.wheel += 1,
                        egui::Event::Key { .. } => input.keys += 1,
                        egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } => input.pointer += 1,
                        egui::Event::Touch { .. } => input.touch += 1,
                        egui::Event::MouseMoved(_) => input.raw_motion += 1,
                        _ => input.other += 1,
                    }
                }
                (input, i.unstable_dt * 1000.0, i.focused, i.viewport_rect().size())
            });
            let (workspace, layout) = if self.studio.foreground {
                ("Studio", "Studio")
            } else if self.star_map.open {
                ("Star chart", "3D")
            } else if self.doc.paper.is_some() {
                ("Artboard", "2D")
            } else {
                ("Map", match self.layout { Layout::Flat => "2D", Layout::Globe => "3D", Layout::Split => "2D + 3D" })
            };
            let phases = [
                ("输入与切换 · ms", input_ms),
                ("地图派生数据 · ms", laps[0]),
                ("面板与任务轮询 · ms", laps[1]),
                ("工作区绘图 · ms", laps[2]),
                ("窗口、时间与保存管理 · ms", laps[3]),
            ].into_iter().map(|(name, ms)| map_perf::Phase { name, ms }).collect();
            self.performance.end_frame(map_perf::Frame {
                number, ui_ms, interval_ms, input, phases, workspace, layout,
                tool: if self.land_move.active { self.land_move.mode_name() } else { self.tool.name() },
                year: self.doc.year, viewport: [viewport.x, viewport.y],
                pixels_per_point: ctx.pixels_per_point(), zoom: self.cam2d.zoom,
                globe_distance: self.globe.dist, faces: self.doc.topo.faces.len(),
                edges: self.doc.topo.edges.len(), entities: self.doc.entities.len(),
                triangles: self.scene.vertex_count() / 3, lines: self.scene.line_count(),
                history_generation: self.history.generation(), dirty: self.dirty, focused,
            });
        }
    }
}
