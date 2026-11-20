//! Independent star chart workspace. Navigation and playback are transient;
//! template edits are normal project edits with undo and save support.
mod render;

use crate::app::App;
use eframe::egui::{self, Color32, Sense, TextureHandle};
use map_core::star_chart::{Body, Galaxy, Moon, Star, StarChart, AU_KM};
use map_core::Document;
use std::sync::{mpsc, Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    System,
    Lunar,
    Galaxy,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Star,
    Home,
    Planet(usize),
}

struct Surface {
    doc: Document,
    image: TextureHandle,
}
type SurfaceJob = mpsc::Receiver<(Document, Result<egui::ColorImage, String>)>;

pub struct StarMap {
    pub open: bool,
    mode: Mode,
    system_camera: render::Camera,
    galaxy_camera: render::Camera,
    lunar_camera: render::Camera,
    lunar_target: render::LunarTarget,
    lunar_follow: bool,
    lunar_speed: f64,
    eclipse_note: Option<&'static str>,
    target: Target,
    follow: bool,
    playing: bool,
    days: f64,
    speed: f64,
    labels: bool,
    orbits: bool,
    lighting: bool,
    surface: Option<Surface>,
    surface_job: Option<SurfaceJob>,
    surface_failed: bool,
    points: Option<(Galaxy, Vec<render::PointStar>)>,
    draft: Option<(Option<Arc<StarChart>>, StarChart)>,
}
impl Default for StarMap {
    fn default() -> Self {
        Self {
            open: false,
            mode: Mode::System,
            system_camera: render::Camera::system(),
            galaxy_camera: render::Camera::galaxy(),
            lunar_camera: render::Camera::lunar(),
            lunar_target: render::LunarTarget::Home,
            lunar_follow: false,
            lunar_speed: 1.2,
            eclipse_note: None,
            target: Target::Home,
            follow: false,
            playing: true,
            days: 0.0,
            speed: 32.0,
            labels: true,
            orbits: true,
            lighting: true,
            surface: None,
            surface_job: None,
            surface_failed: false,
            points: None,
            draft: None,
        }
    }
}

fn same_surface(a: &Document, b: &Document) -> bool {
    a.topo.faces.ptr_eq(&b.topo.faces)
        && a.topo.edges.ptr_eq(&b.topo.edges)
        && a.topo.nodes.ptr_eq(&b.topo.nodes)
        && a.style.ocean == b.style.ocean
        && a.style.land == b.style.land
}

fn make_surface(doc: &Document) -> Result<egui::ColorImage, String> {
    // Only the base geography goes on the planet. Use the same export
    // projection and topology as the map, with no labels or decorations.
    let mut bare = Document::new(&doc.meta.name, doc.radius());
    bare.topo = doc.topo.clone();
    bare.style.ocean = doc.style.ocean;
    bare.style.land = doc.style.land;
    let options = map_io::scene::ExportOptions {
        width: 512,
        height: 256,
        projection: map_core::projection::Projection::Equirectangular,
        labels: false,
        legend: false,
        graticule: false,
        ..Default::default()
    };
    let theme = map_core::theme::evaluate(&bare);
    let scene = map_io::scene::build_scene(&bare, &theme, &options);
    let pixmap = map_io::png::render(&scene, None)?;
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [options.width as usize, options.height as usize],
        pixmap.data(),
    ))
}

impl StarMap {
    fn surface_tick(&mut self, ctx: &egui::Context, doc: &Document) {
        if let Some(job) = &self.surface_job {
            match job.try_recv() {
                Ok((source, image)) => {
                    if same_surface(&source, doc) {
                        match image {
                            Ok(pixels) => {
                                let image = ctx.load_texture(
                                    "star-chart-home-globe",
                                    pixels,
                                    egui::TextureOptions::LINEAR_REPEAT,
                                );
                                self.surface = Some(Surface { doc: source, image });
                            }
                            Err(_) => self.surface_failed = true,
                        }
                    }
                    self.surface_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(_) => {
                    self.surface_failed = true;
                    self.surface_job = None;
                }
            }
        }
        if !self.surface_failed
            && self
                .surface
                .as_ref()
                .is_none_or(|s| !same_surface(&s.doc, doc))
        {
            let source = doc.clone();
            let wake = ctx.clone();
            let (tx, rx) = mpsc::channel();
            self.surface_job = Some(rx);
            std::thread::spawn(move || {
                let result = make_surface(&source);
                let _ = tx.send((source, result));
                wake.request_repaint();
            });
        }
    }
}

impl App {
    pub fn star_chart(&mut self, ui: &mut egui::Ui) {
        let mut apply = None;
        let chart = self.doc.star_chart.clone();
        let state = &mut self.star_map;
        let stale = state
            .draft
            .as_ref()
            .is_none_or(|(source, _)| match (source, &chart) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            });
        if stale {
            state.draft = Some((chart.clone(), chart.as_deref().cloned().unwrap_or_default()));
        }
        let has_climate = self.doc.climate.is_some();
        let (star, home) = state.draft.as_ref().unwrap().1.resolved(&self.doc);
        if state.mode != Mode::Galaxy {
            state.surface_tick(ui.ctx(), &self.doc);
        }
        egui::Panel::top("star-chart-controls").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("返回地图").clicked() {
                    state.open = false;
                }
                ui.separator();
                ui.selectable_value(&mut state.mode, Mode::System, "本恒星系");
                ui.selectable_value(&mut state.mode, Mode::Lunar, "地月系");
                ui.selectable_value(&mut state.mode, Mode::Galaxy, "银河系");
                ui.separator();
                ui.checkbox(&mut state.labels, "名称");
                if state.mode != Mode::Galaxy {
                    ui.checkbox(&mut state.orbits, "轨道");
                    ui.checkbox(&mut state.lighting, "光照");
                    if ui
                        .button(if state.playing { "暂停" } else { "播放" })
                        .clicked()
                    {
                        state.playing = !state.playing;
                        state.eclipse_note = None;
                    }
                    let speed = if state.mode == Mode::Lunar {
                        &mut state.lunar_speed
                    } else {
                        &mut state.speed
                    };
                    ui.add(
                        egui::DragValue::new(speed)
                            .range(0.1..=5000.0)
                            .speed(if state.mode == Mode::Lunar { 0.08 } else { 2.0 })
                            .suffix(" 地球日/秒"),
                    );
                    let time = ui.add(
                        egui::DragValue::new(&mut state.days)
                            .range(-1.0e8..=1.0e8)
                            .speed(3.0)
                            .suffix(" 地球日"),
                    );
                    if time.changed() {
                        state.eclipse_note = None;
                    }
                    if ui.button("归零").clicked() {
                        state.days = 0.0;
                        state.eclipse_note = None;
                    }
                }
                if ui.button("重置视角").clicked() {
                    match state.mode {
                        Mode::System => {
                            state.system_camera = render::Camera::system();
                            state.follow = false;
                        }
                        Mode::Lunar => {
                            state.lunar_camera = render::Camera::lunar();
                            state.lunar_follow = false;
                        }
                        Mode::Galaxy => state.galaxy_camera = render::Camera::galaxy(),
                    }
                }
                if state.mode == Mode::Lunar && ui.button("朝向恒星").clicked() {
                    state.lunar_camera.face_star(&home, state.days);
                }
            });
        });
        egui::Panel::right("star-chart-parameters")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let (_, draft) = state.draft.as_mut().unwrap();
                    match state.mode {
                        Mode::System => {
                            ui.heading(&star.name);
                            ui.weak("太阳系模板 · 距离和球体大小按示意比例显示");
                            ui.checkbox(&mut draft.use_environment, "从当前行星和气候读取参数");
                            ui.horizontal(|ui| {
                                if ui.button("载入太阳系模板").clicked() {
                                    let galaxy = draft.galaxy.clone();
                                    let moon = draft.moon.clone();
                                    *draft = StarChart {
                                        galaxy,
                                        moon,
                                        ..Default::default()
                                    };
                                    state.target = Target::Home;
                                }
                                if ui.button("跟随选中").clicked() {
                                    state.follow = !state.follow;
                                }
                                if state.follow {
                                    ui.weak("跟随中");
                                }
                            });
                            if let Target::Planet(i) = state.target {
                                if i >= draft.planets.len() {
                                    state.target = Target::Home;
                                }
                            }
                            egui::ComboBox::from_id_salt("star-chart-body")
                                .selected_text(match state.target {
                                    Target::Star => &star.name,
                                    Target::Home => &home.name,
                                    Target::Planet(i) => &draft.planets[i].name,
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut state.target,
                                        Target::Star,
                                        &star.name,
                                    );
                                    ui.selectable_value(
                                        &mut state.target,
                                        Target::Home,
                                        &home.name,
                                    );
                                    for (i, p) in draft.planets.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut state.target,
                                            Target::Planet(i),
                                            &p.name,
                                        );
                                    }
                                });
                            match state.target {
                                Target::Star => {
                                    ui.text_edit_singleline(&mut draft.star.name);
                                    if draft.use_environment && has_climate {
                                        ui.label(format!(
                                            "{:.3} M☉ · {:.3} R☉ · {:.3} L☉ · {:.0} K",
                                            star.mass_solar,
                                            star.radius_solar,
                                            star.luminosity_solar,
                                            star.temperature
                                        ));
                                    } else {
                                        edit_star(ui, &mut draft.star);
                                    }
                                }
                                Target::Home => {
                                    ui.label(format!("工程行星　{}", home.name));
                                    if ui.button("查看地月系").clicked() {
                                        state.mode = Mode::Lunar;
                                    }
                                    if draft.use_environment {
                                        ui.label(format!("半径 {:.3} R⊕", home.radii_earth));
                                        if has_climate {
                                            ui.label(format!(
                                                "轨道 {:.3} AU　偏心率 {:.4}",
                                                home.orbit.au, home.orbit.eccentricity
                                            ));
                                            ui.label(format!(
                                                "公转 {:.1} 日　自转 {:.1} 小时",
                                                home.orbit.days, home.spin_hours
                                            ));
                                            number(
                                                ui,
                                                "轨道倾角 °",
                                                &mut draft.home.orbit.inclination,
                                                -180.0..=180.0,
                                            );
                                            number(
                                                ui,
                                                "初始轨道相位 °",
                                                &mut draft.home.orbit.phase,
                                                0.0..=360.0,
                                            );
                                        } else {
                                            edit_body(ui, &mut draft.home, false);
                                        }
                                    } else {
                                        edit_body(ui, &mut draft.home, true);
                                    }
                                }
                                Target::Planet(i) => {
                                    ui.text_edit_singleline(&mut draft.planets[i].name);
                                    edit_body(ui, &mut draft.planets[i], true);
                                    if ui.button("移除此行星").clicked() {
                                        draft.planets.remove(i);
                                        state.target = Target::Home;
                                    }
                                }
                            }
                            if ui
                                .add_enabled(
                                    draft.planets.len() < 24,
                                    egui::Button::new("添加行星"),
                                )
                                .clicked()
                            {
                                let mut body = StarChart::default().home;
                                body.name = format!("行星 {}", draft.planets.len() + 2);
                                body.orbit.phase =
                                    (draft.planets.len() as f64 * 137.5).rem_euclid(360.0);
                                draft.planets.push(body);
                                state.target = Target::Planet(draft.planets.len() - 1);
                            }
                        }
                        Mode::Lunar => {
                            use render::LunarTarget;
                            ui.heading("地月系");
                            ui.horizontal_wrapped(|ui| {
                                ui.selectable_value(&mut state.lunar_target, LunarTarget::Home, &home.name);
                                ui.selectable_value(&mut state.lunar_target, LunarTarget::Moon, &draft.moon.name);
                                ui.checkbox(&mut state.lunar_follow, "跟随");
                            });
                            let illumination = render::moon_phase(&home, &draft.moon, state.days) * 100.0;
                            ui.label(format!("几何月相　亮面 {illumination:.1}%"));
                            ui.horizontal_wrapped(|ui| {
                                for (target, name) in [(LunarTarget::Home, "下一次日食"), (LunarTarget::Moon, "下一次月食")] {
                                    if ui.button(name).clicked() {
                                        if let Some(day) = render::event_time(&star, &home, &draft.moon, state.days, target) {
                                            state.days = day;
                                            state.playing = false;
                                            state.lighting = true;
                                            state.lunar_target = target;
                                            state.lunar_follow = true;
                                            state.lunar_camera = render::Camera::lunar();
                                            render::aim_eclipse(&mut state.lunar_camera, &star, &home, &draft.moon, day, target);
                                            state.eclipse_note = Some(if target == LunarTarget::Home { "已停在下一次日食附近。" } else { "已停在下一次月食附近。" });
                                        } else {
                                            state.eclipse_note = Some("在搜索区间内没有找到食现象。");
                                        }
                                    }
                                }
                            });
                            if let Some(note) = state.eclipse_note { ui.weak(note); }
                            ui.separator();
                            ui.label("月球参数");
                            ui.text_edit_singleline(&mut draft.moon.name);
                            edit_moon(ui, &mut draft.moon);
                            if ui.button("恢复月球模板").clicked() {
                                draft.moon = Moon::default();
                                state.eclipse_note = None;
                            }
                            ui.weak("球体与轨道采用示意比例。光影按实际参数求解。恒星和远方星系固定在天空背景。");
                        }
                        Mode::Galaxy => {
                            ui.heading(&draft.galaxy.name);
                            ui.weak("银河系模板 · 可旋转的棒旋星系示意");
                            ui.text_edit_singleline(&mut draft.galaxy.name);
                            ui.checkbox(&mut draft.galaxy.barred, "中央棒");
                            ui.add(egui::Slider::new(&mut draft.galaxy.arms, 2..=6).text("旋臂"));
                            ui.add(
                                egui::Slider::new(&mut draft.galaxy.radius_kly, 15.0..=150.0)
                                    .text("盘面半径（千光年）"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut draft.galaxy.home_kly,
                                    0.0..=draft.galaxy.radius_kly,
                                )
                                .text("本恒星系距核心（千光年）"),
                            );
                            ui.add(
                                egui::Slider::new(&mut draft.galaxy.home_angle, 0.0..=360.0)
                                    .text("本恒星系方位 °"),
                            );
                            if ui.button("载入银河系模板").clicked() {
                                draft.galaxy = Galaxy::default();
                            }
                            if ui.button("查看本恒星系").clicked() {
                                state.mode = Mode::System;
                            }
                        }
                    }
                    ui.separator();
                    let changed = chart
                        .as_deref()
                        .map_or_else(|| *draft != StarChart::default(), |old| old != draft);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(changed, egui::Button::new("应用到工程"))
                            .clicked()
                        {
                            apply = Some(draft.clone());
                        }
                        if ui
                            .add_enabled(changed, egui::Button::new("还原面板"))
                            .clicked()
                        {
                            *draft = chart.as_deref().cloned().unwrap_or_default();
                        }
                    });
                    ui.weak("拖动旋转 · 滚轮缩放 · 单击选择");
                });
            });
        if state.mode != Mode::Galaxy && state.playing {
            let speed = if state.mode == Mode::Lunar {
                state.lunar_speed
            } else {
                state.speed
            };
            state.days += ui.input(|i| f64::from(i.stable_dt.min(0.05))) * speed;
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
                if rect.width() < 4.0 || rect.height() < 4.0 {
                    return;
                }
                let response = ui.interact(
                    rect,
                    ui.scope_id().with("astronomy-canvas"),
                    Sense::click_and_drag(),
                );
                let (_, draft) = state.draft.as_ref().unwrap();
                match state.mode {
                    Mode::System => {
                        state.system_camera.input(ui, &response, rect);
                        let (star, home) = draft.resolved(&self.doc);
                        if let Some(target) =
                            render::system(ui, rect, &response, state, &star, &home, &draft.planets)
                        {
                            state.target = target;
                        }
                    }
                    Mode::Lunar => {
                        state.lunar_camera.input(ui, &response, rect);
                        state.lunar_camera.limit_local_zoom();
                        let (star, home) = draft.resolved(&self.doc);
                        if let Some(body) = render::local_system(
                            ui,
                            rect,
                            &response,
                            state,
                            &star,
                            &home,
                            &draft.moon,
                        ) {
                            state.lunar_target = body;
                        }
                    }
                    Mode::Galaxy => {
                        state.galaxy_camera.input(ui, &response, rect);
                        render::galaxy(ui, rect, &response, state);
                    }
                }
            });
        if let Some(chart) = apply {
            self.edit("编辑星图", move |doc| {
                chart.validate()?;
                doc.star_chart = Some(Arc::new(chart));
                Ok(())
            });
        }
    }
}

fn edit_moon(ui: &mut egui::Ui, moon: &mut Moon) {
    number(ui, "半径 km", &mut moon.radius_km, 1.0..=30_000.0);
    let mut distance = moon.distance_km();
    let before = distance;
    number(ui, "轨道半长轴 km", &mut distance, 1_000.0..=10_000_000.0);
    if distance != before {
        moon.orbit.au = distance / AU_KM;
    }
    number(ui, "偏心率", &mut moon.orbit.eccentricity, 0.0..=0.92);
    number(
        ui,
        "轨道倾角 °",
        &mut moon.orbit.inclination,
        -180.0..=180.0,
    );
    number(ui, "公转周期（日）", &mut moon.orbit.days, 0.1..=100_000.0);
    number(ui, "初始相位 °", &mut moon.orbit.phase, 0.0..=360.0);
    ui.checkbox(&mut moon.synchronous, "同步自转");
    if !moon.synchronous {
        number(ui, "自转周期（日）", &mut moon.spin_days, 0.01..=100_000.0);
    }
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>) {
    ui.horizontal(|ui| {
        ui.label(label);
        let speed = (value.abs() * 0.01).clamp(0.001, 4.0);
        ui.add(egui::DragValue::new(value).speed(speed).range(range));
    });
}
fn edit_star(ui: &mut egui::Ui, star: &mut Star) {
    number(ui, "质量 M☉", &mut star.mass_solar, 0.06..=100.0);
    number(ui, "半径 R☉", &mut star.radius_solar, 0.03..=100.0);
    number(
        ui,
        "光度 L☉",
        &mut star.luminosity_solar,
        0.00001..=100000.0,
    );
    number(ui, "温度 K", &mut star.temperature, 2200.0..=50000.0);
}
fn edit_body(ui: &mut egui::Ui, body: &mut Body, allow_radius: bool) {
    if allow_radius {
        number(ui, "半径 R⊕", &mut body.radii_earth, 0.05..=50.0);
    }
    number(ui, "半长轴 AU", &mut body.orbit.au, 0.015..=200.0);
    number(ui, "偏心率", &mut body.orbit.eccentricity, 0.0..=0.92);
    number(
        ui,
        "轨道倾角 °",
        &mut body.orbit.inclination,
        -180.0..=180.0,
    );
    number(ui, "公转周期（日）", &mut body.orbit.days, 0.1..=100000.0);
    number(ui, "初始轨道相位 °", &mut body.orbit.phase, 0.0..=360.0);
    number(
        ui,
        "自转周期（小时）",
        &mut body.spin_hours,
        0.1..=2400000.0,
    );
    number(ui, "地轴倾角 °", &mut body.tilt, 0.0..=180.0);
    ui.checkbox(&mut body.retrograde, "逆向自转");
    ui.checkbox(&mut body.synchronous, "潮汐锁定");
    ui.checkbox(&mut body.rings, "行星环");
    ui.horizontal(|ui| {
        ui.label("球体颜色");
        let mut color = Color32::from_rgb(body.color[0], body.color[1], body.color[2]);
        if ui.color_edit_button_srgba(&mut color).changed() {
            body.color = [color.r(), color.g(), color.b()];
        }
    });
}
