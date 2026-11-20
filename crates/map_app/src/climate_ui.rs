use crate::app::App;
mod panels;
#[cfg(test)]
mod tests;
use eframe::egui::{self, Color32};
use map_core::climate::*;
use map_core::{Document, LonLat};
use map_render::underlay::RgbaImage;
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Map,
    Planet,
    Ground,
    Records,
}
struct Job {
    input: Document,
    mode: Run,
    rx: mpsc::Receiver<Result<Scenario, String>>,
}
#[derive(Clone, Copy, PartialEq)]
enum Run {
    Manual,
    Live,
    Fit,
}
struct Display {
    source: Arc<Scenario>,
    field: Field,
    period: Period,
    pixels: Arc<RgbaImage>,
    key: u64,
    stats: [f64; 5],
}
pub struct ClimateUi {
    pub open: bool,
    tab: Tab,
    observed: Option<Document>,
    job: Option<Job>,
    due: Option<Instant>,
    display: Option<Display>,
    pub picking: bool,
    point: LonLat,
    pick_corner: Option<bool>,
    comparison: Option<(Arc<Scenario>, [f64; 5])>,
    radius: f64,
    manual: f32,
    lock: bool,
    surface: Surface,
    height: f32,
    set_surface: bool,
    set_height: bool,
    name: String,
    notice: String,
}
impl Default for ClimateUi {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Map,
            observed: None,
            job: None,
            due: None,
            display: None,
            picking: false,
            point: LonLat::new(0., 0.),
            pick_corner: None,
            comparison: None,
            radius: 4.,
            manual: 15.,
            lock: true,
            surface: Surface::Land,
            height: 1000.,
            set_surface: true,
            set_height: true,
            name: "方案 1".into(),
            notice: String::new(),
        }
    }
}
fn same_inputs(a: &Document, b: &Document) -> bool {
    let (Some(x), Some(y)) = (&a.climate, &b.climate) else {
        return false;
    };
    a.year == b.year
        && a.topo.faces.ptr_eq(&b.topo.faces)
        && a.topo.edges.ptr_eq(&b.topo.edges)
        && a.topo.nodes.ptr_eq(&b.topo.nodes)
        && a.kind_changes.ptr_eq(&b.kind_changes)
        && a.kind_base.ptr_eq(&b.kind_base)
        && a.areas.ptr_eq(&b.areas)
        && a.rivers.ptr_eq(&b.rivers)
        && a.symbols.ptr_eq(&b.symbols)
        && match (&a.terrain, &b.terrain) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
        && x.settings.planet == y.settings.planet
        && x.region == y.region
        && x.background == y.background
        && x.surface_edits.ptr_eq(&y.surface_edits)
}
fn run_model(doc: &Document, mode: Run) -> Result<Scenario, String> {
    let project = doc.climate.as_ref().ok_or("没有气候设置")?;
    project.region.validate()?;
    project.settings.planet.resolve()?;
    let ground = sample(doc, &project.surface_edits, project.background);
    let settings = if mode == Run::Fit {
        fit(&project.settings, &ground, project.region, doc.year)?.0
    } else {
        project.settings.clone()
    };
    Scenario::new(settings, doc.year, project.region, &ground)
}
impl ClimateUi {
    fn observe(&mut self, doc: &Document, p: &Project, now: Instant) {
        let changed = self.observed.as_ref().is_none_or(|old| {
            !same_inputs(old, doc) || old.climate.as_ref().is_none_or(|q| q.live != p.live)
        });
        if changed {
            if !p.live && p.current.is_some() && self.observed.is_some() {
                self.notice = "手动模式保留上次结果；可运行模拟以使用当前地图与参数。".into();
            }
            self.observed = Some(doc.clone());
            self.due = p.live.then(|| now + Duration::from_millis(260));
        }
    }
    pub fn image(&self) -> Option<(u64, Arc<RgbaImage>)> {
        self.display.as_ref().map(|d| (d.key, d.pixels.clone()))
    }
    fn refresh_display(&mut self, p: &Project) {
        let Some(s) = p.shown() else {
            self.display = None;
            return;
        };
        if self.display.as_ref().is_some_and(|d| {
            Arc::ptr_eq(&d.source, s) && d.field == p.field && d.period == p.period
        }) {
            return;
        }
        static KEY: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0xC117_0000_0000_0000);
        let stats = self
            .display
            .as_ref()
            .filter(|d| Arc::ptr_eq(&d.source, s))
            .map(|d| d.stats)
            .unwrap_or_else(|| s.summary(s.region));
        let (w, h, pixels) = s.raster(p.field, p.period);
        let pixels = Arc::new(RgbaImage {
            w: w as u32,
            h: h as u32,
            pixels,
        });
        self.display = Some(Display {
            source: s.clone(),
            field: p.field,
            period: p.period,
            pixels,
            stats,
            key: KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        });
    }
}
impl App {
    pub fn climate_open(&mut self) {
        if self.doc.climate.is_none() {
            let mut p = Project::default();
            p.settings.planet[Param::Radius] = (self.doc.radius() / 6371.0088).clamp(0.05, 12.);
            if let Ok(planet) = p.settings.planet.resolve() {
                p.settings.planet = planet;
            }
            self.edit("启用行星气候", move |d| {
                d.climate = Some(Arc::new(p));
                Ok(())
            });
        }
        self.climate_ui.open = true;
    }
    fn climate_start(&mut self, mode: Run) {
        let document = self.doc.clone();
        let copy = document.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_model(&copy, mode));
        });
        self.climate_ui.job = Some(Job {
            input: document,
            mode,
            rx,
        });
        self.climate_ui.due = None;
        self.climate_ui.notice = if mode == Run::Fit {
            "正在搜索接近目标的组合…"
        } else {
            "正在计算 12 个轨道月份…"
        }
        .into();
    }
    pub fn climate_tick(&mut self, ctx: &egui::Context) {
        let Some(p) = self.doc.climate.clone() else {
            if self.climate_ui.observed.is_some() || self.climate_ui.open {
                self.climate_ui = Default::default();
            }
            return;
        };
        self.climate_ui.observe(&self.doc, &p, Instant::now());
        let answer = self
            .climate_ui
            .job
            .as_ref()
            .and_then(|j| match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("气候计算线程已退出".into())),
            });
        if let Some(answer) = answer {
            let job = self.climate_ui.job.take().unwrap();
            let valid = match job.mode {
                Run::Manual => true,
                Run::Live => p.live && same_inputs(&job.input, &self.doc),
                Run::Fit => {
                    same_inputs(&job.input, &self.doc)
                        && job
                            .input
                            .climate
                            .as_ref()
                            .is_some_and(|old| old.settings.goals == p.settings.goals)
                }
            };
            if valid {
                match answer {
                    Ok(mut s) => {
                        if let Some(old) = &p.current {
                            s.retain_locks(old);
                            let start = job.input.climate.as_ref().and_then(|q| q.current.as_ref());
                            for (key, edit) in &old.edits {
                                if start.and_then(|initial| initial.edits.get(key)) != Some(edit) {
                                    s.edits.insert(*key, edit.clone());
                                }
                            }
                        }
                        self.edit("更新气候参考", move |d| {
                            let p = Arc::make_mut(d.climate.as_mut().ok_or("没有气候设置")?);
                            if job.mode == Run::Fit {
                                p.settings = s.settings.clone();
                            }
                            p.current = Some(Arc::new(s));
                            Ok(())
                        });
                        self.climate_ui.notice = if job.mode == Run::Fit {
                            "搜索完成；实际值见概览，可与目标比较。"
                        } else {
                            "参考结果已更新。"
                        }
                        .into();
                        if job.mode == Run::Fit {
                            self.climate_ui.observed = Some(self.doc.clone());
                            self.climate_ui.due = None;
                        }
                    }
                    Err(e) => self.climate_ui.notice = e,
                }
            } else {
                self.climate_ui.notice = "输入已改变，已忽略旧计算。".into();
            }
        }
        if self.climate_ui.job.is_none()
            && self.climate_ui.due.is_some_and(|d| d <= Instant::now())
            && p.live
            && self.pending.is_none()
        {
            self.climate_start(Run::Live);
        }
        if self.climate_ui.job.is_some() || self.climate_ui.due.is_some() {
            ctx.request_repaint_after(Duration::from_millis(70));
        }
        if let Some(p) = &self.doc.climate {
            self.climate_ui.refresh_display(p);
        }
    }
    pub fn climate_pick(&mut self, point: LonLat) {
        self.climate_ui.point = point;
        self.climate_ui.picking = false;
        if let Some(first) = self.climate_ui.pick_corner.take() {
            self.edit("设定气候模拟范围", |d| {
                let p = Arc::make_mut(d.climate.as_mut().ok_or("没有气候设置")?);
                if first {
                    p.region.west = point.lon;
                    p.region.south = point.lat;
                } else {
                    p.region.east = point.lon;
                    p.region.north = point.lat;
                }
                Ok(())
            });
        }
    }
    pub fn climate_window(&mut self, ctx: &egui::Context) {
        if !self.climate_ui.open {
            return;
        }
        let Some(original) = self.doc.climate.clone() else {
            return;
        };
        let mut p = (*original).clone();
        let mut open = true;
        let mut run = None;
        let mut changed = false;
        let mut clear = false;
        let mut external = None;
        let state = &mut self.climate_ui;
        egui::Window::new(crate::i18n::tr("行星环境与气候"))
            .open(&mut open)
            .default_width(465.)
            .default_height(560.)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .selectable_value(&mut p.live, false, crate::i18n::tr("手动模拟"))
                        .changed();
                    changed |= ui.selectable_value(&mut p.live, true, crate::i18n::tr("实时更新")).changed();
                    if ui
                        .add_enabled(state.job.is_none(), egui::Button::new(crate::i18n::tr("运行模拟")))
                        .clicked()
                    {
                        run = Some(Run::Manual);
                    }
                    changed |= ui.checkbox(&mut p.visible, crate::i18n::tr("显示图层")).changed();
                });
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (Tab::Map, "气候图"),
                        (Tab::Planet, "行星参数"),
                        (Tab::Ground, "地表与修订"),
                        (Tab::Records, "方案与导出"),
                    ] {
                        ui.selectable_value(&mut state.tab, tab, crate::i18n::tr(label));
                    }
                });
                ui.separator();
                match state.tab {
                    Tab::Map => {
                        changed |= state.map_tab(ui, &mut p);
                    }
                    Tab::Planet => {
                        changed |= state.planet_tab(ui, &mut p, &mut run, &mut external);
                    }
                    Tab::Ground => {
                        changed |= state.ground_tab(ui, &mut p);
                    }
                    Tab::Records => {
                        changed |= state.records_tab(ui, &mut p, &mut clear, &mut external);
                    }
                }
                if !state.notice.is_empty() {
                    ui.separator();
                    ui.label(&state.notice);
                }
            });
        self.climate_ui.open = open;
        if !open {
            self.climate_ui.picking = false;
            self.climate_ui.pick_corner = None;
        }
        if clear {
            self.climate_ui.job = None;
            self.climate_ui.due = None;
            p.selected = None;
            p.current = None;
            changed = true;
        }
        if let Some(action) = external {
            match self.climate_file(action, &mut p) {
                Ok(modified) => changed |= modified,
                Err(e) => self.climate_ui.notice = e,
            }
        }
        if run.is_some() && p.selected.take().is_some() {
            changed = true;
        }
        if changed {
            self.edit("气候设置与参考图层", move |d| {
                d.climate = Some(Arc::new(p));
                Ok(())
            });
        }
        if clear {
            self.climate_ui.observed = Some(self.doc.clone());
        }
        if let Some(mode) = run {
            self.climate_start(mode);
        }
    }
}
#[derive(Clone, Copy)]
enum FileAction {
    Load,
    Save,
    Png,
    Csv,
}
fn numeric(
    ui: &mut egui::Ui,
    v: &mut f64,
    limits: std::ops::RangeInclusive<f64>,
    speed: f64,
) -> bool {
    ui.add(
        egui::DragValue::new(v)
            .range(limits)
            .speed(speed)
            .max_decimals(5),
    )
    .changed()
}
fn period_menu(ui: &mut egui::Ui, value: &mut Period) -> bool {
    let before = *value;
    egui::ComboBox::from_id_salt("climate-period")
        .selected_text(value.name())
        .show_ui(ui, |ui| {
            for p in std::iter::once(Period::Annual)
                .chain((0..12).map(Period::Month))
                .chain((0..4).map(Period::Season))
            {
                ui.selectable_value(value, p, p.name());
            }
        });
    before != *value
}
impl ClimateUi {
    fn position(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::tr("经度"));
            numeric(ui, &mut self.point.lon, -180.0..=180., 0.5);
            ui.label(crate::i18n::tr("纬度"));
            numeric(ui, &mut self.point.lat, -90.0..=90., 0.5);
            if ui.selectable_label(self.picking, crate::i18n::tr("从地图取点")).clicked() {
                self.picking = !self.picking;
                self.pick_corner = None;
            }
        });
    }
    fn map_tab(&mut self, ui: &mut egui::Ui, p: &mut Project) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("climate-field")
                .selected_text(p.field.name())
                .show_ui(ui, |ui| {
                    for f in Field::ALL {
                        changed |= ui.selectable_value(&mut p.field, f, f.name()).changed();
                    }
                });
            changed |= period_menu(ui, &mut p.period);
            changed |= ui
                .add(
                    egui::Slider::new(&mut p.opacity, 0.0..=1.)
                        .text(crate::i18n::tr("透明叠加"))
                        .show_value(false),
                )
                .changed();
        });
        let (lo, hi) = p.field.scale(p.period);
        ui.horizontal_wrapped(|ui| {
            let steps = if matches!(p.field, Field::Zone | Field::Vegetation) {
                hi as usize
            } else {
                4
            };
            for k in 0..=steps {
                let v = lo + (hi - lo) * k as f32 / steps as f32;
                let [r, g, b, _] = p.field.color(v, p.period);
                ui.colored_label(Color32::from_rgb(r, g, b), "■");
                ui.label(p.field.format(v));
            }
        });
        ui.collapsing("模拟范围（西界可跨日界线）", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (name, v, limit) in [
                    ("西", &mut p.region.west, 180.),
                    ("东", &mut p.region.east, 180.),
                    ("南", &mut p.region.south, 90.),
                    ("北", &mut p.region.north, 90.),
                ] {
                    ui.label(name);
                    changed |= numeric(ui, v, -limit..=limit, 0.5);
                }
            });
            ui.horizontal(|ui| {
                for (first, label) in [(true, "地图取西南角"), (false, "地图取东北角")]
                {
                    if ui.button(crate::i18n::tr(label)).clicked() {
                        self.pick_corner = Some(first);
                        self.picking = true;
                    }
                }
                if ui.button(crate::i18n::tr("全球")).clicked() {
                    p.region = Region::default();
                    changed = true;
                }
            });
            if let Err(e) = p.region.validate() {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        });
        self.position(ui);
        let Some(s) = p.shown() else {
            ui.label(crate::i18n::tr("运行一次模拟，即可查看全球概览和取点数据。"));
            return changed;
        };
        if let Some(d) = &self.display {
            let [temp, rain, ice, dry, range] = d.stats;
            ui.label(crate::trf!(
                "区域面积加权平均　{temp:.1} °C　{rain:.0} mm / 公转年　年温差 {range:.1} °C"
            ));
            ui.label(crate::trf!(
                "冰雪覆盖 {:.0}%　干旱 / 半干旱 {:.0}%",
                100. * ice,
                100. * dry
            ));
        }
        ui.label(crate::trf!(
            "结果对应地图年份　{}",
            s.year.map_or("最新".into(), |y| y.to_string())
        ));
        let i = index(self.point);
        if !s.region.contains(center(i)) {
            ui.label(crate::i18n::tr("该位置在此结果的范围外。"));
            return changed;
        }
        let g = s.cells[i].ground;
        ui.label(crate::trf!(
            "取点地表　{}　海拔 {:.0} m{}{}",
            g.surface.name(),
            g.height,
            if g.river { "　河流水汽" } else { "" },
            if g.forest { "　森林蒸散" } else { "" }
        ));
        let west = s.cells[i / W * W + (i % W + W - 1) % W].ground.height;
        let east = s.cells[i / W * W + (i % W + 1) % W].ground.height;
        let mut natural = (**s).clone();
        natural.edits.clear();
        ui.weak(crate::trf!(
            "{}　{}　邻近东西地形高差 {:.0} m",
            if s.settings.planet.locked_rotation {
                "同步自转日照"
            } else {
                "纬度和季节日照"
            },
            if g.surface == Surface::Unknown {
                "未知地表按均匀先验计算"
            } else if g.water(0.) > 0.5 {
                "水体缓冲季节温差"
            } else {
                "陆地热惯量较低"
            },
            (west - east).abs()
        ));
        egui::Grid::new("climate-data").show(ui, |ui| {
            for f in Field::ALL {
                ui.label(f.name());
                ui.label(f.format(s.value(i, f, p.period)));
                if let Some(e) = s.edits.get(&(i, f)) {
                    ui.label(crate::trf!(
                        "{} · 本次模拟 {}",
                        if e.authored {
                            "人工修订"
                        } else {
                            "保留值"
                        },
                        f.format(natural.value(i, f, p.period))
                    ));
                    if e.locked {
                        ui.label(crate::i18n::tr("已固定"));
                    }
                }
                ui.end_row();
            }
        });
        changed
    }
}
