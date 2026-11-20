//! Reshaping a piece's borders (smooth / natural) with a preview: the new
//! borders are shown on the map with the area change per country, and
//! nothing changes until the user applies it. Border geometry is shared by
//! every year, so the summary covers all year views, not just this one.

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, RichText, Stroke};
use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::shape::LineStyle;
use map_core::topo::{EdgeId, FaceId};
use map_core::EntityId;
use std::collections::{BTreeSet, HashMap};

pub struct Reshape {
    pub face: FaceId,
    pub style: LineStyle,
    pub amount: f64,
    pub min_len: f64,
    /// History generation it was computed for; any other edit discards it.
    pub generation: u64,
    pub keep_area: bool,
    pub result: Result<Candidate, String>,
}

pub struct Candidate {
    pub topo: map_core::topo::Topology,
    /// Changed edges: old and new full lines.
    pub edges: Vec<(Vec<LonLat>, Vec<LonLat>)>,
    /// Per entity in the viewed year: area before and after (km²).
    pub now: Vec<(Option<EntityId>, f64, f64)>,
    /// Largest relative area change of any country over all year views.
    pub worst: f64,
    /// Number of year views that share this geometry.
    pub views: usize,
}

impl Reshape {
    pub fn label(&self) -> &'static str {
        if self.style == LineStyle::Smooth { "平滑边界" } else { "自然化边界" }
    }
}

fn compute(doc: &Document, face: FaceId, style: LineStyle, amount: f64, min_len: f64, keep_area: bool) -> Result<Candidate, String> {
    let mut d = doc.clone();
    let n = d.restyle_face(face, style, amount, min_len, keep_area)?;
    if n == 0 {
        return Err("边界没有变化（已经足够平滑，或细节小于当前缩放精度）".into());
    }
    let mut edges = Vec::new();
    let mut faces: BTreeSet<FaceId> = BTreeSet::new();
    let changed: Vec<EdgeId> = d.topo.edges.iter().filter(|(id, e)| doc.topo.edges.get(*id).is_none_or(|o| o.pts != e.pts)).map(|(id, _)| *id).collect();
    for id in changed {
        let (o, e) = (&doc.topo.edges[&id], &d.topo.edges[&id]);
        edges.push((doc.topo.edge_full(o), d.topo.edge_full(e)));
        faces.extend([e.left, e.right]);
    }
    let r = doc.radius();
    // area change per touched face is the same in every year; who owns it is not
    let delta: HashMap<FaceId, (f64, f64)> = faces.iter().filter(|f| **f != map_core::topo::BACKGROUND).map(|f| (*f, (doc.topo.face_area(*f, r), d.topo.face_area(*f, r)))).collect();
    let all: HashMap<FaceId, f64> = doc.topo.faces.keys().filter(|f| **f != map_core::topo::BACKGROUND).map(|f| (*f, delta.get(f).map(|x| x.0).unwrap_or_else(|| doc.topo.face_area(*f, r)))).collect();
    let per_year = |y: Option<i32>| -> HashMap<Option<EntityId>, (f64, f64)> {
        let mut out: HashMap<Option<EntityId>, (f64, f64)> = HashMap::new();
        for (f, (a, b)) in &delta {
            let o = doc.owner_at(*f, y);
            let x = out.entry(o).or_default();
            x.1 += b - a;
        }
        for (f, a) in &all {
            let o = doc.owner_at(*f, y);
            if let Some(x) = out.get_mut(&o) {
                x.0 += a;
            }
        }
        out.into_iter().map(|(k, (tot, dd))| (k, (tot, tot + dd))).collect()
    };
    let mut years: Vec<Option<i32>> = vec![None];
    years.extend(doc.change_years().into_iter().map(Some));
    let mut worst: f64 = 0.0;
    for y in &years {
        for (e, (a, b)) in per_year(*y) {
            if e.is_some() && a > 0.0 {
                worst = worst.max((b - a).abs() / a);
            }
        }
    }
    let mut now: Vec<(Option<EntityId>, f64, f64)> = per_year(doc.year).into_iter().map(|(e, (a, b))| (e, a, b)).collect();
    now.sort_by(|x, y| (y.2 - y.1).abs().total_cmp(&(x.2 - x.1).abs()));
    Ok(Candidate { topo: d.topo, edges, now, worst, views: years.len() })
}

impl App {
    pub fn start_reshape(&mut self, face: FaceId, style: LineStyle) {
        let min_len = 4.0 * 57.29578 / self.cam2d.zoom.max(1e-9);
        let amount = self.roughness as f64;
        let keep_area = self.keep_area;
        let result = compute(&self.doc, face, style, amount, min_len, keep_area);
        self.reshape = Some(Reshape { face, style, amount, min_len, generation: self.history.generation(), keep_area, result });
    }

    /// The preview window: summary, keep-area switch, apply / cancel.
    pub fn reshape_window(&mut self, ctx: &egui::Context) {
        let Some(rs) = &self.reshape else { return };
        if rs.generation != self.history.generation() || !self.doc.topo.faces.contains_key(&rs.face) {
            self.reshape = None;
            return;
        }
        let (mut apply, mut cancel, mut redo) = (false, false, false);
        let mut keep = rs.keep_area;
        let mut amount = rs.amount as f32;
        let label = rs.label();
        egui::Window::new(crate::trf!("预览：{label}")).collapsible(false).resizable(false).default_width(330.0).show(ctx, |ui| {
            match &rs.result {
                Err(e) => {
                    ui.colored_label(Color32::from_rgb(190, 60, 20), e);
                }
                Ok(c) => {
                    ui.label(crate::trf!("将改变 {} 条边界（橙色为新边界，灰色为原边界）。", c.edges.len()));
                    if c.views > 1 {
                        ui.label(RichText::new(crate::trf!("边界形状是所有年份共用的：这会同时改变 {} 个年份视图中的地图。", c.views)).color(Color32::from_rgb(190, 90, 10)));
                    }
                    ui.separator();
                    let when = self.doc.year.map(|y| format!("{y} 年")).unwrap_or_else(|| "当前（无年份）".into());
                    ui.label(RichText::new(crate::trf!("面积变化（{when}）")).strong());
                    egui::Grid::new("reshape_rows").striped(true).show(ui, |ui| {
                        for (e, a, b) in c.now.iter().take(8) {
                            let name = match e {
                                Some(e) => self.entity_name(Some(*e)),
                                None => "无归属".into(),
                            };
                            ui.label(name);
                            let d = b - a;
                            ui.label(format!("{:+.1} km²", d));
                            ui.label(if *a > 0.0 { format!("{:+.4}%", d / a * 100.0) } else { String::new() });
                            ui.end_row();
                        }
                    });
                    ui.label(crate::trf!("所有年份中单个国家的最大面积变化：{:.4}%", c.worst * 100.0));
                }
            }
            ui.separator();
            if ui.checkbox(&mut keep, crate::i18n::tr("保持面积")).on_hover_text(crate::i18n::tr("修正每条边界，使两侧面积不变：所有国家在所有年份的面积都保持，只改变形状")).changed() {
                redo = true;
            }
            if rs.style == LineStyle::Natural && ui.add(egui::Slider::new(&mut amount, 0.05..=1.0).text(crate::i18n::tr("粗糙度"))).drag_stopped() {
                redo = true;
            }
            ui.horizontal(|ui| {
                apply = ui.add_enabled(rs.result.is_ok(), egui::Button::new(crate::i18n::tr("应用"))).clicked();
                cancel = ui.button(crate::i18n::tr("取消")).clicked();
            });
        });
        if cancel {
            self.reshape = None;
            self.set_status(crate::trf!("{label}：已取消"), false);
        } else if apply {
            let Some(rs) = self.reshape.take() else { return };
            let (Ok(c), keep) = (rs.result, rs.keep_area) else { return };
            let n = c.edges.len();
            let topo = c.topo;
            if self.edit(label, move |d| {
                d.topo = topo;
                Ok(())
            }).is_some() {
                let how = if keep { "，面积保持" } else { "" };
                self.set_status(crate::trf!("{label}：改变了 {n} 条边{how}"), false);
            }
        } else if redo {
            self.keep_area = keep;
            self.roughness = amount;
            // same detail as when it was opened, whatever the zoom is now
            let (face, style, min_len) = (rs.face, rs.style, rs.min_len);
            let result = compute(&self.doc, face, style, amount as f64, min_len, keep);
            self.reshape = Some(Reshape { face, style, amount: amount as f64, min_len, generation: self.history.generation(), keep_area: keep, result });
        }
    }

    /// Old borders in grey, new ones in orange.
    pub fn draw_reshape(&self, painter: &egui::Painter, vg: &ViewGeom) {
        let Some(Ok(c)) = self.reshape.as_ref().map(|r| &r.result) else { return };
        let line = |pts: &[LonLat], stroke: Stroke| {
            let s: Vec<_> = pts.iter().map(|p| vg.to_screen(*p)).collect();
            for w in s.windows(2) {
                if let (Some(a), Some(b)) = (w[0], w[1]) {
                    if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                        painter.line_segment([a, b], stroke);
                    }
                }
            }
        };
        for (old, _) in &c.edges {
            line(old, Stroke::new(1.0, Color32::from_rgb(110, 110, 110)));
        }
        for (_, new) in &c.edges {
            line(new, Stroke::new(2.2, Color32::from_rgb(235, 110, 20)));
        }
    }
}
