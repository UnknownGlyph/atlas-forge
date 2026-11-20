//! Roads and travel in the interface (`map_core::roads`): a window that
//! works out a road network between the cities (how many neighbours each
//! may link to, how much detour earns a shortcut, steep ground dearer) and
//! draws it on the map until it is applied (one undo step, as routes); and
//! the way between two cities along the routes, with its days of travel.
//! Travel times also show for a measured length and a selected line.

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, Stroke};
use map_core::doc::format_num;
use map_core::roads::{self, RoadPlan, RoadSpec, MODES};
use map_core::doc::LineId;
use map_core::{EntityId, EntityKind};

/// The window's settings, the plan shown, and the travel asked about.
#[derive(Default)]
pub struct Roads {
    pub open: bool,
    pub spec: RoadSpec,
    pub plan: Option<(RoadSpec, Result<RoadPlan, String>)>,
    pub from: Option<EntityId>,
    pub to: Option<EntityId>,
    /// The way found between them: its routes.
    pub way: Vec<LineId>,
    /// The settings last applied, and how many lines the map had then:
    /// until either changes the same roads are not offered again.
    pub applied: Option<(RoadSpec, usize)>,
}

const PLAN: Color32 = Color32::from_rgb(150, 95, 40);

/// Travel times of a length, one line per way of travelling on land.
pub fn travel_lines(km: f64) -> String {
    MODES.iter().filter(|m| !m.by_sea).map(|m| format!("{}：{}", m.name, roads::format_days(roads::days(km, m)))).collect::<Vec<_>>().join("　")
}

impl App {
    pub fn roads_window(&mut self, ctx: &egui::Context) {
        if !self.roads.open {
            self.roads.plan = None;
            self.roads.way.clear();
            return;
        }
        let stale = self.roads.plan.as_ref().is_none_or(|(s, _)| *s != self.roads.spec);
        if stale {
            let spec = self.roads.spec.clone();
            let plan = self.doc.road_plan(&spec);
            self.roads.plan = Some((spec, plan));
        }
        let mut cities: Vec<(EntityId, String)> = self.doc.entities.iter().filter(|(_, x)| x.kind == EntityKind::City && x.pos.is_some()).map(|(e, x)| (*e, x.name.clone())).collect();
        cities.sort_by(|a, b| a.1.cmp(&b.1));
        let name = |e: Option<EntityId>| e.and_then(|e| cities.iter().find(|c| c.0 == e)).map(|c| c.1.clone()).unwrap_or_else(|| "（选择城市）".into());
        let done = self.roads.applied.as_ref().is_some_and(|(s, n)| *s == self.roads.spec && *n == self.doc.rivers.len());
        let mut open = true;
        let mut apply = false;
        egui::Window::new(crate::i18n::tr("道路与旅行")).open(&mut open).resizable(false).default_width(340.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("道路网")).strong());
            let s = &mut self.roads.spec;
            egui::Grid::new("roads-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(crate::i18n::tr("相邻城市"));
                ui.add(egui::Slider::new(&mut s.neighbours, 1..=8)).on_hover_text(crate::i18n::tr("每座城市最多与几座最近的城市考虑相连"));
                ui.end_row();
                ui.label(crate::i18n::tr("捷径"));
                ui.add(egui::Slider::new(&mut s.detour, 1.0..=4.0).step_by(0.1)).on_hover_text(crate::i18n::tr("沿已有道路比直接相连远出这个倍数时，加一条捷径；1.0 只要能连通的最少道路"));
                ui.end_row();
                ui.label(crate::i18n::tr("地形"));
                ui.add_enabled(self.doc.terrain.is_some(), egui::Checkbox::new(&mut s.terrain, "陡坡代价更高")).on_disabled_hover_text(crate::i18n::tr("这张地图没有地形（新世界向导会生成地形）"));
                ui.end_row();
            });
            match self.roads.plan.as_ref().map(|p| &p.1) {
                _ if done => {
                    ui.weak(crate::i18n::tr("已应用这组道路。改变设置可得到新的方案（已有的道路不计入，应用会再画一组）。"));
                }
                Some(Ok(plan)) => {
                    let km: f64 = plan.roads.iter().map(|r| r.km).sum();
                    ui.label(crate::trf!("{} 座城市，{} 条道路，共 {} km", plan.cities, plan.roads.len(), format_num(km.round())));
                    if !plan.alone.is_empty() {
                        let names: Vec<String> = plan.alone.iter().filter_map(|e| self.doc.entities.get(e)).map(|x| x.name.clone()).take(6).collect();
                        ui.weak(crate::trf!("无法经陆路连通：{}{}", names.join("、"), if plan.alone.len() > 6 { "……" } else { "" }));
                    }
                    ui.weak(crate::i18n::tr("地图上棕色虚线为预览；应用后成为普通线路（所有年份相同，可逐条修改或删除）。"));
                    if ui.add_enabled(!plan.roads.is_empty(), egui::Button::new(crate::i18n::tr("应用"))).clicked() {
                        apply = true;
                    }
                }
                Some(Err(err)) => {
                    ui.colored_label(Color32::from_rgb(220, 90, 60), err);
                }
                None => {}
            }
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::tr("旅行")).strong());
            ui.horizontal(|ui| {
                for (k, slot) in [(0, &mut self.roads.from), (1, &mut self.roads.to)] {
                    egui::ComboBox::from_id_salt(("travel", k)).width(130.0).selected_text(name(*slot)).show_ui(ui, |ui| {
                        for (e, n) in &cities {
                            ui.selectable_value(slot, Some(*e), n);
                        }
                    });
                    if k == 0 {
                        ui.label("→");
                    }
                }
            });
            self.roads.way.clear();
            if let (Some(a), Some(b)) = (self.roads.from, self.roads.to) {
                let pos = |e: EntityId| self.doc.entities.get(&e).and_then(|x| x.pos);
                let straight = pos(a).zip(pos(b)).map(|(p, q)| map_core::measure::great_circle(p, q, self.doc.radius())).unwrap_or(0.0);
                match self.doc.travel_way(a, b) {
                    Some((lines, km)) if a != b => {
                        ui.label(crate::trf!("沿线路 {} km（{} 段；直线 {} km）", format_num(km.round()), lines.len(), format_num(straight.round())));
                        ui.label(travel_lines(km));
                        self.roads.way = lines;
                    }
                    _ if a != b => {
                        ui.weak(crate::i18n::tr("两城之间没有相连的线路（线路的两端要在城市上）。按直线："));
                        ui.label(format!("{} km　{}", format_num(straight.round()), travel_lines(straight)));
                        let sea = MODES.iter().find(|m| m.by_sea).unwrap();
                        ui.weak(crate::trf!("{}（直线，仅供参考）：{}", sea.name, roads::format_days(roads::days(straight, sea))));
                    }
                    _ => {}
                }
            }
        });
        if apply {
            if let Some((_, Ok(plan))) = self.roads.plan.take() {
                let n = plan.roads.len();
                if self.edit("生成道路", |d| d.apply_roads(&plan)).is_some() {
                    self.set_status(crate::trf!("已生成 {n} 条道路"), false);
                    self.roads.applied = Some((self.roads.spec.clone(), self.doc.rivers.len()));
                }
            }
        }
        if !open {
            self.roads.open = false;
        }
    }

    /// The plan's roads (dashed) and the travelled way (bright) on a view.
    pub fn draw_roads(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if !self.roads.open {
            return;
        }
        let jump = (90.0 * vg.px_per_deg()) as f32;
        let line = |pts: &[map_core::LonLat], f: &mut dyn FnMut(Vec<egui::Pos2>)| {
            let mut run: Vec<egui::Pos2> = Vec::new();
            for p in pts {
                match vg.to_screen(*p) {
                    Some(q) if run.last().is_none_or(|l| (q - *l).length() < jump) => run.push(q),
                    q => {
                        if run.len() >= 2 {
                            f(std::mem::take(&mut run));
                        }
                        run = q.into_iter().collect();
                    }
                }
            }
            if run.len() >= 2 {
                f(run);
            }
        };
        let done = self.roads.applied.as_ref().is_some_and(|(s, n)| *s == self.roads.spec && *n == self.doc.rivers.len());
        if let (Some((_, Ok(plan))), false) = (&self.roads.plan, done) {
            for road in &plan.roads {
                line(&road.pts, &mut |run| painter.extend(egui::Shape::dashed_line(&run, Stroke::new(1.8, PLAN), 6.0, 3.0)));
            }
        }
        for id in &self.roads.way {
            if let Some(l) = self.doc.rivers.get(id) {
                line(&l.pts, &mut |run| {
                    painter.add(egui::Shape::line(run, Stroke::new(5.0, Color32::from_rgba_unmultiplied(255, 200, 40, 170))));
                });
            }
        }
    }
}
