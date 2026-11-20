//! Local views in the interface (`map_core::focus`): a region and a
//! stretch of years kept in the project, to look at a war or another local
//! change on its own. Entering one fits both views to the region and the
//! timeline to the years; outside the region the land is faded, its edge is
//! drawn, and the land that changed hands since its first year is outlined.
//! A small window says what changed, and the years can be played through.

use crate::app::App;
use crate::view::{ViewGeom, ViewKind};
use eframe::egui::{self, Color32, RichText, Stroke};
use map_core::focus::{Focus, FocusReport};
use map_core::geom::{self, LonLat};
use map_core::{Document, FaceId};
use std::collections::HashSet;

/// The colour of a local view's edge and of the land that changed hands.
pub const ACCENT: Color32 = Color32::from_rgb(215, 35, 35);

/// What a local view shows, worked out again only when the map, the view
/// or the year changes.
pub struct FocusView {
    src: Document,
    focus: Focus,
    /// The faces of the region.
    pub faces: Vec<FaceId>,
    pub inside: HashSet<FaceId>,
    /// Of those, the ones not as they were in the first year (in the year shown).
    pub changed: HashSet<FaceId>,
    /// What changed from the first year to the year shown (within its years).
    pub report: Option<FocusReport>,
    /// The years in its stretch in which something there changes.
    pub years: Vec<i32>,
    /// Counts up each time it is worked out again (for the map's overlay).
    pub gen: u64,
}

/// Years a local view steps through: its first, every change, its last.
fn steps(f: &Focus, years: &[i32]) -> Vec<i32> {
    let Some((from, to)) = f.years else { return vec![] };
    let mut v = vec![from];
    v.extend(years.iter().copied());
    v.push(to);
    v.sort();
    v.dedup();
    v
}

impl App {
    pub fn active_focus(&self) -> Option<&Focus> {
        self.focus.and_then(|id| self.doc.focus(id))
    }

    /// Keep what the active local view shows up to date (each frame).
    pub fn update_focus_view(&mut self) {
        let Some(f) = self.active_focus().cloned() else {
            self.focus = None;
            self.focus_view = None;
            self.focus_play = None;
            return;
        };
        let d = &self.doc;
        let (same_faces, same_all) = match &self.focus_view {
            Some(v) => {
                let faces = v.src.topo.faces.ptr_eq(&d.topo.faces) && v.focus.bounds == f.bounds;
                let all = faces
                    && v.focus.years == f.years
                    && v.src.year == d.year
                    && v.src.owner_base.ptr_eq(&d.owner_base)
                    && v.src.owner_changes.ptr_eq(&d.owner_changes)
                    && v.src.kind_changes.ptr_eq(&d.kind_changes)
                    && v.src.entities.ptr_eq(&d.entities)
                    && v.src.layers.ptr_eq(&d.layers)
                    && v.src.active_layer == d.active_layer;
                (faces, all)
            }
            None => (false, false),
        };
        if same_all {
            return;
        }
        let faces = match (&self.focus_view, same_faces) {
            (Some(v), true) => v.faces.clone(),
            _ => d.faces_in(&f),
        };
        let (changed, report, years) = match f.years {
            Some((from, to)) => {
                let y = d.year.unwrap_or(to).clamp(from, to);
                (d.changed_between(&faces, from, y).into_iter().collect(), Some(d.change_report(&faces, from, y)), d.change_years_of(&faces, from, to))
            }
            None => (HashSet::new(), None, vec![]),
        };
        let gen = self.focus_view.as_ref().map_or(0, |v| v.gen + 1);
        let inside = faces.iter().copied().collect();
        self.focus_view = Some(FocusView { src: d.clone(), focus: f, faces, inside, changed, report, years, gen });
    }

    /// The part of the world the flat view shows (west, south, east, north).
    fn visible_region(&self) -> [f64; 4] {
        let r = self.last_flat_rect;
        let c = r.center();
        let mut pts: Vec<LonLat> = Vec::new();
        for i in 0..=12 {
            for j in 0..=12 {
                let p = egui::pos2(r.left() + r.width() * i as f32 / 12.0, r.top() + r.height() * j as f32 / 12.0);
                if let Some(ll) = self.cam2d.screen_to_lonlat([(p.x - c.x) as f64, (p.y - c.y) as f64]) {
                    pts.push(ll);
                }
            }
        }
        let Some(mid) = self.cam2d.screen_to_lonlat([0.0, 0.0]).or(pts.first().copied()) else { return [-180.0, -90.0, 180.0, 90.0] };
        // longitudes as offsets from the middle, so a view across the seam is one stretch
        let (mut w, mut e, mut s, mut n) = (0.0f64, 0.0f64, mid.lat, mid.lat);
        for p in &pts {
            let d = geom::lon_delta(mid.lon, p.lon);
            (w, e, s, n) = (w.min(d), e.max(d), s.min(p.lat), n.max(p.lat));
        }
        if e - w > 350.0 {
            return [-180.0, s.max(-90.0), 180.0, n.min(90.0)];
        }
        [mid.lon + w, s, mid.lon + e, n]
    }

    /// A new local view: the flat view's region, and ten years from the
    /// year shown (the timeline is turned on). It is entered at once.
    pub fn new_focus(&mut self) {
        let bounds = self.visible_region();
        let from = self.doc.year.unwrap_or(self.timeline_last_year);
        if let Some(id) = self.edit("新建局部视图", |d| d.add_focus("", bounds, Some((from, from + 10)))) {
            self.enter_focus(id, false);
            self.set_status(crate::i18n::tr("已新建局部视图：范围是平面图当前的视野，年份可在局部视图窗口中修改"), false);
        }
    }

    /// Enter a local view: both views to its region (when `fit`), the
    /// timeline to its first year.
    pub fn enter_focus(&mut self, id: u32, fit: bool) {
        let Some(f) = self.doc.focus(id).cloned() else { return };
        self.focus = Some(id);
        self.focus_view = None;
        self.focus_play = None;
        if let Some((from, _)) = f.years {
            self.set_year(Some(from));
        }
        if fit {
            self.fit_to_focus(&f);
        }
    }

    pub fn exit_focus(&mut self) {
        self.focus = None;
        self.focus_view = None;
        self.focus_play = None;
    }

    /// Both views to a region: the flat view filled with it (its central
    /// meridian moved when the region crosses the map's edge), the globe
    /// turned to it and brought closer for a small one.
    fn fit_to_focus(&mut self, f: &Focus) {
        let [w, s, e, n] = f.bounds;
        let cam = &mut self.cam2d;
        let crosses = |lon0: f64| geom::wrap_lon(w - lon0) > geom::wrap_lon(e - 1e-9 - lon0) || e - w >= 360.0;
        if e - w < 360.0 && crosses(cam.lon0) {
            cam.lon0 = f.center().lon.round();
        }
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in f.outline(2.0) {
            let q = cam.proj.forward_deg(geom::wrap_lon(p.lon - cam.lon0), p.lat);
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        let (vw, vh) = (self.last_flat_rect.width() as f64, self.last_flat_rect.height() as f64);
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        if bw > 1e-9 && bh > 1e-9 && vw > 1.0 && vh > 1.0 {
            cam.center = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
            cam.zoom = ((vw / bw).min(vh / bh) * 0.9).clamp(10.0, 5.0e7);
        }
        let c = f.center();
        self.globe.center_lon = c.lon;
        self.globe.center_lat = c.lat.clamp(-89.0, 89.0);
        // as far as the farthest corner from the middle
        let reach = [LonLat::new(w, s), LonLat::new(e, s), LonLat::new(w, n), LonLat::new(e, n)].iter().map(|p| geom::geo_dist_deg(c, *p)).fold(0.0, f64::max);
        self.globe.dist = (1.25 + reach.to_radians() * 2.4).clamp(1.3, 3.2);
        self.nav_touched = true;
    }

    /// The export window, set to a local view: its region (in the flat
    /// view's projection and central meridian), its name and years as the
    /// title, its years for a per-year export.
    fn export_focus(&mut self, f: &Focus) {
        let (proj, lon0) = (self.cam2d.proj, self.cam2d.lon0);
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in f.outline(1.0) {
            let q = proj.forward_deg(geom::wrap_lon(p.lon - lon0), p.lat);
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        let x = &mut self.export;
        x.open = true;
        x.current_view = false;
        x.projection = proj;
        x.lon0 = Some(lon0);
        if b[2] > b[0] && b[3] > b[1] {
            x.bounds = Some(b);
            x.height = ((x.width as f64) * (b[3] - b[1]) / (b[2] - b[0])).round().clamp(100.0, 20000.0) as u32;
        }
        x.title = match f.years {
            Some((a, z)) => format!("{}（{a}–{z}）", f.name),
            None => f.name.clone(),
        };
        if let Some((a, z)) = f.years {
            x.years = crate::panels::YearSource::Range;
            x.range = [a, z, 1];
        }
    }

    /// The years a local view steps through, when one with years is active.
    pub fn focus_steps(&self) -> Option<Vec<i32>> {
        let f = self.active_focus()?;
        f.years?;
        Some(steps(f, self.focus_view.as_ref().filter(|v| v.focus.id == f.id).map_or(&[][..], |v| &v.years)))
    }

    /// The local view's years before and after the year shown.
    pub fn focus_neighbours(&self) -> Option<(Option<i32>, Option<i32>)> {
        let steps = self.focus_steps()?;
        let y = self.doc.year?;
        Some((steps.iter().rev().find(|s| **s < y).copied(), steps.iter().find(|s| **s > y).copied()))
    }

    /// Playing a local view: the next of its years every 0.9 s, to its last.
    pub fn focus_play_tick(&mut self, ctx: &egui::Context) {
        let Some(t0) = self.focus_play else { return };
        let now = ctx.input(|i| i.time);
        if now - t0 >= 0.9 {
            match self.focus_neighbours().and_then(|n| n.1) {
                Some(y) => {
                    self.set_year(Some(y));
                    self.focus_play = Some(now);
                }
                None => self.focus_play = None,
            }
        }
        match self.focus_play {
            Some(_) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            // (stopped at the end: show it)
            None => ctx.request_repaint(),
        }
    }

    /// The local views as a menu: enter one; the active one is marked.
    pub fn focus_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(crate::i18n::tr("局部视图"), |ui| {
            if self.doc.focuses.is_empty() {
                ui.weak(crate::i18n::tr("（还没有局部视图：先把平面图移到要看的地区，再“新建局部视图”）"));
                return;
            }
            let mut enter = None;
            for f in &self.doc.focuses {
                let years = f.years.map_or(String::new(), |(a, b)| format!("  {a}–{b}"));
                if ui.selectable_label(self.focus == Some(f.id), format!("{}{years}", f.name)).clicked() {
                    enter = Some(f.id);
                }
            }
            if let Some(id) = enter {
                self.enter_focus(id, true);
                ui.close();
            }
        });
    }

    /// The region's edge on a view (dashed), and its name in the flat view.
    pub fn draw_focus(&self, painter: &egui::Painter, vg: &ViewGeom) {
        let Some(f) = self.active_focus() else { return };
        let jump = (90.0 * vg.px_per_deg()) as f32;
        let mut runs: Vec<Vec<egui::Pos2>> = vec![vec![]];
        for p in f.outline(1.0) {
            match vg.to_screen(p) {
                Some(q) if runs.last().unwrap().last().is_none_or(|l| (q - *l).length() < jump) => runs.last_mut().unwrap().push(q),
                Some(q) => runs.push(vec![q]),
                None => runs.push(vec![]),
            }
        }
        for run in runs.iter().filter(|r| r.len() >= 2) {
            painter.add(egui::Shape::line(run.clone(), Stroke::new(3.5, Color32::from_rgba_unmultiplied(255, 255, 255, 150))));
            painter.extend(egui::Shape::dashed_line(run, Stroke::new(1.8, ACCENT), 9.0, 5.0));
        }
        if vg.kind == ViewKind::Flat {
            let [w, _, _, n] = f.bounds;
            if let Some(at) = vg.to_screen(LonLat::new(geom::wrap_lon(w), n)) {
                let years = f.years.map_or(String::new(), |(a, b)| format!("  {a}–{b}"));
                let g = painter.layout_no_wrap(format!("{}{years}", f.name), egui::FontId::proportional(13.0), Color32::WHITE);
                let r = egui::Rect::from_min_size(at + egui::vec2(0.0, -g.size().y - 6.0), g.size() + egui::vec2(10.0, 4.0));
                let r = r.translate(egui::vec2(0.0, (vg.rect.top() + 2.0 - r.top()).max(0.0)));
                painter.rect_filled(r, 3.0, ACCENT.gamma_multiply(0.9));
                painter.galley(r.min + egui::vec2(5.0, 2.0), g, Color32::WHITE);
            }
        }
    }

    /// The window of the active local view: its name and years, what
    /// changed, playing its years, leaving it.
    pub fn focus_window(&mut self, ctx: &egui::Context) {
        let Some(f) = self.active_focus().cloned() else { return };
        let mut open = true;
        let screen = ctx.content_rect();
        let mut changed: Option<Focus> = None;
        let mut leave = false;
        let mut delete = false;
        egui::Window::new(crate::i18n::tr("局部视图"))

            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .default_width(270.0)
            .default_pos(egui::pos2(screen.right() - 640.0, screen.top() + 90.0))
            .show(ctx, |ui| {
                let mut name = self.focus_name.clone().unwrap_or_else(|| f.name.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0).hint_text(crate::i18n::tr("名称")));
                if r.changed() {
                    self.focus_name = Some(name.clone());
                }
                if r.lost_focus() {
                    if name.trim() != f.name && !name.trim().is_empty() {
                        changed = Some(Focus { name: name.trim().to_string(), ..f.clone() });
                    }
                    self.focus_name = None;
                }
                ui.horizontal(|ui| {
                    let mut timed = f.years.is_some();
                    if ui.checkbox(&mut timed, crate::i18n::tr("年份")).on_hover_text(crate::i18n::tr("不限年份时，只是一个地区")).changed() {
                        let y = self.doc.year.unwrap_or(self.timeline_last_year);
                        changed = Some(Focus { years: timed.then_some((y, y + 10)), ..f.clone() });
                    }
                    if let Some((mut a, mut b)) = f.years {
                        let ra = ui.add(egui::DragValue::new(&mut a).speed(0.2));
                        ui.label(crate::i18n::tr("至"));
                        let rb = ui.add(egui::DragValue::new(&mut b).speed(0.2));
                        if (ra.changed() || rb.changed()) && !ra.dragged() && !rb.dragged() || ra.drag_stopped() || rb.drag_stopped() {
                            changed = Some(Focus { years: Some((a.min(b), a.max(b))), ..f.clone() });
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let [w, s, e, n] = f.bounds;
                    ui.weak(crate::trf!("范围 {:.0}°–{:.0}°E，{:.0}°–{:.0}°N", w, e, s, n)).on_hover_text(crate::i18n::tr("经度超过 180° 的部分在日界线以东"));
                    if ui.small_button(crate::i18n::tr("设为当前视野")).on_hover_text(crate::i18n::tr("用平面图现在显示的地区作为范围")).clicked() {
                        changed = Some(Focus { bounds: self.visible_region(), ..f.clone() });
                    }
                });
                ui.separator();
                let view = self.focus_view.as_ref().filter(|v| v.focus.id == f.id);
                match (f.years, view.and_then(|v| v.report.as_ref())) {
                    (Some(_), Some(rep)) => {
                        let names = |e: Option<map_core::EntityId>| self.entity_name(e);
                        if rep.to == rep.from {
                            ui.label(crate::trf!("{}：起始状态", self.doc.calendar.show(rep.from)));
                        } else {
                            ui.label(RichText::new(crate::trf!("{}至{}的变化", self.doc.calendar.show(rep.from), self.doc.calendar.show(rep.to))).strong());
                        }
                        let moved: f64 = rep.transfers.iter().map(|t| t.2).sum();
                        if rep.transfers.is_empty() && rep.to_land == 0.0 && rep.to_water == 0.0 {
                            ui.weak(if rep.to == rep.from { crate::i18n::tr("红线标出此后易手的土地；拖动时间轴或播放来查看。") } else { crate::i18n::tr("这一地区的归属没有变化。") });
                        } else {
                            ui.label(crate::trf!("易手土地 {} km²（红线标出）", fmt_km2(moved)));
                            egui::Grid::new("focus-areas").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                                for (e, a, b) in rep.areas.iter().take(8) {
                                    ui.label(names(*e));
                                    let d = b - a;
                                    let txt = format!("{}{} km²", if d > 0.0 { "+" } else { "−" }, fmt_km2(d.abs()));
                                    ui.label(RichText::new(txt).color(if d > 0.0 { Color32::from_rgb(40, 150, 70) } else { Color32::from_rgb(200, 60, 50) }));
                                    ui.end_row();
                                }
                            });
                            for (a, b, km2) in rep.transfers.iter().take(5) {
                                ui.weak(format!("{} → {}：{} km²", names(*a), names(*b), fmt_km2(*km2)));
                            }
                            if rep.to_land > 0.0 || rep.to_water > 0.0 {
                                ui.weak(crate::trf!("陆地变水域 {} km²，水域变陆地 {} km²", fmt_km2(rep.to_water), fmt_km2(rep.to_land)));
                            }
                        }
                        let (a, b) = f.years.unwrap_or_default();
                        ui.weak(crate::trf!("{a}–{b} 年间这一地区有 {} 个变化年份", view.map_or(0, |v| v.years.len())));
                    }
                    (Some(_), None) => {
                        ui.weak(crate::i18n::tr("正在计算……"));
                    }
                    (None, _) => {
                        ui.weak(crate::i18n::tr("不限年份：只聚焦地区。勾选“年份”可查看一段时间里的变化。"));
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if f.years.is_some() {
                        let playing = self.focus_play.is_some();
                        if ui.button(if playing { crate::i18n::tr("暂停") } else { crate::i18n::tr("播放") }).on_hover_text(crate::i18n::tr("从当前年份起，逐个变化年份播放到最后一年")).clicked() {
                            if playing {
                                self.focus_play = None;
                            } else {
                                self.stop_timeline_playback();
                                if self.focus_neighbours().and_then(|n| n.1).is_none() {
                                    // at the end: from the start again
                                    if let Some((a, _)) = f.years {
                                        self.set_year(Some(a));
                                    }
                                }
                                self.focus_play = Some(ui.input(|i| i.time));
                            }
                        }
                    }
                    if ui.button(crate::i18n::tr("导出…")).on_hover_text(crate::i18n::tr("打开导出窗口，范围、标题与逐年导出的年份设为这个局部视图")).clicked() {
                        self.export_focus(&f);
                    }
                    if ui.button(crate::i18n::tr("时间播放与视频")).clicked() {
                        self.open_playback(true);
                    }
                    if ui.button(crate::i18n::tr("回到范围")).on_hover_text(crate::i18n::tr("两个视图移回这个地区")).clicked() {
                        self.fit_to_focus(&f);
                    }
                    if ui.button(crate::i18n::tr("删除")).on_hover_text(crate::i18n::tr("删除这个局部视图（地图本身不变，可撤销）")).clicked() {
                        delete = true;
                    }
                    if ui.button(crate::i18n::tr("退出")).clicked() {
                        leave = true;
                    }
                });
            });
        if let Some(nf) = changed {
            let years_moved = nf.years != f.years;
            self.edit("修改局部视图", move |d| d.update_focus(nf));
            if years_moved {
                if let Some((a, b)) = self.active_focus().and_then(|x| x.years) {
                    let y = self.doc.year.unwrap_or(a).clamp(a, b);
                    self.set_year(Some(y));
                }
            }
        }
        if delete {
            let id = f.id;
            self.edit("删除局部视图", move |d| d.remove_focus(id));
            self.exit_focus();
        } else if leave || !open {
            self.exit_focus();
        }
    }

    /// Face colours with the land outside the active local view faded.
    pub fn focus_colors(&self, colors: &std::collections::HashMap<FaceId, [u8; 4]>) -> Option<std::collections::HashMap<FaceId, [u8; 4]>> {
        let v = self.focus_view.as_ref()?;
        Some(
            colors
                .iter()
                .map(|(f, c)| {
                    if v.inside.contains(f) {
                        (*f, *c)
                    } else {
                        let fade = |x: u8| (x as f32 * 0.35 + 205.0 * 0.65) as u8;
                        (*f, [fade(c[0]), fade(c[1]), fade(c[2]), c[3]])
                    }
                })
                .collect(),
        )
    }
}

/// Square kilometres, in thousands with separators.
fn fmt_km2(v: f64) -> String {
    let n = v.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}
