//! The desktop front end for the local declarative plug-in format.
use crate::app::App;
use eframe::egui;
use map_api::{
    automation::{run_batch, BatchReport, ParameterKind, Plugin},
    Session,
};
use map_core::Document;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::mpsc};

struct Preview {
    generation: u64,
    base: Document,
    label: String,
    report: BatchReport,
    changes: Vec<String>,
    draft: Option<Document>,
}

#[derive(Default)]
pub struct PluginUi {
    pub open: bool,
    plugin: Option<Plugin>,
    selected: usize,
    fields: BTreeMap<String, String>,
    job: Option<mpsc::Receiver<Preview>>,
    preview: Option<Preview>,
    error: Option<String>,
}

impl PluginUi {
    fn reset_fields(&mut self) {
        self.fields.clear();
        self.preview = None;
        if let Some(command) = self
            .plugin
            .as_ref()
            .and_then(|p| p.commands.get(self.selected))
        {
            for (name, field) in &command.parameters {
                let text = match field.default.as_ref() {
                    Some(v) if matches!(field.kind, ParameterKind::String) => {
                        v.as_str().unwrap_or("").to_owned()
                    }
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                self.fields.insert(name.clone(), text);
            }
        }
    }
}

impl App {
    pub fn plugin_window(&mut self, ctx: &egui::Context) {
        if !self.plugins.open {
            return;
        }
        let ui_state = &mut self.plugins;
        if let Some(job) = &ui_state.job {
            match job.try_recv() {
                Ok(preview) => {
                    ui_state.preview = Some(preview);
                    ui_state.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(90))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    ui_state.job = None;
                    ui_state.error = Some("插件处理线程已退出。".into());
                }
            }
        }
        let mut open = true;
        let mut choose = false;
        let mut run = false;
        let mut apply = false;
        let generation = self.history.generation();
        egui::Window::new(crate::i18n::tr("本地插件…"))
            .open(&mut open)
            .default_width(510.0)
            .show(ctx, |ui| {
                let busy = ui_state.job.is_some();
                choose = ui
                    .add_enabled(!busy, egui::Button::new(crate::i18n::tr("打开插件清单…")))
                    .clicked();
                if let Some(plugin) = &ui_state.plugin {
                    ui.label(format!("{}  ·  {}", plugin.name, plugin.version));
                    let prev = ui_state.selected;
                    ui.add_enabled_ui(!busy, |ui| {
                        egui::ComboBox::from_id_salt("atlas-plugin-command")
                            .selected_text(&plugin.commands[ui_state.selected].label)
                            .show_ui(ui, |ui| {
                                for (i, c) in plugin.commands.iter().enumerate() {
                                    ui.selectable_value(&mut ui_state.selected, i, &c.label);
                                }
                            });
                        let command = &plugin.commands[ui_state.selected];
                        if !command.description.is_empty() {
                            ui.label(&command.description);
                        }
                        if prev == ui_state.selected {
                            for (name, field) in &command.parameters {
                                ui.horizontal(|ui| {
                                    ui.label(&field.label);
                                    let value =
                                        ui_state.fields.get_mut(name).expect("fields from command");
                                    if ui.text_edit_singleline(value).changed() {
                                        ui_state.preview = None;
                                    }
                                });
                            }
                            run = ui.button(crate::i18n::tr("生成草稿并检查结果")).clicked();
                        }
                    });
                    if prev != ui_state.selected {
                        ui_state.reset_fields();
                    }
                } else {
                    ui.weak(crate::i18n::tr("打开 JSON 插件清单，选择要运行的命令。"));
                }
                if busy {
                    ui.spinner();
                }
                if let Some(error) = &ui_state.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(preview) = &ui_state.preview {
                    ui.separator();
                    if let Some(index) = preview.report.failed_at {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            crate::trf!("第 {index} 步失败，草稿已回滚。"),
                        );
                    }
                    for change in &preview.changes {
                        ui.label(change);
                    }
                    egui::CollapsingHeader::new(crate::i18n::tr("请求结果"))
                        .default_open(!preview.report.ok || preview.draft.is_none())
                        .show(ui, |ui| {
                            let json =
                                serde_json::to_string_pretty(&preview.report).unwrap_or_default();
                            let short: String = json.chars().take(12_000).collect();
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    ui.monospace(&short);
                                });
                        });
                    if preview.draft.is_some() {
                        let current = generation == preview.generation;
                        if !current {
                            ui.weak(crate::i18n::tr("地图已变动，请重新生成草稿。"));
                        }
                        apply = ui
                            .add_enabled(current, egui::Button::new(crate::i18n::tr("应用到地图（可撤销）")))
                            .clicked();
                    }
                }
            });
        self.plugins.open = open;

        if choose {
            if let Some(path) = self
                .file_dialog()
                .add_filter(crate::i18n::tr("Atlas Forge 插件"), &["json"])
                .pick_file()
            {
                match Plugin::load(&path) {
                    Ok(plugin) => {
                        self.plugins.plugin = Some(plugin);
                        self.plugins.selected = 0;
                        self.plugins.error = None;
                        self.plugins.reset_fields();
                    }
                    Err(error) => self.plugins.error = Some(error),
                }
            }
        }
        if run {
            self.plugins.preview = None;
            let plugin = self.plugins.plugin.as_ref().expect("selected plug-in");
            let command = &plugin.commands[self.plugins.selected];
            let input = command
                .parameters
                .iter()
                .map(|(name, field)| {
                    let text = &self.plugins.fields[name];
                    let value = field
                        .kind
                        .parse(text)
                        .map_err(|e| format!("{}：{e}", field.label))?;
                    Ok((name.clone(), value))
                })
                .collect::<Result<Map<String, Value>, String>>();
            match input.and_then(|input| command.expand(&input)) {
                Ok(requests) => {
                    let label = format!("插件 {} · {}", plugin.name, command.label);
                    let document = self.doc.clone();
                    let generation = self.history.generation();
                    let (tx, rx) = mpsc::channel();
                    self.plugins.error = None;
                    self.plugins.job = Some(rx);
                    crate::perf_ui::spawn("执行插件草稿", move || {
                        let mut session = Session::new(document, "当前地图");
                        let report = run_batch(&mut session, &requests);
                        let draft = session.draft().cloned();
                        let changes = session.edit_log().to_vec();
                        let _ = tx.send(Preview {
                            generation,
                            base: session.original().clone(),
                            label,
                            report,
                            changes,
                            draft,
                        });
                    });
                }
                Err(error) => self.plugins.error = Some(error),
            }
        }
        if apply {
            if let Some(preview) = self.plugins.preview.take() {
                if preview.generation == self.history.generation() && preview.base == self.doc {
                    if let Some(draft) = preview.draft {
                        self.edit(&preview.label, move |doc| {
                            *doc = draft;
                            Ok(())
                        });
                    }
                } else {
                    self.plugins.error = Some("地图已变动，请重新生成草稿。".into());
                }
            }
        }
    }
}
