//! A flat project graph. IDs are project-local and stable. The active map
//! lives in Document; all other boards contain complete ordinary documents.
use crate::doc::{Document, EntityId, LineId};
use crate::features::FeatureRef;
use crate::geom::LonLat;
use crate::topo::{EdgeId, FaceId};
use crate::underlay::Blob;
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

pub type BookId = u64;
pub const ROOT: BookId = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Node {
    Map(BookId),
    Article(BookId),
    Asset(BookId),
    Event(BookId, u32),
    Entity(BookId, u64),
    Face(BookId, u64),
    Edge(BookId, u64),
    Feature(BookId, u8, u64),
    Place(BookId, BookId),
}

impl Node {
    pub fn board(self) -> Option<BookId> {
        match self {
            Self::Map(id)
            | Self::Event(id, _)
            | Self::Entity(id, _)
            | Self::Face(id, _)
            | Self::Edge(id, _)
            | Self::Feature(id, _, _)
            | Self::Place(id, _) => Some(id),
            Self::Article(_) | Self::Asset(_) => None,
        }
    }
    pub fn uri(self) -> String {
        match self {
            Self::Map(id) => format!("atlas-map:{id}"),
            Self::Article(id) => format!("atlas-doc:{id}"),
            Self::Asset(id) => format!("atlas-media:{id}"),
            Self::Event(map, id) => format!("atlas-event:{map}/{id}"),
            Self::Entity(map, id) => format!("atlas-entity:{map}/{id}"),
            Self::Face(map, id) => format!("atlas-face:{map}/{id}"),
            Self::Edge(map, id) => format!("atlas-edge:{map}/{id}"),
            Self::Feature(map, kind, id) => format!("atlas-feature:{map}/{kind}/{id}"),
            Self::Place(map, id) => format!("atlas-place:{map}/{id}"),
        }
    }
    pub fn parse(uri: &str) -> Option<Self> {
        let (kind, key) = uri.split_once(':')?;
        Some(match kind {
            "atlas-map" => Self::Map(key.parse().ok()?),
            "atlas-doc" => Self::Article(key.parse().ok()?),
            "atlas-media" => Self::Asset(key.parse().ok()?),
            _ => {
                let (map, rest) = key.split_once('/')?;
                let map = map.parse().ok()?;
                match kind {
                    "atlas-event" => Self::Event(map, rest.parse().ok()?),
                    "atlas-entity" => Self::Entity(map, rest.parse().ok()?),
                    "atlas-face" => Self::Face(map, rest.parse().ok()?),
                    "atlas-edge" => Self::Edge(map, rest.parse().ok()?),
                    "atlas-place" => Self::Place(map, rest.parse().ok()?),
                    "atlas-feature" => {
                        let (kind, id) = rest.split_once('/')?;
                        Self::Feature(map, kind.parse().ok()?, id.parse().ok()?)
                    }
                    _ => return None,
                }
            }
        })
    }
}

pub fn feature(board: BookId, feature: FeatureRef) -> Node {
    let (kind, id) = match feature {
        FeatureRef::River(id) => (0, id),
        FeatureRef::Area(id) => (1, id),
        FeatureRef::Text(id) => (2, id),
        FeatureRef::Symbol(id) => (3, id),
        FeatureRef::Route(id) => (4, id),
    };
    Node::Feature(board, kind, id.0)
}
fn unpack_feature(kind: u8, id: u64) -> Option<FeatureRef> {
    let id = LineId(id);
    Some(match kind {
        0 => FeatureRef::River(id),
        1 => FeatureRef::Area(id),
        2 => FeatureRef::Text(id),
        3 => FeatureRef::Symbol(id),
        4 => FeatureRef::Route(id),
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub category: String,
    pub tags: String,
    pub description: String,
    pub from: Option<i32>,
    pub until: Option<i32>,
}

impl Meta {
    pub fn visible(&self, year: Option<i32>) -> bool {
        year.is_none_or(|y| self.from.is_none_or(|v| y >= v) && self.until.is_none_or(|v| y <= v))
    }
    pub fn matches(&self, term: &str) -> bool {
        [&self.category, &self.tags, &self.description]
            .into_iter()
            .any(|v| v.to_lowercase().contains(term))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Article {
    pub name: String,
    pub markdown: String,
    pub meta: Meta,
    pub attachments: BTreeSet<BookId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub bytes: Blob,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub board: BookId,
    pub name: String,
    pub point: LonLat,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub doc: Option<Arc<Document>>,
    pub meta: Meta,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeLink {
    #[default]
    Keep,
    Inherit,
    Fixed(i32),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: Node,
    pub to: Node,
    pub child: bool,
    pub label: String,
    pub time: TimeLink,
    pub from_year: Option<i32>,
    pub until_year: Option<i32>,
}

impl Link {
    pub fn new(from: Node, to: Node, child: bool) -> Self {
        Self {
            from,
            to,
            child,
            label: String::new(),
            time: TimeLink::Keep,
            from_year: None,
            until_year: None,
        }
    }
    pub fn visible(&self, year: Option<i32>) -> bool {
        year.is_none_or(|y| {
            self.from_year.is_none_or(|v| y >= v) && self.until_year.is_none_or(|v| y <= v)
        })
    }
    pub fn year(&self, source: Option<i32>) -> Option<Option<i32>> {
        match self.time {
            TimeLink::Keep => None,
            TimeLink::Inherit => Some(source),
            TimeLink::Fixed(y) => Some(Some(y)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldBook {
    pub current: BookId,
    next: BookId,
    pub boards: OrdMap<BookId, Board>,
    pub articles: OrdMap<BookId, Arc<Article>>,
    pub assets: OrdMap<BookId, Attachment>,
    pub places: OrdMap<BookId, Place>,
    pub links: OrdMap<BookId, Link>,
    pub event_meta: OrdMap<(BookId, u32), Meta>,
    #[serde(default, skip_serializing_if = "OrdMap::is_empty")]
    pub anchors: OrdMap<Node, LonLat>,
}

impl Default for WorldBook {
    fn default() -> Self {
        Self {
            current: ROOT,
            next: 1,
            boards: std::iter::once((ROOT, Board::default())).collect(),
            articles: OrdMap::new(),
            assets: OrdMap::new(),
            places: OrdMap::new(),
            links: OrdMap::new(),
            event_meta: OrdMap::new(),
            anchors: OrdMap::new(),
        }
    }
}
impl WorldBook {
    pub fn current(doc: &Document) -> BookId {
        doc.book.as_ref().map_or(ROOT, |b| b.current)
    }
    pub fn edit(doc: &mut Document) -> &mut Self {
        Arc::make_mut(doc.book.get_or_insert_with(|| Arc::new(Self::default())))
    }
    pub(crate) fn retained_bytes(before: &Document, after: &Document) -> usize {
        use imbl::ordmap::DiffItem;
        let Some(old) = &before.book else { return 0 };
        if after.book.as_ref().is_some_and(|new| Arc::ptr_eq(old, new)) {
            return 0;
        }
        let empty = Self::default();
        let new = after.book.as_deref().unwrap_or(&empty);
        let articles: usize = old
            .articles
            .diff(&new.articles)
            .map(|change| match change {
                DiffItem::Remove(_, article)
                | DiffItem::Update {
                    old: (_, article), ..
                } => {
                    article.markdown.capacity()
                        + article.name.capacity()
                        + article.meta.description.capacity()
                }
                DiffItem::Add(_, _) => 0,
            })
            .sum();
        let assets: usize = old
            .assets
            .diff(&new.assets)
            .map(|change| match change {
                DiffItem::Remove(_, asset) => asset.bytes.0.capacity(),
                DiffItem::Update {
                    old: (_, asset),
                    new: (_, updated),
                } if !Arc::ptr_eq(&asset.bytes.0, &updated.bytes.0) => asset.bytes.0.capacity(),
                _ => 0,
            })
            .sum();
        articles + assets
    }
    fn alloc(&mut self) -> BookId {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("project id space exhausted");
        id
    }
    pub fn map<'a>(&'a self, current: &'a Document, id: BookId) -> Option<&'a Document> {
        if self.current == id {
            Some(current)
        } else {
            self.boards.get(&id)?.doc.as_deref()
        }
    }
    pub fn active_map<'a>(doc: &'a Document, id: BookId) -> Option<&'a Document> {
        if let Some(book) = &doc.book {
            book.map(doc, id)
        } else {
            (id == ROOT).then_some(doc)
        }
    }
    pub fn add_map(&mut self, mut doc: Document) -> BookId {
        doc.book = None;
        let id = self.alloc();
        self.boards.insert(
            id,
            Board {
                doc: Some(Arc::new(doc)),
                meta: Meta::default(),
            },
        );
        id
    }
    pub fn add_article(&mut self, name: String) -> BookId {
        let id = self.alloc();
        self.articles.insert(
            id,
            Arc::new(Article {
                name,
                ..Default::default()
            }),
        );
        id
    }
    pub fn add_asset(&mut self, asset: Attachment) -> BookId {
        let id = self.alloc();
        self.assets.insert(id, asset);
        id
    }
    pub fn add_place(&mut self, name: String, point: LonLat) -> Node {
        let id = self.alloc();
        self.places.insert(
            id,
            Place {
                board: self.current,
                name,
                point,
            },
        );
        Node::Place(self.current, id)
    }
    pub fn switch(doc: &mut Document, id: BookId, year: Option<Option<i32>>) -> Result<(), String> {
        let active = Self::current(doc);
        if active != id {
            if doc
                .book
                .as_ref()
                .and_then(|b| b.boards.get(&id))
                .and_then(|b| b.doc.as_ref())
                .is_none()
            {
                return Err("找不到目标画板".into());
            }
            let mut archive = doc.book.take().unwrap();
            let book = Arc::make_mut(&mut archive);
            let next = book.boards.get_mut(&id).unwrap().doc.take().unwrap();
            let previous = std::mem::replace(doc, (*next).clone());
            book.boards.get_mut(&active).unwrap().doc = Some(Arc::new(previous));
            book.current = id;
            doc.book = Some(archive);
        }
        if let Some(year) = year {
            doc.set_year(year);
        }
        Ok(())
    }
    pub fn label(&self, doc: &Document, node: Node) -> Option<String> {
        Some(match node {
            Node::Map(id) => self.map(doc, id)?.meta.name.clone(),
            Node::Article(id) => self.articles.get(&id)?.name.clone(),
            Node::Asset(id) => self.assets.get(&id)?.name.clone(),
            Node::Event(id, event) => self.map(doc, id)?.event(event)?.name.clone(),
            Node::Entity(id, e) => self.map(doc, id)?.entities.get(&EntityId(e))?.name.clone(),
            Node::Face(id, f) => {
                self.map(doc, id)?.topo.faces.get(&FaceId(f))?;
                format!("地块 {f}")
            }
            Node::Edge(id, edge) => {
                self.map(doc, id)?.topo.edges.get(&EdgeId(edge))?;
                format!("边界 {edge}")
            }
            Node::Feature(id, kind, f) => {
                let board = self.map(doc, id)?;
                let reference = unpack_feature(kind, f)?;
                let selected = board
                    .features()
                    .into_iter()
                    .find(|info| info.what == reference)?;
                if selected.name.is_empty() {
                    reference.kind_name().to_owned()
                } else {
                    selected.name
                }
            }
            Node::Place(board, id) => self
                .places
                .get(&id)
                .filter(|p| p.board == board)?
                .name
                .clone(),
        })
    }
    pub fn connections(&self, node: Node) -> impl Iterator<Item = (BookId, &Link)> {
        self.links
            .iter()
            .filter(move |(_, l)| l.from == node || l.to == node)
            .map(|(id, l)| (*id, l))
    }
    pub fn anchor(doc: &Document, node: Node) -> Option<LonLat> {
        let map = Self::active_map(doc, node.board()?)?;
        match node {
            Node::Face(_, id) if id != 0 => {
                let face = FaceId(id);
                map.topo.faces.get(&face)?;
                let (outer, holes) = map.topo.face_planar(face);
                let ([x, y], _) = crate::geom::interior_point(&outer, &holes)?;
                Some(LonLat::new(crate::geom::wrap_lon(x), y))
            }
            Node::Edge(_, id) => {
                let points = map.topo.edge_full(map.topo.edges.get(&EdgeId(id))?);
                let i = points.len().checked_sub(2)? / 2;
                let (a, b) = (points[i], points[i + 1]);
                Some(LonLat::new(
                    crate::geom::wrap_lon(a.lon + crate::geom::lon_delta(a.lon, b.lon) * 0.5),
                    (a.lat + b.lat) * 0.5,
                ))
            }
            _ => None,
        }
    }
    pub fn resolve(&self, doc: &Document, node: Node) -> Node {
        let Some(point) = self.anchors.get(&node) else {
            return node;
        };
        let Some(map) = node.board().and_then(|id| self.map(doc, id)) else {
            return node;
        };
        match node {
            Node::Face(board, id) if !map.topo.faces.contains_key(&FaceId(id)) => {
                let face = map.topo.face_at(*point);
                if face.0 != 0 {
                    Node::Face(board, face.0)
                } else {
                    node
                }
            }
            Node::Edge(board, id) if !map.topo.edges.contains_key(&EdgeId(id)) => map
                .topo
                .nearest_on_edge(*point, 0.0001)
                .map_or(node, |(edge, _, _)| Node::Edge(board, edge.0)),
            _ => node,
        }
    }
    pub fn connections_at<'a>(
        &'a self,
        doc: &'a Document,
        node: Node,
    ) -> impl Iterator<Item = (BookId, &'a Link)> {
        self.links
            .iter()
            .filter(move |(_, link)| {
                self.resolve(doc, link.from) == node
                    || self.resolve(doc, link.to) == node
                    || link.from == node
                    || link.to == node
            })
            .map(|(id, link)| (*id, link))
    }
    pub fn register_anchor(doc: &mut Document, node: Node) {
        if let Some(anchor) = Self::anchor(doc, node) {
            Self::edit(doc).anchors.insert(node, anchor);
        }
    }
    pub fn add_link(&mut self, link: Link) -> Result<BookId, String> {
        self.check_link(&link, None)?;
        let id = self.alloc();
        self.links.insert(id, link);
        Ok(id)
    }
    pub fn update_link(&mut self, id: BookId, link: Link) -> Result<(), String> {
        if !self.links.contains_key(&id) {
            return Err("链接已解除".into());
        }
        self.check_link(&link, Some(id))?;
        self.links.insert(id, link);
        Ok(())
    }
    fn check_link(&self, link: &Link, except: Option<BookId>) -> Result<(), String> {
        if link.from == link.to {
            return Err("链接需要两个不同的对象".into());
        }
        if matches!((link.from_year,link.until_year),(Some(a),Some(b)) if a>b) {
            return Err("链接结束年早于开始年".into());
        }
        if link.child {
            let (Some(parent), Node::Map(child)) = (link.from.board(), link.to) else {
                return Err("层级链接需从画板、地图对象或事件指向一张画板".into());
            };
            if child == ROOT || self.descendants_except(child, except).contains(&parent) {
                return Err("该层级链接会形成环路，请使用交叉链接".into());
            }
        }
        Ok(())
    }
    fn descendants_except(&self, root: BookId, except: Option<BookId>) -> Vec<BookId> {
        // Expand each board at most once. Shared children are exported once.
        let mut by_parent = std::collections::BTreeMap::<BookId, Vec<BookId>>::new();
        for (_, l) in self
            .links
            .iter()
            .filter(|(id, l)| Some(**id) != except && l.child)
        {
            match (l.from.board(), l.to) {
                (Some(parent), Node::Map(child))
                    if self.boards.contains_key(&parent) && self.boards.contains_key(&child) =>
                {
                    by_parent.entry(parent).or_default().push(child);
                }
                _ => {}
            }
        }
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        let mut pending = VecDeque::from([root]);
        while let Some(board) = pending.pop_front() {
            if !seen.insert(board) {
                continue;
            }
            result.push(board);
            if let Some(children) = by_parent.get(&board) {
                pending.extend(children.iter().copied());
            }
        }
        result
    }
    pub fn descendants(&self, root: BookId) -> Vec<BookId> {
        self.descendants_except(root, None)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.boards.contains_key(&ROOT) || !self.boards.contains_key(&self.current) {
            return Err("资料工程缺少活动画板或顶层地图".into());
        }
        if self.boards.iter().any(|(id, b)| {
            if *id == self.current {
                b.doc.is_some()
            } else {
                b.doc.as_ref().is_none_or(|d| d.book.is_some())
            }
        }) {
            return Err("资料工程画板快照不完整".into());
        }
        let max = self
            .boards
            .keys()
            .chain(self.articles.keys())
            .chain(self.assets.keys())
            .chain(self.places.keys())
            .chain(self.links.keys())
            .copied()
            .max()
            .unwrap_or(0);
        if self.next <= max {
            return Err("资料工程标识序列无效".into());
        }
        Ok(())
    }
}
