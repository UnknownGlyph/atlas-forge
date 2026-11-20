use crate::app::App;
use eframe::egui;
use map_core::world_book::WorldBook;
use map_io::checkpoints::{Entry, Kind};

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub title: String,
    entries: Vec<Entry>,
    revision: Option<u64>,
    confirm: Option<(Entry, Command)>,
}

#[derive(Clone, Copy)]
enum Command {
    Restore,
    Delete,
}

impl App {
    pub fn checkpoints_window(&mut self, ctx: &egui::Context) {
        if !self.checkpoints.open {
            return;
        }
        if self.checkpoints.revision != Some(self.autosave.revision) {
            match self.autosave.store.list() {
                Ok(entries) => self.checkpoints.entries = entries,
                Err(e) => self.set_status(format!("保存点列表读取失败：{e}"), true),
            }
            self.checkpoints.revision = Some(self.autosave.revision);
        }
        let mut open = true;
        let mut capture = false;
        let mut confirmed = None;
        let panel = &mut self.checkpoints;
        egui::Window::new(crate::i18n::tr("保存点与自动保存…"))
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut panel.title)
                            .hint_text("保存点名称")
                            .desired_width(250.0),
                    );
                    capture = ui
                        .add_enabled(
                            !panel.title.trim().is_empty(),
                            egui::Button::new("创建保存点"),
                        )
                        .clicked();
                    if ui.button("自动保存设置").clicked() {
                        self.prefs_open = true;
                    }
                });
                ui.label(if self.prefs.autosave_on {
                    format!(
                        "自动保存开启 · 保留最近 {} 步 · 命名保存点单独保留",
                        self.autosave.keep
                    )
                } else {
                    "自动保存已关闭 · 可手动创建保存点".into()
                });
                ui.weak(self.autosave.store.directory().display().to_string());
                ui.separator();
                if panel.entries.is_empty() {
                    ui.label("此工程尚无保存点。");
                }
                egui::ScrollArea::vertical()
                    .max_height(390.0)
                    .show(ui, |ui| {
                        for entry in &panel.entries {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(if entry.kind == Kind::Manual {
                                    "保存点"
                                } else {
                                    "自动"
                                });
                                ui.strong(&entry.title);
                                ui.weak(format!(
                                    "{} · {}",
                                    entry.board,
                                    crate::autosave::ago(entry.saved_at)
                                ));
                                if ui.small_button("恢复…").clicked() {
                                    panel.confirm = Some((entry.clone(), Command::Restore));
                                }
                                if ui.small_button("删除…").clicked() {
                                    panel.confirm = Some((entry.clone(), Command::Delete));
                                }
                            });
                        }
                    });
                if let Some((entry, command)) = panel.confirm.as_ref() {
                    ui.separator();
                    let prompt = match command {
                        Command::Restore => format!(
                            "恢复「{}」会替换当前编辑内容，可用 Ctrl+Z 返回恢复前的内容。",
                            entry.title
                        ),
                        Command::Delete => format!("删除「{}」的磁盘版本？", entry.title),
                    };
                    ui.label(prompt);
                    ui.horizontal(|ui| {
                        if ui.button("确认").clicked() {
                            confirmed = panel.confirm.take();
                        }
                        if ui.button("取消").clicked() {
                            panel.confirm = None;
                        }
                    });
                }
            });
        self.checkpoints.open = open;
        if capture {
            self.end_pending();
            self.finish_pending_settings();
            let title = self.checkpoints.title.trim();
            match self
                .autosave
                .store
                .capture(&self.doc, Kind::Manual, title, self.autosave.keep)
            {
                Ok(entry) => {
                    self.checkpoints.title.clear();
                    self.autosave.revision += 1;
                    self.set_status(format!("已创建保存点「{}」", entry.title), false);
                }
                Err(e) => self.set_status(format!("创建保存点失败：{e}"), true),
            }
        }
        if let Some((entry, command)) = confirmed {
            match command {
                Command::Delete => match self.autosave.store.delete(&entry.id) {
                    Ok(()) => {
                        self.autosave.revision += 1;
                        self.set_status("已删除保存点", false);
                    }
                    Err(e) => self.set_status(format!("删除保存点失败：{e}"), true),
                },
                Command::Restore => match self.autosave.store.load(&entry.id) {
                    Ok(mut restored) => {
                        self.end_pending();
                        self.finish_pending_settings();
                        let board = WorldBook::current(&self.doc);
                        if WorldBook::active_map(&restored, board).is_some() {
                            let _ = WorldBook::switch(&mut restored, board, None);
                        }
                        if self
                            .edit("恢复保存点", |doc| {
                                *doc = restored;
                                Ok(())
                            })
                            .is_some()
                        {
                            self.restore_book_view(WorldBook::current(&self.doc));
                            self.set_status(format!("已恢复「{}」，可撤销", entry.title), false);
                        }
                    }
                    Err(e) => self.set_status(format!("恢复保存点失败：{e}"), true),
                },
            }
        }
    }
}
