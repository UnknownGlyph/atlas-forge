use crate::{app::App, i18n::tr};
use eframe::egui;
use map_core::{
    climate::Region,
    environment::{
        self, Binding, Field, Generation, Layer, Parameter, Project, Suitability, Version,
    },
    underlay::Georef,
    world_book::WorldBook,
    worldview::{Catalog, Kind},
    Document, LonLat,
};
use map_render::underlay::RgbaImage;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

struct Job {
    input: Document,
    output: JoinHandle<Result<Generation, String>>,
    manual: bool,
}

struct Display {
    source: Arc<Generation>,
    layer: Layer,
    image: Arc<RgbaImage>,
    key: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Generate,
    Layers,
    Edit,
}

#[derive(Default)]
pub(crate) struct EnvironmentUi {
    pub open: bool,
    pub picking: bool,
    section: Section,
    draft: Option<environment::Settings>,
    observed: Option<Document>,
    due: Option<Instant>,
    job: Option<Job>,
    display: Option<Display>,
    point: Option<LonLat>,
    edit_value: f64,
    area_cells: u16,
    lock: bool,
    name: String,
    notice: String,
}

impl EnvironmentUi {
    pub(crate) fn leave_board(&mut self) {
        let job = self.job.take();
        *self = Self { job, ..Self::default() };
    }

    pub(crate) fn reload(&mut self) {
        self.draft = None;
        self.picking = false;
    }

    pub(crate) fn stop_picking(&mut self) {
        self.picking = false;
    }

    fn refresh_display(&mut self, project: &Project) {
        let Some(source) = project
            .shown()
            .filter(|_| project.visible && project.opacity > 0.0)
        else {
            self.display = None;
            return;
        };
        if self
            .display
            .as_ref()
            .is_some_and(|d| Arc::ptr_eq(&d.source, source) && d.layer == project.layer)
        {
            return;
        }
        static KEYS: AtomicU64 = AtomicU64::new(0xE471_2000_0000_0000);
        self.display = Some(Display {
            layer: project.layer.clone(),
            source: source.clone(),
            image: Arc::new(RgbaImage {
                w: source.grid.width.into(),
                h: source.grid.height.into(),
                pixels: source.image(&project.layer),
            }),
            key: KEYS.fetch_add(1, Ordering::Relaxed),
        });
    }

    pub(crate) fn raster(&self) -> Option<(u64, Arc<RgbaImage>, Georef)> {
        let view = self.display.as_ref()?;
        Some((view.key, view.image.clone(), view.source.grid.georef()))
    }
}

fn same_arc<T>(one: Option<&Arc<T>>, other: Option<&Arc<T>>) -> bool {
    match (one, other) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn same_inputs(a: &Document, b: &Document) -> bool {
    let (Some(x), Some(y)) = (&a.environment, &b.environment) else {
        return false;
    };
    WorldBook::current(a) == WorldBook::current(b)
        && a.year == b.year
        && a.radius() == b.radius()
        && a.topo.faces.ptr_eq(&b.topo.faces)
        && a.topo.edges.ptr_eq(&b.topo.edges)
        && a.topo.nodes.ptr_eq(&b.topo.nodes)
        && a.areas.ptr_eq(&b.areas)
        && a.rivers.ptr_eq(&b.rivers)
        && a.symbols.ptr_eq(&b.symbols)
        && same_arc(a.terrain.as_ref(), b.terrain.as_ref())
        && match (&a.climate, &b.climate) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.settings == b.settings
                        && a.background == b.background
                        && a.surface_edits.ptr_eq(&b.surface_edits))
            }
            _ => false,
        }
        && x.settings == y.settings
        && x.edits.ptr_eq(&y.edits)
        && (y.settings.bindings.is_empty()
            || (same_arc(a.book.as_ref(), b.book.as_ref())
                && a.owner.ptr_eq(&b.owner)
                && a.entities.ptr_eq(&b.entities)
                && a.fields.ptr_eq(&b.fields)
                && a.values.ptr_eq(&b.values)))
}

enum Action {
    Generate,
    Settings,
    Scope(Region),
    Overlay(Layer, bool, f32, bool, Option<usize>),
    Archive,
    CopySettings(usize),
    RemoveVersion(usize),
    Edit(Option<f32>, bool),
    Export,
}

impl App {
    pub(crate) fn open_environment(&mut self) {
        if self.doc.paper.is_some() {
            return;
        }
        if self.doc.environment.is_none() {
            self.edit("启用环境生成", |doc| {
                doc.environment = Some(Arc::new(Project::default()));
                Ok(())
            });
        }
        self.environment_ui.open = true;
        if self.studio.foreground {
            self.studio.show_environment();
        }
    }

    fn start_environment(&mut self, manual: bool, ctx: &egui::Context) {
        if self.environment_ui.job.is_some() {
            self.environment_ui.notice = "已有计算正在运行。".into();
            return;
        }
        let Some(project) = self.doc.environment.clone() else {
            return;
        };
        let input = self.doc.clone();
        let worker = input.clone();
        let repaint = ctx.clone();
        let output = std::thread::spawn(move || {
            let result = environment::generate(&worker, &project);
            repaint.request_repaint();
            result
        });
        self.environment_ui.job = Some(Job {
            input,
            output,
            manual,
        });
        self.environment_ui.due = None;
        self.environment_ui.notice = "正在生成环境图层…".into();
    }

    pub(crate) fn environment_tick(&mut self, ctx: &egui::Context) {
        let Some(project) = self.doc.environment.clone() else {
            self.environment_ui.display = None;
            return;
        };
        let state = &mut self.environment_ui;
        let changed = state.observed.as_ref().is_none_or(|old| {
            !same_inputs(old, &self.doc)
                || old
                    .environment
                    .as_ref()
                    .is_none_or(|p| p.live != project.live)
        });
        if changed {
            state.observed = Some(self.doc.clone());
            state.due = project
                .live
                .then(|| Instant::now() + Duration::from_millis(370));
            if !project.live && project.current.is_some() && state.job.is_none() {
                state.notice = "手动模式保留上次结果。".into();
            }
        }
        if state.job.as_ref().is_some_and(|j| j.output.is_finished()) {
            let job = state.job.take().unwrap();
            let result = job
                .output
                .join()
                .unwrap_or_else(|_| Err("环境计算线程意外停止。".into()));
            if same_inputs(&job.input, &self.doc) {
                match result {
                    Ok(next) => {
                        let size = (next.grid.width, next.grid.height);
                        let apply = |doc: &mut Document| -> Result<(), String> {
                            let project =
                                Arc::make_mut(doc.environment.as_mut().ok_or("缺少环境设置。")?);
                            if next.value(&project.layer, 0).is_none() {
                                project.layer = Layer::default();
                            }
                            project.current = Some(Arc::new(next));
                            if job.manual {
                                project.selected = None;
                            }
                            Ok(())
                        };
                        if job.manual {
                            self.edit("生成环境图层", apply);
                        } else if apply(&mut self.doc).is_ok() {
                            self.dirty = true;
                        }
                        self.environment_ui.notice =
                            format!("已生成 {} × {} 网格。", size.0, size.1);
                    }
                    Err(error) => self.environment_ui.notice = error,
                }
            } else {
                self.environment_ui.notice = "地图或输入已改变，已忽略旧计算。".into();
            }
        }
        if self.environment_ui.job.is_none()
            && self.environment_ui.due.is_some_and(|t| t <= Instant::now())
        {
            self.start_environment(false, ctx);
        }
        if self.environment_ui.job.is_some() || self.environment_ui.due.is_some() {
            ctx.request_repaint_after(Duration::from_millis(160));
        }
        if let Some(project) = &self.doc.environment {
            self.environment_ui.refresh_display(project);
        }
    }

    pub(crate) fn environment_pick(&mut self, at: LonLat) {
        self.environment_ui.picking = false;
        self.environment_ui.point = Some(at);
        self.environment_ui.edit_value = self
            .doc
            .environment
            .as_ref()
            .and_then(|p| p.current.as_ref()?.at(&p.layer, at))
            .map_or(0.0, f64::from);
    }

    pub(crate) fn environment_window(&mut self, ctx: &egui::Context) {
        if !self.environment_ui.open || self.studio.foreground {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("环境生成与图层"))
            .id(egui::Id::unique("environment-workspace"))
            .default_width(460.0)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| self.environment_panel(ui, false));
        self.environment_ui.open = open;
        if !open {
            self.environment_ui.picking = false;
        }
    }

    pub(crate) fn environment_panel(&mut self, ui: &mut egui::Ui, studio: bool) {
        let Some(project) = self.doc.environment.clone() else {
            if ui.button(tr("启用环境生成")).clicked() {
                self.open_environment();
            }
            return;
        };
        let local = self.active_focus().map(environment::from_focus);
        let state = &mut self.environment_ui;
        let mut action = None;
        if studio {
            ui.heading(tr("环境生成"));
        }
        ui.horizontal_wrapped(|ui| {
            for (section, title) in [
                (Section::Generate, "生成参数"),
                (Section::Layers, "图层与版本"),
                (Section::Edit, "局部修改"),
            ] {
                ui.selectable_value(&mut state.section, section, tr(title));
            }
        });
        if !state.notice.is_empty() {
            ui.label(&state.notice);
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("environment-pane")
            .show(ui, |ui| match state.section {
                Section::Generate => generation_controls(
                    ui,
                    state,
                    &project,
                    Catalog::get(&self.doc),
                    local,
                    &mut action,
                ),
                Section::Layers => layer_controls(ui, state, &project, &mut action),
                Section::Edit => edit_controls(ui, state, &project, &mut action),
            });
        if let Some(action) = action {
            self.apply_environment(action, ui.ctx());
        }
    }

    fn apply_environment(&mut self, action: Action, ctx: &egui::Context) {
        let action = match action {
            Action::Generate => {
                self.start_environment(true, ctx);
                return;
            }
            Action::Export => {
                let Some(project) = self.doc.environment.as_ref() else {
                    return;
                };
                let Some(result) = project.shown() else {
                    return;
                };
                let Some(path) = self
                    .file_dialog()
                    .set_file_name("environment-layer.png")
                    .add_filter("PNG", &["png"])
                    .save_file()
                else {
                    return;
                };
                let size = result.grid;
                match image::save_buffer(
                    &path,
                    &result.image(&project.layer),
                    size.width.into(),
                    size.height.into(),
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => self.set_status("环境图层已导出。", false),
                    Err(error) => self.set_status(error.to_string(), true),
                }
                return;
            }
            other => other,
        };
        let state = &self.environment_ui;
        let point = state.point;
        let count = state.area_cells;
        let draft = state.draft.clone();
        let name = state.name.trim().to_string();
        let regenerate = matches!(action, Action::Edit(..));
        let sync_draft = matches!(action, Action::Scope(..) | Action::CopySettings(..));
        let changed = self
            .edit("修改环境图层", |doc| {
                let project = Arc::make_mut(doc.environment.as_mut().ok_or("缺少环境设置。")?);
                match action {
                    Action::Settings => {
                        let settings = draft.ok_or("没有待保存的参数。")?;
                        settings.validate()?;
                        project.settings = settings;
                    }
                    Action::Scope(region) => project.settings.region = region,
                    Action::Overlay(layer, visible, opacity, live, selected) => {
                        project.layer = layer;
                        project.visible = visible;
                        project.opacity = opacity;
                        project.live = live;
                        project.selected = selected;
                    }
                    Action::Archive => {
                        let result = project.current.clone().ok_or("尚未生成结果。")?;
                        let label = if name.is_empty() {
                            format!("环境方案 {}", project.saved.len() + 1)
                        } else {
                            name
                        };
                        project.saved.push(Version {
                            name: label,
                            result,
                        });
                    }
                    Action::CopySettings(i) => {
                        let version = project.saved.get(i).ok_or("版本已移除。")?;
                        project.settings = version.result.settings.clone();
                    }
                    Action::RemoveVersion(i) => {
                        if i >= project.saved.len() {
                            return Err("版本已移除。".into());
                        }
                        project.saved.remove(i);
                        project.selected = None;
                    }
                    Action::Edit(value, locked) => {
                        let point = point.ok_or("请在地图上选择一个位置。")?;
                        let grid = project.current.as_ref().ok_or("尚未生成结果。")?.grid;
                        let index = grid.index(point).ok_or("此位置不在结果范围内。")?;
                        let w = i32::from(grid.width);
                        let (x, y) = (index as i32 % w, index as i32 / w);
                        let r = i32::from(count);
                        for dy in -r..=r {
                            for dx in -r..=r {
                                if dx * dx + dy * dy > r * r {
                                    continue;
                                }
                                let (a, b) = (x + dx, y + dy);
                                let a = if grid.whole_longitude() {
                                    a.rem_euclid(w)
                                } else {
                                    a
                                };
                                if (0..w).contains(&a) && (0..i32::from(grid.height)).contains(&b) {
                                    project.edit_at(
                                        grid.center((b * w + a) as usize),
                                        project.layer.clone(),
                                        value,
                                        locked,
                                    )?;
                                }
                            }
                        }
                        project.selected = None;
                    }
                    Action::Generate | Action::Export => unreachable!(),
                }
                Ok(())
            })
            .is_some();
        if changed && sync_draft {
            self.environment_ui.draft = None;
        }
        if changed && regenerate {
            self.start_environment(true, ctx);
        }
    }
}

fn generation_controls(
    ui: &mut egui::Ui,
    state: &mut EnvironmentUi,
    project: &Project,
    catalog: Option<&Catalog>,
    local: Option<Region>,
    action: &mut Option<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("全局范围")).clicked() {
            *action = Some(Action::Scope(Region::default()));
        }
        if ui
            .add_enabled(local.is_some(), egui::Button::new(tr("采用当前局部视图")))
            .clicked()
        {
            *action = local.map(Action::Scope);
        }
    });
    let region = project.settings.region;
    ui.label(format!(
        "∅ {:.2}° – {:.2}°   ↕ {:.2}° – {:.2}°",
        region.west, region.east, region.south, region.north
    ));
    let draft = state.draft.get_or_insert_with(|| project.settings.clone());
    ui.collapsing(tr("范围与分辨率"), |ui| {
        ui.add(
            egui::DragValue::new(&mut draft.region.west)
                .speed(0.1)
                .range(-180.0..=180.0)
                .prefix(tr("西 ")),
        );
        ui.add(
            egui::DragValue::new(&mut draft.region.east)
                .speed(0.1)
                .range(-180.0..=180.0)
                .prefix(tr("东 ")),
        );
        ui.add(
            egui::DragValue::new(&mut draft.region.south)
                .speed(0.1)
                .range(-90.0..=90.0)
                .prefix(tr("南 ")),
        );
        ui.add(
            egui::DragValue::new(&mut draft.region.north)
                .speed(0.1)
                .range(-90.0..=90.0)
                .prefix(tr("北 ")),
        );
        ui.add(egui::Slider::new(&mut draft.resolution, 32..=768).text(tr("网格长边")));
    });
    ui.checkbox(&mut draft.generate_unknown, tr("为未指定地表生成海陆"));
    ui.horizontal(|ui| {
        ui.label(tr("地形种子"));
        ui.add(egui::DragValue::new(&mut draft.seed));
    });
    for (value, range, label) in [
        (&mut draft.relief, 0.0..=18000.0, "起伏尺度 m"),
        (&mut draft.sea_level, -1.0..=1.0, "未知区海陆阈值"),
        (&mut draft.rainfall, 0.0..=10.0, "降水倍率"),
        (&mut draft.soil_capacity, 10.0..=5000.0, "土壤蓄水尺度 mm"),
        (&mut draft.habitat_pressure, 0.0..=1.0, "人类扰动度"),
    ] {
        ui.add(egui::Slider::new(value, range).text(tr(label)));
    }
    ui.collapsing(tr("世界观参数联动"), |ui| {
        bindings(ui, &mut draft.bindings, catalog)
    });
    ui.collapsing(tr("资源和物种适宜性"), |ui| {
        ui.label(tr("结果范围 0–100。可用变量包括 elevation、slope、runoff、rain、wetness、vegetation、diversity、temperature、land、latitude、noise。"));
        let mut delete = None;
        for (i, rule) in draft.rules.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut rule.name).hint_text(tr("图层名称")));
                    if ui.small_button("×").clicked() { delete = Some(i); }
                });
                ui.add(egui::TextEdit::singleline(&mut rule.expression).hint_text(tr("数值公式")));
            });
        }
        if let Some(i) = delete { draft.rules.remove(i); }
        if ui.add_enabled(draft.rules.len() < 32, egui::Button::new(tr("添加适宜性图层"))).clicked() {
            draft.rules.push(Suitability { name: String::new(), expression: "vegetation * land".into() });
        }
    });
    let pending = *draft != project.settings;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(pending, egui::Button::new(tr("保存参数")))
            .clicked()
        {
            *action = Some(Action::Settings);
        }
        if ui
            .add_enabled(
                !pending && state.job.is_none(),
                egui::Button::new(tr("生成图层")),
            )
            .clicked()
        {
            *action = Some(Action::Generate);
        }
    });
    let mut live = project.live;
    if ui.checkbox(&mut live, tr("输入改变时自动生成")).changed() {
        *action = Some(Action::Overlay(
            project.layer.clone(),
            project.visible,
            project.opacity,
            live,
            project.selected,
        ));
    }
}

fn bindings(ui: &mut egui::Ui, data: &mut Vec<Binding>, catalog: Option<&Catalog>) {
    let mut delete = None;
    for (i, binding) in data.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("parameter")
                    .selected_text(tr(binding.parameter.name()))
                    .show_ui(ui, |ui| {
                        for parameter in Parameter::ALL {
                            ui.selectable_value(
                                &mut binding.parameter,
                                parameter,
                                tr(parameter.name()),
                            );
                        }
                    });
                if ui.small_button("×").clicked() {
                    delete = Some(i);
                }
            });
            if let Some(catalog) = catalog {
                let record_name = catalog
                    .records
                    .get(&binding.record)
                    .map_or(tr("选择设定条目"), |r| &r.name);
                egui::ComboBox::from_id_salt("source-record")
                    .selected_text(record_name)
                    .show_ui(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(210.0)
                            .show(ui, |ui| {
                                for (&id, record) in &catalog.records {
                                    if ui
                                        .selectable_value(&mut binding.record, id, &record.name)
                                        .changed()
                                    {
                                        binding.key.clear();
                                    }
                                }
                            });
                    });
                if let Some(schema) = catalog
                    .records
                    .get(&binding.record)
                    .and_then(|r| catalog.schemas.get(&r.schema))
                {
                    let name = schema
                        .fields
                        .get(&binding.key)
                        .map_or(tr("选择数值属性"), |f| &f.name);
                    egui::ComboBox::from_id_salt("source-field")
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            for (key, field) in
                                schema.fields.iter().filter(|(_, f)| f.kind == Kind::Number)
                            {
                                ui.selectable_value(&mut binding.key, key.clone(), &field.name);
                            }
                        });
                }
            }
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::DragValue::new(&mut binding.scale)
                        .speed(0.025)
                        .prefix(tr("倍率 ")),
                );
                ui.add(
                    egui::DragValue::new(&mut binding.offset)
                        .speed(0.1)
                        .prefix(tr("偏移 ")),
                );
            });
        });
    }
    if let Some(i) = delete {
        data.remove(i);
    }
    if let Some(catalog) = catalog.filter(|c| !c.records.is_empty()) {
        let free = Parameter::ALL
            .into_iter()
            .find(|p| !data.iter().any(|b| b.parameter == *p));
        if ui
            .add_enabled(free.is_some(), egui::Button::new(tr("关联参数")))
            .clicked()
        {
            if let Some(parameter) = free {
                data.push(Binding {
                    parameter,
                    record: *catalog.records.keys().next().unwrap(),
                    key: String::new(),
                    scale: 1.0,
                    offset: 0.0,
                });
            }
        }
    } else {
        ui.label(tr("先在世界观设定中创建带数值属性的记录。"));
    }
}

fn layer_controls(
    ui: &mut egui::Ui,
    state: &mut EnvironmentUi,
    project: &Project,
    action: &mut Option<Action>,
) {
    let mut selected = project.selected;
    egui::ComboBox::from_id_salt("environment-version")
        .selected_text(
            project
                .selected
                .and_then(|s| project.saved.get(s))
                .map_or(tr("当前生成结果"), |v| &v.name),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected, None, tr("当前生成结果"));
            for (i, version) in project.saved.iter().enumerate() {
                ui.selectable_value(&mut selected, Some(i), &version.name);
            }
        });
    let result = selected
        .and_then(|i| project.saved.get(i).map(|v| &v.result))
        .or(project.current.as_ref());
    let (mut visible, mut opacity, mut layer) =
        (project.visible, project.opacity, project.layer.clone());
    if let Some(result) = result {
        if result.value(&layer, 0).is_none() {
            layer = Layer::default();
        }
        egui::ComboBox::from_id_salt("environment-layer")
            .selected_text(layer_label(&layer))
            .show_ui(ui, |ui| {
                for view in result.layers() {
                    let label = layer_label(&view).to_string();
                    ui.selectable_value(&mut layer, view, label);
                }
            });
        ui.checkbox(&mut visible, tr("显示地图叠加层"));
        ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text(tr("不透明度")));
        ui.label(format!(
            "{} × {}  ·  {}",
            result.grid.width,
            result.grid.height,
            result
                .year
                .map_or_else(|| tr("未选择年份").to_string(), |year| year.to_string())
        ));
        if ui.button(tr("导出当前图层 PNG")).clicked() {
            *action = Some(Action::Export);
        }
    } else {
        ui.label(tr("先生成环境图层。"));
    }
    if layer != project.layer
        || visible != project.visible
        || opacity != project.opacity
        || selected != project.selected
    {
        *action = Some(Action::Overlay(
            layer,
            visible,
            opacity,
            project.live,
            selected,
        ));
    }
    ui.separator();
    if let Some(i) = selected {
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("沿用此版本参数")).clicked() {
                *action = Some(Action::CopySettings(i));
            }
            ui.menu_button(tr("删除…"), |ui| {
                if ui.button(tr("删除保存版本")).clicked() {
                    *action = Some(Action::RemoveVersion(i));
                    ui.close();
                }
            });
        });
    } else if project.current.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.name).hint_text(tr("版本名称")));
            if ui.button(tr("保留当前结果")).clicked() {
                *action = Some(Action::Archive);
            }
        });
    }
}

fn layer_label(layer: &Layer) -> &str {
    match layer {
        Layer::Standard(field) => tr(field.name()),
        Layer::Suitability(name) => name,
    }
}

fn edit_controls(
    ui: &mut egui::Ui,
    state: &mut EnvironmentUi,
    project: &Project,
    action: &mut Option<Action>,
) {
    let Some(result) = &project.current else {
        ui.label(tr("先生成环境图层。"));
        return;
    };
    if project.selected.is_some() {
        ui.label(tr("正在查看保存版本。切回当前生成结果后可修改。"));
        return;
    }
    if ui
        .checkbox(&mut state.picking, tr("点击地图取样"))
        .changed()
        && state.picking
    {
        state.notice = "在二维地图或球体上点击一个位置。".into();
    }
    let Some(point) = state.point.filter(|p| result.grid.index(*p).is_some()) else {
        return;
    };
    ui.label(format!("{:.4}°, {:.4}°", point.lon, point.lat));
    for field in Field::ALL {
        if let Some(value) = result.at(&Layer::Standard(field), point) {
            if let Some(name) = field.category(value) {
                ui.label(format!("{} · {}", tr(field.name()), tr(name)));
            } else {
                ui.label(format!("{} · {value:.2}", tr(field.name())));
            }
        }
    }
    ui.separator();
    ui.strong(layer_label(&project.layer));
    if matches!(
        project.layer,
        Layer::Standard(Field::Biome | Field::Landform)
    ) {
        let Layer::Standard(field) = project.layer else {
            unreachable!()
        };
        egui::ComboBox::from_id_salt("manual-category")
            .selected_text(tr(field
                .category(state.edit_value as f32)
                .unwrap_or("请选择")))
            .show_ui(ui, |ui| {
                for n in 0..8 {
                    ui.selectable_value(
                        &mut state.edit_value,
                        f64::from(n),
                        tr(field.category(n as f32).unwrap_or("")),
                    );
                }
            });
    } else {
        ui.add(egui::DragValue::new(&mut state.edit_value).speed(0.1));
    }
    ui.add(egui::Slider::new(&mut state.area_cells, 0..=12).text(tr("半径格数")));
    ui.checkbox(&mut state.lock, tr("锁定已修改网格"));
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(state.job.is_none(), |ui| {
            if ui.button(tr("写入数值")).clicked() {
                *action = Some(Action::Edit(Some(state.edit_value as f32), state.lock));
            }
            if ui.button(tr("冻结当前结果")).clicked() {
                *action = Some(Action::Edit(None, true));
            }
            if ui.button(tr("清除手动修改")).clicked() {
                *action = Some(Action::Edit(None, false));
            }
        });
    });
}
