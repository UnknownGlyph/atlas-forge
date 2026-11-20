//! "Select by attribute" (按属性选择): the drawn features of some kinds,
//! whose name or legend text contains a word, in a layer, following a
//! style, shown in the selected year, hidden or not — selected in one go,
//! or added to what is selected. The query itself is
//! `map_core::features::FeatureQuery`.

use crate::app::App;
use eframe::egui;
use map_core::features::{FeatureQuery, FeatureRef};

const KINDS: [&str; 5] = ["地图文字", "符号", "线路", "河流", "斜线区域"];

impl App {
    pub fn query_window(&mut self, ctx: &egui::Context) {
        let Some(mut q) = self.query.take() else { return };
        let mut open = true;
        let mut pick: Option<bool> = None;
        let found = self.doc.query_features(&q);
        let pickable: Vec<FeatureRef> = found.iter().copied().filter(|f| self.doc.pickable(*f)).collect();
        egui::Window::new(crate::i18n::tr("按属性选择")).open(&mut open).resizable(false).default_width(300.0).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::tr("种类"));
                for (i, k) in KINDS.iter().enumerate() {
                    ui.checkbox(&mut q.kinds[i], *k);
                }
            });
            ui.weak(crate::i18n::tr("一个都不勾选即所有种类"));
            egui::Grid::new("query-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(crate::i18n::tr("名称或图例"));
                ui.add(egui::TextEdit::singleline(&mut q.text).hint_text(crate::i18n::tr("包含的文字（留空不限）")).desired_width(190.0));
                ui.end_row();
                ui.label(crate::i18n::tr("图层"));
                let group_name = |g: Option<Option<map_core::stack::GroupId>>| match g {
                    None => "不限".to_string(),
                    Some(None) => "不在任何图层".to_string(),
                    Some(Some(id)) => self.doc.groups.iter().find(|x| x.id == id).map_or("（已删除）".into(), |x| x.name.clone()),
                };
                egui::ComboBox::from_id_salt("query-group").selected_text(group_name(q.group)).width(180.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.group, None, crate::i18n::tr("不限"));
                    ui.selectable_value(&mut q.group, Some(None), crate::i18n::tr("不在任何图层"));
                    for g in self.doc.groups.iter().rev() {
                        ui.selectable_value(&mut q.group, Some(Some(g.id)), &g.name);
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("样式"));
                let style_name = |s: Option<Option<map_core::styles::StyleId>>| match s {
                    None => "不限".to_string(),
                    Some(None) => "未使用样式".to_string(),
                    Some(Some(id)) => self.doc.style(id).map_or("（已删除）".into(), |x| x.name.clone()),
                };
                egui::ComboBox::from_id_salt("query-style").selected_text(style_name(q.style)).width(180.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.style, None, crate::i18n::tr("不限"));
                    ui.selectable_value(&mut q.style, Some(None), crate::i18n::tr("未使用样式"));
                    for s in &self.doc.styles {
                        ui.selectable_value(&mut q.style, Some(Some(s.id)), &s.name);
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("年份"));
                let mut in_year = q.year.is_some();
                let label = match self.doc.year {
                    Some(y) => format!("只要 {y} 年显示的（符号、区域有年份）"),
                    None => "只要所选年份显示的（先开启时间轴）".to_string(),
                };
                if ui.add_enabled(self.doc.year.is_some(), egui::Checkbox::new(&mut in_year, label)).changed() {
                    q.year = if in_year { self.doc.year } else { None };
                }
                if q.year.is_some() && self.doc.year.is_some() {
                    q.year = self.doc.year;
                }
                ui.end_row();
                ui.label(crate::i18n::tr("显示"));
                let shown = |h: Option<bool>| match h {
                    None => "不限",
                    Some(false) => "显示中的",
                    Some(true) => "隐藏的",
                };
                egui::ComboBox::from_id_salt("query-hidden").selected_text(shown(q.hidden)).width(180.0).show_ui(ui, |ui| {
                    for h in [None, Some(false), Some(true)] {
                        ui.selectable_value(&mut q.hidden, h, shown(h));
                    }
                });
                ui.end_row();
            });
            ui.separator();
            let locked = found.len() - pickable.len();
            ui.label(crate::trf!("符合 {} 个要素{}", found.len(), if locked > 0 { format!("（其中 {locked} 个隐藏、锁定或在锁定图层中，不能选中）") } else { String::new() }));
            ui.horizontal(|ui| {
                if ui.add_enabled(!pickable.is_empty(), egui::Button::new(crate::i18n::tr("选择"))).clicked() {
                    pick = Some(false);
                }
                if ui.add_enabled(!pickable.is_empty(), egui::Button::new(crate::i18n::tr("加入选择"))).clicked() {
                    pick = Some(true);
                }
                if ui.button(crate::i18n::tr("重置条件")).clicked() {
                    q = FeatureQuery::default();
                }
            });
        });
        if let Some(add) = pick {
            let mut all = pickable.clone();
            if add {
                all.extend(self.picked.iter().copied());
                all.sort();
                all.dedup();
            }
            let n = all.len();
            self.set_selection(all);
            self.set_status(crate::trf!("已选择 {n} 个要素"), false);
        }
        if open {
            self.query = Some(q);
        }
    }
}
