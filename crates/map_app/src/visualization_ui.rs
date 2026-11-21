use crate::{app::App, i18n::tr, view::ViewGeom};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use map_core::{
    environment::{Field, Layer},
    visualization::{Data, Dataset, Icon, Plot, Project, Rule, Spec, Years},
    world_book::WorldBook,
    worldview::{Catalog, Id, Kind},
    Document, EntityId, LonLat,
};
use std::{path::PathBuf, sync::Arc, thread::JoinHandle};

mod plot;

struct Prepared {
    data: Dataset,
    index: plot::Index,
}

struct Evaluated {
    spec: Arc<Spec>,
    input: Document,
    data: Result<Arc<Prepared>, String>,
}

struct Job {
    spec: Arc<Spec>,
    input: Document,
    worker: JoinHandle<Result<Prepared, String>>,
}

struct Import {
    path: PathBuf,
    names: Vec<String>,
    label: Option<usize>,
    x: Option<usize>,
    value: usize,
    lon: Option<usize>,
    lat: Option<usize>,
    node: Option<usize>,
}

#[derive(Default)]
pub(crate) struct VisualizationUi {
    open: bool,
    editor: Option<Spec>,
    chosen: Option<Id>,
    search: String,
    options: bool,
    import: Option<Import>,
    notice: String,
    preview_draft: Option<Arc<Spec>>,
    preview: Option<Evaluated>,
    job: Option<Job>,
}

impl VisualizationUi {
    pub(crate) fn reload(&mut self, doc: &Document) {
        if let Some(id) = self.chosen {
            self.editor = doc
                .visualization
                .as_ref()
                .and_then(|p| p.specs.get(&id))
                .map(|s| (**s).clone());
            if self.editor.is_none() {
                self.chosen = None;
            }
        }
        self.preview_draft = None;
    }

    fn active_spec(&self, doc: &Document) -> Option<Arc<Spec>> {
        self.preview_draft.clone().or_else(|| {
            doc.visualization
                .as_ref()?
                .specs
                .get(&self.chosen?)
                .cloned()
        })
    }

    fn preview(&mut self, doc: &Document, ctx: &egui::Context) {
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.job.take().unwrap();
            let data = job
                .worker
                .join()
                .unwrap_or_else(|_| Err("图表计算线程意外停止。".into()))
                .map(Arc::new);
            self.preview = Some(Evaluated {
                spec: job.spec,
                input: job.input,
                data,
            });
        }
        let Some(spec) = self.active_spec(doc) else {
            self.preview = None;
            return;
        };
        let fresh = self
            .preview
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.spec, &spec) && same_data(&p.input, doc));
        if !fresh && self.job.is_none() {
            let input = doc.clone();
            let d = input.clone();
            let s = spec.clone();
            let repaint = ctx.clone();
            let worker = crate::perf_ui::spawn("生成专题可视化", move || {
                let result = s.evaluate(&d).map(|data| Prepared {
                    index: plot::Index::new(&s, &data),
                    data,
                });
                repaint.request_repaint();
                result
            });
            self.job = Some(Job {
                spec,
                input,
                worker,
            });
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(180));
        }
    }
}

fn same_data(a: &Document, b: &Document) -> bool {
    let same = |x: Option<&Arc<map_core::environment::Generation>>,
                y: Option<&Arc<map_core::environment::Generation>>| {
        match (x, y) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    };
    let env = |d: &Document| d.environment.as_ref().and_then(|p| p.current.clone());
    WorldBook::current(a) == WorldBook::current(b)
        && a.year == b.year
        && a.owner.ptr_eq(&b.owner)
        && a.entities.ptr_eq(&b.entities)
        && a.owner_changes.ptr_eq(&b.owner_changes)
        && a.values.ptr_eq(&b.values)
        && a.fields.ptr_eq(&b.fields)
        && a.topo.faces.ptr_eq(&b.topo.faces)
        && a.radius() == b.radius()
        && match (&a.book, &b.book) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
        && match (&a.terrain, &b.terrain) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
        && same(env(a).as_ref(), env(b).as_ref())
}

enum Action {
    Save,
    Preview,
    Overlay(Option<Id>),
    ViewOnMap(Id),
    Delete(Id),
    BrowseCsv,
    ReadCsv,
    WriteCsv,
}

impl App {
    pub(crate) fn open_visualizations(&mut self) {
        self.visualization_ui.open = true;
        if self.studio.foreground {
            self.studio.show_visualizations();
        }
    }

    pub(crate) fn visualization_window(&mut self, ctx: &egui::Context) {
        if !self.visualization_ui.open || self.studio.foreground {
            return;
        }
        self.visualization_ui.preview(&self.doc, ctx);
        let mut open = true;
        egui::Window::new(tr("数据可视化"))
            .id(egui::Id::unique("atlas-charts"))
            .default_size([870.0, 620.0])
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                let mut action = None;
                ui.horizontal_wrapped(|ui| {
                    self.catalog_chart(ui);
                    ui.separator();
                    ui.selectable_value(&mut self.visualization_ui.options, false, tr("图表预览"));
                    ui.selectable_value(&mut self.visualization_ui.options, true, tr("数据与规则"));
                });
                ui.separator();
                if self.visualization_ui.options {
                    action = self.chart_editor(ui);
                } else {
                    self.chart_preview(ui);
                }
                self.chart_action(action);
            });
        self.visualization_ui.open &= open;
    }

    pub(crate) fn studio_visualizations(&mut self, ui: &mut egui::Ui) {
        self.visualization_ui.preview(&self.doc, ui.ctx());
        egui::Panel::left("studio-chart-index")
            .default_size(195.0)
            .size_range(170.0..=280.0)
            .show(ui, |ui| {
                ui.heading(tr("数据可视化"));
                self.catalog_chart(ui);
                ui.separator();
                ui.checkbox(&mut self.visualization_ui.options, tr("编辑数据与规则"));
            });
        let mut action = None;
        if self.visualization_ui.options {
            egui::Panel::right("studio-chart-rules")
                .default_size(405.0)
                .size_range(320.0..=600.0)
                .resizable(true)
                .show(ui, |ui| action = self.chart_editor(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.chart_preview(ui));
        self.chart_action(action);
    }

    fn catalog_chart(&mut self, ui: &mut egui::Ui) {
        let state = &mut self.visualization_ui;
        if ui.button(tr("新建图表")).clicked() {
            state.chosen = None;
            state.editor = Some(Spec::default());
            state.options = true;
            state.preview_draft = None;
            state.import = None;
        }
        if let Some(project) = &self.doc.visualization {
            let label = state
                .chosen
                .and_then(|id| project.specs.get(&id))
                .map_or(tr("选择图表"), |s| &s.name);
            egui::ComboBox::from_id_salt("saved-chart")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(tr("搜索")));
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            let search = state.search.to_lowercase();
                            for (&id, spec) in &project.specs {
                                if spec.name.to_lowercase().contains(&search)
                                    && ui
                                        .selectable_label(state.chosen == Some(id), &spec.name)
                                        .clicked()
                                {
                                    state.chosen = Some(id);
                                    state.editor = Some((**spec).clone());
                                    state.preview_draft = None;
                                    state.import = None;
                                    ui.close();
                                }
                            }
                        });
                });
        }
        if !state.notice.is_empty() {
            ui.label(&state.notice);
        }
    }

    fn chart_editor(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let state = &mut self.visualization_ui;
        let Some(spec) = &mut state.editor else {
            ui.label(tr("选择或新建一个图表。"));
            return None;
        };
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("保存修改")).clicked() {
                action = Some(Action::Save);
            }
            if ui.button(tr("预览草稿")).clicked() {
                action = Some(Action::Preview);
            }
            if let Some(id) = state.chosen {
                ui.menu_button(tr("删除…"), |ui| {
                    if ui.button(tr("删除当前图表")).clicked() {
                        action = Some(Action::Delete(id));
                        ui.close();
                    }
                });
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("chart-editor")
            .show(ui, |ui| {
                labeled_text(ui, "名称", &mut spec.name);
                egui::ComboBox::from_id_salt("chart-kind")
                    .selected_text(tr(spec.plot.name()))
                    .show_ui(ui, |ui| {
                        for mode in Plot::ALL {
                            if ui
                                .selectable_value(&mut spec.plot, mode, tr(mode.name()))
                                .changed()
                            {
                                if mode == Plot::Relations {
                                    spec.data = Data::Network {
                                        schema: None,
                                        relation: None,
                                    };
                                }
                                if mode == Plot::Profile
                                    && !matches!(spec.data, Data::Transect { .. })
                                {
                                    spec.data = default_transect();
                                }
                            }
                        }
                    });
                source(ui, &mut spec.data, &self.doc, &mut action);
                if let Some(import) = &mut state.import {
                    import_form(ui, import, &mut action);
                }
                ui.separator();
                ui.collapsing(tr("可视化规则"), |ui| rule_form(ui, &mut spec.rule));
                if let Some(id) = state.chosen.filter(|_| spec.plot.geographic()) {
                    let is_active = self
                        .doc
                        .visualization
                        .as_ref()
                        .is_some_and(|p| p.overlay == Some(id));
                    let mut overlay = is_active;
                    if ui.checkbox(&mut overlay, tr("在地图中显示")).changed() {
                        action = Some(Action::Overlay(overlay.then_some(id)));
                    }
                }
            });
        action
    }

    fn chart_preview(&mut self, ui: &mut egui::Ui) {
        let state = &mut self.visualization_ui;
        if state.job.is_some() {
            ui.label(tr("正在更新图表…"));
        }
        let Some(result) = &state.preview else {
            return;
        };
        ui.heading(&result.spec.name);
        let prepared = match &result.data {
            Ok(data) => data.clone(),
            Err(e) => {
                ui.colored_label(Color32::LIGHT_RED, e);
                return;
            }
        };
        let data = &prepared.data;
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{} {}", data.rows.len(), tr("条数据")));
            if !data.edges.is_empty() {
                ui.label(format!("{} {}", data.edges.len(), tr("条关系")));
            }
            if ui.button(tr("导出数据 CSV")).clicked() {
                action = Some(Action::WriteCsv);
            }
            let geo = state.chosen.filter(|&id| {
                state.preview_draft.is_none()
                    && self
                        .doc
                        .visualization
                        .as_ref()
                        .and_then(|p| p.specs.get(&id))
                        .is_some_and(|s| s.plot.geographic())
            });
            if let Some(id) = geo {
                if ui.button(tr("在地图查看")).clicked() {
                    action = Some(Action::ViewOnMap(id));
                }
            }
        });
        for warning in &data.warnings {
            ui.weak(warning);
        }
        plot::draw(ui, &result.spec, data, &prepared.index);
        self.chart_action(action);
    }

    pub(crate) fn delete_chart_selection(&mut self) {
        if let Some(id) = self.visualization_ui.chosen {
            self.chart_action(Some(Action::Delete(id)));
        }
    }

    fn chart_action(&mut self, action: Option<Action>) {
        let Some(action) = action else { return };
        match action {
            Action::Save => {
                let Some(spec) = self.visualization_ui.editor.clone() else {
                    return;
                };
                let previous = self.visualization_ui.chosen;
                if let Some(id) = self.edit("保存可视化规则", |doc| {
                    Arc::make_mut(
                        doc.visualization
                            .get_or_insert_with(|| Arc::new(Project::default())),
                    )
                    .insert(previous, spec)
                }) {
                    self.visualization_ui.chosen = Some(id);
                    self.visualization_ui.preview_draft = None;
                    self.visualization_ui.notice.clear();
                }
            }
            Action::Preview => {
                if let Some(spec) = self.visualization_ui.editor.clone() {
                    self.visualization_ui.preview_draft = Some(Arc::new(spec));
                    if !self.studio.foreground {
                        self.visualization_ui.options = false;
                    }
                }
            }
            Action::Delete(id) | Action::Overlay(Some(id)) | Action::ViewOnMap(id) => {
                let remove = matches!(action, Action::Delete(_));
                let view = matches!(action, Action::ViewOnMap(_));
                if self
                    .edit("修改可视化图层", |doc| {
                        let p =
                            Arc::make_mut(doc.visualization.as_mut().ok_or("可视化工程不存在。")?);
                        if remove {
                            p.specs.remove(&id);
                            if p.overlay == Some(id) {
                                p.overlay = None;
                            }
                        } else if p.specs.get(&id).is_some_and(|spec| spec.plot.geographic()) {
                            p.overlay = Some(id);
                        }
                        Ok(())
                    })
                    .is_some()
                    && remove
                {
                    self.visualization_ui.chosen = None;
                    self.visualization_ui.editor = None;
                    self.visualization_ui.preview_draft = None;
                }
                if view {
                    self.visualization_ui.open = false;
                    self.studio.show_map();
                }
            }
            Action::Overlay(None) => {
                self.edit("隐藏可视化图层", |doc| {
                    Arc::make_mut(doc.visualization.as_mut().ok_or("可视化工程不存在。")?)
                        .overlay = None;
                    Ok(())
                });
            }
            Action::BrowseCsv => {
                let Some(path) = self.file_dialog().add_filter("CSV", &["csv"]).pick_file() else {
                    return;
                };
                match map_io::chart_csv::headers(&path) {
                    Ok(names) if !names.is_empty() => {
                        self.visualization_ui.import = Some(Import {
                            path,
                            value: names.len().saturating_sub(1),
                            names,
                            label: Some(0),
                            x: None,
                            lon: None,
                            lat: None,
                            node: None,
                        });
                    }
                    Ok(_) => self.visualization_ui.notice = "表格没有列。".into(),
                    Err(e) => self.visualization_ui.notice = e,
                }
            }
            Action::ReadCsv => {
                let Some(form) = self.visualization_ui.import.as_ref() else {
                    return;
                };
                match map_io::chart_csv::read(
                    &form.path,
                    map_io::chart_csv::Columns {
                        label: form.label,
                        x: form.x,
                        y: form.value,
                        coordinates: form.lon.zip(form.lat),
                        source: form.node,
                    },
                ) {
                    Ok(rows) => {
                        let count = rows.len();
                        if let Some(spec) = &mut self.visualization_ui.editor {
                            spec.data = Data::Rows(rows);
                        }
                        self.visualization_ui.import = None;
                        self.visualization_ui.notice = format!("已导入 {count} 行。");
                    }
                    Err(e) => self.visualization_ui.notice = e,
                }
            }
            Action::WriteCsv => {
                let Some(data) = self
                    .visualization_ui
                    .preview
                    .as_ref()
                    .and_then(|p| p.data.as_ref().ok())
                    .cloned()
                else {
                    return;
                };
                let Some(path) = self
                    .file_dialog()
                    .set_file_name("chart-data.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file()
                else {
                    return;
                };
                if let Err(e) = map_io::chart_csv::write(&path, &data.data) {
                    self.set_status(e, true);
                }
            }
        }
    }

    pub(crate) fn draw_visualization_icons(
        &self,
        painter: &egui::Painter,
        vg: &ViewGeom,
        taken: &mut crate::label_boxes::LabelBoxes,
    ) {
        let mut buckets = std::collections::HashSet::new();
        let clip = painter.clip_rect().expand(35.0);
        for icon in &self.theme.data_icons {
            let Some(at) = vg.to_screen(icon.at).filter(|p| clip.contains(*p)) else {
                continue;
            };
            let bucket = ((at.x / 5.0).floor() as i32, (at.y / 5.0).floor() as i32);
            if !buckets.insert(bucket) {
                continue;
            }
            let [r, g, b, a] = icon.color;
            let color = Color32::from_rgba_unmultiplied(r, g, b, a);
            let points = icon
                .icon
                .vertices(icon.size)
                .into_iter()
                .map(|[x, y]| at + Vec2::new(x, y))
                .collect();
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                Stroke::new(0.8, Color32::WHITE),
            ));
            if icon.label.is_empty() {
                continue;
            }
            let g =
                painter.layout_no_wrap(icon.label.clone(), egui::FontId::proportional(11.0), color);
            let origin = Pos2::new(at.x - g.size().x * 0.5, at.y - icon.size - g.size().y - 2.0);
            let box_ = Rect::from_min_size(origin, g.size()).expand(2.0);
            if clip.contains_rect(box_) && !taken.intersects(box_) {
                painter.rect_filled(box_, 2.0, Color32::from_white_alpha(215));
                painter.galley(origin, g, color);
                taken.push(box_);
            }
        }
    }
}
fn labeled_text(ui: &mut egui::Ui, title: &'static str, text: &mut String) {
    ui.horizontal(|ui| {
        ui.label(tr(title));
        ui.text_edit_singleline(text);
    });
}

fn default_transect() -> Data {
    Data::Transect {
        from: LonLat::new(-15.0, 0.0),
        to: LonLat::new(15.0, 0.0),
        layer: Layer::Standard(Field::Elevation),
        samples: 192,
    }
}

fn source(ui: &mut egui::Ui, data: &mut Data, doc: &Document, action: &mut Option<Action>) {
    let modes = [
        "导入或手工数据",
        "地图统计字段",
        "地图字段的历史",
        "世界观属性",
        "世界观记录的历史",
        "世界观关系",
        "沿线采样",
    ];
    let prev = match data {
        Data::Rows(_) => 0,
        Data::MapField { .. } => 1,
        Data::MapHistory { .. } => 2,
        Data::Property { .. } => 3,
        Data::RecordHistory { .. } => 4,
        Data::Network { .. } => 5,
        Data::Transect { .. } => 6,
    };
    let mut mode = prev;
    let catalog = Catalog::get(doc);
    egui::ComboBox::from_id_salt("chart-source")
        .selected_text(tr(modes[mode]))
        .show_ui(ui, |ui| {
            for (i, name) in modes.into_iter().enumerate() {
                ui.selectable_value(&mut mode, i, tr(name));
            }
        });
    if mode != prev {
        let area = map_core::doc::AREA_KEY.to_string();
        *data = match mode {
            1 => Data::MapField {
                key: area,
                top_level: true,
            },
            2 => Data::MapHistory {
                key: area,
                entity: doc.entities.keys().next().copied().unwrap_or(EntityId(0)),
                years: Years::default(),
            },
            3 => Data::Property {
                schema: None,
                key: String::new(),
            },
            4 => Data::RecordHistory {
                record: 0,
                key: String::new(),
                years: Years::default(),
            },
            5 => Data::Network {
                schema: None,
                relation: None,
            },
            6 => default_transect(),
            _ => Data::default(),
        };
    }
    match data {
        Data::Rows(rows) => {
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("选择 CSV…")).clicked() {
                    *action = Some(Action::BrowseCsv);
                }
                if ui.button(tr("添加数据行")).clicked() {
                    rows.push(map_core::visualization::Row {
                        label: format!("{} {}", tr("数据"), rows.len() + 1),
                        x: (rows.len() + 1) as f64,
                        value: 0.0,
                        point: None,
                        node: None,
                    });
                }
                ui.label(format!("{} {}", rows.len(), tr("行")));
            });
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("chart-table-data")
                .max_height(240.0)
                .show_rows(ui, 54.0, rows.len(), |ui, range| {
                    for i in range {
                        let row = &mut rows[i];
                        ui.push_id(i, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut row.label).desired_width(100.0),
                                );
                                ui.add(egui::DragValue::new(&mut row.x).speed(0.1).prefix("X "));
                                ui.add(
                                    egui::DragValue::new(&mut row.value).speed(0.1).prefix("Y "),
                                );
                                if ui.small_button("×").clicked() {
                                    remove = Some(i);
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                let mut enabled = row.point.is_some();
                                if ui.checkbox(&mut enabled, tr("设定经纬度")).changed() {
                                    row.point = enabled.then_some(LonLat::new(0.0, 0.0));
                                }
                                if let Some(at) = &mut row.point {
                                    coordinates(ui, at);
                                }
                                if let Some(node) = row.node {
                                    ui.weak(node.uri());
                                }
                            });
                        });
                    }
                });
            if let Some(id) = remove {
                rows.remove(id);
            }
        }
        Data::MapField { key, top_level } => {
            choose_map_field(ui, key, doc);
            ui.checkbox(top_level, tr("仅顶层单位"));
        }
        Data::MapHistory { key, entity, years } => {
            choose_map_field(ui, key, doc);
            let mut selected = entity.0;
            choose_id(
                ui,
                "地图单位",
                &mut selected,
                doc.entities.iter().map(|(id, e)| (id.0, e.name.as_str())),
            );
            *entity = EntityId(selected);
            edit_years(ui, years);
        }
        Data::Property { schema, key } => {
            if let Some(c) = catalog {
                let mut id = schema.unwrap_or(0);
                if choose_id(
                    ui,
                    "实体类型",
                    &mut id,
                    std::iter::once((0, tr("全部类型")))
                        .chain(c.schemas.iter().map(|(&id, s)| (id, s.name.as_str()))),
                ) {
                    *schema = (id != 0).then_some(id);
                }
                if let Some(schema) = schema.and_then(|id| c.schemas.get(&id)) {
                    choose_key(
                        ui,
                        "数值属性",
                        key,
                        schema
                            .fields
                            .iter()
                            .filter(|(_, f)| f.kind == Kind::Number)
                            .map(|(k, f)| (k.as_str(), f.name.as_str())),
                    );
                } else {
                    labeled_text(ui, "属性标识", key);
                }
            } else {
                ui.label(tr("先创建世界观记录。"));
            }
        }
        Data::RecordHistory { record, key, years } => {
            if let Some(c) = catalog {
                choose_id(
                    ui,
                    "世界观记录",
                    record,
                    c.records.iter().map(|(&id, r)| (id, r.name.as_str())),
                );
                if let Some(schema) = c.records.get(record).and_then(|r| c.schemas.get(&r.schema)) {
                    choose_key(
                        ui,
                        "数值属性",
                        key,
                        schema
                            .fields
                            .iter()
                            .filter(|(_, f)| f.kind == Kind::Number)
                            .map(|(k, f)| (k.as_str(), f.name.as_str())),
                    );
                }
                edit_years(ui, years);
            } else {
                ui.label(tr("先创建世界观记录。"));
            }
        }
        Data::Network { schema, relation } => {
            if let Some(c) = catalog {
                let mut id = schema.unwrap_or(0);
                choose_id(
                    ui,
                    "实体类型",
                    &mut id,
                    std::iter::once((0, tr("全部类型")))
                        .chain(c.schemas.iter().map(|(&id, s)| (id, s.name.as_str()))),
                );
                *schema = (id != 0).then_some(id);
                let mut id = relation.unwrap_or(0);
                choose_id(
                    ui,
                    "关系类型",
                    &mut id,
                    std::iter::once((0, tr("全部关系"))).chain(
                        c.relation_types
                            .iter()
                            .map(|(&id, r)| (id, r.name.as_str())),
                    ),
                );
                *relation = (id != 0).then_some(id);
            } else {
                ui.label(tr("先创建世界观关系。"));
            }
        }
        Data::Transect {
            from,
            to,
            layer,
            samples,
        } => {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("剖面起点"));
                coordinates(ui, from);
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("剖面终点"));
                coordinates(ui, to);
            });
            egui::ComboBox::from_id_salt("profile-layer")
                .selected_text(transect_name(layer))
                .show_ui(ui, |ui| {
                    if let Some(r) = doc.environment.as_ref().and_then(|p| p.current.as_ref()) {
                        for view in r.layers() {
                            let name = transect_name(&view).to_owned();
                            ui.selectable_value(layer, view, name);
                        }
                    } else {
                        ui.selectable_value(layer, Layer::Standard(Field::Elevation), tr("海拔 m"));
                    }
                });
            ui.add(egui::Slider::new(samples, 16..=1024).text(tr("采样数")));
        }
    }
}

fn transect_name(layer: &Layer) -> &str {
    match layer {
        Layer::Standard(field) => tr(field.name()),
        Layer::Suitability(name) => name,
    }
}

fn coordinates(ui: &mut egui::Ui, at: &mut LonLat) {
    ui.add(
        egui::DragValue::new(&mut at.lon)
            .range(-180.0..=180.0)
            .speed(0.1)
            .suffix("°")
            .prefix(tr("经 ")),
    );
    ui.add(
        egui::DragValue::new(&mut at.lat)
            .range(-90.0..=90.0)
            .speed(0.1)
            .suffix("°")
            .prefix(tr("纬 ")),
    );
}

fn choose_map_field(ui: &mut egui::Ui, key: &mut String, doc: &Document) {
    choose_key(
        ui,
        "地图字段",
        key,
        std::iter::once((map_core::doc::AREA_KEY, tr("面积 (km²)"))).chain(
            doc.fields
                .values()
                .filter(|f| f.kind == map_core::FieldKind::Number)
                .map(|f| (f.key.as_str(), f.name.as_str())),
        ),
    );
}

fn choose_key<'a>(
    ui: &mut egui::Ui,
    title: &'static str,
    key: &mut String,
    values: impl Iterator<Item = (&'a str, &'a str)>,
) {
    let values: Vec<_> = values.collect();
    let title_text = values
        .iter()
        .copied()
        .find(|(k, _)| *k == key)
        .map_or(tr("请选择"), |(_, n)| n);
    ui.horizontal_wrapped(|ui| {
        ui.label(tr(title));
        egui::ComboBox::from_id_salt(title)
            .selected_text(title_text)
            .show_ui(ui, |ui| {
                for (id, name) in values {
                    ui.selectable_value(key, id.to_owned(), name);
                }
            });
    });
}

fn choose_id<'a>(
    ui: &mut egui::Ui,
    title: &'static str,
    key: &mut Id,
    values: impl Iterator<Item = (Id, &'a str)> + Clone,
) -> bool {
    let previous = *key;
    let title_text = values
        .clone()
        .find(|(id, _)| *id == previous)
        .map_or(tr("请选择"), |(_, n)| n);
    ui.horizontal_wrapped(|ui| {
        ui.label(tr(title));
        egui::ComboBox::from_id_salt(title)
            .selected_text(title_text)
            .show_ui(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(235.0)
                    .show(ui, |ui| {
                        for (id, name) in values {
                            ui.selectable_value(key, id, name);
                        }
                    });
            });
    });
    *key != previous
}

fn edit_years(ui: &mut egui::Ui, years: &mut Years) {
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::DragValue::new(&mut years.from).prefix(tr("从 ")));
        ui.add(egui::DragValue::new(&mut years.until).prefix(tr("至 ")));
        ui.add(
            egui::DragValue::new(&mut years.step)
                .range(1..=100_000)
                .prefix(tr("年间隔 ")),
        );
    });
}

fn import_form(ui: &mut egui::Ui, import: &mut Import, action: &mut Option<Action>) {
    ui.group(|ui| {
        ui.strong(
            import
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("CSV"),
        );
        let mut value = Some(import.value);
        let labels = [
            ("名称列", &mut import.label),
            ("横轴列", &mut import.x),
            ("数值列", &mut value),
            ("经度列", &mut import.lon),
            ("纬度列", &mut import.lat),
            ("地图关联 URI 列", &mut import.node),
        ];
        for (title, selected) in labels {
            ui.horizontal(|ui| {
                ui.label(tr(title));
                egui::ComboBox::from_id_salt(title)
                    .selected_text(
                        selected
                            .and_then(|i| import.names.get(i))
                            .map_or(tr("无"), String::as_str),
                    )
                    .show_ui(ui, |ui| {
                        if title != "数值列" {
                            ui.selectable_value(selected, None, tr("无"));
                        }
                        for (i, name) in import.names.iter().enumerate() {
                            ui.selectable_value(selected, Some(i), name);
                        }
                    });
            });
        }
        if let Some(i) = value {
            import.value = i;
        }
        if ui
            .add_enabled(
                import.lon.is_some() == import.lat.is_some(),
                egui::Button::new(tr("读取数据")),
            )
            .clicked()
        {
            *action = Some(Action::ReadCsv);
        }
    });
}

fn rule_form(ui: &mut egui::Ui, rule: &mut Rule) {
    labeled_text(ui, "数值转换公式", &mut rule.expression);
    ui.label(tr("公式可使用 value、x、year。"));
    labeled_text(ui, "单位", &mut rule.unit);
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("色带"));
        for stop in &mut rule.ramp {
            let mut color = Color32::from_rgba_unmultiplied(stop[0], stop[1], stop[2], stop[3]);
            if ui.color_edit_button_srgba(&mut color).changed() {
                *stop = color.to_srgba_unmultiplied();
            }
        }
        if rule.ramp.len() < 12 && ui.small_button("+").clicked() {
            rule.ramp.push([45, 104, 150, 255]);
        }
        if rule.ramp.len() > 2 && ui.small_button("−").clicked() {
            rule.ramp.pop();
        }
    });
    let mut fixed = rule.bounds.is_some();
    if ui.checkbox(&mut fixed, tr("固定颜色范围")).changed() {
        rule.bounds = fixed.then_some([0.0, 100.0]);
    }
    if let Some([minimum, maximum]) = &mut rule.bounds {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(minimum).prefix(tr("最小 ")));
            ui.add(egui::DragValue::new(maximum).prefix(tr("最大 ")));
        });
    }
    ui.horizontal_wrapped(|ui| {
        if rule.breaks.len() < 24 && ui.button(tr("增加专题图分级阈值")).clicked() {
            rule.breaks
                .push(rule.breaks.last().copied().map_or(10.0, |x| x + 10.0));
        }
        if !rule.breaks.is_empty() && ui.small_button("−").clicked() {
            rule.breaks.pop();
        }
        for x in &mut rule.breaks {
            ui.add(egui::DragValue::new(x).speed(0.2));
        }
    });
    egui::ComboBox::from_id_salt("symbol-shape")
        .selected_text(tr(rule.icon.name()))
        .show_ui(ui, |ui| {
            for icon in Icon::ALL {
                ui.selectable_value(&mut rule.icon, icon, tr(icon.name()));
            }
        });
    ui.horizontal_wrapped(|ui| {
        let [lo, hi] = &mut rule.radii;
        ui.add(
            egui::DragValue::new(lo)
                .range(1.0..=48.0)
                .prefix(tr("最小尺寸 ")),
        );
        ui.add(
            egui::DragValue::new(hi)
                .range(1.0..=48.0)
                .prefix(tr("最大尺寸 ")),
        );
    });
    ui.checkbox(&mut rule.map_labels, tr("显示图标名称"));
}
