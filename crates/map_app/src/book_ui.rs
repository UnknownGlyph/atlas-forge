//! Project-wide articles, map boards and their links.
mod render;

use crate::{app::App, view::ViewGeom};
use eframe::egui::{self, Color32, RichText};
use map_core::world_book::{
    self, Article, Attachment, BookId, Link, Meta, Node, TimeLink, WorldBook, ROOT,
};
use map_core::{Document, LonLat};
use map_io::dossier::{self, Format, OptionsForExport};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    All,
    Maps,
    Articles,
    Events,
    Assets,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Object,
    Face,
    Edge,
    Place,
    Event(Node),
}

#[derive(Clone, Copy)]
struct CameraState {
    flat: map_render::camera::Camera2D,
    globe: map_render::camera::GlobeCamera,
    layout: crate::app::Layout,
    tab: crate::panels::Tab,
    timeline_year: i32,
}
pub struct BookUi {
    pub open: bool,
    pub pick: Option<Pick>,
    selected: Option<Node>,
    tab: Tab,
    filter: String,
    checked: BTreeSet<BookId>,
    reading: Vec<Node>,
    map_path: Vec<BookId>,
    views: BTreeMap<BookId, CameraState>,
    target: Option<Node>,
    link_editor: Option<(Option<BookId>, Link)>,
    editor: bool,
    delete: Option<Node>,
    format: Format,
    map_format: usize,
    opts: OptionsForExport,
    follow_year: bool,
    paper_size: map_core::paper::Paper,
    textures: BTreeMap<BookId, (map_core::underlay::Blob, egui::TextureHandle)>,
}
impl Default for BookUi {
    fn default() -> Self {
        Self {
            open: false,
            pick: None,
            selected: None,
            tab: Tab::All,
            filter: String::new(),
            checked: BTreeSet::new(),
            reading: Vec::new(),
            map_path: vec![ROOT],
            views: BTreeMap::new(),
            target: None,
            link_editor: None,
            editor: false,
            delete: None,
            format: Format::Markdown,
            map_format: 0,
            opts: OptionsForExport {
                events: true,
                maps: false,
                attachments: true,
            },
            follow_year: true,
            paper_size: Default::default(),
            textures: BTreeMap::new(),
        }
    }
}
enum Action {
    Select(Node),
    Open(Node, Option<Option<i32>>),
    Back,
    NewMap(BoardKind, bool),
    NewArticle(bool),
    NewEvent,
    WriteArticle(BookId, Article),
    RenameMap(BookId, String, Meta),
    Event(BookId, map_core::events::Event, Meta),
    RenamePlace(BookId, String),
    SaveLink(Option<BookId>, Link),
    RemoveLink(BookId),
    Attach(BookId),
    Snapshot(BookId),
    Export(Vec<BookId>, bool),
    ExportAssets(Vec<BookId>),
    ExportAsset(BookId),
    RenameAsset(BookId, String),
    Delete(Node),
    SaveMap(BookId),
    ExportMaps(BookId, bool),
    Bundle(BookId),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoardKind {
    Paper,
    Geographic,
    Copy,
}
const MAP_FORMATS: [(map_io::batch::Format, &str); 8] = [
    (map_io::batch::Format::Png, "PNG"),
    (map_io::batch::Format::Svg, "SVG"),
    (map_io::batch::Format::Pdf, "PDF"),
    (map_io::batch::Format::GeoJsonCountries, "GeoJSON 国家"),
    (map_io::batch::Format::GeoJsonFaces, "GeoJSON 地块"),
    (map_io::batch::Format::ShpCountries, "Shapefile 国家"),
    (map_io::batch::Format::ShpFaces, "Shapefile 地块"),
    (map_io::batch::Format::Csv, "CSV"),
];

fn name(book: &WorldBook, doc: &Document, node: Node) -> String {
    let label = book
        .label(doc, book.resolve(doc, node))
        .unwrap_or_else(|| format!("未找到 · {}", node.uri()));
    if let Some(board) = node.board().filter(|_| !matches!(node, Node::Map(_))) {
        return format!(
            "{label}  ·  {}",
            book.map(doc, board)
                .map_or("画板", |d| d.meta.name.as_str())
        );
    }
    label
}
fn metadata(ui: &mut egui::Ui, meta: &mut Meta) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("分类");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut meta.category).desired_width(116.0))
            .changed();
        ui.label("标签");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut meta.tags).desired_width(160.0))
            .changed();
    });
    ui.horizontal_wrapped(|ui| {
        changed |= optional_year(ui, &mut meta.from, "起始年");
        changed |= optional_year(ui, &mut meta.until, "结束年");
    });
    ui.label("说明");
    changed |= ui.text_edit_multiline(&mut meta.description).changed();
    changed
}
fn optional_year(ui: &mut egui::Ui, year: &mut Option<i32>, label: &str) -> bool {
    let mut yes = year.is_some();
    let mut changed = ui.checkbox(&mut yes, label).changed();
    if yes {
        let mut v = year.unwrap_or(0);
        changed |= ui
            .add(egui::DragValue::new(&mut v).range(-1_000_000..=1_000_000))
            .changed();
        *year = Some(v);
    } else {
        *year = None;
    }
    changed
}
fn catalog(
    book: &WorldBook,
    doc: &Document,
    focus: Node,
    all_objects: bool,
) -> Vec<(Node, String)> {
    let mut nodes = BTreeSet::from([focus]);
    nodes.extend(book.articles.keys().map(|id| Node::Article(*id)));
    nodes.extend(book.assets.keys().map(|id| Node::Asset(*id)));
    for id in book.boards.keys().copied() {
        nodes.insert(Node::Map(id));
        if let Some(d) = book.map(doc, id) {
            nodes.extend(d.events.iter().map(|e| Node::Event(id, e.id)));
            if all_objects {
                nodes.extend(d.entities.keys().map(|e| Node::Entity(id, e.0)));
            }
        }
    }
    if all_objects {
        nodes.extend(book.places.iter().map(|(id, p)| Node::Place(p.board, *id)));
        for (_, link) in &book.links {
            nodes.extend([link.from, link.to]);
        }
    }
    let mut result: Vec<_> = nodes
        .into_iter()
        .map(|node| (node, name(book, doc, node)))
        .collect();
    result.sort_by(|a, b| a.1.cmp(&b.1));
    result
}
fn is_in(node: Node, tab: Tab) -> bool {
    match tab {
        Tab::All => true,
        Tab::Maps => matches!(node, Node::Map(_)),
        Tab::Articles => matches!(node, Node::Article(_)),
        Tab::Events => matches!(node, Node::Event(..)),
        Tab::Assets => matches!(node, Node::Asset(_)),
    }
}
fn heading(ui: &mut egui::Ui, name: &str) {
    ui.heading(name);
    ui.separator();
}

impl App {
    pub fn book_context(&mut self, node: Node) {
        self.book_ui.open = true;
        if self.book_ui.selected != Some(node) {
            if let Some(prior) = self.book_ui.selected {
                self.book_ui.reading.push(prior);
            }
            self.book_ui.selected = Some(node);
            self.book_ui.target = None;
            self.book_ui.link_editor = None;
        }
    }
    pub fn book_related(&mut self, node: Node, jump: bool) -> bool {
        let Some(book) = self.doc.book.as_ref() else {
            return false;
        };
        let linked: Vec<_> = book
            .connections_at(&self.doc, node)
            .filter(|(_, l)| !self.book_ui.follow_year || l.visible(self.doc.year))
            .map(|(_, l)| l)
            .collect();
        if linked.is_empty() {
            return false;
        }
        if jump {
            let maps: Vec<_> = linked
                .iter()
                .filter_map(|l| {
                    let outgoing = l.from == node || book.resolve(&self.doc, l.from) == node;
                    let to = if outgoing { l.to } else { l.from };
                    let year = if outgoing {
                        l.year(self.doc.year)
                    } else {
                        None
                    };
                    matches!(to, Node::Map(_)).then_some((to, year))
                })
                .collect();
            if let [(map, year)] = maps.as_slice() {
                self.book_action(Action::Open(*map, *year));
                return true;
            }
        }
        self.book_context(node);
        true
    }
    pub fn book_current_context(&mut self) {
        let board = WorldBook::current(&self.doc);
        let what = self
            .selection()
            .into_iter()
            .next()
            .map(|f| world_book::feature(board, f))
            .or_else(|| self.selected.map(|e| Node::Entity(board, e.0)))
            .or_else(|| self.selected_face.map(|f| Node::Face(board, f.0)))
            .unwrap_or(Node::Map(board));
        self.book_context(what);
    }
    pub fn book_pick(
        &mut self,
        point: LonLat,
        tol: f64,
        feature: Option<map_core::features::FeatureRef>,
    ) -> bool {
        let Some(pick) = self.book_ui.pick.take() else {
            return false;
        };
        let board = WorldBook::current(&self.doc);
        let geo = match pick {
            Pick::Place => self.edit("添加资料地点", |doc| {
                Ok(WorldBook::edit(doc).add_place("新地点".into(), point))
            }),
            Pick::Event(event) => self.edit("标记事件地点", |doc| {
                let book = WorldBook::edit(doc);
                let place = book.add_place("事件地点".into(), point);
                book.add_link(Link::new(place, event, false))?;
                Ok(event)
            }),
            Pick::Edge => self
                .doc
                .topo
                .nearest_on_edge(point, tol)
                .map(|(e, _, _)| Node::Edge(board, e.0)),
            Pick::Face => Some(Node::Face(board, self.doc.topo.face_at(point).0)),
            Pick::Object => feature.map(|f| world_book::feature(board, f)).or_else(|| {
                let face = self.doc.topo.face_at(point);
                self.doc
                    .city_at(point, tol)
                    .or_else(|| self.doc.owner.get(&face).copied())
                    .map(|e| Node::Entity(board, e.0))
                    .or(Some(Node::Face(board, face.0)))
            }),
        };
        if let Some(node) = geo {
            self.book_context(node);
        } else {
            self.set_status("该位置没有可拾取的边界", true);
            self.book_ui.open = true;
        }
        true
    }
    pub fn book_place_at(&self, point: LonLat, tol: f64) -> Option<Node> {
        let book = self.doc.book.as_ref()?;
        book.places
            .iter()
            .filter(|(_, p)| p.board == book.current)
            .filter_map(|(id, p)| {
                let distance = map_core::geom::geo_dist_deg(point, p.point);
                (distance <= tol).then_some((Node::Place(book.current, *id), distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| {
                let mut events = self
                    .book_place_events(book)
                    .filter(|(place, _, _)| *place == x.0);
                match (events.next(), events.next()) {
                    (Some((_, event, _)), None) => event,
                    _ => x.0,
                }
            })
    }
    fn book_place_events<'a>(
        &'a self,
        book: &'a WorldBook,
    ) -> impl Iterator<Item = (Node, Node, &'a str)> {
        book.links.values().filter_map(|link| {
            let (place, target) = match (link.from, link.to) {
                (p @ Node::Place(board, _), e @ Node::Event(_, _))
                | (e @ Node::Event(_, _), p @ Node::Place(board, _))
                    if board == book.current =>
                {
                    (p, e)
                }
                _ => return None,
            };
            let Node::Event(board, id) = target else {
                return None;
            };
            let event = book.map(&self.doc, board)?.event(id)?;
            if self.book_ui.follow_year
                && (!link.visible(self.doc.year)
                    || self
                        .doc
                        .year
                        .is_some_and(|y| !(event.year..=event.last()).contains(&y)))
            {
                return None;
            }
            Some((place, target, event.name.as_str()))
        })
    }
    pub fn draw_book_points(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if let Some(book) = &self.doc.book {
            if book.places.is_empty() {
                return;
            }
            let events: BTreeMap<_, _> = self
                .book_place_events(book)
                .map(|(point, _, label)| (point, label))
                .collect();
            for (id, place) in book.places.iter().filter(|(_, p)| p.board == book.current) {
                if let Some(at) = vg.to_screen(place.point).filter(|p| vg.rect.contains(*p)) {
                    let event = events.get(&Node::Place(book.current, *id));
                    let fill = if event.is_some() {
                        Color32::from_rgb(161, 87, 25)
                    } else {
                        Color32::from_rgb(22, 92, 139)
                    };
                    painter.circle(at, 4.5, fill, egui::Stroke::new(1.0, Color32::WHITE));
                    painter.text(
                        at + egui::vec2(7.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        event.copied().unwrap_or(&place.name),
                        egui::FontId::proportional(11.0),
                        fill,
                    );
                }
            }
        }
    }
    fn save_camera(&mut self, id: BookId) {
        self.book_ui.views.insert(
            id,
            CameraState {
                flat: self.cam2d,
                globe: self.globe,
                layout: self.layout,
                tab: self.tab,
                timeline_year: self.timeline_last_year,
            },
        );
    }
    pub(crate) fn restore_book_view(&mut self, id: BookId) {
        self.reset_board_view();
        if let Some(view) = self.book_ui.views.get(&id) {
            self.cam2d = view.flat;
            self.globe = view.globe;
            self.layout = view.layout;
            self.tab = view.tab;
            self.timeline_last_year = view.timeline_year;
            self.fit_pending = false;
        }
        if let Some(year) = self.doc.year {
            self.timeline_last_year = year;
        }
    }
    fn enter_book_map(
        &mut self,
        id: BookId,
        year: Option<Option<i32>>,
        add_history: bool,
    ) -> Result<(), String> {
        self.end_pending();
        self.finish_pending_settings();
        let prior = WorldBook::current(&self.doc);
        if prior == id {
            if let Some(year) = year {
                self.set_year(year);
            }
            return Ok(());
        }
        self.save_camera(prior);
        WorldBook::switch(&mut self.doc, id, year)?;
        if add_history {
            if let Some(last) = self.book_ui.map_path.iter().position(|item| *item == id) {
                self.book_ui.map_path.truncate(last + 1);
            } else {
                self.book_ui.map_path.push(id);
            }
        }
        self.restore_book_view(id);
        Ok(())
    }
    pub fn book_bar(&mut self, ui: &mut egui::Ui) {
        let mut act = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("世界资料").clicked() {
                self.book_context(Node::Map(WorldBook::current(&self.doc)));
            }
            if self.book_ui.pick.is_some() {
                ui.strong("在地图上取点");
                if ui.button("取消").clicked() {
                    self.book_ui.pick = None;
                    self.book_ui.open = true;
                }
            }
            if let Some(book) = self.doc.book.as_ref() {
                let current = book.current;
                if current != ROOT {
                    if ui.button("返回上层").clicked() {
                        act = Some(Action::Back);
                    }
                    if ui.button("世界地图").clicked() {
                        act = Some(Action::Open(Node::Map(ROOT), None));
                    }
                }
                for id in &self.book_ui.map_path {
                    if let Some(d) = book.map(&self.doc, *id) {
                        if ui.selectable_label(*id == current, &d.meta.name).clicked() {
                            act = Some(Action::Open(Node::Map(*id), None));
                        }
                    }
                }
                if !self.book_ui.map_path.contains(&current) {
                    ui.label(&self.doc.meta.name);
                }
            }
        });
        if let Some(act) = act {
            self.book_action(act);
            ui.ctx().request_repaint();
        }
    }
    pub fn book_window(&mut self, ctx: &egui::Context) {
        if !self.book_ui.open {
            return;
        }
        let mut actions = Vec::new();
        let mut open = true;
        let doc = &self.doc;
        let empty = WorldBook::default();
        let book = doc.book.as_deref().unwrap_or(&empty);
        let state = &mut self.book_ui;
        let node = state.selected.unwrap_or(Node::Map(book.current));
        egui::Window::new("世界资料库")
            .open(&mut open)
            .default_size([880., 630.])
            .scroll(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("新文档").clicked() {
                        actions.push(Action::NewArticle(false));
                    }
                    if ui.button("新事件").clicked() {
                        actions.push(Action::NewEvent);
                    }
                    if ui.button("新画板").clicked() {
                        actions.push(Action::NewMap(BoardKind::Paper, false));
                    }
                    ui.menu_button("画布尺寸", |ui| {
                        ui.label("用于新建平面画布");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut state.paper_size.width)
                                    .range(64..=8192)
                                    .suffix(" px"),
                            );
                            ui.label("×");
                            ui.add(
                                egui::DragValue::new(&mut state.paper_size.height)
                                    .range(64..=8192)
                                    .suffix(" px"),
                            );
                        });
                    });
                    if ui.button("新建地理地图").clicked() {
                        actions.push(Action::NewMap(BoardKind::Geographic, false));
                    }
                    if ui.button("地图取对象…").clicked() {
                        state.pick = Some(Pick::Object);
                    }
                    if !state.reading.is_empty() && ui.button("返回资料").clicked() {
                        if let Some(prior) = state.reading.pop() {
                            state.selected = Some(prior);
                            state.link_editor = None;
                            state.target = None;
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (Tab::All, "全部"),
                        (Tab::Maps, "画板"),
                        (Tab::Articles, "文档"),
                        (Tab::Events, "事件"),
                        (Tab::Assets, "图片与附件"),
                    ] {
                        ui.selectable_value(&mut state.tab, tab, label);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut state.filter)
                            .hint_text("搜索名称、分类和标签")
                            .desired_width(240.0),
                    );
                    ui.checkbox(&mut state.follow_year, "按当前年份筛选");
                });
                let term = state.filter.trim().to_lowercase();
                let items = catalog(book, doc, node, false);
                let selection = state.checked.clone();
                ui.separator();
                ui.columns(2, |columns| {
                    let a = &mut columns[0];
                    egui::ScrollArea::vertical()
                        .id_salt("book-catalog")
                        .max_height(485.0)
                        .show(a, |ui| {
                            for (item, label) in &items {
                                let meta = match item {
                                    Node::Map(id) => book.boards.get(id).map(|b| &b.meta),
                                    Node::Article(id) => book.articles.get(id).map(|a| &a.meta),
                                    Node::Event(b, e) => book.event_meta.get(&(*b, *e)),
                                    _ => None,
                                };
                                let visible = doc.year.is_none_or(|year| {
                                    if let Node::Event(id, e) = *item {
                                        return book
                                            .map(doc, id)
                                            .and_then(|d| d.event(e))
                                            .is_some_and(|event| {
                                                (event.year..=event.last()).contains(&year)
                                            });
                                    }
                                    meta.is_none_or(|m| m.visible(Some(year)))
                                });
                                if !is_in(*item, state.tab)
                                    || (state.follow_year && *item != node && !visible)
                                    || (!term.is_empty()
                                        && !label.to_lowercase().contains(&term)
                                        && !meta.is_some_and(|m| m.matches(&term)))
                                {
                                    continue;
                                }
                                ui.horizontal(|ui| {
                                    if let Node::Article(id) = item {
                                        let mut checked = selection.contains(id);
                                        if ui.checkbox(&mut checked, "").changed() {
                                            if checked {
                                                state.checked.insert(*id);
                                            } else {
                                                state.checked.remove(id);
                                            }
                                        }
                                    }
                                    if ui.selectable_label(*item == node, label).clicked() {
                                        actions.push(Action::Select(*item));
                                    }
                                });
                            }
                        });
                    a.separator();
                    a.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("article-format")
                            .selected_text(state.format.extension())
                            .show_ui(ui, |ui| {
                                for f in Format::ALL {
                                    ui.selectable_value(
                                        &mut state.format,
                                        f,
                                        f.extension().to_uppercase(),
                                    );
                                }
                            });
                        ui.checkbox(&mut state.opts.events, "事件信息");
                        ui.checkbox(&mut state.opts.maps, "关联地图截图");
                        ui.checkbox(&mut state.opts.attachments, "附件目录");
                    });
                    let ids: Vec<_> = state
                        .checked
                        .iter()
                        .copied()
                        .filter(|id| book.articles.contains_key(id))
                        .collect();
                    if !ids.is_empty() {
                        a.horizontal_wrapped(|ui| {
                            if ui.button("合并导出").clicked() {
                                actions.push(Action::Export(ids.clone(), false));
                            }
                            if ui.button("批量逐篇").clicked() {
                                actions.push(Action::Export(ids.clone(), true));
                            }
                            if ui.button("导出附件").clicked() {
                                actions.push(Action::ExportAssets(ids));
                            }
                        });
                    }
                    let pane = &mut columns[1];
                    heading(pane, &name(book, doc, node));
                    match node {
                        Node::Article(id) => {
                            if let Some(article) = book.articles.get(&id) {
                                render::article(pane, doc, book, id, article, state, &mut actions);
                            }
                        }
                        Node::Asset(id) => {
                            if let Some(asset) = book.assets.get(&id) {
                                let mut title = asset.name.clone();
                                if pane.text_edit_singleline(&mut title).changed() {
                                    actions.push(Action::RenameAsset(id, title));
                                }
                                pane.weak(format!("{:.1} KB", asset.bytes.0.len() as f64 / 1024.0));
                                if let Some(texture) = render::texture(pane, book, state, id) {
                                    let size = texture.size_vec2();
                                    let k = (pane.available_width().max(100.0) / size.x)
                                        .min(300.0 / size.y)
                                        .min(1.0);
                                    pane.image((texture.id(), size * k));
                                }
                                if pane.button("单独导出附件").clicked() {
                                    actions.push(Action::ExportAsset(id));
                                }
                            }
                        }
                        Node::Map(id) => {
                            if let (Some(board), Some(d)) =
                                (book.boards.get(&id), book.map(doc, id))
                            {
                                map_panel(
                                    pane,
                                    &mut actions,
                                    state,
                                    id,
                                    d,
                                    board.meta.clone(),
                                    book,
                                );
                            } else if id == ROOT {
                                pane.label("顶层地图");
                            }
                        }
                        Node::Event(id, event) => {
                            if let Some(e) = book.map(doc, id).and_then(|d| d.event(event)) {
                                event_panel(
                                    pane,
                                    &mut actions,
                                    id,
                                    e,
                                    book.event_meta
                                        .get(&(id, event))
                                        .cloned()
                                        .unwrap_or_default(),
                                );
                            }
                        }
                        Node::Place(_, id) => {
                            if let Some(place) = book.places.get(&id) {
                                let mut label = place.name.clone();
                                if pane.text_edit_singleline(&mut label).changed() {
                                    actions.push(Action::RenamePlace(id, label));
                                }
                                pane.label(format!(
                                    "{:.3}°, {:.3}°",
                                    place.point.lon, place.point.lat
                                ));
                            }
                        }
                        _ => {
                            pane.weak(node.uri());
                        }
                    }
                    if let Some(id) = node.board() {
                        if !matches!(node, Node::Map(_)) && pane.button("打开所在画板").clicked()
                        {
                            actions.push(Action::Open(Node::Map(id), None));
                        }
                        if let Node::Event(_, event) = node {
                            if let Some(e) = book.map(doc, id).and_then(|m| m.event(event)) {
                                pane.horizontal_wrapped(|ui| {
                                    if ui.button("查看事件起始年").clicked() {
                                        actions
                                            .push(Action::Open(Node::Map(id), Some(Some(e.year))));
                                    }
                                    if ui.button("在当前地图标记事件…").clicked() {
                                        state.pick = Some(Pick::Event(node));
                                    }
                                });
                            }
                        }
                    }
                    pane.separator();
                    connections(pane, doc, book, node, state, &mut actions);
                    if book.label(doc, node).is_some()
                        && matches!(node, Node::Map(1..) | Node::Article(_))
                    {
                        if pane.small_button("删除此项…").clicked() {
                            state.delete = Some(node);
                        }
                        if state.delete == Some(node) {
                            pane.label("确认删除此项？关联资料与其它画板会保留。可撤销。");
                            pane.horizontal(|ui| {
                                if ui.button("确认删除").clicked() {
                                    actions.push(Action::Delete(node));
                                    state.delete = None;
                                }
                                if ui.button("取消").clicked() {
                                    state.delete = None;
                                }
                            });
                        }
                    }
                });
            });
        state.open = open && state.pick.is_none();
        if !actions.is_empty() {
            ctx.request_repaint();
        }
        for action in actions {
            self.book_action(action);
        }
    }
}
fn map_panel(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    state: &mut BookUi,
    id: BookId,
    doc: &Document,
    mut meta: Meta,
    book: &WorldBook,
) {
    let mut title = doc.meta.name.clone();
    let mut changed = ui.text_edit_singleline(&mut title).changed();
    ui.horizontal_wrapped(|ui| {
        if ui.button("进入画板").clicked() {
            actions.push(Action::Open(Node::Map(id), None));
        }
        if ui.button("复制画板").clicked() {
            actions.push(Action::NewMap(BoardKind::Copy, false));
        }
        if ui.button("另存画板").clicked() {
            actions.push(Action::SaveMap(id));
        }
    });
    if let Some(paper) = doc.paper {
        ui.weak(format!("平面画布 · {} × {} px", paper.width, paper.height));
    } else {
        ui.weak("地理地图");
    }
    let children = book.descendants(id).len().saturating_sub(1);
    ui.label(format!(
        "下属画板 {children} 张 · {}",
        doc.year.map_or("所有年份".into(), |v| v.to_string())
    ));
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("book-map-format")
            .selected_text(MAP_FORMATS[state.map_format].1)
            .show_ui(ui, |ui| {
                for (i, (_, label)) in MAP_FORMATS.iter().enumerate() {
                    ui.selectable_value(&mut state.map_format, i, *label);
                }
            });
        if ui.button("导出画板").clicked() {
            actions.push(Action::ExportMaps(id, false));
        }
        if ui.button("导出下属画板").clicked() {
            actions.push(Action::ExportMaps(id, true));
        }
        if ui.button("画板和资料一起导出").clicked() {
            actions.push(Action::Bundle(id));
        }
    });
    changed |= metadata(ui, &mut meta);
    if changed {
        actions.push(Action::RenameMap(id, title, meta));
    }
}
fn event_panel(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    board: BookId,
    source: &map_core::events::Event,
    mut meta: Meta,
) {
    let mut event = source.clone();
    let mut changed = ui.text_edit_singleline(&mut event.name).changed();
    ui.horizontal(|ui| {
        ui.label("开始年");
        changed |= ui
            .add(egui::DragValue::new(&mut event.year).range(-1_000_000..=1_000_000))
            .changed();
        changed |= optional_year(ui, &mut event.end, "结束年");
    });
    ui.label("事件记述");
    changed |= ui.text_edit_multiline(&mut event.note).changed();
    changed |= metadata(ui, &mut meta);
    if changed {
        actions.push(Action::Event(board, event, meta));
    }
}
fn connections(
    ui: &mut egui::Ui,
    doc: &Document,
    book: &WorldBook,
    node: Node,
    state: &mut BookUi,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("关联").strong());
        if ui.button("新建关联文档").clicked() {
            actions.push(Action::NewArticle(true));
        }
        if ui.button("新建关联画板").clicked() {
            actions.push(Action::NewMap(BoardKind::Paper, true));
        }
        if ui
            .small_button("关联地理地图")
            .on_hover_text("新建带地理投影和球体视图的画板")
            .clicked()
        {
            actions.push(Action::NewMap(BoardKind::Geographic, true));
        }
        ui.menu_button("地图取点", |ui| {
            for (pick, label) in [
                (Pick::Object, "实体或要素"),
                (Pick::Face, "地块"),
                (Pick::Edge, "边界"),
                (Pick::Place, "放置资料地点"),
            ] {
                if ui.button(label).clicked() {
                    state.pick = Some(pick);
                    ui.close();
                }
            }
        });
    });
    let source_year = node
        .board()
        .and_then(|id| book.map(doc, id).map(|d| d.year))
        .unwrap_or(doc.year);
    let mut shown = BTreeSet::new();
    let related = book
        .connections_at(doc, node)
        .chain(book.links.iter().filter_map(|(id, l)| {
            let origin = l.from.board();
            (matches!(node, Node::Map(_)) && l.child && ((origin == node.board()) || l.to == node))
                .then_some((*id, l))
        }));
    for (id, l) in related.filter(|(_, l)| !state.follow_year || l.visible(source_year)) {
        if !shown.insert(id) {
            continue;
        }
        let outgoing = l.from == node
            || book.resolve(doc, l.from) == node
            || (matches!(node, Node::Map(_)) && l.child && l.from.board() == node.board());
        let other = if outgoing { l.to } else { l.from };
        ui.horizontal_wrapped(|ui| {
            ui.weak(if l.child {
                if outgoing {
                    "下层"
                } else {
                    "上层"
                }
            } else {
                "交叉"
            });
            if ui.link(name(book, doc, other)).clicked() {
                actions.push(Action::Open(
                    book.resolve(doc, other),
                    if outgoing { l.year(source_year) } else { None },
                ));
            }
            if !l.label.is_empty() {
                ui.weak(&l.label);
            }
            if ui.small_button("修改").clicked() {
                state.link_editor = Some((Some(id), l.clone()));
            }
            if ui.small_button("解除").clicked() {
                actions.push(Action::RemoveLink(id));
            }
        });
    }
    let all = catalog(book, doc, node, true);
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("book-link-target")
            .selected_text(
                state
                    .target
                    .map_or("选择关联对象".into(), |n| name(book, doc, n)),
            )
            .show_ui(ui, |ui| {
                for (n, label) in all.iter().filter(|(n, _)| *n != node) {
                    ui.selectable_value(&mut state.target, Some(*n), label);
                }
            });
        if let Some(target) = state.target {
            if ui.button("添加链接").clicked() {
                state.link_editor = Some((None, Link::new(node, target, false)));
            }
        }
    });
    if let Some((id, link)) = state.link_editor.as_mut() {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("起点");
                node_menu(ui, "book-link-from", &mut link.from, &all, book, doc);
                ui.label("终点");
                node_menu(ui, "book-link-to", &mut link.to, &all, book, doc);
            });
            ui.checkbox(&mut link.child, "层级链接");
            ui.horizontal_wrapped(|ui| {
                ui.label("链接说明");
                ui.text_edit_singleline(&mut link.label);
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("进入目标画板的年份");
                ui.selectable_value(&mut link.time, TimeLink::Keep, "保留画板年份");
                ui.selectable_value(&mut link.time, TimeLink::Inherit, "继承当前年份");
                if ui
                    .selectable_label(matches!(link.time, TimeLink::Fixed(_)), "固定年")
                    .clicked()
                {
                    link.time = TimeLink::Fixed(source_year.unwrap_or(0));
                }
                if let TimeLink::Fixed(year) = &mut link.time {
                    ui.add(egui::DragValue::new(year).range(-1_000_000..=1_000_000));
                }
            });
            ui.horizontal_wrapped(|ui| {
                optional_year(ui, &mut link.from_year, "显示起年份");
                optional_year(ui, &mut link.until_year, "显示止年份");
            });
            if ui.button("保存链接").clicked() {
                actions.push(Action::SaveLink(*id, link.clone()));
            }
        });
    }
}
fn node_menu(
    ui: &mut egui::Ui,
    id: &str,
    node: &mut Node,
    all: &[(Node, String)],
    book: &WorldBook,
    doc: &Document,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(name(book, doc, *node))
        .show_ui(ui, |ui| {
            for (candidate, label) in all {
                ui.selectable_value(node, *candidate, label);
            }
        });
}

impl App {
    fn book_action(&mut self, action: Action) {
        let selected = self
            .book_ui
            .selected
            .unwrap_or(Node::Map(WorldBook::current(&self.doc)));
        match action {
            Action::Select(node) => self.book_context(node),
            Action::Open(node, year) => {
                if let Some(id) = node.board() {
                    if let Err(e) = self.enter_book_map(id, year, true) {
                        self.set_status(e, true);
                        return;
                    }
                }
                self.book_context(node);
                if let Node::Map(_) = node {
                    self.book_ui.open = false;
                }
                if let Node::Entity(_, id) = node {
                    self.selected = Some(map_core::EntityId(id));
                }
            }
            Action::Back => {
                let id = WorldBook::current(&self.doc);
                if self.book_ui.map_path.last() == Some(&id) {
                    self.book_ui.map_path.pop();
                }
                let target = self
                    .book_ui
                    .map_path
                    .last()
                    .copied()
                    .filter(|id| WorldBook::active_map(&self.doc, *id).is_some())
                    .or_else(|| {
                        self.doc
                            .book
                            .as_ref()
                            .and_then(|b| {
                                b.links.values().find(|l| l.child && l.to == Node::Map(id))
                            })
                            .and_then(|l| l.from.board())
                    })
                    .unwrap_or(ROOT);
                if let Err(e) = self.enter_book_map(target, None, false) {
                    self.set_status(e, true);
                }
                self.book_ui.open = false;
            }
            Action::NewMap(kind, linked) => {
                let mut candidate = if kind == BoardKind::Copy {
                    WorldBook::active_map(
                        &self.doc,
                        selected.board().unwrap_or(WorldBook::current(&self.doc)),
                    )
                    .cloned()
                    .unwrap_or_else(|| self.doc.clone())
                } else if kind == BoardKind::Paper {
                    Document::new_paper("新画板", self.book_ui.paper_size)
                } else {
                    Document::new("新画板", self.doc.radius())
                };
                if kind == BoardKind::Copy {
                    candidate.meta.name = format!("{} · 副本", candidate.meta.name);
                }
                if let Some(id) = self.edit("新建画板", move |d| {
                    if linked {
                        WorldBook::register_anchor(d, selected);
                    }
                    let b = WorldBook::edit(d);
                    let id = b.add_map(candidate);
                    if linked {
                        b.add_link(Link::new(
                            selected,
                            Node::Map(id),
                            selected.board().is_some(),
                        ))?;
                    }
                    Ok(id)
                }) {
                    self.book_context(Node::Map(id));
                }
            }
            Action::NewArticle(linked) => {
                if let Some(id) = self.edit("新建文档", |d| {
                    if linked {
                        WorldBook::register_anchor(d, selected);
                    }
                    let b = WorldBook::edit(d);
                    let id = b.add_article("新文档".into());
                    if linked {
                        b.add_link(Link::new(selected, Node::Article(id), false))?;
                    }
                    Ok(id)
                }) {
                    self.book_ui.editor = true;
                    self.book_context(Node::Article(id));
                }
            }
            Action::NewEvent => {
                let id = WorldBook::current(&self.doc);
                if let Some(event) = self.edit("新建事件", |d| {
                    let y = d.year.unwrap_or(0);
                    let e = d.add_event("新事件", y, None);
                    WorldBook::edit(d);
                    Ok(e)
                }) {
                    self.event_sel = Some(event);
                    self.book_context(Node::Event(id, event));
                }
            }
            Action::WriteArticle(id, a) => {
                self.edit("编辑资料文档", |d| {
                    let b = WorldBook::edit(d);
                    if !b.articles.contains_key(&id) {
                        return Err("文档已不存在".into());
                    }
                    b.articles.insert(id, Arc::new(a));
                    Ok(())
                });
            }
            Action::RenameMap(id, title, m) => {
                self.edit("编辑画板属性", |d| {
                    board_mut(d, id, |d| {
                        d.meta.name = title;
                        Ok(())
                    })?;
                    WorldBook::edit(d)
                        .boards
                        .get_mut(&id)
                        .ok_or("画板已不存在")?
                        .meta = m;
                    Ok(())
                });
            }
            Action::Event(id, event, meta) => {
                let eid = event.id;
                self.edit("编辑事件资料", move |d| {
                    board_mut(d, id, |map| map.update_event(event))?;
                    WorldBook::edit(d).event_meta.insert((id, eid), meta);
                    Ok(())
                });
            }
            Action::RenamePlace(id, label) => {
                self.edit("编辑地点", |d| {
                    WorldBook::edit(d)
                        .places
                        .get_mut(&id)
                        .ok_or("地点已不存在")?
                        .name = label;
                    Ok(())
                });
            }
            Action::SaveLink(id, link) => {
                if self
                    .edit("保存资料链接", |d| {
                        WorldBook::register_anchor(d, link.from);
                        WorldBook::register_anchor(d, link.to);
                        let b = WorldBook::edit(d);
                        if let Some(id) = id {
                            b.update_link(id, link)
                        } else {
                            b.add_link(link).map(|_| ())
                        }
                    })
                    .is_some()
                {
                    self.book_ui.link_editor = None;
                    self.book_ui.target = None;
                }
            }
            Action::RemoveLink(id) => {
                self.edit("解除资料链接", |d| {
                    WorldBook::edit(d).links.remove(&id);
                    Ok(())
                });
            }
            Action::Attach(id) => {
                if let Some(path) = self.file_dialog().pick_file() {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let title = path
                                .file_name()
                                .and_then(|x| x.to_str())
                                .unwrap_or("附件")
                                .to_owned();
                            self.attach_book_file(id, title, bytes);
                        }
                        Err(e) => self.set_status(e.to_string(), true),
                    }
                }
            }
            Action::Snapshot(id) => {
                let (width, height) = self.doc.paper.map_or((1100, 680), |p| (p.width, p.height));
                let opts = map_io::scene::ExportOptions {
                    width,
                    height,
                    title: self.doc.meta.name.clone(),
                    ..Default::default()
                };
                let scene = map_io::scene::build_scene(
                    &self.doc,
                    &map_core::theme::evaluate(&self.doc),
                    &opts,
                );
                let bytes = map_io::png::render(&scene, map_io::png::find_system_font().as_deref())
                    .and_then(|p| p.encode_png().map_err(|e| e.to_string()));
                match bytes {
                    Ok(b) => self.attach_book_file(id, format!("{}.png", self.doc.meta.name), b),
                    Err(e) => self.set_status(e, true),
                }
            }
            Action::Export(ids, batch) => self.export_book_articles(&ids, batch),
            Action::ExportAssets(ids) => {
                if let Some(dir) = self.file_dialog().pick_folder() {
                    match dossier::export_attachments(&self.doc, &ids, &dir) {
                        Ok(n) => self.set_status(format!("已导出 {n} 个附件"), false),
                        Err(e) => self.set_status(e, true),
                    }
                }
            }
            Action::ExportAsset(id) => {
                if let Some(asset) = self
                    .doc
                    .book
                    .as_ref()
                    .and_then(|b| b.assets.get(&id))
                    .cloned()
                {
                    if let Some(path) = self
                        .file_dialog()
                        .set_file_name(dossier::unique_name(&asset.name, id))
                        .save_file()
                    {
                        if same_project(&self.path, &path) {
                            self.set_status("请为附件指定其他路径", true);
                        } else {
                            self.book_result(
                                map_io::recovery::write_atomic(&path, &asset.bytes.0)
                                    .map_err(|e| e.to_string()),
                                "已导出附件",
                            );
                        }
                    }
                }
            }
            Action::RenameAsset(id, title) => {
                self.edit("重命名资料附件", |d| {
                    WorldBook::edit(d)
                        .assets
                        .get_mut(&id)
                        .ok_or("附件已不存在")?
                        .name = title;
                    Ok(())
                });
            }
            Action::Delete(node) => {
                let current = WorldBook::current(&self.doc);
                if self
                    .edit("删除资料项目", |d| {
                        match node {
                            Node::Article(id) => {
                                WorldBook::edit(d).articles.remove(&id);
                            }
                            Node::Map(id) if id != ROOT => {
                                if current == id {
                                    WorldBook::switch(d, ROOT, None)?;
                                }
                                WorldBook::edit(d).boards.remove(&id);
                            }
                            _ => return Err("此处只删除文档或独立画板".into()),
                        }
                        Ok(())
                    })
                    .is_some()
                {
                    let next = WorldBook::current(&self.doc);
                    if current != next {
                        self.restore_book_view(next);
                    }
                    self.book_context(Node::Map(next));
                }
            }
            Action::SaveMap(id) => {
                if let Some(map) = WorldBook::active_map(&self.doc, id) {
                    let mut flat = map.clone();
                    flat.book = None;
                    if let Some(path) = self
                        .file_dialog()
                        .add_filter("Atlas 画板", &["atlas"])
                        .set_file_name(format!(
                            "{}.atlas",
                            dossier::unique_name(&flat.meta.name, id)
                        ))
                        .save_file()
                    {
                        if same_project(&self.path, &path) {
                            self.set_status("请选择不同于当前工程的画板文件", true);
                        } else {
                            self.book_result(map_io::project::save(&path, &flat), "已另存独立画板");
                        }
                    }
                }
            }
            Action::ExportMaps(id, children) => self.export_book_maps(id, children, false),
            Action::Bundle(id) => self.export_book_maps(id, true, true),
        }
    }
    fn attach_book_file(&mut self, id: BookId, title: String, bytes: Vec<u8>) {
        self.edit("插入资料附件", move |d| {
            let b = WorldBook::edit(d);
            let is_image = image::guess_format(&bytes).is_ok();
            let aid = b.add_asset(Attachment {
                name: title.clone(),
                bytes: map_core::underlay::Blob(Arc::new(bytes)),
            });
            let a = Arc::make_mut(b.articles.get_mut(&id).ok_or("文档已不存在")?);
            a.attachments.insert(aid);
            let caption = title.replace(['[', ']', '\n', '\r'], " ");
            a.markdown.push_str(&format!(
                "\n\n{}[{caption}](atlas-asset:{aid})\n",
                if is_image { "!" } else { "" }
            ));
            Ok(())
        });
    }
    fn book_result(&mut self, result: Result<(), String>, ok: &str) {
        match result {
            Ok(()) => self.set_status(ok, false),
            Err(e) => self.set_status(format!("导出失败：{e}"), true),
        }
    }
    fn export_book_articles(&mut self, ids: &[BookId], batch: bool) {
        let format = self.book_ui.format;
        let opts = self.book_ui.opts;
        let Some(book) = &self.doc.book else { return };
        if batch {
            if let Some(dir) = self.file_dialog().pick_folder() {
                let result = ids.iter().try_for_each(|id| {
                    let a = book.articles.get(id).ok_or("文档已不存在")?;
                    dossier::export(
                        &self.doc,
                        &[*id],
                        format,
                        opts,
                        &dir.join(format!(
                            "{}.{}",
                            dossier::unique_name(&a.name, *id),
                            format.extension()
                        )),
                    )
                });
                self.book_result(result, "已批量导出文档");
            }
        } else {
            let stem = if ids.len() == 1 {
                book.articles
                    .get(&ids[0])
                    .map(|a| dossier::unique_name(&a.name, ids[0]))
                    .unwrap_or_default()
            } else {
                "世界资料集".into()
            };
            if let Some(path) = self
                .file_dialog()
                .add_filter("资料文档", &[format.extension()])
                .set_file_name(format!("{stem}.{}", format.extension()))
                .save_file()
            {
                if same_project(&self.path, &path) {
                    self.set_status("请为资料文档指定其他路径", true);
                    return;
                }
                self.book_result(
                    dossier::export(&self.doc, ids, format, opts, &path),
                    "已导出资料",
                );
            }
        }
    }
    fn export_book_maps(&mut self, id: BookId, children: bool, bundle: bool) {
        let Some(dir) = self.file_dialog().pick_folder() else {
            return;
        };
        let ids = if children {
            self.doc
                .book
                .as_ref()
                .map_or(vec![id], |b| b.descendants(id))
        } else {
            vec![id]
        };
        let dir = if bundle { dir.join("地图") } else { dir };
        let format = MAP_FORMATS[self.book_ui.map_format].0;
        let result = dossier::export_maps(&self.doc, &ids, format, &dir).and_then(|_| {
            if !bundle {
                return Ok(());
            }
            let Some(book) = &self.doc.book else {
                return Ok(());
            };
            let mut articles = BTreeSet::new();
            for l in book.links.values() {
                match (l.from, l.to) {
                    (Node::Article(a), other) | (other, Node::Article(a))
                        if other.board().is_some_and(|id| ids.contains(&id)) =>
                    {
                        if book.articles.contains_key(&a) {
                            articles.insert(a);
                        }
                    }
                    _ => {}
                }
            }
            if articles.is_empty() {
                return Ok(());
            }
            let fmt = self.book_ui.format;
            dossier::export(
                &self.doc,
                &articles.into_iter().collect::<Vec<_>>(),
                fmt,
                self.book_ui.opts,
                &dir.parent()
                    .unwrap()
                    .join(format!("关联资料.{}", fmt.extension())),
            )
        });
        self.book_result(
            result,
            if bundle {
                "已导出地图和资料"
            } else {
                "已导出画板"
            },
        );
    }
}
fn board_mut<T>(
    doc: &mut Document,
    id: BookId,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    if WorldBook::current(doc) == id {
        return edit(doc);
    }
    let board = WorldBook::edit(doc)
        .boards
        .get_mut(&id)
        .and_then(|b| b.doc.as_mut())
        .ok_or("找不到画板")?;
    edit(Arc::make_mut(board))
}
fn same_project(project: &Option<PathBuf>, path: &std::path::Path) -> bool {
    project.as_ref().is_some_and(|p| {
        p == path
            || std::fs::canonicalize(p)
                .ok()
                .zip(std::fs::canonicalize(path).ok())
                .is_some_and(|(a, b)| a == b)
    })
}
