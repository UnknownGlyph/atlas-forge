//! The new-world wizard in the interface (`map_core::worldgen`): a window
//! with the world's settings — size of the planet, how much land, how many
//! continents, how ragged the coasts, how many countries and rivers, the
//! names' style, capitals, a random history — and a seed. The world is
//! made in the background and opened as a new, unsaved document.

use crate::app::App;
use eframe::egui;
use map_core::names::NameStyle;
use map_core::worldgen::{WorldReport, WorldSpec};
use map_core::Document;

/// The wizard's settings, and the world being made.
pub struct Wizard {
    pub open: bool,
    pub spec: WorldSpec,
    pub with_history: bool,
    pub history: (i32, i32, usize),
    pub job: Option<std::thread::JoinHandle<Result<(Document, WorldReport), String>>>,
    /// Asked whether unsaved work may be given up for the new world.
    pub confirm: bool,
}

impl Default for Wizard {
    fn default() -> Self {
        Wizard { open: false, spec: WorldSpec::default(), with_history: false, history: (1000, 1600, 120), job: None, confirm: false }
    }
}

impl App {
    pub fn wizard_window(&mut self, ctx: &egui::Context) {
        self.poll_wizard(ctx);
        if !self.wizard.open {
            return;
        }
        let mut open = true;
        let mut go = false;
        let busy = self.wizard.job.is_some();
        egui::Window::new(crate::i18n::tr("新世界向导")).open(&mut open).resizable(false).default_width(360.0).show(ctx, |ui| {
            let w = &mut self.wizard;
            let s = &mut w.spec;
            egui::Grid::new("wizard").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(crate::i18n::tr("名称"));
                ui.text_edit_singleline(&mut s.name);
                ui.end_row();
                ui.label(crate::i18n::tr("星球半径"));
                ui.add(egui::DragValue::new(&mut s.radius_km).range(500.0..=100000.0).speed(10.0).suffix(" km"));
                ui.end_row();
                ui.label(crate::i18n::tr("陆地比例"));
                let mut pct = s.land * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 10.0..=70.0).suffix("%").fixed_decimals(0)).changed() {
                    s.land = pct / 100.0;
                }
                ui.end_row();
                ui.label(crate::i18n::tr("大陆数量"));
                ui.add(egui::Slider::new(&mut s.continents, 1..=8));
                ui.end_row();
                ui.label(crate::i18n::tr("海岸与边界的粗糙度"));
                ui.add(egui::Slider::new(&mut s.roughness, 0.0..=1.0).fixed_decimals(2));
                ui.end_row();
                ui.label(crate::i18n::tr("国家数量")).on_hover_text(crate::i18n::tr("0 为只生成陆地（无主）"));
                ui.add(egui::Slider::new(&mut s.countries, 0..=80));
                ui.end_row();
                ui.label(crate::i18n::tr("名称风格"));
                egui::ComboBox::from_id_salt("wizard-style").selected_text(s.style.name()).show_ui(ui, |ui| {
                    for st in NameStyle::ALL {
                        ui.selectable_value(&mut s.style, st, st.name());
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("河流数量"));
                ui.add(egui::Slider::new(&mut s.rivers, 0..=40));
                ui.end_row();
                ui.label(crate::i18n::tr("首都"));
                ui.checkbox(&mut s.capitals, crate::i18n::tr("每个国家一个首都"));
                ui.end_row();
                ui.label(crate::i18n::tr("随机历史"));
                ui.horizontal(|ui| {
                    ui.add_enabled(s.countries > 0, egui::Checkbox::new(&mut w.with_history, ""));
                    ui.add_enabled_ui(w.with_history && s.countries > 0, |ui| {
                        ui.add(egui::DragValue::new(&mut w.history.0).speed(1.0));
                        ui.label(crate::i18n::tr("至"));
                        ui.add(egui::DragValue::new(&mut w.history.1).speed(1.0));
                        ui.add(egui::DragValue::new(&mut w.history.2).range(10..=600).suffix(crate::i18n::tr(" 次变化")));
                    });
                });
                ui.end_row();
                ui.label(crate::i18n::tr("种子"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.seed).speed(1.0));
                    if ui.small_button(crate::i18n::tr("随机")).clicked() {
                        s.seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.subsec_nanos() as u64 ^ d.as_secs());
                    }
                });
                ui.end_row();
            });
            ui.weak(crate::i18n::tr("海洋留在日界线附近与两极，陆地不会跨越地图边缘。生成的地块、国界、河流与城市都可以照常编辑；高程用于地形晕渲。"));
            ui.separator();
            let dirty = self.dirty;
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                    ui.label(crate::i18n::tr("正在生成…"));
                } else if self.wizard.confirm {
                    ui.colored_label(egui::Color32::from_rgb(220, 120, 40), crate::i18n::tr("当前世界有未保存的修改。"));
                    if ui.button(crate::i18n::tr("放弃修改并生成")).clicked() {
                        go = true;
                        self.wizard.confirm = false;
                    }
                    if ui.button(crate::i18n::tr("取消")).clicked() {
                        self.wizard.confirm = false;
                    }
                } else if ui.button(crate::i18n::tr("生成新世界")).clicked() {
                    if dirty {
                        self.wizard.confirm = true;
                    } else {
                        go = true;
                    }
                }
            });
        });
        if go {
            let mut spec = self.wizard.spec.clone();
            spec.history = self.wizard.with_history.then_some(self.wizard.history);
            self.wizard.job = Some(crate::perf_ui::spawn("生成新世界", move || map_core::worldgen::generate(&spec)));
        }
        if !open {
            self.wizard.open = false;
        }
    }

    fn poll_wizard(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.wizard.job else { return };
        if !job.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let job = self.wizard.job.take().unwrap();
        match job.join() {
            Ok(Ok((doc, r))) => {
                self.new_doc(doc);
                self.wizard.open = false;
                self.set_status(
                    crate::trf!(
                        "新世界：陆地 {:.0}%，{} 块陆地，{} 个国家，{} 条河流，{} 座城市{}，用时 {:.1} s",
                        r.land * 100.0,
                        r.landmasses,
                        r.countries,
                        r.rivers,
                        r.cities,
                        if r.changes > 0 { format!("，{} 次历史变化", r.changes) } else { String::new() },
                        r.ms / 1e3
                    ),
                    false,
                );
            }
            Ok(Err(e)) => self.set_status(crate::trf!("生成新世界失败：{e}"), true),
            Err(_) => self.set_status(crate::i18n::tr("生成新世界时出错（已记录）"), true),
        }
    }
}
