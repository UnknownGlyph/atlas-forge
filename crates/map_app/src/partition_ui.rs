//! Automatic partition in the interface (`map_core::partition`): a window
//! for the selected country — how many units, new countries or its
//! provinces, even sizes, borders along rivers and ridges, the names'
//! style, a seed for another plan — with the plan drawn on the map
//! (dashed outlines and each unit's area) until it is applied (one undo
//! step, from the selected year on) or dropped.

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, Stroke};
use map_core::names::NameStyle;
use map_core::partition::{PartitionSpec, Plan};
use map_core::EntityId;

/// The window's settings and the plan shown.
#[derive(Default)]
pub struct Partition {
    pub target: Option<EntityId>,
    pub spec: PartitionSpec,
    pub plan: Option<(PartitionSpec, Result<Plan, String>)>,
    source: Option<(u64, u64, Option<i32>, map_core::doc::LayerId)>,
    pub adopt: Option<(EntityId, String, std::collections::BTreeSet<EntityId>)>,
}

const PLAN: Color32 = Color32::from_rgb(20, 120, 200);

impl App {
    pub fn adoption_window(&mut self, ctx: &egui::Context) {
        let Some((parent, filter, selected)) = self.partition.adopt.as_mut() else { return; };
        let parent = *parent;
        let mut open = true;
        let mut apply = false;
        egui::Window::new("批量设置为下级").open(&mut open).default_width(430.0).show(ctx, |ui| {
            ui.label(format!("上级 · {}", self.doc.entities.get(&parent).map_or("已删除", |e| e.name.as_str())));
            ui.add(egui::TextEdit::singleline(filter).hint_text("搜索名称或编号（例如 YY-）"));
            let needle = filter.trim().to_lowercase();
            let candidates: Vec<_> = self.doc.entities.iter().filter(|(id, e)| {
                **id != parent && e.kind == map_core::EntityKind::Country && e.layer == self.doc.active_layer
                    && e.parent != Some(parent) && e.exists_in(self.doc.year)
                    && (e.name.to_lowercase().contains(&needle) || e.code.to_lowercase().contains(&needle))
            }).collect();
            ui.horizontal(|ui| {
                if ui.button("全选筛选结果").clicked() {
                    selected.extend(candidates.iter().map(|(id,_)| **id));
                }
                if ui.button("清空选择").clicked() { selected.clear(); }
            });
            egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
                for (id, unit) in candidates {
                    let mut marked = selected.contains(id);
                    if ui.checkbox(&mut marked,format!("{}  ·  {}",unit.name,unit.code)).changed() {
                        if marked { selected.insert(*id); } else { selected.remove(id); }
                    }
                }
            });
            ui.label(format!("已选 {} 个单位",selected.len()));
            if ui.add_enabled(!selected.is_empty(),egui::Button::new("应用上级")).clicked() { apply = true; }
        });
        if apply {
            let ids = selected.clone();
            if self.edit("批量设置为下级", |doc| {
                for id in ids { doc.record_parent(id, Some(parent))?; }
                Ok(())
            }).is_some() { open = false; }
        }
        if !open { self.partition.adopt = None; }
    }

    pub fn open_partition(&mut self, e: EntityId) {
        self.partition.target = Some(e);
        self.partition.plan = None;
        // the new units are named in the country's style (its own, or its culture's)
        if let Some(s) = self.doc.name_style_for(e) {
            self.partition.spec.style = s;
        }
    }

    pub fn partition_window(&mut self, ctx: &egui::Context) {
        self.adoption_window(ctx);
        let Some(e) = self.partition.target.filter(|e| self.doc.entities.contains_key(e)) else {
            self.partition.target = None;
            return;
        };
        // the plan follows the settings (worked out again when they change)
        let source = (self.history.generation(), self.scene.geometry_gen, self.doc.year, self.doc.active_layer);
        let stale = self.partition.source != Some(source)
            || self.partition.plan.as_ref().is_none_or(|(s, _)| *s != self.partition.spec);
        if stale {
            let spec = self.partition.spec.clone();
            let plan = self.doc.partition_plan(e, &spec);
            self.partition.plan = Some((spec, plan));
            self.partition.source = Some(source);
        }
        let name = self.doc.entities[&e].name.clone();
        let mut open = true;
        let mut apply = false;
        egui::Window::new("自动划分").open(&mut open).resizable(false).default_width(300.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(&name).strong());
            let s = &mut self.partition.spec;
            egui::Grid::new("partition-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("单位数");
                ui.add(egui::Slider::new(&mut s.units, 2..=40));
                ui.end_row();
                ui.label("划为");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.provinces, false, "新国家").on_hover_text("原国家保留其中一块，其余成为新的国家");
                    ui.radio_value(&mut s.provinces, true, "下级地区（省）").on_hover_text("全部成为这个国家的下级；政治地图上仍是一个国家，内部画出省界");
                });
                ui.end_row();
                ui.label("面积");
                ui.checkbox(&mut s.balance, "大致均衡");
                ui.end_row();
                ui.label("边界");
                ui.checkbox(&mut s.follow, "尽量沿河流与山脊").on_hover_text("有河流或地形时，跨越它们的代价更高");
                ui.end_row();
                ui.label("名称风格");
                egui::ComboBox::from_id_salt("partition-style").selected_text(s.style.name()).show_ui(ui, |ui| {
                    for st in NameStyle::ALL {
                        ui.selectable_value(&mut s.style, st, st.name());
                    }
                });
                ui.end_row();
                ui.label("方案");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.seed).speed(1.0));
                    if ui.small_button("换一个方案").clicked() {
                        s.seed = s.seed.wrapping_add(1);
                    }
                });
                ui.end_row();
            });
            ui.separator();
            match self.partition.plan.as_ref().map(|p| &p.1) {
                Some(Ok(plan)) => {
                    let total: f64 = plan.areas.iter().sum();
                    let (lo, hi) = plan.areas.iter().fold((f64::MAX, 0.0f64), |(a, b), x| (a.min(*x), b.max(*x)));
                    ui.label(format!("{} 块，面积 {:.0}–{:.0} 万 km²（共 {:.0} 万 km²）", plan.units.len(), lo / 1e4, hi / 1e4, total / 1e4));
                    ui.weak(match self.doc.year {
                        Some(y) => format!("地图上蓝色虚线为预览；应用后从 {} 起生效，更早的年份不变。", self.doc.calendar.show(y)),
                        None => "地图上蓝色虚线为预览；应用对所有年份生效（已有历史的地块需先在时间轴选择年份）。".to_string(),
                    });
                    if ui.button("应用").clicked() {
                        apply = true;
                    }
                }
                Some(Err(err)) => {
                    ui.colored_label(Color32::from_rgb(220, 90, 60), err);
                }
                None => {}
            }
        });
        if apply {
            if let Some((spec, Ok(plan))) = self.partition.plan.take() {
                let n = plan.units.len();
                if let Some(units) = self.edit(&format!("划分 {name}"), |d| d.apply_partition(&plan, &spec)) {
                    self.set_status(format!("已把{name}划分为 {n} 块：{}", units.iter().filter_map(|u| self.doc.entities.get(u)).map(|x| x.name.clone()).collect::<Vec<_>>().join("、")), false);
                    self.partition.target = None;
                    return;
                }
            }
        }
        if !open {
            self.partition.target = None;
            self.partition.plan = None;
        }
    }

    /// The plan on a view: each unit's outline dashed.
    pub fn draw_partition(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if self.partition.target.is_none() {
            return;
        }
        let Some((_, Ok(plan))) = &self.partition.plan else { return };
        let jump = (90.0 * vg.px_per_deg()) as f32;
        for unit in &plan.units {
            for ring in unit.iter().flat_map(|part| std::iter::once(&part.outer).chain(&part.holes)) {
                let mut run: Vec<egui::Pos2> = Vec::new();
                for p in ring.iter().chain(ring.first()) {
                    match vg.to_screen(map_core::LonLat::new(map_core::geom::wrap_lon(p.lon), p.lat)) {
                        Some(q) if run.last().is_none_or(|l| (q - *l).length() < jump) => run.push(q),
                        q => {
                            if run.len() >= 2 {
                                painter.add(egui::Shape::line(run.clone(), Stroke::new(3.0, Color32::from_rgba_unmultiplied(255, 255, 255, 160))));
                                painter.extend(egui::Shape::dashed_line(&run, Stroke::new(1.6, PLAN), 7.0, 4.0));
                            }
                            run = q.into_iter().collect();
                        }
                    }
                }
                if run.len() >= 2 {
                    painter.add(egui::Shape::line(run.clone(), Stroke::new(3.0, Color32::from_rgba_unmultiplied(255, 255, 255, 160))));
                    painter.extend(egui::Shape::dashed_line(&run, Stroke::new(1.6, PLAN), 7.0, 4.0));
                }
            }
        }
    }
}
