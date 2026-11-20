//! Shared authored visualization rules and evaluated data for both desktop shells.
use crate::{
    doc::EntityId,
    environment::{Field, Layer},
    expr, geom, measure,
    stats::Stats,
    theme::{self, ClassMethod, LegendItem, ThemeResult},
    topo::{FaceId, BACKGROUND},
    world_book::{Node, WorldBook},
    worldview::{Catalog, Evaluation, Id},
    Document, FaceKind, LonLat,
};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plot {
    #[default]
    Bars,
    Curve,
    Scatter,
    Relations,
    Profile,
    Symbols,
    Choropleth,
}

impl Plot {
    pub const ALL: [Self; 7] = [
        Self::Bars,
        Self::Curve,
        Self::Scatter,
        Self::Relations,
        Self::Profile,
        Self::Symbols,
        Self::Choropleth,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Bars => "统计柱状图",
            Self::Curve => "曲线图",
            Self::Scatter => "散点图",
            Self::Relations => "关系网络",
            Self::Profile => "地形与环境剖面",
            Self::Symbols => "地图图标",
            Self::Choropleth => "专题地图",
        }
    }

    pub fn geographic(self) -> bool {
        matches!(self, Self::Symbols | Self::Choropleth)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Icon {
    #[default]
    Circle,
    Diamond,
    Triangle,
    Square,
}

impl Icon {
    pub const ALL: [Self; 4] = [Self::Circle, Self::Diamond, Self::Triangle, Self::Square];
    pub fn name(self) -> &'static str {
        match self {
            Self::Circle => "圆形",
            Self::Diamond => "菱形",
            Self::Triangle => "三角形",
            Self::Square => "方形",
        }
    }

    pub fn vertices(self, radius: f32) -> Vec<[f32; 2]> {
        let (n, start) = match self {
            Self::Circle => (12, 0.0),
            Self::Diamond => (4, 0.0),
            Self::Triangle => (3, 0.0),
            Self::Square => (4, std::f32::consts::FRAC_PI_4),
        };
        (0..n)
            .map(|i| {
                let angle = std::f32::consts::TAU * i as f32 / n as f32 + start;
                [radius * angle.sin(), -radius * angle.cos()]
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub label: String,
    pub x: f64,
    pub value: f64,
    pub point: Option<LonLat>,
    pub node: Option<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Years {
    pub from: i32,
    pub until: i32,
    pub step: u32,
}

impl Default for Years {
    fn default() -> Self {
        Self {
            from: 1800,
            until: 2000,
            step: 10,
        }
    }
}

impl Years {
    fn indices(self) -> Result<Vec<i32>, String> {
        let len = i64::from(self.until) - i64::from(self.from);
        if self.step == 0 || len < 0 || len.div_euclid(i64::from(self.step)) > 5000 {
            return Err("时间范围、采样间隔或采样数量无效。".into());
        }
        let mut years = (self.from..=self.until)
            .step_by(self.step as usize)
            .collect::<Vec<_>>();
        if years.last() != Some(&self.until) {
            years.push(self.until);
        }
        Ok(years)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Data {
    Rows(Vec<Row>),
    MapField {
        key: String,
        top_level: bool,
    },
    MapHistory {
        key: String,
        entity: EntityId,
        years: Years,
    },
    Property {
        schema: Option<Id>,
        key: String,
    },
    RecordHistory {
        record: Id,
        key: String,
        years: Years,
    },
    Network {
        schema: Option<Id>,
        relation: Option<Id>,
    },
    Transect {
        from: LonLat,
        to: LonLat,
        layer: Layer,
        samples: u16,
    },
}

impl Default for Data {
    fn default() -> Self {
        Self::Rows(Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub expression: String,
    pub ramp: Vec<[u8; 4]>,
    pub bounds: Option<[f64; 2]>,
    pub breaks: Vec<f64>,
    pub icon: Icon,
    pub radii: [f32; 2],
    pub map_labels: bool,
    pub unit: String,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            expression: "value".into(),
            ramp: vec![
                [217, 237, 229, 255],
                [106, 173, 157, 255],
                [18, 99, 113, 255],
            ],
            bounds: None,
            breaks: Vec::new(),
            icon: Icon::Circle,
            radii: [3.0, 14.0],
            map_labels: false,
            unit: String::new(),
        }
    }
}

impl Rule {
    fn ast(&self) -> Result<expr::Ast, String> {
        if self.expression.len() > 512 {
            return Err("可视化公式最多可输入 512 字节。".into());
        }
        let ast = expr::parse(&self.expression)?;
        let mut vars = Vec::new();
        expr::vars(&ast, &mut vars);
        if vars
            .iter()
            .any(|n| !matches!(n.as_str(), "x" | "value" | "year"))
        {
            return Err("可视化公式可使用 x、value 和 year。".into());
        }
        Ok(ast)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.ast()?;
        if !(2..=12).contains(&self.ramp.len())
            || self
                .bounds
                .is_some_and(|[a, b]| !a.is_finite() || !b.is_finite() || a >= b)
            || self.breaks.len() > 24
            || self.breaks.iter().any(|x| !x.is_finite())
            || self.radii.iter().any(|r| !(1.0..=48.0).contains(r))
            || self.radii[0] > self.radii[1]
        {
            return Err("可视化规则的颜色、区间或符号尺寸无效。".into());
        }
        Ok(())
    }

    pub fn scale(&self, rows: &[Row]) -> [f64; 2] {
        self.bounds.unwrap_or_else(|| {
            let [lo, hi] = rows
                .iter()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[a, b], row| {
                    [a.min(row.value), b.max(row.value)]
                });
            if lo.is_finite() {
                [lo, hi]
            } else {
                [0.0, 1.0]
            }
        })
    }

    fn fraction(value: f64, [low, high]: [f64; 2]) -> f64 {
        if high == low {
            0.5
        } else {
            ((value - low) / (high - low)).clamp(0.0, 1.0)
        }
    }

    pub fn color(&self, value: f64, bounds: [f64; 2]) -> [u8; 4] {
        let t = Self::fraction(value, bounds);
        theme::sample_ramp(&self.ramp, t)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub name: String,
    pub plot: Plot,
    pub data: Data,
    pub rule: Rule,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    last: Id,
    pub specs: OrdMap<Id, Arc<Spec>>,
    pub overlay: Option<Id>,
}

impl Project {
    pub fn insert(&mut self, id: Option<Id>, spec: Spec) -> Result<Id, String> {
        spec.validate()?;
        let id = if let Some(id) = id {
            if !self.specs.contains_key(&id) {
                return Err("该可视化条目已经移除。".into());
            }
            id
        } else {
            self.last = self.last.checked_add(1).ok_or("可视化标识已用尽。")?;
            self.last
        };
        if !spec.plot.geographic() && self.overlay == Some(id) {
            self.overlay = None;
        }
        self.specs.insert(id, Arc::new(spec));
        Ok(id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self
            .overlay
            .is_some_and(|id| self.specs.get(&id).is_none_or(|s| !s.plot.geographic()))
            || self.specs.keys().any(|&id| id == 0 || id > self.last)
        {
            return Err("可视化图层或条目标识无效。".into());
        }
        for spec in self.specs.values() {
            spec.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub rows: Vec<Row>,
    pub edges: Vec<(usize, usize, String)>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MapIcon {
    pub at: LonLat,
    pub size: f32,
    pub color: [u8; 4],
    pub icon: Icon,
    pub label: String,
}

fn point_valid(point: LonLat) -> bool {
    (-180.0..=180.0).contains(&point.lon) && (-90.0..=90.0).contains(&point.lat)
}

impl Spec {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("请填写可视化名称。".into());
        }
        self.rule.validate()?;
        match &self.data {
            Data::Rows(rows) => {
                if rows.len() > 250_000
                    || rows.iter().any(|r| {
                        !r.x.is_finite()
                            || !r.value.is_finite()
                            || r.point.is_some_and(|p| !point_valid(p))
                    })
                {
                    return Err("表格行数过多或含无效数值、坐标。".into());
                }
            }
            Data::MapField { key, .. } | Data::Property { key, .. } if key.trim().is_empty() => {
                return Err("请选择数据字段。".into());
            }
            Data::MapHistory { years, key, .. } | Data::RecordHistory { years, key, .. } => {
                years.indices()?;
                if key.trim().is_empty() {
                    return Err("请选择数据字段。".into());
                }
            }
            Data::Transect {
                from, to, samples, ..
            } => {
                if !point_valid(*from)
                    || !point_valid(*to)
                    || !(2..=2048).contains(samples)
                    || measure::great_circle(*from, *to, 1.0) < 1e-8
                {
                    return Err("剖面需要两个不同的坐标和 2–2048 个采样点。".into());
                }
            }
            _ => {}
        }
        if self.plot == Plot::Relations && !matches!(self.data, Data::Network { .. }) {
            return Err("关系网络需要使用世界观关系作为来源。".into());
        }
        Ok(())
    }

    pub fn evaluate(&self, doc: &Document) -> Result<Dataset, String> {
        self.validate()?;
        let board = WorldBook::current(doc);
        let mut data = match &self.data {
            Data::Rows(rows) => Dataset {
                rows: rows.clone(),
                ..Default::default()
            },
            Data::MapField { key, top_level } => {
                let stats = Stats::new(doc);
                let rows = doc
                    .entities
                    .iter()
                    .filter(|(_, e)| e.exists_in(doc.year) && (!*top_level || e.parent.is_none()))
                    .filter_map(|(&id, e)| {
                        let value = stats.value(id, key)?;
                        Some(Row {
                            label: e.name.clone(),
                            x: id.0 as f64,
                            value,
                            point: e.pos,
                            node: Some(Node::Entity(board, id.0)),
                        })
                    })
                    .collect();
                Dataset {
                    rows,
                    ..Default::default()
                }
            }
            Data::MapHistory { key, entity, years } => {
                let name = doc
                    .entities
                    .get(entity)
                    .ok_or("地图数据对象不存在。")?
                    .name
                    .clone();
                let mut map = doc.clone();
                map.book = None;
                let rows = years
                    .indices()?
                    .into_iter()
                    .filter_map(|year| {
                        map.set_year(Some(year));
                        if !map.entities.get(entity)?.exists_in(Some(year)) {
                            return None;
                        }
                        let value = Stats::new(&map).value(*entity, key)?;
                        Some(Row {
                            label: name.clone(),
                            x: f64::from(year),
                            value,
                            point: None,
                            node: None,
                        })
                    })
                    .collect();
                Dataset {
                    rows,
                    ..Default::default()
                }
            }
            Data::Property { schema, key } => {
                let catalog = Catalog::get(doc).ok_or("工程还没有世界观资料。")?;
                let mut eval = Evaluation::new(doc, doc.year).unwrap();
                let mut data = Dataset::default();
                for (&id, record) in &catalog.records {
                    if schema.is_some_and(|s| s != record.schema)
                        || !record.meta.visible(doc.year)
                        || !catalog.schemas[&record.schema].fields.contains_key(key)
                    {
                        continue;
                    }
                    match eval.get(id, key) {
                        Ok(Some(found)) => {
                            if let Some(value) = found.value.as_number() {
                                data.rows.push(Row {
                                    label: record.name.clone(),
                                    x: id as f64,
                                    value,
                                    point: None,
                                    node: Some(Node::Record(id)),
                                });
                            }
                        }
                        Err(e) => {
                            if data.warnings.len() < 12 {
                                data.warnings.push(e);
                            }
                        }
                        _ => {}
                    }
                }
                data
            }
            Data::RecordHistory { record, key, years } => {
                let catalog = Catalog::get(doc).ok_or("工程还没有世界观资料。")?;
                let name = &catalog
                    .records
                    .get(record)
                    .ok_or("世界观资料不存在。")?
                    .name;
                let mut data = Dataset::default();
                for year in years.indices()? {
                    match Evaluation::new(doc, Some(year)).unwrap().get(*record, key) {
                        Ok(Some(resolved)) => {
                            if let Some(value) = resolved.value.as_number() {
                                data.rows.push(Row {
                                    label: name.clone(),
                                    x: f64::from(year),
                                    value,
                                    point: None,
                                    node: None,
                                });
                            }
                        }
                        Err(e) => {
                            if data.warnings.len() < 12 {
                                data.warnings.push(e);
                            }
                        }
                        _ => {}
                    }
                }
                data
            }
            Data::Network { schema, relation } => network(doc, *schema, *relation)?,
            Data::Transect {
                from,
                to,
                layer,
                samples,
            } => transect(doc, *from, *to, layer, *samples)?,
        };
        let transform = self.rule.ast()?;
        // Relationship edges index the original node array.
        if self.plot != Plot::Relations {
            let year = doc.year;
            let use_sample_year = matches!(
                self.data,
                Data::MapHistory { .. } | Data::RecordHistory { .. }
            );
            data.rows.retain_mut(|row| {
                let v = expr::eval(&transform, &|key| match key {
                    "x" => Some(row.x),
                    "value" => Some(row.value),
                    "year" if use_sample_year => Some(row.x),
                    "year" => year.map(f64::from),
                    _ => None,
                });
                if let Some(v) = v {
                    row.value = v;
                    true
                } else {
                    false
                }
            });
            data.edges.clear();
        }
        Ok(data)
    }
}

fn network(doc: &Document, schema: Option<Id>, relation: Option<Id>) -> Result<Dataset, String> {
    let catalog = Catalog::get(doc).ok_or("工程还没有世界观资料。")?;
    let mut out = Dataset::default();
    let mut positions = BTreeMap::new();
    for (id, record) in catalog
        .records
        .iter()
        .filter(|(_, r)| schema.is_none_or(|ty| ty == r.schema) && r.meta.visible(doc.year))
    {
        let i = out.rows.len();
        positions.insert(*id, i);
        out.rows.push(Row {
            label: record.name.clone(),
            x: i as f64,
            value: 1.0,
            point: None,
            node: Some(Node::Record(*id)),
        });
    }
    for r in catalog
        .relations
        .values()
        .filter(|r| relation.is_none_or(|ty| ty == r.kind) && r.meta.visible(doc.year))
    {
        if let (Some(&a), Some(&b)) = (positions.get(&r.from), positions.get(&r.to)) {
            out.rows[a].value += 1.0;
            if a != b {
                out.rows[b].value += 1.0;
            }
            out.edges
                .push((a, b, catalog.relation_types[&r.kind].name.clone()));
        }
    }
    Ok(out)
}

fn transect(
    doc: &Document,
    from: LonLat,
    to: LonLat,
    layer: &Layer,
    samples: u16,
) -> Result<Dataset, String> {
    let result = doc.environment.as_ref().and_then(|p| p.current.as_ref());
    let fallback = matches!(layer, Layer::Standard(Field::Elevation))
        .then_some(doc.terrain.as_ref())
        .flatten();
    if result.is_none() && fallback.is_none() {
        return Err("请先生成环境网格或导入高程图。".into());
    }
    let angle = measure::great_circle(from, to, 1.0);
    let path = measure::great_circle_path(
        from,
        to,
        (angle.to_degrees() / f64::from(samples - 1)).max(0.000001),
    );
    let rows = path
        .iter()
        .filter_map(|&p| {
            let value = result
                .and_then(|g| g.at(layer, p))
                .map(f64::from)
                .or_else(|| fallback.map(|t| t.at(p.lon, p.lat)))?;
            let km = measure::great_circle(from, p, doc.radius());
            Some(Row {
                label: format!("{:.2}°, {:.2}°", p.lon, p.lat),
                x: km,
                value,
                point: Some(p),
                node: None,
            })
        })
        .collect();
    Ok(Dataset {
        rows,
        ..Default::default()
    })
}

fn record_anchors(doc: &Document) -> BTreeMap<Id, Vec<Node>> {
    let mut anchors = BTreeMap::<_, BTreeSet<_>>::new();
    let Some(book) = &doc.book else {
        return BTreeMap::new();
    };
    let board = WorldBook::current(doc);
    for link in book.links.values().filter(|link| link.visible(doc.year)) {
        match (link.from, link.to) {
            (Node::Record(id), node) | (node, Node::Record(id)) if node.board() == Some(board) => {
                anchors
                    .entry(id)
                    .or_default()
                    .insert(book.resolve(doc, node));
            }
            _ => {}
        }
    }
    anchors
        .into_iter()
        .map(|(id, nodes)| (id, nodes.into_iter().collect()))
        .collect()
}

fn linked_nodes(row: &Row, board: u64, records: &BTreeMap<Id, Vec<Node>>) -> Vec<Node> {
    if let Some(Node::Record(id)) = row.node {
        records.get(&id).cloned().unwrap_or_default()
    } else {
        row.node
            .into_iter()
            .filter(|n| n.board() == Some(board))
            .collect()
    }
}

pub fn apply_to_map(doc: &Document, active: &mut ThemeResult) {
    let Some(project) = &doc.visualization else {
        return;
    };
    let Some(spec) = project.overlay.and_then(|id| project.specs.get(&id)) else {
        return;
    };
    let Ok(dataset) = spec.evaluate(doc) else {
        return;
    };
    let bounds = spec.rule.scale(&dataset.rows);
    let records = record_anchors(doc);
    let board = WorldBook::current(doc);
    let anchors = |row| linked_nodes(row, board, &records);

    let native = WorldBook::default();
    let book = doc.book.as_deref().unwrap_or(&native);
    if spec.plot == Plot::Symbols {
        for row in &dataset.rows {
            let mut points = row.point.into_iter().collect::<Vec<_>>();
            if points.is_empty() {
                points.extend(
                    anchors(row)
                        .into_iter()
                        .filter_map(|node| book.position(doc, node)),
                );
            }
            let [a, b] = spec.rule.radii;
            let size = a + (b - a) * (Rule::fraction(row.value, bounds) as f32).sqrt();
            let color = spec.rule.color(row.value, bounds);
            active
                .data_icons
                .extend(points.into_iter().map(|at| MapIcon {
                    at,
                    size,
                    color,
                    icon: spec.rule.icon,
                    label: if spec.rule.map_labels {
                        row.label.clone()
                    } else {
                        String::new()
                    },
                }));
        }
    } else if spec.plot == Plot::Choropleth {
        let breaks = if spec.rule.breaks.is_empty() {
            vec![]
        } else {
            theme::compute_breaks(
                &dataset.rows.iter().map(|r| r.value).collect::<Vec<_>>(),
                ClassMethod::Manual,
                spec.rule.breaks.len() + 1,
                &spec.rule.breaks,
            )
        };
        let color = |value| {
            if breaks.is_empty() {
                spec.rule.color(value, bounds)
            } else {
                theme::sample_ramp(
                    &spec.rule.ramp,
                    theme::class_of(value, &breaks) as f64 / breaks.len() as f64,
                )
            }
        };
        let mut values = BTreeMap::<FaceId, (f64, u32)>::new();
        for row in &dataset.rows {
            let mut faces = BTreeSet::new();
            for node in anchors(row) {
                match node {
                    Node::Entity(_, id) => faces.extend(doc.faces_of_tree(EntityId(id))),
                    Node::Face(_, id) => {
                        faces.insert(FaceId(id));
                    }
                    _ => {}
                }
            }
            if faces.is_empty() {
                if let Some(at) = row.point {
                    faces.insert(doc.topo.face_at(at));
                }
            }
            for face in faces.into_iter().filter(|&f| {
                f != BACKGROUND && doc.face_kind_at(f, doc.year) == Some(FaceKind::Land)
            }) {
                let value = values.entry(face).or_default();
                value.0 += row.value;
                value.1 += 1;
            }
        }
        for (face, (sum, count)) in values {
            active
                .face_colors
                .insert(face, color(sum / f64::from(count)));
        }
        if !dataset.rows.is_empty() {
            let mut edges = vec![bounds[0]];
            if breaks.is_empty() {
                edges.push((bounds[0] + bounds[1]) * 0.5);
            } else {
                edges.extend(breaks.iter().copied());
            }
            edges.push(bounds[1]);
            active.legend = edges
                .windows(2)
                .map(|p| LegendItem {
                    label: format!("{:.2} – {:.2} {}", p[0], p[1], spec.rule.unit),
                    color: color((p[0] + p[1]) * 0.5),
                })
                .collect();
        }
    }
    active.title = spec.name.clone();
}

/// Geometry is separate from indicator evaluation, so moving the map camera
/// doesn't re-evaluate saved charts or their data.
pub fn same_map_inputs(a: &Document, b: &Document) -> bool {
    let active = |d: &Document| d.visualization.as_ref().and_then(|v| v.overlay);
    if active(a).is_none() && active(b).is_none() {
        return true;
    }
    let same = |a: Option<&Arc<Project>>, b: Option<&Arc<Project>>| match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    same(a.visualization.as_ref(), b.visualization.as_ref())
        && match (&a.book, &b.book) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
        && match (&a.environment, &b.environment) {
            (Some(a), Some(b)) => match (&a.current, &b.current) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            },
            (None, None) => true,
            _ => false,
        }
}

pub fn place_row(label: String, x: f64, value: f64, lon: f64, lat: f64) -> Row {
    Row {
        label,
        x,
        value,
        point: Some(LonLat::new(geom::wrap_lon(lon), lat)),
        node: None,
    }
}
