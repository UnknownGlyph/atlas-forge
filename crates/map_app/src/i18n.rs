//! The interface's language: Chinese (the source text) or English. Texts
//! are written in Chinese where they are used; `tr` gives the English of a
//! text in the catalogue below (generated for the commands, menus and tools
//! first), or the text itself when there is none yet — so parts not yet
//! translated stay in Chinese. The language comes from the preferences.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Zh, Lang::En];
    /// As stored in the preferences.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }
    pub fn from_code(c: &str) -> Lang {
        if c == "en" { Lang::En } else { Lang::Zh }
    }
    /// Its own name, as shown in the language picker.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Zh => "中文",
            Lang::En => "English",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::Zh }
}

fn catalogue() -> &'static HashMap<&'static str, &'static str> {
    static C: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    C.get_or_init(|| EN.iter().copied().collect())
}

/// The text in the interface's language.
pub fn tr(zh: &'static str) -> &'static str {
    match get() {
        Lang::Zh => zh,
        Lang::En => catalogue().get(zh).copied().unwrap_or(zh),
    }
}

/// English for each Chinese text (commands, menus, tools first).
pub const EN: &[(&str, &str)] = &[
    ("新建空白世界", "New blank world"),
    ("从空白开始；用“绘制陆地”画出第一块大陆。", "Start from nothing; draw the first continent with Draw land."),
    ("新建示例世界", "New sample world"),
    ("一个带国家、城市、河流与人口数据的小世界。", "A small world with countries, cities, rivers and population data."),
    ("新世界向导…", "New world wizard…"),
    ("按星球大小、陆地比例、大陆数、国家与河流数量和名称风格生成大陆、地形、河流、国家、首都，可选一段随机历史；结果都可以继续编辑。", "Make continents, terrain, rivers, countries and capitals from the planet's size, the share of land, the number of continents, countries and rivers and a name style, with a random history if wanted; everything stays editable."),
    ("新建测试世界（多层、长历史）", "New test world (layers, long history)"),
    ("约 60 个国家、1000–2000 年间 240 次边界与海岸变化、建国与终止、文化层与宗教比例层、人口数据、河流与符号。用于检查功能与性能。", "About 60 countries, 240 border and coast changes between 1000 and 2000, foundings and endings, a culture layer and a religion share layer, population data, rivers and symbols. For checking features and speed."),
    ("新建测试世界（精细边界，约百万三角形）", "New test world (fine borders, about a million triangles)"),
    ("同上，边界与海岸再细化到约 3 km 一段（约 40 万个顶点、100 万个三角形），用于检查大地图的性能。", "As above, with borders and coasts refined to about 3 km a segment (about 400 000 vertices, a million triangles), for checking the speed of large maps."),
    ("打开…", "Open…"),
    ("保存", "Save"),
    ("另存为…", "Save as…"),
    ("导入 GeoJSON / Shapefile 为新世界（共享边界）…", "Import GeoJSON / Shapefile as a new world (shared borders)…"),
    ("相邻国家共享边界；替换当前世界，可撤销。", "Neighbouring countries share their borders; replaces the current world, can be undone."),
    ("导入 GeoJSON / Shapefile 为独立岛屿…", "Import GeoJSON / Shapefile as separate islands…"),
    ("每个多边形作为独立的陆地加入当前世界。", "Each polygon joins the current world as land of its own."),
    ("导入 CSV 数据…", "Import CSV data…"),
    ("按编号或名称把数据列并入字段。", "Merge columns of data into fields by code or by name."),
    ("导入参考图片…", "Import reference image…"),
    ("作为底图描摹；用“配准参考图”对齐。", "A picture to trace over; line it up with Georeference."),
    ("导出…", "Export…"),
    ("图片、矢量图、数据、逐年导出、动画与地图集。", "Pictures, vector maps, data, one file per year, animations and atlases."),
    ("版式视图…", "Layout view…"),
    ("按导出的样子显示整页，拖动标题、图例、比例尺、指北针，添加并摆放插图框；所见即所得。", "The whole page as it will be exported: drag the title, legend, scale bar and north arrow, add and place inset maps; what you see is what you get."),
    ("撤销", "Undo"),
    ("剪切", "Cut"),
    ("把所选要素移到剪贴板。", "Move the selected features to the clipboard."),
    ("复制", "Copy"),
    ("把所选的线、区域、符号或文字复制到剪贴板；可粘贴到另一个窗口。", "Copy the selected lines, areas, symbols or text to the clipboard; they can be pasted into another window."),
    ("粘贴", "Paste"),
    ("粘贴到鼠标所在位置；鼠标不在地图上时放在原位置旁边。", "Paste where the pointer is; beside the original place when the pointer is not on the map."),
    ("原处再制", "Duplicate"),
    ("在原处旁边复制所选要素。", "Copy the selected features next to themselves."),
    ("删除所选要素", "Delete selected features"),
    ("删除所选的线、区域、符号或文字。", "Delete the selected lines, areas, symbols or text."),
    ("全选要素", "Select all features"),
    ("选中全部未隐藏、未锁定的线、区域、符号与文字。", "Select every line, area, symbol and text that is not hidden or locked."),
    ("取消选择", "Deselect"),
    ("按属性选择…", "Select by attribute…"),
    ("按种类、名称或图例文字、图层、样式、年份、是否隐藏选择要素。", "Select features by kind, name or legend text, layer, style, year, hidden or not."),
    ("整理地块", "Tidy faces"),
    ("合并在所有年份、所有分区层里都完全相同的相邻地块（多次按年份修改后留下的碎块）。任何年份的地图都不会改变。", "Merge neighbouring faces that are the same in every year and every layer (pieces left by many edits by year). No year's map changes."),
    ("校验拓扑", "Check topology"),
    ("检查地图的拓扑是否完好。", "Check that the map's topology is sound."),
    ("偏好设置…", "Preferences…"),
    ("界面缩放、恢复快照间隔、撤销步数、启动方式。", "Interface size, recovery snapshots, undo steps, how the program starts, language."),
    ("所选建为图层", "Make a layer of the selection"),
    ("新建一个图层并把所选要素移入；没有选中要素时新建空图层。图层在“图层”标签中管理。", "Make a new layer and move the selected features into it; with nothing selected, an empty layer. Layers are managed in the Layers tab."),
    ("移出图层", "Take out of layer"),
    ("所选要素离开它们的图层，回到最下面按原来的顺序绘制。", "The selected features leave their layers and are drawn at the bottom in their old order."),
    ("复制样式", "Copy style"),
    ("记下所选要素的外观（及它使用的样式），之后可粘贴给其他同类要素。也可用吸管工具（I）。", "Remember the selected feature's look (and the style it uses) to paste onto features of the same kind. The eyedropper (I) does the same."),
    ("粘贴样式", "Paste style"),
    ("所选的同类要素换成复制的外观；原要素使用样式时，它们也使用这个样式。", "Give the selected features of the same kind the copied look; when the original uses a style, they use it too."),
    ("隐藏所选", "Hide selected"),
    ("隐藏的要素不绘制、不导出；在“图层”标签中可重新显示。", "Hidden features are not drawn or exported; show them again in the Layers tab."),
    ("锁定所选", "Lock selected"),
    ("锁定的要素仍然显示，但不会被选中或移动；在“图层”标签中可解锁。", "Locked features are still drawn but cannot be selected or moved; unlock them in the Layers tab."),
    ("全部解锁", "Unlock all"),
    ("经纬网", "Graticule"),
    ("比例尺与指北针", "Scale bar and north arrow"),
    ("在两个视图的左下角显示按视图中心量出的比例尺和指向北方的指北针。", "Show in the lower left of both views a scale bar measured at the view's centre and an arrow pointing north."),
    ("缩小", "Zoom out"),
    ("缩放到所选", "Zoom to selection"),
    ("把所选的国家、地块或要素放大到充满视图。", "Zoom the selected country, face or feature to fill the view."),
    ("历史记录", "History"),
    ("列出可以撤销和重做的每一步；单击其中一步，直接回到那一步之后的状态。", "List every step that can be undone or redone; click one to go straight back to the state after it."),
    ("新建局部视图", "New local view"),
    ("把平面图当前显示的地区与当前年份起的十年存为一个局部视图：单独查看一场战争等局部变化——区域外的土地变淡，期间易手的土地用红线标出，可逐年播放。不改变地图，也不用图层。", "Keep the region the flat map shows and the ten years from the current one as a local view: follow a war or another local change on its own — the land outside fades, land that changed hands is outlined in red, the years can be played. The map does not change and no layer is used."),
    ("退出局部视图", "Leave local view"),
    ("按年份查看与编辑", "View and edit by year"),
    ("开启后地图显示所选年份的状态，修改从该年起生效；关闭则显示最新状态。", "When on, the map shows the chosen year and edits take effect from that year on; when off, the latest state."),
    ("上一个变化年份", "Previous change year"),
    ("下一个变化年份", "Next change year"),
    ("上一年", "Previous year"),
    ("下一年", "Next year"),
    ("事件时间线…", "Event timeline…"),
    ("给一年或一段年份起名（战争、条约、继承），附说明并关联局部视图；在时间轴上显示为标记；导出编年表。", "Name a year or a span of years (a war, a treaty, a succession), with notes and a local view; shown as marks on the timeline; export a chronicle."),
    ("名称生成器…", "Name generator…"),
    ("按风格生成国名、城市名、地区、海洋、河流、山脉与人名；单击即给所选国家或城市改名，或复制。", "Names of countries, cities, regions, seas, rivers, mountains and people in a style; click one to rename the selected country or city, or to copy it."),
    ("道路与旅行…", "Roads and travel…"),
    ("在城市之间生成道路网（绕开海与湖，陡坡代价更高），预览后应用为线路；查两城之间沿线路的路程与步行、骑马等所需天数。", "Build a road network between the cities (around seas and lakes, steep ground dearer), previewed and applied as routes; the distance between two cities along the routes and the days on foot, on horseback and more."),
    ("命令面板…", "Command palette…"),
    ("按名称查找并执行任何命令。", "Find any command by name and run it."),
    ("快捷键一览", "Keyboard shortcuts"),
    ("选择 / 浏览  V", "Select / browse  V"),
    ("绘制陆地  L", "Draw land  L"),
    ("绘制湖泊  K", "Draw lake  K"),
    ("切分 / 画边界  S", "Split / draw border  S"),
    ("重画国界  G", "Redraw border  G"),
    ("编辑顶点  E", "Edit vertices  E"),
    ("领土画笔  B", "Territory brush  B"),
    ("合并相邻面  M", "Merge neighbouring faces  M"),
    ("添加城市  C", "Add city  C"),
    ("河流 / 线路  R", "River / route  R"),
    ("配准参考图  A", "Georeference image  A"),
    ("标注  T", "Labels  T"),
    ("地图符号  Y", "Map symbols  Y"),
    ("斜线区域  H", "Hatched area  H"),
    ("测量  D", "Measure  D"),
    ("钢笔 / 曲线  P", "Pen / curves  P"),
    ("吸管  I", "Eyedropper  I"),
    ("单击选择国家/地区；拖动平移（球体为旋转），滚轮缩放。", "Click to select a country or region; drag to pan (rotate the globe), wheel to zoom."),
    ("单击加点或按住拖动手绘；靠近边界/海岸自动吸附，两点都在边界上时沿边界描线（Ctrl 临时关闭）；Shift 约束 15° 方向；拖动草稿点移动、拖动中点插入、右键删除；单击起点/双击/回车完成，Backspace 撤回，Esc 取消。", "Click to add points or hold and drag to draw freehand; snaps to borders and coasts nearby and follows a border when both points lie on it (Ctrl turns this off); Shift keeps to 15° steps; drag a draft point to move it, a midpoint to insert one, right-click to delete; click the first point, double-click or press Enter to finish, Backspace to step back, Esc to cancel."),
    ("与绘制陆地相同的操作；画出的区域变为水域（可跨越边界）。", "Drawn like land; the region becomes water (it may cross borders)."),
    ("从边界上一点开始，单击或拖动手绘穿过面内部，落到边界上即完成 → 一个面切成两个；草稿点可拖动、右键删除。", "Start on a border, click or drag across the face and end on a border: the face is cut in two; draft points can be dragged or right-clicked away."),
    ("从边界上一点出发画新的走向，落回边界即完成：新旧边界之间的土地归对侧所有（对侧是海则变为水域）。这是领土变化，选了年份时只从该年起生效，更早的地图不变。光标停在边界上时预览移交的区域。", "Draw a new course from a point on a border back onto it: the land between the old and the new border goes to the other side (water, if the other side is sea). A change of territory: with a year chosen it applies from that year on and earlier maps stay. Hovering over a border previews the land handed over."),
    ("拖动顶点（共享边界两侧同步）；拖动边上任意位置插入新顶点；右键删除顶点。", "Drag a vertex (both sides of a shared border move together); drag anywhere on an edge to insert a vertex; right-click to delete one."),
    ("左键单击/拖动把陆地分配给“当前画笔国家”；右键取消归属；Shift+单击（或勾选“填充”）把相连的一整片分配给它。", "Click or drag to give land to the brush's country; right-click to take it away; Shift+click (or tick Fill) gives a whole connected patch."),
    ("依次单击两个相邻的面，删除它们之间的边界。", "Click two neighbouring faces in turn to remove the border between them."),
    ("单击空白处添加城市（自动归属所在国家）；单击城市选中，拖动城市移动，右键城市删除。", "Click empty land to add a city (it joins the country there); click a city to select it, drag to move it, right-click to delete it."),
    ("单击添加折线点，双击或回车结束；工具面板中选择画河流还是线路（道路、商路、行军路线）。点会吸附到同类线的端点和线上（线路还吸附到城市），Ctrl 临时关闭；从一条线的端点开始（或在端点结束）会延长那条线。画好的线用选择工具单击选中。", "Click to add points, double-click or press Enter to finish; choose a river or a route (road, trade route, campaign) in the tool panel. Points snap to the ends of lines of the same kind and onto them (routes also to cities); Ctrl turns this off; starting or ending at a line's end extends that line. Select a finished line with the select tool."),
    ("把参考图片上的地物拖到它的真实位置，图片随之变形对齐；右键单击配准点删除。", "Drag features of the reference image to their true places and the image bends to fit; right-click a control point to delete it."),
    ("单击各点沿大圆测量距离，单击第一个点闭合并给出面积；右键撤回一个点，Esc 清除。按本星球半径计算，不会保存。", "Click points to measure along great circles; click the first point to close and get the area; right-click steps back, Esc clears. On this planet's radius; not saved."),
    ("像绘制陆地一样画出区域（单击加点或拖动手绘，可吸附并沿边界描线；单击起点/双击/回车完成），用斜线标出争议、占领、势力范围等；不改变领土归属。单击区域边线选中，在国家页的面板中修改。", "Draw a region as you draw land (click or drag, snapping and following borders; click the first point, double-click or Enter to finish) and hatch it to mark a dispute, an occupation, a sphere of influence; ownership does not change. Click its outline to select it and edit it in the country page's panel."),
    ("单击空白处放置符号（山峰、战役、首都、港口、要塞、遗址），单击选中，拖动移动，右键或 Delete 删除；可命名并设置显示的年份范围（未选年份时全部显示）。", "Click empty space to place a symbol (peak, battle, capital, port, fortress, ruin), click to select, drag to move, right-click or Delete to remove; it can be named and shown for a span of years (all years when none is chosen)."),
    ("用贝塞尔曲线画河流、线路或斜线区域（在选项栏选择）：单击放尖角锚点，按住拖动放平滑锚点并拉出控制柄；画区域时单击第一个锚点闭合；双击或回车完成，右键或 Backspace 撤回上一个锚点，Esc 放弃。没有在画时，单击一条线或区域选中它，拖动其锚点与控制柄修改。", "Draw rivers, routes or hatched areas with Bézier curves (chosen in the options bar): click for a corner anchor, hold and drag for a smooth one with handles; click the first anchor to close an area; double-click or Enter to finish, right-click or Backspace to step back, Esc to give up. When not drawing, click a line or area to select it and drag its anchors and handles."),
    ("先用选择工具选中要修改的线路、区域、文字或符号，再用吸管单击作样板的要素：所选的同类要素换成它的外观；样板使用某个样式时，它们也使用这个样式。", "Select the routes, areas, text or symbols to change with the select tool, then click a feature with the eyedropper: the selected features of the same kind take its look; when it uses a style, they use it too."),
    ("双击空白处添加地图文字；拖动国家标注到想要的位置（所有年份相同，始终显示，导出同样生效）；单击标注选中国家，在国家属性中旋转、缩放、隐藏或恢复自动。此工具下显示全部标注（含被避让和隐藏的）。", "Double-click empty space to add map text; drag a country label where you want it (the same in every year, always shown, exported too); click a label to select its country and rotate, scale, hide or restore it in the country's properties. This tool shows every label, the ones hidden or crowded out included."),
    ("AI 助手…", "AI assistant…"),
    ("把任务交给语言模型（Gemini 或 DeepSeek）：它查看地图并修改一份草稿，看过前后对比再决定是否采用。", "Give a task to a language model (Gemini or DeepSeek): it looks at the map and edits a draft, which you apply after seeing it before and after."),
    ("文件", "File"),
    ("编辑", "Edit"),
    ("对象", "Object"),
    ("视图", "View"),
    ("工具", "Tools"),
    ("时间", "Time"),
    ("帮助", "Help"),
    ("偏好设置", "Preferences"),
    ("界面语言", "Language"),
    ("开始", "Start"),
    ("看看示例世界", "Look at the sample world"),
    ("国家", "Countries"),
    ("数据表", "Data table"),
    ("字段", "Fields"),
    ("专题图", "Thematic"),
    ("样式", "Styles"),
    ("图层", "Layers"),
    ("参考图", "References"),
    ("撤销 ", "Undo "),
    ("时间轴", "Timeline"),
    ("重做", "Redo"),
    ("对齐到最西", "Align to the westernmost"),
    ("各要素的中心对齐到最西的经度；每个要素整体移动，形状不变。", "Move each feature as a whole so its centre is at the westernmost longitude; shapes do not change."),
    ("经向居中对齐", "Centre east–west"),
    ("对齐到最东", "Align to the easternmost"),
    ("对齐到最北", "Align to the northernmost"),
    ("纬向居中对齐", "Centre north–south"),
    ("对齐到最南", "Align to the southernmost"),
    ("沿经度均匀分布", "Spread evenly east–west"),
    ("最西与最东的不动，其余按经度等距排开。", "The westernmost and easternmost stay; the others are spaced evenly by longitude."),
    ("沿纬度均匀分布", "Spread evenly north–south"),
    ("二维地图", "Flat map"),
    ("三维球体", "Globe"),
    ("二维 + 三维", "Flat map + globe"),
    ("缩放到全图", "Zoom to the whole map"),
    ("放大", "Zoom in"),
    ("选择 / 浏览", "Select / browse"),
    ("绘制陆地", "Draw land"),
    ("绘制湖泊", "Draw lake"),
    ("切分 / 画边界", "Split / draw border"),
    ("重画国界", "Redraw border"),
    ("编辑顶点", "Edit vertices"),
    ("领土画笔", "Territory brush"),
    ("合并相邻面", "Merge neighbouring faces"),
    ("添加城市", "Add city"),
    ("河流 / 线路", "River / route"),
    ("钢笔 / 曲线", "Pen / curves"),
    ("标注", "Labels"),
    ("地图符号", "Map symbols"),
    ("斜线区域", "Hatched area"),
    ("吸管", "Eyedropper"),
    ("测量", "Measure"),
    ("配准参考图", "Georeference image"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command, menu and tool has its English, and no text twice.
    #[test]
    fn the_catalogue_covers_commands_menus_and_tools() {
        let mut seen = std::collections::HashSet::new();
        for (zh, en) in EN {
            assert!(seen.insert(*zh), "{zh} twice");
            // English, with no Chinese left in it
            assert!(!en.is_empty() && !en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{3000}'..='\u{303f}').contains(&c)), "{zh}: {en}");
        }
        let c = catalogue();
        let mut missing: Vec<&str> = Vec::new();
        for a in crate::actions::table_source() {
            missing.extend([a.0, a.1].into_iter().filter(|s| !s.is_empty() && !c.contains_key(s)));
        }
        missing.extend(crate::actions::Menu::ALL.map(|m| m.source_name()).into_iter().filter(|s| !c.contains_key(s)));
        for t in crate::view::Tool::ALL {
            missing.extend([t.source_label(), t.source_help()].into_iter().filter(|s| !c.contains_key(s)));
        }
        assert!(missing.is_empty(), "no English for:
{}", missing.join("
"));
    }

    #[test]
    fn switching_language() {
        set(Lang::En);
        assert_eq!(tr("保存"), "Save");
        assert_eq!(tr("没有译文的文字"), "没有译文的文字");
        set(Lang::Zh);
        assert_eq!(tr("保存"), "保存");
    }
}
