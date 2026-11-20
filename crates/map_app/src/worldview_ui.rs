use crate::{app::App, i18n::tr};
use eframe::egui::{self, Color32};
use map_core::{
    world_book::{Link, Meta, Node, TimeLink, WorldBook},
    worldview::{
        Aggregate, Catalog, Direction, Evaluation, Field, Id, Input, Kind, Record, Relation,
        RelationType, Rule, Schema, Source, Template, Value,
    },
    Document,
};
use std::sync::Arc;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Category {
    #[default]
    Records,
    Types,
    RelationTypes,
    Relations,
    Templates,
}

impl Category {
    const ALL: [Self; 5] = [
        Self::Records,
        Self::Types,
        Self::RelationTypes,
        Self::Relations,
        Self::Templates,
    ];

    fn name(self) -> &'static str {
        tr(match self {
            Self::Records => "实体记录",
            Self::Types => "实体类型",
            Self::RelationTypes => "关系类型",
            Self::Relations => "关系实例",
            Self::Templates => "记录模板",
        })
    }
}

#[derive(Clone)]
enum Draft {
    Record(Record),
    Schema(Schema),
    RelationType(RelationType),
    Relation(Relation),
    Template(Template),
}

impl Draft {
    fn read(catalog: &Catalog, category: Category, id: Id) -> Option<Self> {
        Some(match category {
            Category::Records => Self::Record((**catalog.records.get(&id)?).clone()),
            Category::Types => Self::Schema((**catalog.schemas.get(&id)?).clone()),
            Category::RelationTypes => {
                Self::RelationType((**catalog.relation_types.get(&id)?).clone())
            }
            Category::Relations => Self::Relation((**catalog.relations.get(&id)?).clone()),
            Category::Templates => Self::Template((**catalog.templates.get(&id)?).clone()),
        })
    }

    fn store(self, catalog: &mut Catalog, id: Id) {
        match self {
            Self::Record(x) => {
                catalog.records.insert(id, Arc::new(x));
            }
            Self::Schema(x) => {
                catalog.schemas.insert(id, Arc::new(x));
            }
            Self::RelationType(x) => {
                catalog.relation_types.insert(id, Arc::new(x));
            }
            Self::Relation(x) => {
                catalog.relations.insert(id, Arc::new(x));
            }
            Self::Template(x) => {
                catalog.templates.insert(id, Arc::new(x));
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct WorldviewUi {
    open: bool,
    category: Category,
    search: String,
    only_year: bool,
    id: Option<Id>,
    draft: Option<Draft>,
    new_key: String,
    dated: Option<i32>,
    error: Option<String>,
}

impl WorldviewUi {
    fn select(&mut self, catalog: &Catalog, id: Id, year: Option<i32>) {
        self.id = Some(id);
        self.draft = Draft::read(catalog, self.category, id);
        self.dated = year;
        self.new_key.clear();
        self.error = None;
    }

    fn set_category(&mut self, category: Category) {
        if self.category != category {
            self.category = category;
            self.id = None;
            self.draft = None;
            self.new_key.clear();
            self.error = None;
        }
    }

    pub(crate) fn reload(&mut self, doc: &Document) {
        if let Some(id) = self.id {
            self.draft = Catalog::get(doc).and_then(|c| Draft::read(c, self.category, id));
            if self.draft.is_none() {
                self.id = None;
            }
        }
    }
}

enum Action {
    Save,
    Remove,
    Associate(Id),
    BrowseLinks(Id),
    Open(Node, Option<Option<i32>>),
    MakeTemplate,
}

impl App {
    pub(crate) fn open_worldview(&mut self, record: Option<Id>) {
        self.worldview_ui.open = true;
        if let (Some(id), Some(catalog)) = (record, Catalog::get(&self.doc)) {
            self.worldview_ui.set_category(Category::Records);
            self.worldview_ui.select(catalog, id, self.doc.year);
        }
        if self.studio.foreground {
            self.studio.show_worldview();
        }
    }

    pub(crate) fn worldview_window(&mut self, ctx: &egui::Context) {
        if !self.worldview_ui.open || self.studio.foreground {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("世界观设定"))
            .id(egui::Id::unique("worldview-inspector"))
            .default_size([860.0, 610.0])
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                modes(ui, &mut self.worldview_ui, false);
                ui.separator();
                let current = self.current_book_node();
                let empty = Catalog::default();
                let catalog = Catalog::get(&self.doc).unwrap_or(&empty);
                let mut action = None;
                ui.columns(2, |columns| {
                    index(
                        &mut columns[0],
                        &mut self.worldview_ui,
                        catalog,
                        self.doc.year,
                    );
                    action = detail(
                        &mut columns[1],
                        &mut self.worldview_ui,
                        catalog,
                        &self.doc,
                        current,
                    );
                });
                self.apply_worldview(action);
            });
        self.worldview_ui.open = open;
    }

    pub(crate) fn studio_worldview(&mut self, ui: &mut egui::Ui) {
        let empty = Catalog::default();
        let catalog = Catalog::get(&self.doc).unwrap_or(&empty);
        egui::Panel::left("studio-worldview-index")
            .size_range(185.0..=350.0)
            .default_size(250.0)
            .show(ui, |ui| {
                ui.heading(tr("世界观设定"));
                modes(ui, &mut self.worldview_ui, true);
                ui.separator();
                index(ui, &mut self.worldview_ui, catalog, self.doc.year);
            });
        let current = self.current_book_node();
        let mut action = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(self.worldview_ui.category.name());
            action = detail(ui, &mut self.worldview_ui, catalog, &self.doc, current);
        });
        self.apply_worldview(action);
    }

    fn apply_worldview(&mut self, action: Option<Action>) {
        let Some(action) = action else { return };
        match action {
            Action::Save | Action::Remove => {
                let Some(draft) = self.worldview_ui.draft.clone() else {
                    return;
                };
                let chosen = self.worldview_ui.id;
                let category = self.worldview_ui.category;
                let remove = matches!(action, Action::Remove);
                let mut validation = None;
                let result = self.edit("修改世界观设定", |doc| {
                    let change = |doc: &mut Document| -> Result<Id, String> {
                        if remove
                            && category == Category::Records
                            && chosen.is_some_and(|id| {
                                doc.book.as_ref().is_some_and(|b| {
                                    b.connections(Node::Record(id)).next().is_some()
                                })
                            })
                        {
                            return Err("请先解除条目与资料或地图的链接。".into());
                        }
                        let catalog = Catalog::edit(doc);
                        let id = chosen.map_or_else(|| catalog.alloc(), Ok)?;
                        if remove {
                            match category {
                                Category::Records => {
                                    catalog.records.remove(&id);
                                }
                                Category::Types => {
                                    catalog.schemas.remove(&id);
                                }
                                Category::RelationTypes => {
                                    catalog.relation_types.remove(&id);
                                }
                                Category::Relations => {
                                    catalog.relations.remove(&id);
                                }
                                Category::Templates => {
                                    catalog.templates.remove(&id);
                                }
                            }
                        } else {
                            draft.store(catalog, id);
                        }
                        catalog.validate()?;
                        Ok(id)
                    };
                    change(doc).inspect_err(|error| validation = Some(error.clone()))
                });
                if let Some(id) = result {
                    self.worldview_ui.id = (!remove).then_some(id);
                    if remove {
                        self.worldview_ui.draft = None;
                    }
                }
                self.worldview_ui.error = validation;
            }
            Action::Associate(id) => {
                let to = self.current_book_node();
                let from = Node::Record(id);
                let mut link = Link::new(from, to, false);
                link.time = TimeLink::Inherit;
                self.edit("关联世界观条目", |doc| {
                    WorldBook::register_anchor(doc, to);
                    let book = WorldBook::edit(doc);
                    if book
                        .connections(from)
                        .any(|(_, link)| link.to == to || link.from == to)
                    {
                        return Err("已有此关联，可在世界资料中调整。".into());
                    }
                    book.add_link(link)
                });
            }
            Action::BrowseLinks(id) => self.book_context(Node::Record(id)),
            Action::Open(node, year) => {
                if self.studio.foreground && node.board().is_some() {
                    self.studio.show_map();
                }
                self.open_book_node(node, year);
            }
            Action::MakeTemplate => {
                let Some(Draft::Record(record)) = &self.worldview_ui.draft else {
                    return;
                };
                let values = record
                    .values
                    .iter()
                    .filter_map(|(key, series)| {
                        let value = series
                            .pinned
                            .as_ref()
                            .or_else(|| series.manual(self.doc.year))?;
                        Some((key.clone(), value.clone()))
                    })
                    .collect();
                let template = Template {
                    name: record.name.clone(),
                    schema: record.schema,
                    meta: record.meta.clone(),
                    values,
                };
                self.worldview_ui.set_category(Category::Templates);
                self.worldview_ui.draft = Some(Draft::Template(template));
            }
        }
    }
}

fn modes(ui: &mut egui::Ui, state: &mut WorldviewUi, vertical: bool) {
    let buttons = |ui: &mut egui::Ui| {
        for category in Category::ALL {
            if ui
                .selectable_label(category == state.category, category.name())
                .clicked()
            {
                state.set_category(category);
            }
        }
    };
    if vertical {
        ui.vertical(buttons);
    } else {
        ui.horizontal_wrapped(buttons);
    }
}

fn list(catalog: &Catalog, state: &WorldviewUi, year: Option<i32>) -> Vec<(Id, String)> {
    let mut entries: Vec<_> = match state.category {
        Category::Records => catalog
            .records
            .iter()
            .filter(|(_, r)| !state.only_year || r.meta.visible(year))
            .map(|(&id, r)| (id, r.name.clone()))
            .collect(),
        Category::Types => catalog
            .schemas
            .iter()
            .map(|(&id, x)| (id, x.name.clone()))
            .collect(),
        Category::RelationTypes => catalog
            .relation_types
            .iter()
            .map(|(&id, x)| (id, x.name.clone()))
            .collect(),
        Category::Templates => catalog
            .templates
            .iter()
            .map(|(&id, x)| (id, x.name.clone()))
            .collect(),
        Category::Relations => catalog
            .relations
            .iter()
            .filter(|(_, r)| !state.only_year || r.meta.visible(year))
            .map(|(&id, r)| (id, relation_name(catalog, r)))
            .collect(),
    };
    if !state.search.is_empty() {
        let term = state.search.trim().to_lowercase();
        entries.retain(|(id, name)| {
            name.to_lowercase().contains(&term)
                || (state.category == Category::Records && catalog.records[id].meta.matches(&term))
        });
    }
    entries.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    entries
}

fn relation_name(catalog: &Catalog, relation: &Relation) -> String {
    format!(
        "{} → {} · {}",
        catalog
            .records
            .get(&relation.from)
            .map_or("—", |r| r.name.as_str()),
        catalog
            .records
            .get(&relation.to)
            .map_or("—", |r| r.name.as_str()),
        catalog
            .relation_types
            .get(&relation.kind)
            .map_or("—", |r| r.name.as_str())
    )
}

fn index(ui: &mut egui::Ui, state: &mut WorldviewUi, catalog: &Catalog, year: Option<i32>) {
    ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(tr("搜索名称、标签与备注")));
    if matches!(state.category, Category::Records | Category::Relations) && year.is_some() {
        ui.checkbox(&mut state.only_year, tr("仅显示当前年份"));
    }
    if ui.button(tr("新建")).clicked() {
        let schema = catalog.schemas.keys().next().copied().unwrap_or_default();
        state.id = None;
        state.dated = year;
        state.error = None;
        state.new_key.clear();
        state.draft = Some(match state.category {
            Category::Records => Draft::Record(Record {
                schema,
                ..Default::default()
            }),
            Category::Types => Draft::Schema(Schema::default()),
            Category::RelationTypes => Draft::RelationType(RelationType::default()),
            Category::Relations => Draft::Relation(Relation {
                kind: catalog
                    .relation_types
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or_default(),
                ..Default::default()
            }),
            Category::Templates => Draft::Template(Template {
                schema,
                ..Default::default()
            }),
        });
    }
    if state.category == Category::Records && !catalog.templates.is_empty() {
        ui.menu_button(tr("从模板新建"), |ui| {
            for (&id, template) in &catalog.templates {
                if ui.button(&template.name).clicked() {
                    if let Ok(record) = catalog.from_template(id, template.name.clone()) {
                        state.id = None;
                        state.dated = year;
                        state.error = None;
                        state.draft = Some(Draft::Record(record));
                    }
                    ui.close();
                }
            }
        });
    }
    ui.separator();
    let entries = list(catalog, state, year);
    let height = ui.text_style_height(&egui::TextStyle::Body) + 5.0;
    egui::ScrollArea::vertical()
        .id_salt("worldview-record-index")
        .show_rows(ui, height, entries.len(), |ui, range| {
            for (id, label) in &entries[range] {
                if ui.selectable_label(state.id == Some(*id), label).clicked() {
                    state.select(catalog, *id, year);
                }
            }
        });
}

fn detail(
    ui: &mut egui::Ui,
    state: &mut WorldviewUi,
    catalog: &Catalog,
    doc: &Document,
    current: Node,
) -> Option<Action> {
    let Some(object) = &mut state.draft else {
        ui.label(tr("选择或新建一个条目。"));
        if catalog.schemas.is_empty() {
            ui.label(tr("从实体类型开始定义人物、家族、国家或任何其他内容。"));
        }
        return None;
    };
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("保存修改")).clicked() {
            action = Some(Action::Save);
        }
        if let Some(id) = state.id {
            if ui.button(tr("撤回未保存修改")).clicked() {
                if let Some(saved) = Draft::read(catalog, state.category, id) {
                    *object = saved;
                }
                state.error = None;
            }
            ui.menu_button(tr("删除…"), |ui| {
                if ui.button(tr("删除当前条目")).clicked() {
                    action = Some(Action::Remove);
                    ui.close();
                }
            });
        }
    });
    if let Some(error) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, error);
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("worldview-properties")
        .show(ui, |ui| match object {
            Draft::Schema(schema) => schema_form(ui, schema, &mut state.new_key, catalog),
            Draft::Record(record) => {
                record_form(ui, record, state.id, &mut state.dated, catalog, doc);
                if let Some(id) = state.id {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr("存为模板")).clicked() {
                            action = Some(Action::MakeTemplate);
                        }
                        let selected_name = doc
                            .book
                            .as_ref()
                            .and_then(|b| b.label(doc, current))
                            .unwrap_or_else(|| doc.meta.name.clone());
                        if ui
                            .button(tr("关联当前地图选择"))
                            .on_hover_text(selected_name)
                            .clicked()
                        {
                            action = Some(Action::Associate(id));
                        }
                        if ui.button(tr("查看或编辑全部关联")).clicked() {
                            action = Some(Action::BrowseLinks(id));
                        }
                    });
                    show_links(ui, doc, id, &mut action);
                    for relation in catalog
                        .relations
                        .values()
                        .filter(|r| (r.from == id || r.to == id) && r.meta.visible(doc.year))
                    {
                        let other = if relation.from == id {
                            relation.to
                        } else {
                            relation.from
                        };
                        if ui.link(relation_name(catalog, relation)).clicked() {
                            action = Some(Action::Open(Node::Record(other), None));
                        }
                    }
                }
            }
            Draft::RelationType(kind) => {
                text(ui, "名称", &mut kind.name);
                text(ui, "说明", &mut kind.description);
                choose_schema(ui, "来源类型", &mut kind.source, catalog, true);
                choose_schema(ui, "目标类型", &mut kind.target, catalog, true);
            }
            Draft::Relation(relation) => {
                picker(
                    ui,
                    "关系类型",
                    &mut relation.kind,
                    catalog
                        .relation_types
                        .iter()
                        .map(|(&id, x)| (id, x.name.as_str())),
                );
                let kind = catalog.relation_types.get(&relation.kind);
                record_picker(
                    ui,
                    "起点",
                    &mut relation.from,
                    kind.and_then(|k| k.source),
                    catalog,
                );
                record_picker(
                    ui,
                    "终点",
                    &mut relation.to,
                    kind.and_then(|k| k.target),
                    catalog,
                );
                meta_input(ui, &mut relation.meta);
            }
            Draft::Template(template) => {
                text(ui, "名称", &mut template.name);
                let mut id = Some(template.schema);
                choose_schema(ui, "实体类型", &mut id, catalog, false);
                if let Some(id) = id {
                    if template.schema != id {
                        template.values.clear();
                    }
                    template.schema = id;
                }
                meta_input(ui, &mut template.meta);
                if let Some(schema) = catalog.schemas.get(&template.schema) {
                    for (key, field) in &schema.fields {
                        ui.push_id(key, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(&field.name);
                                let mut value = template.values.get(key).cloned();
                                value_input(ui, &mut value, field.kind, catalog);
                                match value {
                                    Some(value) => {
                                        template.values.insert(key.clone(), value);
                                    }
                                    None => {
                                        template.values.remove(key);
                                    }
                                }
                            });
                        });
                    }
                }
            }
        });
    action
}

fn text(ui: &mut egui::Ui, label: &'static str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(tr(label));
        ui.text_edit_singleline(value);
    });
}

fn choose_schema(
    ui: &mut egui::Ui,
    label: &'static str,
    value: &mut Option<Id>,
    catalog: &Catalog,
    optional: bool,
) {
    let mut id = value.unwrap_or_default();
    picker(
        ui,
        label,
        &mut id,
        optional
            .then_some((0, tr("任意类型")))
            .into_iter()
            .chain(catalog.schemas.iter().map(|(&id, x)| (id, x.name.as_str()))),
    );
    *value = (id != 0).then_some(id);
}

fn record_picker(
    ui: &mut egui::Ui,
    label: &'static str,
    id: &mut Id,
    schema: Option<Id>,
    catalog: &Catalog,
) {
    picker(
        ui,
        label,
        id,
        catalog
            .records
            .iter()
            .filter(|(_, record)| schema.is_none_or(|id| record.schema == id))
            .map(|(&id, record)| (id, record.name.as_str())),
    );
}

fn picker<'a>(
    ui: &mut egui::Ui,
    label: &'static str,
    id: &mut Id,
    entries: impl Iterator<Item = (Id, &'a str)> + Clone,
) {
    let selected = entries
        .clone()
        .find(|(key, _)| key == id)
        .map_or(tr("请选择"), |(_, name)| name);
    ui.horizontal(|ui| {
        ui.label(tr(label));
        egui::ComboBox::from_id_salt(label)
            .width(170.0)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                let height = ui.text_style_height(&egui::TextStyle::Body) + 5.0;
                let entries: Vec<_> = entries.collect();
                egui::ScrollArea::vertical().max_height(260.0).show_rows(
                    ui,
                    height,
                    entries.len(),
                    |ui, range| {
                        for (key, name) in &entries[range] {
                            if ui.selectable_value(id, *key, *name).clicked() {
                                ui.close();
                            }
                        }
                    },
                );
            });
    });
}

fn dated_input(ui: &mut egui::Ui, label: &'static str, year: &mut Option<i32>, initial: i32) {
    ui.horizontal(|ui| {
        let mut on = year.is_some();
        if ui.checkbox(&mut on, tr(label)).changed() {
            *year = on.then_some(initial);
        }
        if let Some(value) = year {
            ui.add(
                egui::DragValue::new(value)
                    .range(-1_000_000..=1_000_000)
                    .speed(1.0),
            );
        }
    });
}

fn meta_input(ui: &mut egui::Ui, meta: &mut Meta) {
    ui.collapsing(tr("分类、时间与说明"), |ui| {
        text(ui, "分类", &mut meta.category);
        text(ui, "标签", &mut meta.tags);
        ui.add(
            egui::TextEdit::multiline(&mut meta.description)
                .desired_rows(3)
                .hint_text(tr("说明")),
        );
        dated_input(ui, "开始年", &mut meta.from, 1900);
        dated_input(ui, "结束年", &mut meta.until, meta.from.unwrap_or(1900));
    });
}

fn value_input(ui: &mut egui::Ui, value: &mut Option<Value>, kind: Kind, catalog: &Catalog) {
    if kind == Kind::Reference {
        let mut id = match value.as_ref() {
            Some(Value::Reference(id)) => *id,
            _ => 0,
        };
        picker(
            ui,
            "实体引用",
            &mut id,
            std::iter::once((0, tr("无")))
                .chain(catalog.records.iter().map(|(&id, r)| (id, r.name.as_str()))),
        );
        *value = (id != 0).then_some(Value::Reference(id));
        return;
    }
    let mut on = value.is_some();
    if ui.checkbox(&mut on, tr("设置")).changed() {
        *value = if on { Value::blank(kind) } else { None };
    }
    match value {
        Some(Value::Text(s)) => {
            ui.text_edit_singleline(s);
        }
        Some(Value::Number(n)) => {
            ui.add(egui::DragValue::new(n).speed(0.1));
        }
        Some(Value::Flag(b)) => {
            ui.checkbox(b, tr("是"));
        }
        _ => {}
    }
}

fn kind_name(kind: Kind) -> &'static str {
    tr(match kind {
        Kind::Text => "文本",
        Kind::Number => "数值",
        Kind::Flag => "布尔",
        Kind::Reference => "实体引用",
    })
}

fn schema_form(ui: &mut egui::Ui, schema: &mut Schema, new_key: &mut String, catalog: &Catalog) {
    text(ui, "名称", &mut schema.name);
    text(ui, "说明", &mut schema.description);
    ui.separator();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(new_key).hint_text(tr("属性标识，如 population")));
        if ui
            .add_enabled(
                !new_key.trim().is_empty(),
                egui::Button::new(tr("添加属性")),
            )
            .clicked()
        {
            let key = new_key.trim().to_string();
            schema.fields.entry(key.clone()).or_insert_with(|| Field {
                name: key,
                ..Default::default()
            });
            new_key.clear();
        }
    });
    let mut remove = None;
    for (key, field) in &mut schema.fields {
        ui.push_id(key, |ui| {
            egui::CollapsingHeader::new(format!("{} · {}", field.name, key))
                .id_salt("field-input")
                .show(ui, |ui| {
                    text(ui, "名称", &mut field.name);
                    text(ui, "单位", &mut field.unit);
                    let previous = field.kind;
                    egui::ComboBox::from_id_salt("kind")
                        .selected_text(kind_name(field.kind))
                        .show_ui(ui, |ui| {
                            for kind in [Kind::Text, Kind::Number, Kind::Flag, Kind::Reference] {
                                ui.selectable_value(&mut field.kind, kind, kind_name(kind));
                            }
                        });
                    if previous != field.kind {
                        field.default = None;
                        if field.kind != Kind::Number {
                            field.rule = None;
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("默认值"));
                        value_input(ui, &mut field.default, field.kind, catalog);
                    });
                    if field.kind == Kind::Number {
                        rule_form(ui, &mut field.rule, catalog);
                    }
                    if ui.button(tr("移除属性")).clicked() {
                        remove = Some(key.clone());
                    }
                });
        });
    }
    if let Some(key) = remove {
        schema.fields.remove(&key);
    }
}

fn rule_form(ui: &mut egui::Ui, rule: &mut Option<Rule>, catalog: &Catalog) {
    let mut enabled = rule.is_some();
    if ui.checkbox(&mut enabled, tr("使用数值规则")).changed() {
        *rule = enabled.then(Rule::default);
    }
    let Some(rule) = rule else { return };
    text(ui, "公式", &mut rule.expression);
    ui.label(tr("公式可引用其他数值属性、year、map_area 及下列来源。"));
    let draft_id = ui.scope_id().with("binding");
    let mut input_key = ui.data_mut(|data| data.get_temp::<String>(draft_id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut input_key).hint_text(tr("变量标识")));
        if ui
            .add_enabled(
                !input_key.trim().is_empty(),
                egui::Button::new(tr("添加来源")),
            )
            .clicked()
        {
            rule.inputs
                .entry(input_key.trim().into())
                .or_insert_with(|| Input::MapField {
                    key: map_core::doc::AREA_KEY.to_string(),
                    aggregate: Aggregate::Sum,
                });
            input_key.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(draft_id, input_key));
    let mut delete = None;
    for (key, input) in &mut rule.inputs {
        ui.push_id(key, |ui| {
            let previous = match input {
                Input::MapField { .. } => 0,
                Input::RelatedCount { .. } => 1,
                Input::RelatedField { .. } => 2,
                Input::Environment { .. } => 3,
            };
            let mut mode = previous;
            ui.horizontal(|ui| {
                ui.strong(key);
                let modes = [tr("地图字段"), tr("关联实体数量"), tr("关联实体属性")];
                egui::ComboBox::from_id_salt("input-kind")
                    .selected_text(modes[mode])
                    .show_ui(ui, |ui| {
                        for (index, name) in modes.into_iter().enumerate() {
                            ui.selectable_value(&mut mode, index, name);
                        }
                    });
                if ui.small_button("×").clicked() {
                    delete = Some(key.clone());
                }
            });
            if mode != previous {
                let relation = catalog
                    .relation_types
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or_default();
                let direction = Direction::Outgoing;
                *input = match mode {
                    1 => Input::RelatedCount {
                        relation,
                        direction,
                    },
                    2 => Input::RelatedField {
                        relation,
                        direction,
                        key: String::new(),
                        aggregate: Aggregate::Sum,
                    },
                    3 => Input::Environment {
                        layer: map_core::environment::Layer::default(),
                        aggregate: Aggregate::Mean,
                    },
                    _ => Input::MapField {
                        key: map_core::doc::AREA_KEY.into(),
                        aggregate: Aggregate::Sum,
                    },
                };
            }
            match input {
                Input::Environment { layer, aggregate } => {
                    use map_core::environment::{Field, Layer};
                    egui::ComboBox::from_id_salt("environment-stat")
                        .selected_text(layer.name())
                        .show_ui(ui, |ui| {
                            for field in Field::ALL {
                                ui.selectable_value(
                                    layer,
                                    Layer::Standard(field),
                                    tr(field.name()),
                                );
                            }
                            ui.menu_button(tr("自定义图层名"), |ui| {
                                if let Layer::Suitability(name) = layer {
                                    ui.text_edit_singleline(name);
                                } else if ui.button(tr("填写图层名")).clicked() {
                                    *layer = Layer::Suitability(String::new());
                                }
                            });
                        });
                    aggregate_picker(ui, aggregate);
                }
                Input::MapField { key, aggregate } => {
                    text(ui, "地图字段标识", key);
                    aggregate_picker(ui, aggregate);
                }
                Input::RelatedCount {
                    relation,
                    direction,
                } => relation_binding(ui, relation, direction, catalog),
                Input::RelatedField {
                    relation,
                    direction,
                    key,
                    aggregate,
                } => {
                    relation_binding(ui, relation, direction, catalog);
                    text(ui, "目标属性标识", key);
                    aggregate_picker(ui, aggregate);
                }
            }
        });
    }
    if let Some(key) = delete {
        rule.inputs.remove(&key);
    }
}

fn aggregate_picker(ui: &mut egui::Ui, aggregate: &mut Aggregate) {
    let items = [
        (Aggregate::Sum, "合计"),
        (Aggregate::Mean, "平均"),
        (Aggregate::Min, "最小"),
        (Aggregate::Max, "最大"),
    ];
    let label = items
        .iter()
        .find(|(value, _)| value == aggregate)
        .unwrap()
        .1;
    egui::ComboBox::from_id_salt("aggregation")
        .selected_text(tr(label))
        .show_ui(ui, |ui| {
            for (value, name) in items {
                ui.selectable_value(aggregate, value, tr(name));
            }
        });
}

fn relation_binding(ui: &mut egui::Ui, id: &mut Id, direction: &mut Direction, catalog: &Catalog) {
    picker(
        ui,
        "关系类型",
        id,
        catalog
            .relation_types
            .iter()
            .map(|(&id, value)| (id, value.name.as_str())),
    );
    ui.horizontal_wrapped(|ui| {
        for (value, name) in [
            (Direction::Outgoing, "出向"),
            (Direction::Incoming, "入向"),
            (Direction::Both, "双向"),
        ] {
            ui.selectable_value(direction, value, tr(name));
        }
    });
}

fn record_form(
    ui: &mut egui::Ui,
    record: &mut Record,
    id: Option<Id>,
    year: &mut Option<i32>,
    catalog: &Catalog,
    doc: &Document,
) {
    text(ui, "名称", &mut record.name);
    if id.is_none() {
        let mut choice = Some(record.schema);
        choose_schema(ui, "实体类型", &mut choice, catalog, false);
        if let Some(choice) = choice {
            if record.schema != choice {
                record.values.clear();
            }
            record.schema = choice;
        }
    }
    let Some(schema) = catalog.schemas.get(&record.schema) else {
        ui.label(tr("请先创建实体类型。"));
        return;
    };
    ui.label(&schema.name);
    meta_input(ui, &mut record.meta);
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        dated_input(ui, "编辑指定年份", year, doc.year.unwrap_or(1900));
        if let Some(cursor) = doc.year {
            if ui
                .button(format!("{} {cursor}", tr("使用时间轴年份")))
                .clicked()
            {
                *year = Some(cursor);
            }
        }
    });
    if let Some(year) = doc.year {
        ui.label(format!("{} {year}", tr("时间轴结果")));
    }
    if year.is_none() {
        ui.label(tr("正在编辑基础值。"));
    }
    let mut evaluation = id.and_then(|_| Evaluation::new(doc, doc.year));
    for (key, field) in &schema.fields {
        ui.push_id(key, |ui| {
            let series = record.values.entry(key.clone()).or_default();
            let mut value = match year {
                Some(year) => series.history.get(year).cloned(),
                None => series.base.clone(),
            };
            let resolved = evaluation
                .as_mut()
                .zip(id)
                .map_or(Ok(None), |(eval, id)| eval.get(id, key));
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(&field.name);
                    if !field.unit.is_empty() {
                        ui.label(&field.unit);
                    }
                    match &resolved {
                        Ok(Some(resolved)) => {
                            let tag = tr(match resolved.source {
                                Source::Pinned => "锁定值",
                                Source::Manual => "手动值",
                                Source::Computed => "规则结果",
                                Source::Default => "默认值",
                            });
                            ui.label(format!("{tag} {}", value_text(&resolved.value, catalog)));
                        }
                        Err(message) => {
                            ui.colored_label(Color32::LIGHT_RED, message);
                        }
                        _ => {}
                    };
                });
                let previous = value.clone();
                ui.add_enabled_ui(series.pinned.is_none(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        value_input(ui, &mut value, field.kind, catalog);
                    });
                });
                if value != previous {
                    series.write(*year, value.clone());
                }
                let shown = resolved.ok().flatten().map(|r| r.value);
                let candidate = if *year == doc.year {
                    value.or(shown)
                } else {
                    shown.or(value)
                }
                .or_else(|| series.manual(*year).cloned());
                if series.pinned.is_some() {
                    if ui.small_button(tr("解除属性锁定")).clicked() {
                        series.pinned = None;
                    }
                } else if ui
                    .add_enabled(candidate.is_some(), egui::Button::new(tr("锁定此值")))
                    .clicked()
                {
                    series.pinned = candidate;
                }
                if !series.history.is_empty() {
                    ui.collapsing(tr("已录入年份"), |ui| {
                        for (at, value) in &series.history {
                            if ui
                                .link(format!("{at} · {}", value_text(value, catalog)))
                                .clicked()
                            {
                                *year = Some(*at);
                            }
                        }
                    });
                }
            });
        });
    }
    record
        .values
        .retain(|_, s| s.base.is_some() || s.pinned.is_some() || !s.history.is_empty());
}

fn value_text(value: &Value, catalog: &Catalog) -> String {
    match value {
        Value::Text(v) => v.clone(),
        Value::Number(v) => format!("{v}"),
        Value::Flag(v) => tr(if *v { "是" } else { "否" }).to_owned(),
        Value::Reference(id) => catalog
            .records
            .get(id)
            .map_or_else(|| id.to_string(), |r| r.name.clone()),
    }
}

fn show_links(ui: &mut egui::Ui, doc: &Document, id: Id, action: &mut Option<Action>) {
    let Some(book) = doc.book.as_ref() else {
        return;
    };
    for (_, link) in book
        .connections(Node::Record(id))
        .filter(|(_, link)| link.visible(doc.year))
    {
        let node = if link.from == Node::Record(id) {
            link.to
        } else {
            link.from
        };
        let node = book.resolve(doc, node);
        if let Some(name) = book.label(doc, node) {
            if ui.link(name).clicked() {
                *action = Some(Action::Open(node, link.year(doc.year)));
            }
        }
    }
}
