//! Independent star chart workspace. Navigation and playback are transient;
//! template edits are normal project edits with undo and save support.
mod extended;
mod render;

use crate::{app::App, i18n::tr};
use eframe::egui::{self, Color32, Sense, TextureHandle};
use map_core::star_chart::{
    Body, BodyKey, Companion, Galaxy, GalaxyStar, Moon, Star, StarChart, AU_KM,
};
use map_core::Document;
use map_core::star_chart::gaia::Center as GaiaCenter;
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
    Companion(usize),
}

impl Target {
    fn key(self) -> BodyKey {
        match self {
            Self::Star => BodyKey::Star,
            Self::Home => BodyKey::Home,
            Self::Planet(i) => BodyKey::Planet(i),
            Self::Companion(i) => BodyKey::Companion(i),
        }
    }
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
    driver: Option<extended::Driver>,
    gravity_error: Option<String>,
    actual_step: f64,
    models: extended::Models,
    gaia_center: GaiaCenter,
    gaia_visible: bool,
    observed: Option<usize>,
    galaxy_selected: Option<usize>,
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
            driver: None,
            gravity_error: None,
            actual_step: 0.0,
            models: Default::default(),
            gaia_center: GaiaCenter::Galaxy,
            gaia_visible: true,
            observed: None,
            galaxy_selected: None,
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
        let mut asset_command = None;
        let mut catalogue_export = None;
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
        let lunar_phase =
            render::displayed_phase(state, &home, &state.draft.as_ref().unwrap().1.moon);
        if state.mode != Mode::Galaxy {
            state.surface_tick(ui.ctx(), &self.doc);
        }
        egui::Panel::top("star-chart-controls").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::i18n::tr("返回地图")).clicked() {
                    state.open = false;
                }
                ui.separator();
                ui.selectable_value(&mut state.mode, Mode::System, crate::i18n::tr("本恒星系"));
                ui.selectable_value(&mut state.mode, Mode::Lunar, crate::i18n::tr("地月系"));
                ui.selectable_value(&mut state.mode, Mode::Galaxy, crate::i18n::tr("银河系"));
                ui.separator();
                ui.checkbox(&mut state.labels, crate::i18n::tr("名称"));
                if state.mode != Mode::Galaxy {
                    let (_, chart) = state.draft.as_mut().unwrap();
                    ui.checkbox(&mut chart.gravity, tr("相互引力"))
                        .on_hover_text(tr("计算天体间的牛顿引力。关闭时沿各自设定的轨道运行。"));
                    ui.checkbox(&mut state.orbits, crate::i18n::tr("轨道"));
                    ui.checkbox(&mut state.lighting, crate::i18n::tr("光照"));
                    if ui
                        .button(if state.playing {
                            crate::i18n::tr("暂停")
                        } else {
                            crate::i18n::tr("播放")
                        })
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
                            .suffix(crate::i18n::tr(" 地球日/秒")),
                    );
                    let time = ui.add(
                        egui::DragValue::new(&mut state.days)
                            .range(-1.0e8..=1.0e8)
                            .speed(3.0)
                            .suffix(crate::i18n::tr(" 地球日")),
                    );
                    if time.changed() {
                        state.eclipse_note = None;
                    }
                    if ui.button(crate::i18n::tr("归零")).clicked() {
                        state.days = 0.0;
                        state.eclipse_note = None;
                    }
                }
                if ui.button(crate::i18n::tr("重置视角")).clicked() {
                    match state.mode {
                        Mode::System => {
                            state.system_camera = render::Camera::system();
                            state.follow = false;
                        }
                        Mode::Lunar => {
                            state.lunar_camera = render::Camera::lunar();
                            state.lunar_follow = false;
                        }
                        Mode::Galaxy => {
                            state.galaxy_camera = if state.gaia_center == GaiaCenter::SolarSystem {
                                render::Camera::neighborhood()
                            } else {
                                render::Camera::galaxy()
                            }
                        }
                    }
                }
                if state.mode == Mode::Lunar && ui.button(crate::i18n::tr("朝向恒星")).clicked()
                {
                    state.lunar_camera.face_star(&home, state.days);
                }
            });
            if let Some(error) = &state.gravity_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
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
                            ui.weak(crate::i18n::tr("太阳系模板 · 距离和球体大小按示意比例显示"));
                            ui.checkbox(&mut draft.use_environment, crate::i18n::tr("从当前行星和气候读取参数"));
                            ui.horizontal(|ui| {
                                if ui.button(crate::i18n::tr("载入太阳系模板")).clicked() {
                                    let galaxy = draft.galaxy.clone();
                                    let moon = draft.moon.clone();
                                    *draft = StarChart {
                                        galaxy,
                                        moon,
                                        ..Default::default()
                                    };
                                    state.target = Target::Home;
                                }
                                if ui.button(crate::i18n::tr("跟随选中")).clicked() {
                                    state.follow = !state.follow;
                                }
                                if state.follow {
                                    ui.weak(crate::i18n::tr("跟随中"));
                                }
                            });
                            if let Target::Planet(i) = state.target {
                                if i >= draft.planets.len() {
                                    state.target = Target::Home;
                                }
                            }
                            if let Target::Companion(i) = state.target {
                                if i >= draft.companions.len() { state.target = Target::Star; }
                            }
                            egui::ComboBox::from_id_salt("star-chart-body")
                                .selected_text(match state.target {
                                    Target::Star => &star.name,
                                    Target::Home => &home.name,
                                    Target::Planet(i) => &draft.planets[i].name,
                                    Target::Companion(i) => &draft.companions[i].star.name,
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
                                    for (i, p) in draft.companions.iter().enumerate() {
                                        ui.selectable_value(&mut state.target,Target::Companion(i),&p.star.name);
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
                                    ui.label(crate::trf!("工程行星　{}", home.name));
                                    if ui.button(crate::i18n::tr("查看地月系")).clicked() {
                                        state.mode = Mode::Lunar;
                                    }
                                    if draft.use_environment {
                                        ui.label(crate::trf!("半径 {:.3} R⊕", home.radii_earth));
                                        if has_climate {
                                            ui.label(crate::trf!(
                                                "轨道 {:.3} AU　偏心率 {:.4}",
                                                home.orbit.au, home.orbit.eccentricity
                                            ));
                                            ui.label(crate::trf!(
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
                                    let mass = draft.planets[i].effective_mass_earth();
                                    ui.text_edit_singleline(&mut draft.planets[i].name);
                                    if draft.planets[i].effective_mass_earth() != mass && draft.planets[i].mass_earth.is_none() {
                                        draft.planets[i].mass_earth = Some(mass);
                                    }
                                    ui.text_edit_singleline(&mut draft.planets[i].category).on_hover_text(tr("自定义天体类别"));
                                    edit_body(ui, &mut draft.planets[i], true);
                                    if ui.button(crate::i18n::tr("移除此行星")).clicked() {
                                        draft.planets.remove(i);
                                        state.target = Target::Home;
                                    }
                                }
                                Target::Companion(i) => {
                                    let item = &mut draft.companions[i];
                                    ui.text_edit_singleline(&mut item.star.name);
                                    edit_star(ui,&mut item.star);
                                    edit_orbit(ui,&mut item.orbit, 0.02..=200.0, 0.1..=100_000.0);
                                    if ui.button(tr("移除伴星")).clicked() {
                                        draft.companions.remove(i);
                                        state.target = Target::Star;
                                    }
                                }
                            }
                            let key = state.target.key();
                            if let Some(model) = draft.model_slot(key) {
                                extended::model_row(ui,key,model,&self.doc,&mut asset_command);
                            }
                            if ui.add_enabled(draft.companions.len() < 2,egui::Button::new(tr("添加伴星"))).clicked() {
                                let mass = 0.6;
                                let au = 8.0 + 22.0 * draft.companions.len() as f64;
                                let mut star = draft.star.clone();
                                star.name = format!("{} {}", tr("伴星"), draft.companions.len()+1);
                                star.mass_solar = mass;
                                star.radius_solar = 0.65;
                                star.luminosity_solar = 0.12;
                                star.temperature = 4600.0;
                                star.model = None;
                                let period = std::f64::consts::TAU * (au*au*au / (map_core::star_chart::G * (star.mass_solar + draft.star.mass_solar))).sqrt();
                                draft.companions.push(Companion { star, orbit: map_core::star_chart::Orbit { au, days:period, eccentricity:0.05, inclination:5.0, phase:110.0 } });
                                state.target = Target::Companion(draft.companions.len()-1);
                            }
                            if ui
                                .add_enabled(
                                    draft.planets.len() < 24,
                                    egui::Button::new(tr("添加自定义天体")),
                                )
                                .clicked()
                            {
                                let mut body = StarChart::default().home;
                                body.name = format!("行星 {}", draft.planets.len() + 2);
                                body.orbit.phase =
                                    (draft.planets.len() as f64 * 137.5).rem_euclid(360.0);
                                body.orbit.au = 1.6 + draft.planets.len() as f64 * 0.8;
                                body.orbit.days = 365.256 * body.orbit.au.powf(1.5);
                                draft.planets.push(body);
                                state.target = Target::Planet(draft.planets.len() - 1);
                            }
                        }
                        Mode::Lunar => {
                            use render::LunarTarget;
                            ui.heading(crate::i18n::tr("地月系"));
                            if let LunarTarget::Satellite(i) = state.lunar_target {
                                if i >= draft.satellites.len() { state.lunar_target = LunarTarget::Moon; }
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.selectable_value(&mut state.lunar_target, LunarTarget::Home, &home.name);
                                ui.selectable_value(&mut state.lunar_target, LunarTarget::Moon, &draft.moon.name);
                                for (i,moon) in draft.satellites.iter().enumerate() {
                                    ui.selectable_value(&mut state.lunar_target, LunarTarget::Satellite(i), &moon.name);
                                }
                                ui.checkbox(&mut state.lunar_follow, crate::i18n::tr("跟随"));
                            });
                            let illumination = lunar_phase * 100.0;
                            ui.label(crate::trf!("几何月相　亮面 {illumination:.1}%"));
                            ui.horizontal_wrapped(|ui| {
                                for (target, name) in [(LunarTarget::Home, "下一次日食"), (LunarTarget::Moon, "下一次月食")] {
                                    if ui.add_enabled(!draft.gravity,egui::Button::new(name)).clicked() {
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
                            if let LunarTarget::Satellite(i) = state.lunar_target {
                                let satellite = &mut draft.satellites[i];
                                ui.text_edit_singleline(&mut satellite.name);
                                ui.text_edit_singleline(&mut satellite.category).on_hover_text(tr("自定义天体类别"));
                                edit_moon(ui, satellite);
                                if ui.button(tr("移除卫星")).clicked() {
                                    draft.satellites.remove(i);
                                    state.lunar_target = LunarTarget::Moon;
                                }
                            } else {
                                ui.label(crate::i18n::tr("月球参数"));
                                ui.text_edit_singleline(&mut draft.moon.name);
                                edit_moon(ui, &mut draft.moon);
                            }
                            let key = state.lunar_target.key();
                            if let Some(model) = draft.model_slot(key) {
                                extended::model_row(ui,key,model,&self.doc,&mut asset_command);
                            }
                            ui.horizontal_wrapped(|ui| {
                                for (artificial,label) in [(false, "添加卫星"),(true,"添加人造天体")] {
                                    if ui.add_enabled(draft.satellites.len() < 16,egui::Button::new(tr(label))).clicked() {
                                        let mut moon = Moon::default();
                                        moon.name = format!("{} {}", tr(if artificial {"人造天体"} else {"卫星"}),draft.satellites.len()+1);
                                        moon.category = tr(if artificial {"人造天体"} else {"自然卫星"}).into();
                                        let scale = (home.radii_earth.powi(3)).max(0.01);
                                        let km = if artificial {(home.radii_earth * 6371.0 + 1200.0) + 600.0 * draft.satellites.len() as f64}
                                            else { (home.radii_earth * 6371.0 + 250000.0) + 170000.0 * draft.satellites.len() as f64 };
                                        moon.radius_km = if artificial {0.015} else {120.0};
                                        moon.mass_kg = artificial.then_some(420000.0);
                                        moon.orbit.au = km / AU_KM;
                                        moon.orbit.days = std::f64::consts::TAU * (moon.orbit.au.powi(3)/(map_core::star_chart::G * 3.00349e-6 * scale)).sqrt();
                                        moon.orbit.eccentricity = 0.01;
                                        moon.orbit.phase = (180.0 + 57.0 * draft.satellites.len() as f64).rem_euclid(360.0);
                                        draft.satellites.push(moon);
                                        state.lunar_target = LunarTarget::Satellite(draft.satellites.len()-1);
                                    }
                                }
                            });
                            if ui.button(crate::i18n::tr("恢复月球模板")).clicked() {
                                draft.moon = Moon::default();
                                state.eclipse_note = None;
                            }
                            ui.weak(crate::i18n::tr("球体与轨道采用示意比例。光影按实际参数求解。恒星和远方星系固定在天空背景。"));
                        }
                        Mode::Galaxy => {
                            ui.heading(&draft.galaxy.name);
                            ui.weak(crate::i18n::tr("银河系模板 · 可旋转的棒旋星系示意"));
                            ui.text_edit_singleline(&mut draft.galaxy.name);
                            let before = state.gaia_center;
                            ui.horizontal_wrapped(|ui| {
                                ui.selectable_value(&mut state.gaia_center, GaiaCenter::Galaxy, tr("银河系中心"));
                                ui.selectable_value(&mut state.gaia_center, GaiaCenter::SolarSystem, tr("太阳系中心"));
                            });
                            if before != state.gaia_center {
                                state.galaxy_camera = if state.gaia_center == GaiaCenter::SolarSystem {render::Camera::neighborhood()} else {render::Camera::galaxy()};
                            }
                            ui.checkbox(&mut state.gaia_visible,tr("Gaia DR3 观测星表"));
                            ui.weak("8,000 · ESA/Gaia/DPAC");
                            if let Some(star) = state.observed.and_then(|i| map_core::star_chart::gaia::stars().get(i)) {
                                ui.monospace(format!("Gaia DR3 {}",star.source_id));
                                ui.label(format!("{:.3} kly · G {:.2} · l {:.2}° · b {:.2}°",star.distance_kly,star.magnitude,star.galactic_longitude,star.galactic_latitude));
                                let p = star.coordinates(state.gaia_center,draft.galaxy.home_kly,draft.galaxy.home_angle);
                                ui.monospace(format!("X {:.3} · Y {:.3} · Z {:.3} kly",p.x,p.y,p.z));
                            }
                            if ui.button(tr("导出当前星表坐标")).clicked() {
                                catalogue_export = Some((state.gaia_center,draft.galaxy.home_kly,draft.galaxy.home_angle));
                            }
                            ui.checkbox(&mut draft.galaxy.barred, crate::i18n::tr("中央棒"));
                            ui.add(egui::Slider::new(&mut draft.galaxy.arms, 2..=6).text(crate::i18n::tr("旋臂")));
                            ui.add(
                                egui::Slider::new(&mut draft.galaxy.radius_kly, 15.0..=150.0)
                                    .text(crate::i18n::tr("盘面半径（千光年）")),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut draft.galaxy.home_kly,
                                    0.0..=draft.galaxy.radius_kly,
                                )
                                .text(crate::i18n::tr("本恒星系距核心（千光年）")),
                            );
                            ui.add(
                                egui::Slider::new(&mut draft.galaxy.home_angle, 0.0..=360.0)
                                    .text(crate::i18n::tr("本恒星系方位 °")),
                            );
                            if ui.button(crate::i18n::tr("载入银河系模板")).clicked() {
                                draft.galaxy = Galaxy::default();
                            }
                            if ui.button(crate::i18n::tr("查看本恒星系")).clicked() {
                                state.mode = Mode::System;
                            }
                            if ui.add_enabled(draft.galaxy.stars.len() < 128,egui::Button::new(tr("添加自定义恒星"))).clicked() {
                                let angle = draft.galaxy.home_angle.to_radians();
                                draft.galaxy.stars.push(GalaxyStar {
                                    name: format!("{} {}",tr("恒星"),draft.galaxy.stars.len()+1),
                                    position_kly: [draft.galaxy.home_kly*angle.cos()+1.5,0.2,draft.galaxy.home_kly*angle.sin()],
                                    temperature:5772.0,category:tr("恒星").into(),model:None,
                                });
                                state.galaxy_selected = Some(draft.galaxy.stars.len()-1);
                            }
                            egui::ComboBox::from_id_salt("galaxy-authored-stars")
                                .selected_text(state.galaxy_selected.and_then(|i|draft.galaxy.stars.get(i)).map_or(tr("选择自定义恒星"),|s| &s.name))
                                .show_ui(ui, |ui| {
                                    for (i,s) in draft.galaxy.stars.iter().enumerate() {
                                        ui.selectable_value(&mut state.galaxy_selected,Some(i),&s.name);
                                    }
                                });
                            if let Some(i) = state.galaxy_selected {
                                if let Some(s) = draft.galaxy.stars.get_mut(i) {
                                    ui.text_edit_singleline(&mut s.name);
                                    ui.text_edit_singleline(&mut s.category).on_hover_text(tr("自定义天体类别"));
                                    ui.horizontal(|ui| {
                                        for (axis, value) in ["X","Y","Z"].into_iter().zip(&mut s.position_kly) {
                                            ui.add(egui::DragValue::new(value).range(-400.0..=400.0).speed(0.1).prefix(axis).suffix(" kly"));
                                        }
                                    });
                                    number(ui,tr("温度 K"),&mut s.temperature,2200.0..=50000.0);
                                    extended::model_row(ui,BodyKey::GalaxyStar(i),&mut s.model,&self.doc,&mut asset_command);
                                    if ui.button(tr("移除此恒星")).clicked() {
                                        draft.galaxy.stars.remove(i);
                                        state.galaxy_selected = None;
                                    }
                                }
                            }
                        }
                    }
                    ui.separator();
                    let changed = chart
                        .as_deref()
                        .map_or_else(|| *draft != StarChart::default(), |old| old != draft);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(changed, egui::Button::new(crate::i18n::tr("应用到工程")))
                            .clicked()
                        {
                            apply = Some(draft.clone());
                        }
                        if ui
                            .add_enabled(changed, egui::Button::new(crate::i18n::tr("还原面板")))
                            .clicked()
                        {
                            *draft = chart.as_deref().cloned().unwrap_or_default();
                        }
                    });
                    ui.weak(crate::i18n::tr("拖动旋转 · 滚轮缩放 · 单击选择"));
                });
            });
        if state.mode != Mode::Galaxy {
            let speed = if state.mode == Mode::Lunar {
                state.lunar_speed
            } else {
                state.speed
            };
            let dt = if state.playing {
                ui.input(|i| f64::from(i.stable_dt.min(0.05))) * speed
            } else {
                0.0
            };
            state.tick_gravity(&self.doc, dt, ui);
            if state.playing {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if let Some((_, chart)) = &state.draft {
            state.models.update(&self.doc, chart, ui.ctx());
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
                            render::system(ui, rect, &response, state, &star, &home, draft)
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
                            &draft.satellites,
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
                if chart.model_ids().any(|id| {
                    !doc.book
                        .as_ref()
                        .is_some_and(|book| book.assets.contains_key(&id))
                }) {
                    return Err(tr("星图引用的模型已不存在。").into());
                }
                doc.star_chart = Some(Arc::new(chart));
                Ok(())
            });
        }
        if let Some(command) = asset_command {
            match command {
                extended::AssetCommand::Inspect(id) => self.open_model_asset(id),
                extended::AssetCommand::Import(key) => self.import_astronomy_model(key),
            }
        }
        if let Some((center, radius, angle)) = catalogue_export {
            let name = match center {
                GaiaCenter::Galaxy => "Gaia-DR3-Galactic-Center.csv",
                GaiaCenter::SolarSystem => "Gaia-DR3-Solar-System.csv",
            };
            if let Some(path) = self.file_dialog().add_filter("CSV",&["csv"]).set_file_name(name).save_file() {
                if !path.extension().is_some_and(|x|x.eq_ignore_ascii_case("csv")) {
                    self.set_status(tr("请选择 CSV 文件。"),true);
                } else {
                    match map_io::gaia_csv::export(&path,center,radius,angle) {
                        Ok(()) => self.set_status(tr("已导出 8000 颗观测恒星的坐标。"),false),
                        Err(e) => self.set_status(e,true),
                    }
                }
            }
        }
    }

    fn import_astronomy_model(&mut self, key: BodyKey) {
        let Some(path) = self
            .file_dialog()
            .add_filter("GLB / glTF / OBJ", &["glb", "gltf", "obj"])
            .pick_file()
        else {
            return;
        };
        let (asset, _) = match map_io::model::import(&path) {
            Ok(pair) => pair,
            Err(e) => {
                self.set_status(e, true);
                return;
            }
        };
        let id = self.edit(tr("导入 3D 模型"), |doc| {
            Ok(map_core::world_book::WorldBook::edit(doc).add_asset(asset))
        });
        if let Some((_, chart)) = self.star_map.draft.as_mut() {
            if let (Some(id), Some(slot)) = (id, chart.model_slot(key)) {
                *slot = Some(id);
            }
        }
    }
}

fn edit_moon(ui: &mut egui::Ui, moon: &mut Moon) {
    number(ui, "半径 km", &mut moon.radius_km, 0.000001..=30_000.0);
    let mass = moon
        .mass_kg
        .unwrap_or(7.342e22 * (moon.radius_km / 1737.4).powi(3));
    let mut kg = mass;
    number(ui, tr("质量 kg"), &mut kg, 0.0..=6.0e25);
    if kg != mass {
        moon.mass_kg = Some(kg);
    }
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
    number(
        ui,
        "公转周期（日）",
        &mut moon.orbit.days,
        0.001..=100_000.0,
    );
    number(ui, "初始相位 °", &mut moon.orbit.phase, 0.0..=360.0);
    ui.checkbox(&mut moon.synchronous, crate::i18n::tr("同步自转"));
    if !moon.synchronous {
        number(ui, "自转周期（日）", &mut moon.spin_days, 0.01..=100_000.0);
    }
}

fn number(
    ui: &mut egui::Ui,
    label: &'static str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    ui.horizontal(|ui| {
        ui.label(tr(label));
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
        number(ui, "半径 R⊕", &mut body.radii_earth, 0.000000001..=50.0);
    }
    let before = body.effective_mass_earth();
    let mut mass = before;
    number(ui, tr("质量 M⊕"), &mut mass, 0.0..=30000.0);
    if mass != before {
        body.mass_earth = Some(mass);
    }
    edit_orbit(ui, &mut body.orbit, 0.015..=200.0, 0.1..=100_000.0);
    number(
        ui,
        "自转周期（小时）",
        &mut body.spin_hours,
        0.1..=2400000.0,
    );
    number(ui, "地轴倾角 °", &mut body.tilt, 0.0..=180.0);
    ui.checkbox(&mut body.retrograde, crate::i18n::tr("逆向自转"));
    ui.checkbox(&mut body.synchronous, crate::i18n::tr("潮汐锁定"));
    ui.checkbox(&mut body.rings, crate::i18n::tr("行星环"));
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("球体颜色"));
        let mut color = Color32::from_rgb(body.color[0], body.color[1], body.color[2]);
        if ui.color_edit_button_srgba(&mut color).changed() {
            body.color = [color.r(), color.g(), color.b()];
        }
    });
}

fn edit_orbit(
    ui: &mut egui::Ui,
    orbit: &mut map_core::star_chart::Orbit,
    distance: std::ops::RangeInclusive<f64>,
    duration: std::ops::RangeInclusive<f64>,
) {
    number(ui, "半长轴 AU", &mut orbit.au, distance);
    number(ui, "偏心率", &mut orbit.eccentricity, 0.0..=0.92);
    number(ui, "轨道倾角 °", &mut orbit.inclination, -180.0..=180.0);
    number(ui, "公转周期（日）", &mut orbit.days, duration);
    number(ui, "初始轨道相位 °", &mut orbit.phase, 0.0..=360.0);
}
