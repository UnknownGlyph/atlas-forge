//! Project-wide, user-defined types, records, relationships, and temporal rules.
use crate::{
    doc::{Document, EntityId},
    expr,
    stats::Stats,
    topo::FaceId,
    world_book::{BookId, Meta, Node, WorldBook},
};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

pub type Id = u64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Text,
    Number,
    Flag,
    Reference,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Text(String),
    Number(f64),
    Flag(bool),
    Reference(Id),
}

impl Value {
    pub fn blank(kind: Kind) -> Option<Self> {
        Some(match kind {
            Kind::Text => Self::Text(String::new()),
            Kind::Number => Self::Number(0.0),
            Kind::Flag => Self::Flag(false),
            Kind::Reference => return None,
        })
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn matches(&self, kind: Kind, catalog: &Catalog) -> bool {
        match (self, kind) {
            (Self::Text(_), Kind::Text) | (Self::Flag(_), Kind::Flag) => true,
            (Self::Number(n), Kind::Number) => n.is_finite(),
            (Self::Reference(id), Kind::Reference) => catalog.records.contains_key(id),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[default]
    Outgoing,
    Incoming,
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aggregate {
    #[default]
    Sum,
    Mean,
    Min,
    Max,
}

impl Aggregate {
    fn combine(self, items: impl IntoIterator<Item = f64>) -> Option<f64> {
        let mut items = items.into_iter();
        let first = items.next()?;
        let (value, count) = items.fold((first, 1), |(a, n), b| {
            let value = match self {
                Self::Sum | Self::Mean => a + b,
                Self::Min => a.min(b),
                Self::Max => a.max(b),
            };
            (value, n + 1)
        });
        let value = if self == Self::Mean {
            value / f64::from(count)
        } else {
            value
        };
        value.is_finite().then_some(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Input {
    Environment {
        layer: crate::environment::Layer,
        aggregate: Aggregate,
    },
    MapField {
        key: String,
        aggregate: Aggregate,
    },
    RelatedCount {
        relation: Id,
        direction: Direction,
    },
    RelatedField {
        relation: Id,
        direction: Direction,
        key: String,
        aggregate: Aggregate,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub expression: String,
    pub inputs: BTreeMap<String, Input>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub unit: String,
    pub kind: Kind,
    pub default: Option<Value>,
    pub rule: Option<Rule>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub description: String,
    pub fields: BTreeMap<String, Field>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub base: Option<Value>,
    pub history: BTreeMap<i32, Value>,
    pub pinned: Option<Value>,
}

impl Series {
    pub fn manual(&self, year: Option<i32>) -> Option<&Value> {
        let dated = match year {
            Some(year) => self.history.range(..=year).next_back(),
            None => self.history.last_key_value(),
        };
        dated.map(|(_, v)| v).or(self.base.as_ref())
    }

    pub fn write(&mut self, year: Option<i32>, value: Option<Value>) {
        match (year, value) {
            (Some(year), Some(value)) => {
                self.history.insert(year, value);
            }
            (Some(year), None) => {
                self.history.remove(&year);
            }
            (None, value) => self.base = value,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub schema: Id,
    pub name: String,
    pub meta: Meta,
    pub values: BTreeMap<String, Series>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RelationType {
    pub name: String,
    pub description: String,
    pub source: Option<Id>,
    pub target: Option<Id>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub kind: Id,
    pub from: Id,
    pub to: Id,
    pub meta: Meta,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub schema: Id,
    pub meta: Meta,
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    last: Id,
    pub schemas: OrdMap<Id, Arc<Schema>>,
    pub records: OrdMap<Id, Arc<Record>>,
    pub relation_types: OrdMap<Id, Arc<RelationType>>,
    pub relations: OrdMap<Id, Arc<Relation>>,
    pub templates: OrdMap<Id, Arc<Template>>,
}

#[derive(Clone, Copy)]
pub enum Starter {
    Family,
    Person,
    Minerals,
}

impl Starter {
    pub const ALL: [Self; 3] = [Self::Family, Self::Person, Self::Minerals];

    pub fn name(self) -> &'static str {
        match self {
            Self::Family => "家族与谱系",
            Self::Person => "人物关系网",
            Self::Minerals => "国家矿产档案",
        }
    }

    pub fn install(self, catalog: &mut Catalog) -> Result<Id, String> {
        let text = |name: &str| Field { name: name.into(), ..Default::default() };
        let number = |name: &str, unit: &str| Field {
            name: name.into(), unit: unit.into(), kind: Kind::Number, ..Default::default()
        };
        let person = || Schema {
            name: "人物".into(),
            fields: BTreeMap::from([
                ("role".into(), text("身份与职务")),
                ("birth".into(), number("出生年份", "年")),
                ("home".into(), text("出身地")),
                ("profile".into(), text("生平")),
            ]),
            ..Default::default()
        };
        let definition = match self {
            Self::Family => Schema {
                name: "家族".into(),
                fields: BTreeMap::from([
                    ("seat".into(), text("本家或驻地")),
                    ("founding_year".into(), number("创立年份", "年")),
                    ("motto".into(), text("家训")),
                    ("heraldry".into(), text("纹章")),
                ]),
                ..Default::default()
            },
            Self::Person => person(),
            Self::Minerals => {
                let mut years = number("按现有产量可开采年数", "年");
                years.rule = Some(Rule { expression: "reserve / annual_output".into(), ..Default::default() });
                Schema {
                    name: "国家矿产".into(),
                    description: "可将档案关联到地图国家、矿区或港口，再按年份填写储量与产量。".into(),
                    fields: BTreeMap::from([
                        ("mineral".into(), text("矿种")),
                        ("reserve".into(), number("探明储量", "t")),
                        ("annual_output".into(), number("年产量", "t/年")),
                        ("remaining_years".into(), years),
                        ("note".into(), text("矿带与运输")),
                    ]),
                }
            }
        };
        let schema = install_schema(catalog, definition)?;
        match self {
            Self::Family => {
                let person = install_schema(catalog, person())?;
                install_relation(catalog, "所属家族", person, schema)?;
                install_relation(catalog, "亲子", person, person)?;
                install_relation(catalog, "婚姻", person, person)?;
            }
            Self::Person => {
                for relation in ["同盟", "师徒", "对手"] {
                    install_relation(catalog, relation, schema, schema)?;
                }
            }
            Self::Minerals => {}
        }
        if !catalog.templates.values().any(|entry| entry.schema == schema && entry.name == self.name()) {
            let id = catalog.alloc()?;
            catalog.templates.insert(id, Arc::new(Template {
                name: self.name().into(), schema, ..Default::default()
            }));
        }
        Ok(schema)
    }
}

fn install_schema(catalog: &mut Catalog, definition: Schema) -> Result<Id, String> {
    let existing = catalog.schemas.iter().find(|(_, schema)| schema.as_ref() == &definition).map(|(id, _)| *id);
    if let Some(id) = existing { return Ok(id); }
    let id = catalog.alloc()?;
    catalog.schemas.insert(id, Arc::new(definition));
    Ok(id)
}

fn install_relation(catalog: &mut Catalog, name: &str, source: Id, target: Id) -> Result<(), String> {
    if !catalog.relation_types.values().any(|r| r.name == name && r.source == Some(source) && r.target == Some(target)) {
        let id = catalog.alloc()?;
        catalog.relation_types.insert(id, Arc::new(RelationType {
            name: name.into(), source: Some(source), target: Some(target), ..Default::default()
        }));
    }
    Ok(())
}

fn key_valid(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn require_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("请填写名称。".into());
    }
    Ok(())
}

fn validate_meta(meta: &Meta) -> Result<(), String> {
    if matches!((meta.from, meta.until), (Some(start), Some(end)) if end < start) {
        return Err("结束年早于开始年。".into());
    }
    Ok(())
}

fn parse_rule(rule: &Rule) -> Result<expr::Ast, String> {
    if rule.expression.len() > 512 {
        return Err("数值规则最多可输入 512 字节。".into());
    }
    expr::parse(&rule.expression)
}

impl Catalog {
    pub fn get(doc: &Document) -> Option<&Self> {
        doc.book.as_ref()?.worldview.as_deref()
    }

    pub fn edit(doc: &mut Document) -> &mut Self {
        Arc::make_mut(
            WorldBook::edit(doc)
                .worldview
                .get_or_insert_with(|| Arc::new(Self::default())),
        )
    }

    pub fn alloc(&mut self) -> Result<Id, String> {
        self.last = self.last.checked_add(1).ok_or("设定条目标识已用尽。")?;
        Ok(self.last)
    }

    pub fn from_template(&self, id: Id, name: String) -> Result<Record, String> {
        let template = self.templates.get(&id).ok_or("模板已移除。")?;
        require_name(&name)?;
        Ok(Record {
            name,
            schema: template.schema,
            meta: template.meta.clone(),
            values: template
                .values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        Series {
                            base: Some(value.clone()),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn change_years(&self) -> BTreeSet<i32> {
        let mut years = BTreeSet::new();
        self.extend_years(&mut years, None);
        years
    }

    pub fn extend_years(&self, years: &mut BTreeSet<i32>, ids: Option<&BTreeSet<Id>>) {
        let mut meta = |m: &Meta| {
            years.extend(m.from);
            years.extend(m.until.and_then(|year| year.checked_add(1)));
        };
        for (&id, record) in &self.records {
            if ids.is_none_or(|ids| ids.contains(&id)) {
                meta(&record.meta);
            }
        }
        for relation in self.relations.values() {
            if ids.is_none_or(|ids| ids.contains(&relation.from) || ids.contains(&relation.to)) {
                meta(&relation.meta);
            }
        }
        for (&id, record) in &self.records {
            if ids.is_none_or(|ids| ids.contains(&id)) {
                for value in record.values.values() {
                    years.extend(value.history.keys());
                }
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for &id in self
            .schemas
            .keys()
            .chain(self.records.keys())
            .chain(self.relation_types.keys())
            .chain(self.relations.keys())
            .chain(self.templates.keys())
        {
            if id == 0 || id > self.last || !ids.insert(id) {
                return Err("世界观设定标识无效。".into());
            }
        }
        for schema in self.schemas.values() {
            self.validate_schema(schema)?;
        }
        for kind in self.relation_types.values() {
            require_name(&kind.name)?;
            if kind
                .source
                .into_iter()
                .chain(kind.target)
                .any(|id| !self.schemas.contains_key(&id))
            {
                return Err("关系类型引用了不存在的实体类型。".into());
            }
        }
        for record in self.records.values() {
            self.validate_record(record)?;
        }
        for relation in self.relations.values() {
            let kind = self
                .relation_types
                .get(&relation.kind)
                .ok_or("关系类型不存在。")?;
            let from = self
                .records
                .get(&relation.from)
                .ok_or("关系的起点不存在。")?;
            let to = self.records.get(&relation.to).ok_or("关系的终点不存在。")?;
            if !kind.source.is_none_or(|id| id == from.schema)
                || !kind.target.is_none_or(|id| id == to.schema)
            {
                return Err("关系两端的实体类型与定义不符。".into());
            }
            validate_meta(&relation.meta)?;
        }
        for template in self.templates.values() {
            require_name(&template.name)?;
            validate_meta(&template.meta)?;
            let schema = self
                .schemas
                .get(&template.schema)
                .ok_or("模板的实体类型不存在。")?;
            for (key, value) in &template.values {
                let field = schema
                    .fields
                    .get(key)
                    .ok_or_else(|| format!("模板中的属性「{key}」不存在。"))?;
                if !value.matches(field.kind, self) {
                    return Err(format!("模板中的属性「{key}」类型不符。"));
                }
            }
        }
        Ok(())
    }

    fn validate_record(&self, record: &Record) -> Result<(), String> {
        require_name(&record.name)?;
        validate_meta(&record.meta)?;
        let schema = self
            .schemas
            .get(&record.schema)
            .ok_or("记录的实体类型不存在。")?;
        for (key, values) in &record.values {
            let field = schema
                .fields
                .get(key)
                .ok_or_else(|| format!("记录中的属性「{key}」不存在。"))?;
            if values
                .base
                .iter()
                .chain(values.history.values())
                .chain(values.pinned.iter())
                .any(|value| !value.matches(field.kind, self))
            {
                return Err(format!("属性「{key}」有类型不符或指向不存在条目的值。"));
            }
        }
        Ok(())
    }

    fn validate_schema(&self, schema: &Schema) -> Result<(), String> {
        require_name(&schema.name)?;
        let mut dependencies = BTreeMap::new();
        for (key, field) in &schema.fields {
            if !key_valid(key) || matches!(key.as_str(), "year" | "map_area") {
                return Err(format!("属性标识「{key}」无效或已保留。"));
            }
            require_name(&field.name)?;
            if field
                .default
                .as_ref()
                .is_some_and(|v| !v.matches(field.kind, self))
            {
                return Err(format!("属性「{key}」的默认值类型不符。"));
            }
            let Some(rule) = &field.rule else { continue };
            if field.kind != Kind::Number {
                return Err("数值规则只能用于数值属性。".into());
            }
            for (input, binding) in &rule.inputs {
                if !key_valid(input)
                    || schema.fields.contains_key(input)
                    || matches!(input.as_str(), "year" | "map_area")
                {
                    return Err(format!("规则变量「{input}」与属性冲突或无效。"));
                }
                match binding {
                    Input::MapField { key, .. } => require_name(key)?,
                    Input::Environment { .. } => {}
                    Input::RelatedCount { relation, .. } | Input::RelatedField { relation, .. } => {
                        if !self.relation_types.contains_key(relation) {
                            return Err(format!("规则变量「{input}」的关系类型不存在。"));
                        }
                    }
                }
            }
            let mut vars = Vec::new();
            expr::vars(&parse_rule(rule)?, &mut vars);
            vars.sort_unstable();
            vars.dedup();
            let mut local = Vec::new();
            for variable in vars {
                if matches!(variable.as_str(), "year" | "map_area")
                    || rule.inputs.contains_key(&variable)
                {
                    continue;
                }
                match schema.fields.get(&variable) {
                    Some(ref_field) if ref_field.kind == Kind::Number => {
                        if ref_field.rule.is_some() {
                            local.push(variable);
                        }
                    }
                    _ => return Err(format!("规则引用未知数值变量「{variable}」。")),
                }
            }
            dependencies.insert(key.clone(), local);
        }
        let mut stack = BTreeSet::new();
        let mut complete = BTreeSet::new();
        for key in dependencies.keys() {
            validate_order(key, &dependencies, &mut stack, &mut complete)?;
        }
        Ok(())
    }
}

fn validate_order<'a>(
    key: &'a str,
    dependencies: &'a BTreeMap<String, Vec<String>>,
    stack: &mut BTreeSet<&'a str>,
    complete: &mut BTreeSet<&'a str>,
) -> Result<(), String> {
    if complete.contains(key) {
        return Ok(());
    }
    if stack.len() >= 64 || !stack.insert(key) {
        return Err(format!("规则「{key}」存在循环或依赖层数超过 64。"));
    }
    for next in &dependencies[key] {
        validate_order(next, dependencies, stack, complete)?;
    }
    stack.remove(key);
    complete.insert(key);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Pinned,
    Manual,
    Computed,
    Default,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub value: Value,
    pub source: Source,
}

pub struct Evaluation<'a> {
    doc: &'a Document,
    book: &'a WorldBook,
    catalog: &'a Catalog,
    year: Option<i32>,
    active: BTreeSet<(Id, String)>,
    values: BTreeMap<(Id, String), Result<Option<Resolved>, String>>,
    relations: BTreeMap<Id, Vec<&'a Relation>>,
    links: BTreeMap<Id, Vec<(Node, &'a crate::world_book::Link)>>,
    map_fields: BTreeMap<(Node, Option<i32>, String), Option<f64>>,
    map_views: BTreeMap<(BookId, Option<i32>), Document>,
    map_areas: BTreeMap<(BookId, Option<i32>), HashMap<EntityId, f64>>,
    upstream: bool,
}

impl<'a> Evaluation<'a> {
    pub fn new(doc: &'a Document, year: Option<i32>) -> Option<Self> {
        let book = doc.book.as_deref()?;
        let catalog = book.worldview.as_deref()?;
        let mut relations: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for relation in catalog.relations.values().filter(|r| r.meta.visible(year)) {
            relations
                .entry(relation.from)
                .or_default()
                .push(relation.as_ref());
            if relation.to != relation.from {
                relations
                    .entry(relation.to)
                    .or_default()
                    .push(relation.as_ref());
            }
        }
        let mut links: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for link in book.links.values().filter(|l| l.visible(year)) {
            match (link.from, link.to) {
                (Node::Record(id), other) | (other, Node::Record(id))
                    if other.board().is_some() =>
                {
                    links
                        .entry(id)
                        .or_default()
                        .push((book.resolve(doc, other), link));
                }
                _ => {}
            }
        }
        Some(Self {
            doc,
            book,
            catalog,
            year,
            relations,
            links,
            active: BTreeSet::new(),
            values: BTreeMap::new(),
            map_fields: BTreeMap::new(),
            map_views: BTreeMap::new(),
            map_areas: BTreeMap::new(),
            upstream: false,
        })
    }

    pub(crate) fn for_environment(doc: &'a Document, year: Option<i32>) -> Option<Self> {
        let mut evaluation = Self::new(doc, year)?;
        evaluation.upstream = true;
        Some(evaluation)
    }

    pub fn get(&mut self, id: Id, key: &str) -> Result<Option<Resolved>, String> {
        let token = (id, key.to_owned());
        if let Some(value) = self.values.get(&token) {
            return value.clone();
        }
        if self.active.len() >= 64 || !self.active.insert(token.clone()) {
            return Err(format!("条目 {id} 的规则「{key}」循环或依赖过深。"));
        }
        let result = self.resolve(id, key);
        self.active.remove(&token);
        self.values.insert(token, result.clone());
        result
    }

    fn resolve(&mut self, id: Id, key: &str) -> Result<Option<Resolved>, String> {
        let record = self
            .catalog
            .records
            .get(&id)
            .ok_or("关联的设定条目不存在。")?;
        if !record.meta.visible(self.year) {
            return Ok(None);
        }
        let schema = self
            .catalog
            .schemas
            .get(&record.schema)
            .ok_or("条目的实体类型不存在。")?;
        let field = schema
            .fields
            .get(key)
            .ok_or_else(|| format!("「{}」缺少属性「{key}」。", record.name))?;
        if let Some(series) = record.values.get(key) {
            if let Some(value) = &series.pinned {
                return Ok(Some(Resolved {
                    value: value.clone(),
                    source: Source::Pinned,
                }));
            }
            if let Some(value) = series.manual(self.year) {
                return Ok(Some(Resolved {
                    value: value.clone(),
                    source: Source::Manual,
                }));
            }
        }
        if let Some(rule) = &field.rule {
            let expression = parse_rule(rule)?;
            let mut vars = Vec::new();
            expr::vars(&expression, &mut vars);
            let mut args = BTreeMap::new();
            for name in vars.into_iter().collect::<BTreeSet<_>>() {
                let value = match name.as_str() {
                    "year" => self.year.map(f64::from),
                    "map_area" => self.map_input(id, crate::doc::AREA_KEY, Aggregate::Sum),
                    _ => match rule.inputs.get(&name) {
                        Some(input) => self.input(id, input)?,
                        None => self.number(id, &name)?,
                    },
                };
                args.insert(name, value);
            }
            if let Some(value) = expr::eval(&expression, &|key| args.get(key).copied().flatten()) {
                return Ok(Some(Resolved {
                    value: Value::Number(value),
                    source: Source::Computed,
                }));
            }
        }
        Ok(field.default.clone().map(|value| Resolved {
            value,
            source: Source::Default,
        }))
    }

    fn number(&mut self, id: Id, key: &str) -> Result<Option<f64>, String> {
        Ok(self.get(id, key)?.and_then(|r| r.value.as_number()))
    }

    fn related(&self, id: Id, kind: Id, direction: Direction) -> BTreeSet<Id> {
        self.relations
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|relation| relation.kind == kind)
            .flat_map(|relation| {
                let from = (direction != Direction::Outgoing && relation.to == id)
                    .then_some(relation.from);
                let to = (direction != Direction::Incoming && relation.from == id)
                    .then_some(relation.to);
                from.into_iter().chain(to)
            })
            .filter(|target| {
                self.catalog
                    .records
                    .get(target)
                    .is_some_and(|r| r.meta.visible(self.year))
            })
            .collect()
    }

    fn input(&mut self, id: Id, binding: &Input) -> Result<Option<f64>, String> {
        match binding {
            Input::Environment { layer, aggregate } => {
                if self.upstream {
                    return Err("环境生成参数不能依赖自己的环境输出，请选择上游设定属性。".into());
                }
                Ok(self.environment_input(id, layer, *aggregate))
            }
            Input::MapField { key, aggregate } => Ok(self.map_input(id, key, *aggregate)),
            Input::RelatedCount {
                relation,
                direction,
            } => Ok(Some(self.related(id, *relation, *direction).len() as f64)),
            Input::RelatedField {
                relation,
                direction,
                key,
                aggregate,
            } => {
                let mut values = Vec::new();
                for target in self.related(id, *relation, *direction) {
                    let record = &self.catalog.records[&target];
                    if !self.catalog.schemas[&record.schema]
                        .fields
                        .contains_key(key)
                    {
                        continue;
                    }
                    values.extend(self.number(target, key)?);
                }
                Ok(aggregate.combine(values))
            }
        }
    }

    fn environment_input(
        &self,
        id: Id,
        layer: &crate::environment::Layer,
        aggregate: Aggregate,
    ) -> Option<f64> {
        let targets: BTreeSet<_> = self.links.get(&id)?.iter().map(|(node, _)| *node).collect();
        aggregate.combine(targets.into_iter().filter_map(|node| {
            let map = self.book.map(self.doc, node.board()?)?;
            let generation = map.environment.as_ref()?.current.as_ref()?;
            let grid = generation.grid;
            let values = |i| generation.value(layer, i).map(f64::from);
            let area_or_region = match node {
                Node::Map(_) => None,
                Node::Face(_, id) => Some(FaceId(id)),
                Node::Entity(_, id) if map.entities.get(&EntityId(id))?.pos.is_none() => None,
                _ => {
                    let pos = self.book.position(self.doc, node)?;
                    return generation.at(layer, pos).map(f64::from);
                }
            };
            let includes = |point| {
                if matches!(node, Node::Map(_)) {
                    return true;
                }
                let face = map.topo.face_at(point);
                if let Some(target) = area_or_region {
                    return face == target;
                }
                let Node::Entity(_, entity) = node else {
                    return false;
                };
                let mut owner = map.owner_at(face, generation.year);
                // Parent history is acyclic in a valid document.
                for _ in 0..=map.entities.len() {
                    let Some(id) = owner else {
                        break;
                    };
                    if id.0 == entity {
                        return true;
                    }
                    owner = map.parent_at(id, generation.year);
                }
                false
            };
            if aggregate == Aggregate::Mean {
                let (sum, total) = (0..grid.len())
                    .filter_map(|i| {
                        let point = grid.center(i);
                        let value = includes(point).then(|| values(i))??;
                        let weight = point.lat.to_radians().cos();
                        Some((value * weight, weight))
                    })
                    .fold((0.0, 0.0), |(a, b), (v, w)| (a + v, b + w));
                (total > 0.0).then(|| sum / total)
            } else {
                aggregate.combine(
                    (0..grid.len()).filter_map(|i| includes(grid.center(i)).then(|| values(i))?),
                )
            }
        }))
    }

    fn map_input(&mut self, id: Id, key: &str, aggregate: Aggregate) -> Option<f64> {
        let targets: BTreeSet<_> = self
            .links
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|(node, link)| {
                let map = if matches!(node, Node::Entity(..) | Node::Face(..)) {
                    self.book.map(self.doc, node.board()?)?
                } else {
                    return None;
                };
                let year = link.year(self.year).unwrap_or(map.year);
                Some((*node, year))
            })
            .collect();
        aggregate.combine(
            targets
                .into_iter()
                .filter_map(|(node, year)| self.map_number(node, year, key)),
        )
    }

    fn map_number(&mut self, node: Node, year: Option<i32>, key: &str) -> Option<f64> {
        let cache_key = (node, year, key.to_owned());
        if let Some(value) = self.map_fields.get(&cache_key) {
            return *value;
        }
        let board = node.board()?;
        let source = self.book.map(self.doc, board)?;
        let map = if source.year == year {
            source
        } else {
            self.map_views.entry((board, year)).or_insert_with(|| {
                let mut snapshot = source.clone();
                snapshot.book = None;
                snapshot.set_year(year);
                snapshot
            })
        };
        let value = match node {
            Node::Entity(_, id) => {
                let id = EntityId(id);
                map.entities.get(&id).filter(|e| e.exists_in(year))?;
                let areas = self
                    .map_areas
                    .entry((board, year))
                    .or_insert_with(|| map.areas_by_entity());
                Stats::with_borrowed_areas(map, areas).value(id, key)
            }
            Node::Face(_, id) if key == crate::doc::AREA_KEY && id != 0 => {
                map.topo.faces.get(&FaceId(id))?;
                Some(map.topo.face_area(FaceId(id), map.radius()))
            }
            _ => None,
        };
        self.map_fields.insert(cache_key, value);
        value
    }
}
