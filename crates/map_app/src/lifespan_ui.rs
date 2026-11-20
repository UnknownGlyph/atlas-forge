//! Lifespans in the countries tab: when an entity exists, what came after
//! it, its state in the selected year, and what to confirm when land or
//! members are still recorded to an entity that does not exist then.

use crate::app::App;
use eframe::egui::{self, RichText};
use map_core::lifespan::Presence;
use map_core::{EntityId, EntityKind};

impl App {
    /// A short note on an entity's state in the selected year, for lists:
    /// not founded yet, ended, or existing without land.
    pub fn presence_note(&self, e: EntityId, area: Option<f64>) -> Option<&'static str> {
        let x = self.doc.entities.get(&e)?;
        match x.presence(self.doc.year) {
            Presence::NotYet => Some("尚未成立"),
            Presence::Ended => Some("已终止"),
            Presence::Exists if self.doc.year.is_some() && x.kind == EntityKind::Country && area.is_none_or(|a| a <= 0.0) => Some("无领土"),
            Presence::Exists => None,
        }
    }

    /// The selected entity's lifespan and successors.
    pub fn lifespan_panel(&mut self, ui: &mut egui::Ui, sel: EntityId) {
        let Some(ent) = self.doc.entities.get(&sel).cloned() else { return };
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("存续")).strong());
        let near = self.doc.year.unwrap_or(1900);
        let (mut has_f, mut has_t) = (ent.founded.is_some(), ent.ended.is_some());
        let (mut f, mut t) = (ent.founded.unwrap_or(near), ent.ended.unwrap_or(near));
        let rf = ui
            .horizontal(|ui| {
                ui.checkbox(&mut has_f, crate::i18n::tr("成立于"));
                ui.add_enabled(has_f, egui::DragValue::new(&mut f).speed(0.5).suffix(crate::i18n::tr(" 年")))
            })
            .inner;
        let rt = ui
            .horizontal(|ui| {
                ui.checkbox(&mut has_t, crate::i18n::tr("终止于"));
                ui.add_enabled(has_t, egui::DragValue::new(&mut t).speed(0.5).suffix(crate::i18n::tr(" 年")))
            })
            .inner;
        let (founded, ended) = (has_f.then_some(f), has_t.then_some(if has_f { t.max(f) } else { t }));
        if rf.drag_started() || rt.drag_started() {
            self.begin_pending("修改存续年份");
        }
        if (founded, ended) != (ent.founded, ent.ended) {
            if self.pending.is_some() {
                let _ = self.doc.set_lifespan(sel, founded, ended);
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            } else {
                self.edit("修改存续年份", |d| d.set_lifespan(sel, founded, ended));
            }
        }
        if rf.drag_stopped() || rt.drag_stopped() {
            self.end_pending();
        }
        let text = self.doc.entities[&sel].lifespan_text();
        ui.weak(if text.is_empty() {
            "未填写年份：不受年份限制（视为始终存续）。".to_string()
        } else {
            crate::trf!("存续：{text}（含首尾两年）。选定年份时，不存续的实体没有领土与下属，记录保持不变。")
        });
        // what came after it, and before it
        let layer = ent.layer;
        let name = |d: &map_core::Document, e: EntityId| d.entities.get(&e).map_or(String::new(), |x| x.name.clone());
        let mut list = ent.successors.clone();
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::tr("后继"));
            for s in ent.successors.iter() {
                if ui.small_button(format!("{} ×", name(&self.doc, *s))).on_hover_text(crate::i18n::tr("移除这个后继")).clicked() {
                    list.retain(|x| x != s);
                    changed = true;
                }
            }
            let options: Vec<(EntityId, String)> = self
                .doc
                .entities
                .iter()
                .filter(|(id, x)| **id != sel && x.layer == layer && x.kind != EntityKind::City && !ent.successors.contains(id))
                .map(|(id, x)| (*id, x.name.clone()))
                .collect();
            egui::ComboBox::from_id_salt(("successor", sel.0)).selected_text(crate::i18n::tr("添加…")).show_ui(ui, |ui| {
                for (id, n) in options {
                    if ui.selectable_label(false, n).clicked() {
                        list.push(id);
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.edit("修改后继", |d| {
                d.successors_set(sel, list);
                Ok(())
            });
        }
        let pre = self.doc.predecessors(sel);
        if !pre.is_empty() {
            ui.weak(crate::trf!("前身：{}", pre.iter().map(|p| name(&self.doc, *p)).collect::<Vec<_>>().join("、")));
        }
    }

    /// Land and members recorded, in the selected year, to an entity that
    /// does not exist then; with the fixes for an entity that has ended.
    pub fn lifespan_conflicts_section(&mut self, ui: &mut egui::Ui) {
        let Some(year) = self.doc.year else { return };
        let conflicts = self.doc.lifespan_conflicts(year);
        if conflicts.is_empty() {
            return;
        }
        ui.separator();
        ui.label(RichText::new(crate::trf!("需确认（{year} 年）")).strong().color(egui::Color32::from_rgb(220, 150, 40)));
        ui.weak(crate::i18n::tr("下列土地或下属在记录中属于该年不存续的实体，因此显示为未分配或无上级；记录本身没有被改动。"));
        let mut action: Option<(EntityId, Option<EntityId>, String)> = None;
        let mut pick: Option<EntityId> = None;
        for c in &conflicts {
            let Some(x) = self.doc.entities.get(&c.entity) else { continue };
            let state = match c.presence {
                Presence::Ended => format!("{} 年已终止", x.ended.unwrap_or_default()),
                Presence::NotYet => format!("{} 年才成立", x.founded.unwrap_or_default()),
                Presence::Exists => String::new(),
            };
            let mut what = Vec::new();
            if !c.faces.is_empty() {
                what.push(format!("{} 块土地", c.faces.len()));
            }
            if !c.members.is_empty() {
                what.push(format!("下属 {}", c.members.iter().filter_map(|m| self.doc.entities.get(m).map(|x| x.name.clone())).collect::<Vec<_>>().join("、")));
            }
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(self.selected == Some(c.entity), RichText::new(&x.name).strong()).clicked() {
                    pick = Some(c.entity);
                }
                ui.label(crate::trf!("（{state}）仍记录有 {}", what.join("，")));
            });
            if c.presence == Presence::Ended && !c.faces.is_empty() {
                let from = x.ended.unwrap_or_default() + 1;
                ui.horizontal_wrapped(|ui| {
                    for s in &x.successors {
                        let Some(sx) = self.doc.entities.get(s) else { continue };
                        if sx.exists_in(Some(from)) && ui.small_button(crate::trf!("从 {from} 年起转给 {}", sx.name)).clicked() {
                            action = Some((c.entity, Some(*s), format!("转给 {}", sx.name)));
                        }
                    }
                    if ui.small_button(crate::trf!("从 {from} 年起设为未分配")).clicked() {
                        action = Some((c.entity, None, "设为未分配".into()));
                    }
                });
            } else if c.presence == Presence::NotYet {
                ui.weak(crate::i18n::tr("请检查成立年份，或把这些土地在成立之前改给其他国家。"));
            }
        }
        if let Some(e) = pick {
            self.selected = Some(e);
            self.selected_face = None;
        }
        if let Some((e, to, what)) = action {
            if let Some(n) = self.edit("处理终止实体的土地", |d| d.reassign_after_end(e, to)) {
                self.set_status(crate::trf!("已把 {n} 块土地{what}（只改变终止之后的年份，可撤销）"), false);
            }
        }
    }
}
