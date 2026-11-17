//! Fonts in the interface: the fonts a map uses are loaded into egui as
//! families of their own (with the Chinese fallback after them, so a
//! character a font lacks still shows), and a picker lists the system's
//! fonts by name (Chinese fonts first, with their Chinese names). The list
//! itself is `map_io::fonts`.

use crate::app::App;
use eframe::egui::{self, FontData, FontDefinitions, FontFamily, RichText};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The key a font face is registered under ("family", or "family#bold").
pub fn font_key(family: &str, bold: bool) -> String {
    if bold {
        format!("{family}#bold")
    } else {
        family.to_string()
    }
}

impl App {
    /// The fonts the document uses: country names, map text, text styles.
    fn fonts_in_use(&self) -> BTreeSet<(String, bool)> {
        let mut v = BTreeSet::new();
        if let Some(f) = &self.doc.style.label_font {
            v.insert((f.clone(), self.doc.style.label_bold));
        }
        for a in self.doc.annotations.values() {
            if let Some(f) = &a.font {
                v.insert((f.clone(), a.bold));
            }
        }
        for s in &self.doc.styles {
            if let map_core::styles::Look::Text(t) = &s.look {
                if let Some(f) = &t.font {
                    v.insert((f.clone(), t.bold));
                }
            }
        }
        v
    }

    /// Each frame: load the fonts the map has started to use. egui gets a
    /// new set of fonts only when that set changes.
    pub fn sync_fonts(&mut self, ctx: &egui::Context) {
        let wanted: BTreeSet<String> = self
            .fonts_in_use()
            .into_iter()
            .map(|(f, b)| font_key(&f, b))
            .filter(|k| !self.fonts_missing.contains(k))
            .collect();
        if wanted.is_subset(&self.fonts_loaded) {
            return;
        }
        let mut defs = FontDefinitions::default();
        if let Some(cjk) = &self.font {
            defs.font_data.insert("cjk".into(), Arc::new(FontData::from_owned((**cjk).clone())));
            for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                defs.families.entry(fam).or_default().push("cjk".into());
            }
        }
        let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        let mut loaded = BTreeSet::new();
        for key in wanted.iter().chain(self.fonts_loaded.iter()) {
            let (family, bold) = match key.strip_suffix("#bold") {
                Some(f) => (f, true),
                None => (key.as_str(), false),
            };
            match map_io::fonts::load(family, bold) {
                Some((bytes, index)) => {
                    let mut data = FontData::from_owned((*bytes).clone());
                    data.index = index;
                    defs.font_data.insert(key.clone(), Arc::new(data));
                    let mut chain = vec![key.clone()];
                    chain.extend(fallback.iter().cloned());
                    defs.families.insert(FontFamily::Name(key.as_str().into()), chain);
                    loaded.insert(key.clone());
                }
                None => {
                    self.fonts_missing.insert(key.clone());
                }
            }
        }
        if !self.fonts_missing.is_empty() && loaded.len() < wanted.len() {
            let gone: Vec<String> = wanted.difference(&loaded).map(|k| k.trim_end_matches("#bold").to_string()).collect();
            self.set_status(format!("这台电脑上没有字体：{}（用默认字体显示）", gone.join("、")), true);
        }
        ctx.set_fonts(defs);
        self.fonts_loaded = loaded;
    }

    /// The egui family to draw with a font (the default one until it is loaded).
    pub fn font_family(&self, font: &Option<String>, bold: bool) -> FontFamily {
        match font {
            Some(f) if self.fonts_loaded.contains(&font_key(f, bold)) => FontFamily::Name(font_key(f, bold).as_str().into()),
            _ => FontFamily::Proportional,
        }
    }
}

/// A font chooser: the system's fonts by name, a search field, Chinese
/// fonts only if asked; "默认" is none. Returns whether it changed.
pub fn font_picker(ui: &mut egui::Ui, salt: &str, font: &mut Option<String>, bold: &mut bool) -> bool {
    let mut changed = false;
    let shown = font.as_deref().map_or("默认字体".to_string(), map_io::fonts::display_name);
    let id = ui.make_persistent_id(("font-search", salt));
    // (stays open while the search field is typed in; a choice closes it)
    egui::ComboBox::from_id_salt(("font", salt))
        .selected_text(shown)
        .width(140.0)
        .height(320.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
        let (mut query, mut cjk_only): (String, bool) = ui.data_mut(|d| d.get_temp(id).unwrap_or((String::new(), true)));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut query).hint_text("搜索字体").desired_width(110.0));
            ui.checkbox(&mut cjk_only, "只列中文字体");
        });
        ui.data_mut(|d| d.insert_temp(id, (query.clone(), cjk_only)));
        if ui.selectable_label(font.is_none(), "默认字体").on_hover_text("界面与导出所用的默认中文字体").clicked() {
            *font = None;
            changed = true;
            ui.close();
        }
        let Some(all) = map_io::fonts::families_if_ready() else {
            ui.weak("正在读取系统字体……");
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            return;
        };
        let q = query.trim().to_lowercase();
        let mut n = 0;
        for f in all.iter().filter(|f| (!cjk_only || f.cjk) && (q.is_empty() || f.display.to_lowercase().contains(&q) || f.family.to_lowercase().contains(&q))) {
            n += 1;
            let label = if f.display != f.family { format!("{}  {}", f.display, f.family) } else { f.family.clone() };
            let r = ui.selectable_label(font.as_deref() == Some(f.family.as_str()), label).on_hover_text(if f.bold.is_some() { "有粗体" } else { "没有粗体（粗体时仍用常规字形）" });
            if r.clicked() {
                *font = Some(f.family.clone());
                changed = true;
                ui.close();
            }
        }
        if n == 0 {
            ui.weak("没有符合的字体");
        }
    });
    if ui.checkbox(bold, "粗体").changed() {
        changed = true;
    }
    if let Some(f) = font.as_deref() {
        if map_io::fonts::families_if_ready().is_some_and(|all| !all.iter().any(|x| x.family == f)) {
            ui.label(RichText::new("（本机没有）").weak()).on_hover_text("这台电脑上没有这个字体，用默认字体显示与导出");
        }
    }
    changed
}
