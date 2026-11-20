//! The name generator in the interface (`map_core::names`): a window that
//! offers a dozen names of a kind in a style — click one to give it to the
//! selected country or city (or to copy it) — and a "随机" button beside a
//! country's or city's name that renames it at once.

use crate::app::App;
use eframe::egui;
use map_core::names::{self, NameKind, NameStyle};
use map_core::{EntityId, EntityKind};
use std::collections::HashSet;

impl App {
    /// Names already used by countries and cities.
    fn names_taken(&self) -> HashSet<String> {
        self.doc.entities.values().map(|e| e.name.clone()).collect()
    }

    /// Rename a country or city with a new name in the chosen style.
    pub fn random_name(&mut self, e: EntityId) {
        let Some(ent) = self.doc.entities.get(&e) else { return };
        let kind = if ent.kind == EntityKind::City { NameKind::City } else { NameKind::Country };
        self.name_seed = self.name_seed.wrapping_add(1);
        // its own style, or that of the culture it lies in, before the window's
        let style = self.doc.name_style_for(e).unwrap_or(self.name_style);
        let Some(name) = names::unique(style, kind, 1, self.name_seed, &self.names_taken()).pop() else { return };
        self.edit("随机名称", move |d| {
            d.entities.get_mut(&e).ok_or("实体不存在")?.name = name;
            Ok(())
        });
        self.text_edit.retain(|(x, _), _| *x != e);
    }

    pub fn names_window(&mut self, ctx: &egui::Context) {
        if !self.names_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        egui::Window::new(crate::i18n::tr("名称生成器")).open(&mut open).resizable(false).default_width(330.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("风格"));
                egui::ComboBox::from_id_salt("name-style").selected_text(self.name_style.name()).show_ui(ui, |ui| {
                    for s in NameStyle::ALL {
                        ui.selectable_value(&mut self.name_style, s, s.name());
                    }
                });
                ui.label(crate::i18n::tr("种类"));
                egui::ComboBox::from_id_salt("name-kind").selected_text(self.name_kind.name()).show_ui(ui, |ui| {
                    for k in NameKind::ALL {
                        ui.selectable_value(&mut self.name_kind, k, k.name());
                    }
                });
                if ui.button(crate::i18n::tr("换一批")).clicked() {
                    self.name_seed = self.name_seed.wrapping_add(1);
                }
            });
            let target = self.selected.and_then(|e| self.doc.entities.get(&e)).filter(|x| matches!((x.kind, self.name_kind), (EntityKind::Country, NameKind::Country) | (EntityKind::City, NameKind::City)));
            ui.weak(match target {
                Some(x) => crate::trf!("单击一个名称，把“{}”改为它（可撤销）", x.name),
                None => "单击一个名称复制它；选中国家或城市并选对应种类时，单击即改名".to_string(),
            });
            if let Some(s) = self.selected.and_then(|e| self.doc.name_style_for(e)).filter(|s| *s != self.name_style) {
                ui.horizontal(|ui| {
                    ui.weak(crate::trf!("所选单位的名称风格是“{}”", s.name()));
                    if ui.small_button(crate::i18n::tr("改用它")).clicked() {
                        self.name_style = s;
                    }
                });
            }
            let list = names::unique(self.name_style, self.name_kind, 12, self.name_seed, &self.names_taken());
            egui::Grid::new("name-list").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                for (i, n) in list.iter().enumerate() {
                    if ui.button(n).clicked() {
                        chosen = Some(n.clone());
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        });
        if let Some(n) = chosen {
            let target = self.selected.filter(|e| self.doc.entities.get(e).is_some_and(|x| matches!((x.kind, self.name_kind), (EntityKind::Country, NameKind::Country) | (EntityKind::City, NameKind::City))));
            match target {
                Some(e) => {
                    let name = n.clone();
                    self.edit("改名", move |d| {
                        d.entities.get_mut(&e).ok_or("实体不存在")?.name = name;
                        Ok(())
                    });
                    self.text_edit.retain(|(x, _), _| *x != e);
                }
                None => {
                    ctx.copy_text(n.clone());
                    self.set_status(crate::trf!("已复制“{n}”"), false);
                }
            }
        }
        if !open {
            self.names_open = false;
        }
    }
}
