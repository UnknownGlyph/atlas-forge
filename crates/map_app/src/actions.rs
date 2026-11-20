//! The action layer: every command of the program is listed once, with
//! its name, menu, shortcut, when it is available and what it does. The
//! menu bar, the keyboard shortcuts, the command palette, the canvas's
//! context menu and the shortcut list are all made from this one table,
//! so a command added here appears in each of them.

use crate::app::{App, Layout};
use crate::view::Tool;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};
use map_core::arrange::Align;
use map_core::geom;
use map_core::LonLat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    NewBlank,
    NewDemo,
    NewTestWorld,
    NewDetailedWorld,
    NewWorldWizard,
    Open,
    /// One of the files opened lately, by its place in the list.
    OpenRecent(usize),
    Save,
    SaveAs,
    ImportWorld,
    ImportIslands,
    ImportCsv,
    ImportUnderlay,
    Export,
    PageView,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
    SelectBy,
    Deselect,
    Hide,
    Lock,
    UnlockAll,
    GroupSelection,
    Ungroup,
    CopyStyle,
    PasteStyle,
    Align(map_core::arrange::Align),
    Distribute(bool),
    TidyFaces,
    Validate,
    Prefs,
    Layout(Layout),
    Graticule,
    ProgressiveDetail,
    Furniture,
    FitAll,
    ZoomIn,
    ZoomOut,
    ZoomSelection,
    History,
    Checkpoints,
    NewFocus,
    ExitFocus,
    Tool(Tool),
    LandMove,
    LandScale,
    GeographicGuides,
    Timeline,
    TimelinePlayback,
    Studio,
    PrevChange,
    NextChange,
    PrevYear,
    NextYear,
    Palette,
    Shortcuts,
    Tutorial,
    TutorialExample,
    Names,
    Roads,
    Assistant,
    Plugins,
    WorldBook,
    Worldview,
    Environment,
    Visualizations,
    Climate,
    StarChart,
    About,
    Events,
    Diplomacy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    Object,
    View,
    Tools,
    Time,
    Help,
}

impl Menu {
    pub const ALL: [Menu; 7] = [Menu::File, Menu::Edit, Menu::Object, Menu::View, Menu::Tools, Menu::Time, Menu::Help];

    /// The menu's title in the interface's language.
    pub fn name(self) -> &'static str {
        crate::i18n::tr(self.source_name())
    }

    /// The title as written (Chinese).
    pub fn source_name(self) -> &'static str {
        match self {
            Menu::File => "文件",
            Menu::Edit => "编辑",
            Menu::Object => "对象",
            Menu::View => "视图",
            Menu::Tools => "工具",
            Menu::Time => "时间",
            Menu::Help => "帮助",
        }
    }
}

pub struct Action {
    pub act: Act,
    pub menu: Menu,
    pub name: &'static str,
    /// The shortcut shown; `also` is a second one that works too.
    pub keys: Option<KeyboardShortcut>,
    pub also: Option<KeyboardShortcut>,
    pub hint: &'static str,
    /// Starts a new group in its menu.
    pub group: bool,
}

const CMD: Modifiers = Modifiers::COMMAND;
const NONE: Modifiers = Modifiers::NONE;

fn key(m: Modifiers, k: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(m, k))
}

/// Every action, in menu order, in the interface's language.
pub fn table() -> Vec<Action> {
    table_with(true)
}

/// Every action's name and hint as written (Chinese), for checking the
/// language catalogue.
#[cfg(test)]
pub fn table_source() -> Vec<(&'static str, &'static str)> {
    table_with(false).into_iter().map(|a| (a.name, a.hint)).collect()
}

fn table_with(translate: bool) -> Vec<Action> {
    let tr = |s: &'static str| if translate { crate::i18n::tr(s) } else { s };
    let a = |act, menu, name, keys, hint| Action { act, menu, name: tr(name), keys, also: None, hint: tr(hint), group: false };
    let g = |act, menu, name, keys, hint| Action { act, menu, name: tr(name), keys, also: None, hint: tr(hint), group: true };
    let cmd_shift = CMD | Modifiers::SHIFT;
    let mut t = vec![
        a(Act::NewBlank, Menu::File, "新建空白世界", None, "从空白开始；用“绘制陆地”画出第一块大陆。"),
        a(Act::NewDemo, Menu::File, "新建示例世界", None, "一个带国家、城市、河流与人口数据的小世界。"),
        a(Act::NewWorldWizard, Menu::File, "新世界向导…", None, "按星球大小、陆地比例、大陆数、国家与河流数量和名称风格生成大陆、地形、河流、国家、首都，可选一段随机历史；结果都可以继续编辑。"),
        a(Act::NewTestWorld, Menu::File, "新建测试世界（多层、长历史）", None, "约 60 个国家、1000–2000 年间 240 次边界与海岸变化、建国与终止、文化层与宗教比例层、人口数据、河流与符号。用于检查功能与性能。"),
        a(Act::NewDetailedWorld, Menu::File, "新建测试世界（精细边界，约百万三角形）", None, "同上，边界与海岸再细化到约 3 km 一段（约 40 万个顶点、100 万个三角形），用于检查大地图的性能。"),
        g(Act::Open, Menu::File, "打开…", key(CMD, Key::O), ""),
        a(Act::Save, Menu::File, "保存", key(CMD, Key::S), ""),
        a(Act::SaveAs, Menu::File, "另存为…", key(cmd_shift, Key::S), ""),
        a(Act::Checkpoints, Menu::File, "保存点与自动保存…", None, "保存、浏览或恢复工程的磁盘版本。"),
        g(Act::ImportWorld, Menu::File, "导入 GeoJSON / Shapefile 为新世界（共享边界）…", None, "相邻国家共享边界；替换当前世界，可撤销。"),
        a(Act::ImportIslands, Menu::File, "导入 GeoJSON / Shapefile 为独立岛屿…", None, "每个多边形作为独立的陆地加入当前世界。"),
        a(Act::ImportCsv, Menu::File, "导入 CSV 数据…", None, "按编号或名称把数据列并入字段。"),
        a(Act::ImportUnderlay, Menu::File, "导入参考图片…", None, "作为底图描摹；用“配准参考图”对齐。"),
        g(Act::Export, Menu::File, "导出…", key(CMD, Key::E), "图片、矢量图、数据、逐年导出、动画与地图集。"),
        a(Act::PageView, Menu::File, "版式视图…", None, "按导出的样子显示整页，拖动标题、图例、比例尺、指北针，添加并摆放插图框；所见即所得。"),
        a(Act::Undo, Menu::Edit, "撤销", key(CMD, Key::Z), ""),
        Action { also: key(cmd_shift, Key::Z), ..a(Act::Redo, Menu::Edit, "重做", key(CMD, Key::Y), "") },
        g(Act::Cut, Menu::Edit, "剪切", key(CMD, Key::X), "把所选要素移到剪贴板。"),
        a(Act::Copy, Menu::Edit, "复制", key(CMD, Key::C), "把所选的线、区域、符号或文字复制到剪贴板；可粘贴到另一个窗口。"),
        a(Act::Paste, Menu::Edit, "粘贴", key(CMD, Key::V), "粘贴到鼠标所在位置；鼠标不在地图上时放在原位置旁边。"),
        a(Act::Duplicate, Menu::Edit, "原处再制", key(CMD, Key::D), "在原处旁边复制所选要素。"),
        a(Act::Delete, Menu::Edit, "删除所选要素", key(NONE, Key::Delete), "删除所选的线、区域、符号或文字。"),
        g(Act::SelectAll, Menu::Edit, "全选要素", key(CMD, Key::A), "选中全部未隐藏、未锁定的线、区域、符号与文字。"),
        a(Act::Deselect, Menu::Edit, "取消选择", key(cmd_shift, Key::A), ""),
        a(Act::SelectBy, Menu::Edit, "按属性选择…", None, "按种类、名称或图例文字、图层、样式、年份、是否隐藏选择要素。"),
        g(Act::TidyFaces, Menu::Edit, "整理地块", None, "合并在所有年份、所有分区层里都完全相同的相邻地块（多次按年份修改后留下的碎块）。任何年份的地图都不会改变。"),
        a(Act::Validate, Menu::Edit, "校验拓扑", None, "检查地图的拓扑是否完好。"),
        g(Act::Prefs, Menu::Edit, "偏好设置…", key(CMD, Key::Comma), "界面缩放、恢复快照间隔、撤销步数、启动方式。"),
        a(Act::Align(Align::West), Menu::Object, "对齐到最西", None, "各要素的中心对齐到最西的经度；每个要素整体移动，形状不变。"),
        a(Act::Align(Align::MidLon), Menu::Object, "经向居中对齐", None, ""),
        a(Act::Align(Align::East), Menu::Object, "对齐到最东", None, ""),
        a(Act::Align(Align::North), Menu::Object, "对齐到最北", None, ""),
        a(Act::Align(Align::MidLat), Menu::Object, "纬向居中对齐", None, ""),
        a(Act::Align(Align::South), Menu::Object, "对齐到最南", None, ""),
        g(Act::Distribute(true), Menu::Object, "沿经度均匀分布", None, "最西与最东的不动，其余按经度等距排开。"),
        a(Act::Distribute(false), Menu::Object, "沿纬度均匀分布", None, ""),
        g(Act::GroupSelection, Menu::Object, "所选建为图层", key(CMD, Key::G), "新建一个图层并把所选要素移入；没有选中要素时新建空图层。图层在“图层”标签中管理。"),
        a(Act::Ungroup, Menu::Object, "移出图层", key(cmd_shift, Key::G), "所选要素离开它们的图层，回到最下面按原来的顺序绘制。"),
        g(Act::CopyStyle, Menu::Object, "复制样式", None, "记下所选要素的外观（及它使用的样式），之后可粘贴给其他同类要素。也可用吸管工具（I）。"),
        a(Act::PasteStyle, Menu::Object, "粘贴样式", None, "所选的同类要素换成复制的外观；原要素使用样式时，它们也使用这个样式。"),
        g(Act::Hide, Menu::Object, "隐藏所选", None, "隐藏的要素不绘制、不导出；在“图层”标签中可重新显示。"),
        a(Act::Lock, Menu::Object, "锁定所选", None, "锁定的要素仍然显示，但不会被选中或移动；在“图层”标签中可解锁。"),
        a(Act::UnlockAll, Menu::Object, "全部解锁", None, ""),
        a(Act::Layout(Layout::Flat), Menu::View, "二维地图", None, ""),
        a(Act::Layout(Layout::Globe), Menu::View, "三维球体", None, ""),
        a(Act::Layout(Layout::Split), Menu::View, "二维 + 三维", None, ""),
        g(Act::Graticule, Menu::View, "经纬网", None, ""),
        a(Act::Studio, Menu::View, "切换创作工作室", key(CMD | Modifiers::ALT, Key::F10), "在专业界面与创作工作室窗口之间切换，继续编辑同一份工程。"),
        a(Act::ProgressiveDetail, Menu::View, "按缩放显示细节", None, "放大时逐级显示行政边界、地名、城镇和道路。"),
        a(Act::GeographicGuides, Menu::View, "地理参考线…", None, "添加固定的经线和纬线，控制辅助吸附与导出。"),
        a(Act::Furniture, Menu::View, "比例尺与指北针", None, "在两个视图的左下角显示按视图中心量出的比例尺和指向北方的指北针。"),
        Action { also: key(CMD, Key::Num0), ..g(Act::FitAll, Menu::View, "缩放到全图", key(NONE, Key::Home), "") },
        Action { also: key(CMD, Key::Plus), ..a(Act::ZoomIn, Menu::View, "放大", key(CMD, Key::Equals), "") },
        a(Act::ZoomOut, Menu::View, "缩小", key(CMD, Key::Minus), ""),
        a(Act::ZoomSelection, Menu::View, "缩放到所选", key(NONE, Key::F), "把所选的国家、地块或要素放大到充满视图。"),
        g(Act::History, Menu::View, "历史记录", None, "列出可以撤销和重做的每一步；单击其中一步，直接回到那一步之后的状态。"),
        g(Act::NewFocus, Menu::View, "新建局部视图", None, "把平面图当前显示的地区与当前年份起的十年存为一个局部视图：单独查看一场战争等局部变化——区域外的土地变淡，期间易手的土地用红线标出，可逐年播放。不改变地图，也不用图层。"),
        a(Act::ExitFocus, Menu::View, "退出局部视图", None, ""),
    ];
    for tool in Tool::ALL {
        // (the text as written: translated with the rest of the table, not
        // in whatever language happens to be set while it is built)
        t.push(a(Act::Tool(tool), Menu::Tools, tool.source_name(), key(NONE, tool.key()), tool.source_help()));
    }
    t.extend([
        a(Act::Timeline, Menu::Time, "按年份查看与编辑", key(CMD, Key::T), "开启后地图显示所选年份的状态，修改从该年起生效；关闭则显示最新状态。"),
        a(Act::TimelinePlayback, Menu::Time, "时间播放与视频", None, "设置年份间隔、总时长与速度，播放全局或局部视图并导出 MP4。"),
        g(Act::PrevChange, Menu::Time, "上一个变化年份", key(NONE, Key::PageUp), ""),
        a(Act::NextChange, Menu::Time, "下一个变化年份", key(NONE, Key::PageDown), ""),
        a(Act::PrevYear, Menu::Time, "上一年", key(Modifiers::SHIFT, Key::PageUp), ""),
        a(Act::NextYear, Menu::Time, "下一年", key(Modifiers::SHIFT, Key::PageDown), ""),
        g(Act::Events, Menu::Time, "事件时间线…", None, "给一年或一段年份起名（战争、条约、继承），附说明并关联局部视图；在时间轴上显示为标记；导出编年表。"),
        g(Act::Diplomacy, Menu::Time, "外交关系…", None, "记录双边关系、多边协约、时间区间和关联事件。"),
        g(Act::StarChart, Menu::View, "星图", None, ""),
        g(Act::Names, Menu::Tools, "名称生成器…", None, "按风格生成国名、城市名、地区、海洋、河流、山脉与人名；单击即给所选国家或城市改名，或复制。"),
        a(Act::LandMove, Menu::Tools, "整体移动陆块", None, "用鼠标移动相连的陆地、城市和共享边界；可输入精确经纬度偏移。"),
        a(Act::LandScale, Menu::Tools, "整体缩放陆块", None, "围绕选中陆块的中心等比缩放海岸、共享边界和附属要素，并检查冲突。"),
        a(Act::Assistant, Menu::Tools, "AI 助手…", None, "把任务交给语言模型（Gemini 或 DeepSeek）：它查看地图并修改一份草稿，看过前后对比再决定是否采用。"),
        a(Act::Climate, Menu::Tools, "行星环境与气候…", None, "根据恒星、行星与地图地理计算逐月气候参考图。"),
        a(Act::Plugins, Menu::Tools, "本地插件…", None, "打开插件清单，预览命令对地图草稿的修改。"),
        a(Act::WorldBook, Menu::View, "世界资料库…", None, "浏览文档、事件、画板及其关联。"),
        a(Act::Worldview, Menu::View, "世界观设定…", None, "管理自定义实体类型、关系、数值规则和模板。"),
        a(Act::Environment, Menu::Tools, "环境生成与图层…", None, "生成地形、水文、气候、生态及自定义资源图层。"),
        a(Act::Visualizations, Menu::View, "数据可视化…", None, "从地图、世界观和表格数据创建曲线、关系网络、剖面和专题图。"),
        a(Act::Roads, Menu::Tools, "道路与旅行…", None, "在城市之间生成道路网（绕开海与湖，陡坡代价更高），预览后应用为线路；查两城之间沿线路的路程与步行、骑马等所需天数。"),
        a(Act::Palette, Menu::Help, "命令面板…", key(CMD, Key::K), "按名称查找并执行任何命令。"),
        a(Act::Shortcuts, Menu::Help, "快捷键一览", key(NONE, Key::F1), ""),
        a(Act::Tutorial, Menu::Help, "创作教程…", None, "在软件内跟做地图、历史、气候、世界资料与导出。"),
        g(Act::About, Menu::Help, "关于 Atlas Forge", None, ""),
    ]);
    t
}

/// A shortcut as shown: the sign keys by their signs, not their names.
pub fn keys_text(ctx: &egui::Context, k: &KeyboardShortcut) -> String {
    ctx.format_shortcut(k).replace("Equals", "=").replace("Minus", "-").replace("Plus", "+").replace("Comma", ",")
}

/// How specific a shortcut is: more modifiers are tried first, so that
/// Ctrl+Shift+S is not taken for Ctrl+S.
fn specificity(k: &KeyboardShortcut) -> u32 {
    let m = k.modifiers;
    m.shift as u32 + m.alt as u32 + (m.ctrl || m.command || m.mac_cmd) as u32
}

/// Whether a shortcut means something to a text field too (select all,
/// undo, redo): while one has the keyboard it is the field's, not the map's.
fn text_chord(k: &KeyboardShortcut) -> bool {
    let m = k.modifiers;
    (m.ctrl || m.command || m.mac_cmd) && !m.alt && matches!(k.logical_key, Key::A | Key::Z | Key::Y)
}

/// Whether a shortcut is a plain key, which must not fire while typing.
fn plain(k: &KeyboardShortcut) -> bool {
    let m = k.modifiers;
    !(m.ctrl || m.command || m.mac_cmd || m.alt) && !matches!(k.logical_key, Key::F1)
}

/// The command palette's state while it is open.
#[derive(Default)]
pub struct Palette {
    pub query: String,
    pub at: usize,
    /// Give the text field the focus (the frame it opens).
    pub focus: bool,
}

impl App {
    /// The years just before and after the selected one that have changes.
    pub fn change_neighbours(&self) -> (Option<i32>, Option<i32>) {
        let Some(y) = self.doc.year else { return (None, None) };
        let years = self.study_years();
        let later = years.range((std::ops::Bound::Excluded(y), std::ops::Bound::Unbounded)).next().copied();
        (years.range(..y).next_back().copied(), later)
    }

    /// Lon/lat points spanning what is selected.
    fn selection_points(&self) -> Vec<LonLat> {
        let ll = |p: &[f64; 2]| LonLat::new(p[0], p[1]);
        let fs = self.selection();
        if !fs.is_empty() {
            return fs.iter().flat_map(|f| self.doc.feature_points(*f)).collect();
        }
        if let Some(e) = self.selected {
            let Some(x) = self.doc.entities.get(&e) else { return vec![] };
            if let Some(p) = x.pos {
                return vec![p];
            }
            let faces = self.doc.faces_of_tree(e).into_iter().collect();
            return self.doc.dissolve_faces(&faces).iter().flat_map(|p| p.outer.iter().map(ll)).collect();
        }
        if let Some(f) = self.selected_face.filter(|f| *f != map_core::BACKGROUND) {
            return self.doc.topo.face_planar(f).0.iter().map(ll).collect();
        }
        vec![]
    }

    /// Fill the flat view with the selection and turn the globe to it.
    fn zoom_to_selection(&mut self) {
        let pts = self.selection_points();
        if pts.is_empty() {
            return;
        }
        let cam = &mut self.cam2d;
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in &pts {
            let q = cam.proj.forward_deg(geom::wrap_lon(p.lon - cam.lon0), p.lat);
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        cam.center = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        let (w, h) = (self.last_flat_rect.width() as f64, self.last_flat_rect.height() as f64);
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        if bw > 1e-9 || bh > 1e-9 {
            cam.zoom = ((w / bw.max(1e-9)).min(h / bh.max(1e-9)) * 0.8).clamp(10.0, 5.0e7);
        }
        // the globe looks at the middle of it (points near the seam: by vectors)
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        for p in &pts {
            let (lo, la) = (p.lon.to_radians(), p.lat.to_radians());
            x += la.cos() * lo.cos();
            y += la.cos() * lo.sin();
            z += la.sin();
        }
        if x != 0.0 || y != 0.0 {
            self.globe.center_lon = y.atan2(x).to_degrees();
            self.globe.center_lat = z.atan2(x.hypot(y)).to_degrees().clamp(-89.0, 89.0);
        }
        self.nav_touched = true;
    }

    /// Whether drawn features are selected (what Copy, Delete... act on).
    fn feature_selected(&self) -> bool {
        !self.selection().is_empty()
    }

    fn anything_selected(&self) -> bool {
        self.selected.is_some() || self.selected_face.is_some() || self.selected_sym.is_some() || self.selected_ann.is_some() || self.feature_selected()
    }

    /// Whether the action can run now.
    pub fn can(&self, act: Act) -> bool {
        if self.star_map.open && !matches!(act, Act::StarChart | Act::Studio | Act::Assistant | Act::About | Act::Shortcuts | Act::Tutorial | Act::Palette | Act::Prefs |
            Act::Save | Act::SaveAs | Act::Open | Act::OpenRecent(_) | Act::NewBlank | Act::NewDemo | Act::Undo | Act::Redo) {
            return false;
        }
        match act {
            Act::StarChart | Act::Climate | Act::Environment | Act::LandMove | Act::LandScale | Act::GeographicGuides | Act::NewFocus | Act::ImportWorld | Act::ImportIslands | Act::Layout(Layout::Globe | Layout::Split)
                if self.doc.paper.is_some() => false,
            Act::NewTestWorld | Act::NewDetailedWorld => !self.making_test_world(),
            Act::Undo => self.history.can_undo(),
            Act::Redo => self.history.can_redo(),
            Act::Duplicate | Act::Delete | Act::Copy | Act::Cut | Act::Hide | Act::Lock => self.feature_selected(),
            Act::Paste => self.clip.is_some(),
            Act::Align(_) => self.selection().len() >= 2,
            Act::Distribute(_) => self.selection().len() >= 3,
            Act::UnlockAll => !self.doc.locked.is_empty(),
            Act::Ungroup => self.selection_in_layers(),
            Act::CopyStyle => self.selection().iter().any(|f| self.doc.feature_look(*f).is_some()),
            Act::PasteStyle => self.style_clip.is_some() && self.feature_selected(),
            Act::Deselect => self.anything_selected(),
            Act::ZoomSelection => !self.selection_points().is_empty(),
            Act::PrevChange => self.change_neighbours().0.is_some(),
            Act::NextChange => self.change_neighbours().1.is_some(),
            Act::PrevYear | Act::NextYear => self.doc.year.is_some(),
            Act::ExitFocus => self.focus.is_some(),
            _ => true,
        }
    }

    /// For actions that are a state: whether it is on.
    pub fn checked(&self, act: Act) -> Option<bool> {
        match act {
            Act::Layout(l) => Some(self.layout == l),
            Act::StarChart => Some(self.star_map.open),
            Act::Graticule => Some(self.graticule),
            Act::ProgressiveDetail => Some(self.prefs.progressive_detail),
            Act::Furniture => Some(self.furniture),
            Act::Tool(t) => Some(self.tool == t),
            Act::Timeline => Some(self.doc.year.is_some()),
            Act::History => Some(self.history_open),
            _ => None,
        }
    }

    /// The name as shown now (undo and redo say what they would do).
    pub fn action_label(&self, a: &Action) -> String {
        match a.act {
            Act::Undo => self.history.undo_label().map_or(a.name.to_string(), |l| format!("{}{l}", crate::i18n::tr("撤销 "))),
            _ => a.name.to_string(),
        }
    }

    /// Run an action. Actions that would discard unsaved work ask first.
    pub fn run(&mut self, act: Act) {
        if !self.can(act) {
            return;
        }
        if matches!(act, Act::NewBlank | Act::NewDemo | Act::NewTestWorld | Act::NewDetailedWorld | Act::Open | Act::OpenRecent(_) | Act::TutorialExample) && self.dirty {
            self.confirm_replace = Some(act);
            return;
        }
        self.run_now(act);
    }

    /// Run without asking about unsaved work.
    pub fn run_now(&mut self, act: Act) {
        match act {
            Act::NewBlank => {
                self.new_doc(map_core::Document::default());
                self.set_status(crate::i18n::tr("已新建空白世界。使用“绘制陆地”工具 (L) 开始。"), false);
            }
            Act::NewDemo => self.new_doc(map_core::sample::demo_world()),
            Act::TutorialExample => {
                let data = include_bytes!("../../../docs/tutorial/yuanyang.atlas");
                match map_io::project::from_bytes(data) {
                    Ok(doc) => {
                        self.new_doc(doc);
                        self.selected = self.doc.entity_by_code("YY");
                        self.cam2d.proj = map_core::projection::Projection::Equirectangular;
                        self.cam2d.lon0 = 129.0;
                        self.zoom_to_selection();
                        self.fit_pending = false;
                        self.globe.dist = 1.8;
                        self.tutorial.open = false;
                        self.set_status(crate::i18n::tr("已打开远洋人民共和国教程成品"),false);
                    }
                    Err(err) => self.set_status(err,true),
                }
            }
            Act::NewWorldWizard => self.wizard.open = true,
            Act::NewTestWorld => self.new_test_world(false),
            Act::NewDetailedWorld => self.new_test_world(true),
            Act::Open => self.open_dialog(),
            Act::OpenRecent(i) => {
                if let Some(p) = self.prefs.recent.get(i).cloned() {
                    self.open_path(p);
                }
            }
            Act::Save => {
                self.save(false);
            }
            Act::SaveAs => {
                self.save(true);
            }
            Act::ImportWorld => self.open_import_dialog(),
            Act::ImportIslands => self.import_geojson(),
            Act::ImportCsv => self.import_csv(),
            Act::ImportUnderlay => self.import_underlay(),
            Act::PageView => self.page.open = true,
            Act::Export => {
                self.export.open = true;
                self.export.projection = self.cam2d.proj;
                if let Some(paper) = self.doc.paper {
                    self.export.width = paper.width;
                    self.export.height = paper.height;
                    self.export.current_view = false;
                    if self.export.format.geographic() {
                        self.export.format = map_io::batch::Format::Png;
                    }
                }
            }
            Act::Undo => self.undo(),
            Act::Redo => self.redo(),
            Act::Duplicate => self.duplicate_selection(),
            Act::Delete => self.delete_selection(),
            // from a menu; the keys arrive as clipboard events (`clipboard_events`)
            Act::Copy | Act::Cut => self.pending_copy = Some(act == Act::Cut),
            Act::Paste => {
                if let Some(c) = self.clip.clone() {
                    self.paste(c);
                }
            }
            Act::SelectAll => self.select_all_features(),
            Act::SelectBy => self.query = Some(self.query.take().unwrap_or_default()),
            Act::Deselect => {
                self.clear_selection();
                self.selected_sym = None;
                self.selected_ann = None;
            }
            Act::Hide => self.hide_selection(),
            Act::Lock => self.lock_selection(),
            Act::UnlockAll => self.unlock_all(),
            Act::GroupSelection => self.group_selection(),
            Act::Ungroup => self.ungroup_selection(),
            Act::CopyStyle => self.copy_style(),
            Act::PasteStyle => self.paste_style(),
            Act::Align(how) => self.align_selection(how),
            Act::Distribute(by_lon) => self.distribute_selection(by_lon),
            Act::TidyFaces => self.tidy_faces(),
            Act::Validate => match self.doc.topo.validate() {
                Ok(()) => self.set_status(crate::i18n::tr("拓扑校验通过"), false),
                Err(e) => self.set_status(crate::trf!("拓扑错误：{e}"), true),
            },
            Act::Layout(l) => { self.globe_inertia.stop(); self.layout = l; },
            Act::Graticule => self.graticule = !self.graticule,
            Act::ProgressiveDetail => self.prefs.progressive_detail = !self.prefs.progressive_detail,
            Act::Furniture => self.furniture = !self.furniture,
            Act::FitAll => {
                if self.doc.home_view.is_some() {
                    self.apply_home_view();
                } else {
                    self.globe_inertia.stop();
                    self.fit_pending = true;
                }
            },
            Act::ZoomIn => self.zoom_views(1.25),
            Act::ZoomOut => self.zoom_views(0.8),
            Act::History => self.history_open = !self.history_open,
            Act::Checkpoints => self.checkpoints.open = true,
            Act::NewFocus => self.new_focus(),
            Act::TimelinePlayback => self.open_playback(self.focus.is_some()),
            Act::Studio => self.switch_workspace(),
            Act::ExitFocus => self.exit_focus(),
            Act::Prefs => self.prefs_open = true,
            Act::ZoomSelection => self.zoom_to_selection(),
            Act::Tool(t) => self.set_tool(t),
            Act::Timeline => {
                let on = self.doc.year.is_none();
                self.set_year(on.then_some(self.timeline_last_year));
            }
            Act::PrevChange => self.set_year(self.change_neighbours().0),
            Act::NextChange => self.set_year(self.change_neighbours().1),
            Act::PrevYear => self.set_year(self.doc.year.map(|y| y - 1)),
            Act::NextYear => self.set_year(self.doc.year.map(|y| y + 1)),
            Act::Palette => self.palette = Some(Palette { focus: true, ..Default::default() }),
            Act::Shortcuts => self.shortcuts_open = true,
            Act::About => self.about_open = true,
            Act::Tutorial => self.tutorial.open = true,
            Act::Diplomacy => self.diplomacy_ui.open = true,
            Act::StarChart => {
                self.end_pending();
                self.globe_inertia.stop();
                self.star_map.open = !self.star_map.open;
            }
            Act::Names => self.names_open = true,
            Act::Roads => self.roads.open = true,
            Act::Assistant => {
                self.ai.open = !self.ai.open;
            }
            Act::LandMove => self.start_land_move(),
            Act::LandScale => self.start_land_scale(),
            Act::GeographicGuides => self.guide_draft = Some(self.doc.guides.clone()),
            Act::Plugins => self.plugins.open = true,
            Act::WorldBook => self.book_current_context(),
            Act::Worldview => self.open_worldview(None),
            Act::Environment => self.open_environment(),
            Act::Visualizations => self.open_visualizations(),
            Act::Climate => self.climate_open(),
            Act::Events => self.events_open = true,
        }
    }

    /// Run the actions whose shortcuts were pressed.
    pub fn action_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let mut keys: Vec<(KeyboardShortcut, Act)> = table().iter().flat_map(|a| a.keys.into_iter().chain(a.also).map(move |k| (k, a.act))).collect();
        keys.sort_by_key(|(k, _)| std::cmp::Reverse(specificity(k)));
        for (k, act) in keys {
            if plain(&k) && (typing || self.palette.is_some()) {
                continue;
            }
            if typing && text_chord(&k) {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&k)) {
                self.run(act);
            }
        }
    }

    /// One action as a menu entry; true when it was chosen.
    pub fn action_button(&mut self, ui: &mut egui::Ui, a: &Action) -> bool {
        let mut b = egui::Button::new(self.action_label(a));
        if let Some(k) = &a.keys {
            b = b.shortcut_text(keys_text(ui.ctx(), k));
        }
        if let Some(on) = self.checked(a.act) {
            b = b.selected(on);
        }
        let mut r = ui.add_enabled(self.can(a.act), b);
        if !a.hint.is_empty() {
            r = r.on_hover_text(a.hint);
        }
        if r.clicked() {
            self.run(a.act);
            return true;
        }
        false
    }

    /// The menus of the menu bar.
    pub fn action_menus(&mut self, ui: &mut egui::Ui) {
        let all = table();
        for m in Menu::ALL {
            ui.menu_button(m.name(), |ui| {
                let mut first = true;
                for a in all.iter().filter(|a| a.menu == m) {
                    if a.group && !first {
                        ui.separator();
                    }
                    first = false;
                    self.action_button(ui, a);
                    if a.act == Act::Open {
                        self.recent_menu(ui);
                    }
                    if a.act == Act::NewFocus {
                        self.focus_menu(ui);
                    }
                }
            });
        }
    }

    /// The context menu of the map in the select tool: what can be done
    /// with the selection, then the view.
    pub fn canvas_menu(&mut self, ui: &mut egui::Ui) {
        let all = table();
        let find = |act: Act| all.iter().find(|a| a.act == act).unwrap();
        let what = if let Some(e) = self.selected.and_then(|e| self.doc.entities.get(&e)) {
            Some(e.name.clone())
        } else if let Some(r) = self.selected_river.and_then(|id| self.doc.rivers.get(&id)) {
            Some(if r.name.is_empty() { "（未命名的线）".to_string() } else { r.name.clone() })
        } else if let Some(a) = self.selected_area.and_then(|id| self.doc.areas.get(&id)) {
            Some(if a.name.is_empty() { "（未命名的区域）".to_string() } else { a.name.clone() })
        } else {
            self.selected_face.map(|_| "无主的地块".to_string())
        };
        if let Some(w) = what {
            ui.label(RichText::new(w).strong());
            ui.separator();
        }
        self.action_button(ui, find(Act::ZoomSelection));
        if let Some(e) = self.selected.filter(|e| self.doc.entities.get(e).is_some_and(|x| x.pos.is_none())) {
            if ui.add_enabled(self.brush != Some(e), egui::Button::new(crate::i18n::tr("设为画笔国家"))).on_hover_text(crate::i18n::tr("之后用“领土画笔”或绘制陆地时归属这个国家。")).clicked() {
                self.brush = Some(e);
            }
        }
        if self.feature_selected() {
            for act in [Act::Cut, Act::Copy, Act::Duplicate, Act::Delete, Act::Hide, Act::Lock, Act::CopyStyle, Act::PasteStyle] {
                self.action_button(ui, find(act));
            }
        }
        self.action_button(ui, find(Act::Paste));
        self.action_button(ui, find(Act::Deselect));
        ui.separator();
        self.action_button(ui, find(Act::FitAll));
        self.action_button(ui, find(Act::Undo));
        self.action_button(ui, find(Act::Redo));
    }

    /// The command palette: type part of a name, Enter runs the first match.
    pub fn palette_window(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.palette.take() else { return };
        let all = table();
        let q = p.query.trim().to_lowercase();
        let found: Vec<&Action> = all
            .iter()
            .filter(|a| !matches!(a.act, Act::Palette))
            .filter(|a| q.is_empty() || format!("{} {} {}", a.menu.name(), self.action_label(a), a.keys.map(|k| keys_text(ctx, &k)).unwrap_or_default()).to_lowercase().contains(&q))
            .collect();
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (i.consume_key(NONE, Key::ArrowUp), i.consume_key(NONE, Key::ArrowDown), i.consume_key(NONE, Key::Enter), i.consume_key(NONE, Key::Escape))
        });
        if !found.is_empty() {
            if down {
                p.at = (p.at + 1) % found.len();
            }
            if up {
                p.at = (p.at + found.len() - 1) % found.len();
            }
            p.at = p.at.min(found.len() - 1);
        }
        let mut run: Option<Act> = None;
        let mut open = !esc;
        egui::Window::new(crate::i18n::tr("命令面板")).title_bar(false).resizable(false).anchor(egui::Align2::CENTER_TOP, [0.0, 60.0]).min_width(460.0).max_width(460.0).show(ctx, |ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(crate::i18n::tr("输入命令名称… （↑↓ 选择，回车执行，Esc 关闭）")).desired_width(f32::INFINITY));
            if p.focus {
                edit.request_focus();
                p.focus = false;
            }
            if edit.changed() {
                p.at = 0;
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, a) in found.iter().enumerate() {
                    let ok = self.can(a.act);
                    let mut b = egui::Button::new(format!("{}  ›  {}", a.menu.name(), self.action_label(a))).selected(i == p.at).frame(i == p.at);
                    if let Some(k) = &a.keys {
                        b = b.shortcut_text(keys_text(ctx, k));
                    }
                    let r = ui.add_enabled(ok, b.min_size(egui::vec2(ui.available_width(), 0.0)));
                    if i == p.at && (up || down) {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        run = Some(a.act);
                    }
                }
                if found.is_empty() {
                    ui.weak(crate::i18n::tr("没有匹配的命令"));
                }
            });
        });
        if enter {
            run = found.get(p.at).filter(|a| self.can(a.act)).map(|a| a.act);
            open = run.is_none();
        }
        if let Some(act) = run {
            self.run(act);
        } else if open {
            self.palette = Some(p);
        }
    }

    /// All shortcuts, by menu, plus the keys that are not actions.
    pub fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let mut open = true;
        egui::Window::new(crate::i18n::tr("快捷键一览")).open(&mut open).resizable(false).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(520.0).min_scrolled_height(420.0).show(ui, |ui| {
                let all = table();
                for m in Menu::ALL {
                    let rows: Vec<&Action> = all.iter().filter(|a| a.menu == m && a.keys.is_some()).collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.label(RichText::new(m.name()).strong());
                    egui::Grid::new(("keys", m.name())).num_columns(2).min_col_width(150.0).show(ui, |ui| {
                        for a in rows {
                            let mut k = keys_text(ctx, &a.keys.unwrap());
                            if let Some(also) = &a.also {
                                k = format!("{k} 或 {}", keys_text(ctx, also));
                            }
                            ui.label(k);
                            ui.label(a.name);
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.label(RichText::new(crate::i18n::tr("选择工具")).strong());
                egui::Grid::new("keys-select").num_columns(2).min_col_width(150.0).show(ui, |ui| {
                    for (k, what) in [("单击", "选中国家、地块或要素"), ("Shift / Ctrl + 单击", "加选或减选要素"), ("Shift + 拖动", "框选要素"), ("拖动所选要素", "一起移动"), ("双击文字或符号", "用它的工具编辑")] {
                        ui.label(k);
                        ui.label(what);
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new(crate::i18n::tr("绘制时")).strong());
                egui::Grid::new("keys-draw").num_columns(2).min_col_width(150.0).show(ui, |ui| {
                    for (k, what) in [("Enter / 双击", "完成当前绘制"), ("Backspace", "撤回上一个草稿点"), ("Esc", "放弃当前绘制"), ("[ 与 ]", "缩小 / 放大画笔"), ("Shift", "约束 15° 方向"), ("Ctrl", "临时关闭吸附"), ("右键拖动 / 中键拖动", "平移视图（球体为旋转）"), ("空格 + 拖动", "任何工具下平移视图"), ("滚轮", "缩放")] {
                        ui.label(k);
                        ui.label(what);
                        ui.end_row();
                    }
                });
            });
        });
        self.shortcuts_open = open;
    }

    /// Asked before an action replaces a world with unsaved changes.
    pub fn confirm_replace_window(&mut self, ctx: &egui::Context) {
        let Some(act) = self.confirm_replace else { return };
        let mut choice: Option<bool> = None;
        let mut cancel = false;
        egui::Window::new(crate::i18n::tr("放弃未保存的修改？")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(crate::i18n::tr("当前世界有未保存的修改，继续会丢失它们。是否先保存？"));
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("保存并继续")).clicked() {
                    choice = Some(true);
                }
                if ui.button(crate::i18n::tr("不保存，继续")).clicked() {
                    choice = Some(false);
                }
                cancel = ui.button(crate::i18n::tr("取消")).clicked();
            });
        });
        if cancel {
            self.confirm_replace = None;
        }
        if let Some(save) = choice {
            // a cancelled save dialog leaves everything as it is
            if !save || self.save(false) {
                self.confirm_replace = None;
                self.run_now(act);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table for everything: no two actions share a shortcut, every
    /// tool is an action with its key, and every menu has entries.
    #[test]
    fn the_table_is_consistent() {
        let t = table();
        let mut seen: Vec<(KeyboardShortcut, &str)> = Vec::new();
        for a in &t {
            assert!(!a.name.is_empty());
            for k in a.keys.iter().chain(a.also.iter()) {
                if let Some((_, other)) = seen.iter().find(|(s, _)| s == k) {
                    panic!("{} and {other} share a shortcut", a.name);
                }
                seen.push((*k, a.name));
            }
        }
        for tool in Tool::ALL {
            let a = t.iter().find(|a| a.act == Act::Tool(tool)).unwrap_or_else(|| panic!("{tool:?} has no action"));
            assert_eq!(a.keys.map(|k| k.logical_key), Some(tool.key()));
            assert!(!a.name.contains("  "), "the name carries no shortcut letter: {}", a.name);
        }
        for m in Menu::ALL {
            assert!(t.iter().any(|a| a.menu == m), "{} is empty", m.name());
        }
        // plain keys are those that must not fire while typing
        assert!(plain(&KeyboardShortcut::new(NONE, Key::V)) && plain(&KeyboardShortcut::new(Modifiers::SHIFT, Key::PageUp)));
        assert!(!plain(&KeyboardShortcut::new(CMD, Key::S)) && !plain(&KeyboardShortcut::new(NONE, Key::F1)));
        // select all, undo and redo are a text field's own while one has the keyboard
        for a in &t {
            let field = matches!(a.act, Act::SelectAll | Act::Deselect | Act::Undo | Act::Redo);
            for k in a.keys.iter().chain(a.also.iter()) {
                assert_eq!(text_chord(k), field, "{}", a.name);
            }
        }
        // more specific shortcuts are tried first
        assert!(specificity(&KeyboardShortcut::new(CMD | Modifiers::SHIFT, Key::S)) > specificity(&KeyboardShortcut::new(CMD, Key::S)));
    }
}
