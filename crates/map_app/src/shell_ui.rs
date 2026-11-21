//! The program's frame around the map: preferences and the files opened
//! lately (kept between runs, see `map_io::prefs`), the start window, the
//! history panel and the status bar.

use crate::actions::Act;
use crate::app::{App, Layout};
use eframe::egui::{self, RichText};
use map_io::prefs::{self, Prefs};
use std::path::{Path,PathBuf};

impl App {
    pub fn about_window(&mut self, ctx: &egui::Context) {
        use crate::i18n::tr;
        if !self.about_open { return; }
        egui::Window::new(tr("关于 Atlas Forge"))
            .id(egui::Id::unique("about-atlas-forge"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut self.about_open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.heading("Atlas Forge");
                ui.weak(format!("{} {} · {} {}", tr("版本"), env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
                ui.label(tr("为架空世界与历史地图而生的桌面地图创作软件"));
                ui.separator();
                ui.label(tr("软件采用 MIT 许可证。随附 Noto Sans SC 字体采用 SIL Open Font License 1.1。"));
                ui.horizontal(|ui| {
                    for (label, url) in [
                        ("项目主页", "https://github.com/UnknownGlyph/atlas-forge"),
                        ("问题反馈", "https://github.com/UnknownGlyph/atlas-forge/issues"),
                        ("许可证", "https://github.com/UnknownGlyph/atlas-forge/blob/main/LICENSE"),
                    ] {
                        ui.hyperlink_to(tr(label), url).context_menu(|ui| {
                            if ui.button(tr("复制链接")).clicked() {
                                ui.ctx().copy_text(url.to_owned());
                                ui.close();
                            }
                        });
                    }
                });
            });
    }
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::Flat => "flat",
        Layout::Globe => "globe",
        Layout::Split => "split",
    }
}

pub fn layout_named(name: &str) -> Layout {
    match name {
        "flat" => Layout::Flat,
        "globe" => Layout::Globe,
        _ => Layout::Split,
    }
}

/// A file's name, and the folder it is in.
fn name_and_folder(p: &Path) -> (String, String) {
    (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()), p.parent().map(|d| d.display().to_string()).unwrap_or_default())
}

impl App {
    /// Apply preference changes when the pointer is released. Only the
    /// background writer waits for the settings file to reach disk.
    pub fn sync_prefs(&mut self, ctx: &egui::Context) {
        self.prefs.layout = layout_name(self.layout).into();
        self.prefs.graticule = self.graticule;
        self.prefs.furniture = self.furniture;
        if let Err(error) = self.prefs_writer.tick(&self.prefs_path, ctx) {
            self.set_status(crate::trf!("偏好设置未能保存：{error}"), true);
        }
        if self.prefs == self.prefs_applied || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let _timing = crate::perf_ui::span("偏好设置", "应用界面偏好");
        let language = crate::i18n::Lang::from_code(&self.prefs.language);
        if language != crate::i18n::get() {
            crate::i18n::set(language);
            ctx.request_repaint();
        }
        self.history.limit = self.prefs.undo_limit;
        if let Some(e) = self.autosave.configure(&self.prefs) {
            self.set_status(crate::trf!("整理自动保存历史失败：{e}"),true);
        }
        if (ctx.zoom_factor() - self.prefs.ui_scale).abs() > 1e-3 {
            ctx.set_zoom_factor(self.prefs.ui_scale);
        }
        self.prefs_applied = self.prefs.clone();
        self.prefs_writer.queue(&self.prefs);
        if let Err(error) = self.prefs_writer.tick(&self.prefs_path, ctx) {
            self.set_status(crate::trf!("偏好设置未能保存：{error}"), true);
        }
    }

    /// A world was opened from or saved to this file.
    pub fn note_recent(&mut self, p: &Path) {
        // Prefer the current list if a later snapshot has not reached disk.
        // Otherwise another window may have added files in the meantime.
        if !self.prefs_writer.writing() && self.prefs.recent == self.prefs_applied.recent {
            self.prefs.recent = prefs::load(&self.prefs_path).recent;
        }
        self.prefs.note_recent(p);
    }

    /// File menu: the files opened lately.
    pub fn recent_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(crate::i18n::tr("最近打开"), |ui| {
            if self.prefs.recent.is_empty() {
                ui.weak(crate::i18n::tr("（还没有打开或保存过文件）"));
                return;
            }
            let mut open = None;
            for (i, p) in self.prefs.recent.iter().enumerate() {
                let (name, folder) = name_and_folder(p);
                if ui.button(name).on_hover_text(folder).clicked() {
                    open = Some(i);
                }
            }
            ui.separator();
            if ui.button(crate::i18n::tr("清空列表")).clicked() {
                self.prefs.recent.clear();
            }
            if let Some(i) = open {
                self.run(Act::OpenRecent(i));
            }
        });
    }

    pub fn prefs_window(&mut self, ctx: &egui::Context) {
        use crate::i18n::tr;
        use map_io::cloud::Provider;
        if !self.prefs_open { return; }
        let mut open = true;
        let mut pick = None;
        let mut pick_location = None;
        let mut pick_test = false;
        let mut jump = None;
        let id = egui::Id::unique("settings-section");
        let mut section = ctx.data(|data| data.get_temp::<u8>(id)).unwrap_or(0);
        egui::Window::new(tr("设置")).id(egui::Id::unique("atlas-settings"))
            .open(&mut open).collapsible(false).default_size([720.0, 460.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Panel::left("settings-navigation").exact_size(142.0).show(ui, |ui| {
                    ui.selectable_value(&mut section, 0, tr("界面与启动"));
                    ui.selectable_value(&mut section, 1, tr("保存与云同步"));
                    ui.selectable_value(&mut section, 3, tr("文件与缓存"));
                    ui.selectable_value(&mut section, 2, tr("工作流程"));
                });
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 150.0).max(180.0)).show(ui, |ui| {
                let p = &mut self.prefs;
                match section {
                    1 => {
                        ui.checkbox(&mut p.autosave_on,tr("保存本地自动版本"));
                        let any = p.autosave_on || !p.cloud_backup.is_empty();
                        ui.add_enabled_ui(any, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(tr("自动保存间隔"));
                                ui.add(egui::DragValue::new(&mut p.autosave_secs).range(5..=3600).suffix(tr(" 秒")));
                                ui.label(tr("每编辑多少步也保存"));
                                ui.add(egui::DragValue::new(&mut p.autosave_steps).range(0..=1000).suffix(tr(" 步")));
                            });
                        });
                        ui.add_enabled_ui(p.autosave_on, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(tr("保留多少份自动版本"));
                                ui.add(egui::DragValue::new(&mut p.autosave_keep).range(1..=1000).suffix(tr(" 份")));
                            });
                        });
                        ui.separator();
                        ui.label(tr("将版本写入已由桌面客户端同步的文件夹。重连后由客户端继续上传。"));
                        for (i, provider) in Provider::ALL.into_iter().enumerate() {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(tr(provider.label()));
                                    if ui.button(tr("选择同步目录…")).clicked() { pick=Some(provider); }
                                    if p.cloud_backup.folder(provider).is_some()
                                        && ui.button(tr("停用")).clicked() { *p.cloud_backup.folder_mut(provider)=None; }
                                });
                                if let Some(folder)=p.cloud_backup.folder(provider) {
                                    ui.weak(folder.display().to_string());
                                    match &self.autosave.cloud_result[i] {
                                        Some(Ok(path))=>{ui.label(tr("已写入本地同步副本"));ui.weak(path.display().to_string());}
                                        Some(Err(error))=>{ui.colored_label(egui::Color32::LIGHT_RED,error);}
                                        None=>{ui.weak(tr("等待自动保存或下一次手动保存。"));}
                                    }
                                } else {
                                    ui.weak(tr("未启用"));
                                }
                            });
                        }
                        if ui.button(tr("保存点与自动保存…")).clicked() { jump=Some(Act::Checkpoints); }
                    }
                    3 => {
                        for (label, slot, cache) in [
                            ("默认保存目录", &mut p.save_directory, false),
                            ("恢复与缓存目录", &mut p.cache_directory, true),
                        ] {
                            ui.group(|ui| {
                                ui.strong(tr(label));
                                if let Some(path) = slot.as_ref() {
                                    ui.label(path.display().to_string());
                                } else {
                                    ui.weak(tr("系统默认位置"));
                                }
                                ui.horizontal(|ui| {
                                    if ui.button(tr("选择目录…")).clicked() { pick_location = Some(cache); }
                                    if slot.is_some() && ui.button(tr("使用默认目录")).clicked() { *slot = None; }
                                });
                            });
                        }
                        ui.weak(tr("恢复文件、未命名工程版本和崩溃报告的目录在下次启动时生效。"));
                        ui.group(|ui| {
                            ui.strong(tr("默认测试工程"));
                            if let Some(path) = &p.test_project {
                                ui.label(path.display().to_string());
                            } else {
                                ui.weak(tr("伊瑟尔三国工程"));
                            }
                            ui.horizontal(|ui| {
                                if ui.button(tr("选择工程…")).clicked() { pick_test = true; }
                                if p.test_project.is_some() && ui.button(tr("恢复默认")).clicked() { p.test_project = None; }
                            });
                        });
                    }
                    2 => {
                        ui.horizontal(|ui| {
                            ui.label(tr("撤销步数上限"));
                            ui.add(egui::DragValue::new(&mut p.undo_limit).range(20..=5000).suffix(tr(" 步")));
                        });
                        ui.checkbox(&mut p.perf_in_status,tr("显示性能数据（帧时间、三角形数、内存）"));
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr("时间播放与视频")).clicked() {jump=Some(Act::TimelinePlayback);}
                            if ui.button(tr("快捷键一览")).clicked() {jump=Some(Act::Shortcuts);}
                            if ui.button(tr("AI 助手…")).clicked() {jump=Some(Act::Assistant);}
                        });
                        #[cfg(windows)]
                        if ui.button(tr("关联 .atlas 文件（当前用户）")).clicked() {
                            self.status=match associate_atlas_files() {
                                Ok(())=>tr("已关联 .atlas 文件").into(),
                                Err(e)=>format!("关联失败：{e}"),
                            };
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.label(tr("界面语言"));
                            let mut lang=crate::i18n::Lang::from_code(&p.language);
                            egui::ComboBox::from_id_salt("pref-lang").selected_text(lang.name()).show_ui(ui, |ui| {
                                for l in crate::i18n::Lang::ALL { ui.selectable_value(&mut lang,l,l.name()); }
                            });
                            if lang.code()!=p.language {
                                p.language=lang.code().to_string();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("界面缩放"));
                            ui.add(egui::Slider::new(&mut p.ui_scale,0.75..=2.0).step_by(0.05)
                                .custom_formatter(|v,_|format!("{:.0}%",v*100.0)));
                        });
                        ui.checkbox(&mut p.welcome,tr("显示开始窗口"));
                        ui.checkbox(&mut p.reopen_last,tr("打开上次的文件"));
                    }
                }
                ui.separator();
                ui.horizontal_wrapped(|ui|{
                    if ui.button(tr("恢复默认")).clicked() {
                        *p=Prefs {recent:std::mem::take(&mut p.recent),..Default::default()};
                    }
                    ui.weak(crate::trf!("保存在 {}",self.prefs_path.display()));
                });
                });
            });
        ctx.data_mut(|data|data.insert_temp(id,section));
        self.prefs_open=open;
        if let Some(provider)=pick {
            let mut dialog=self.file_dialog().set_title(tr("选择云客户端的本地同步目录"));
            let directory=self.prefs.cloud_backup.folder(provider).map(PathBuf::from)
                .or_else(||(provider==Provider::OneDrive).then(map_io::cloud::suggested_onedrive).flatten());
            if let Some(directory)=directory {dialog=dialog.set_directory(directory);}
            if let Some(folder)=dialog.pick_folder() {*self.prefs.cloud_backup.folder_mut(provider)=Some(folder);}
        }
        if let Some(cache) = pick_location {
            let current = if cache { &self.prefs.cache_directory } else { &self.prefs.save_directory };
            let mut dialog = self.file_dialog().set_title(tr("选择目录…"));
            if let Some(path) = current { dialog = dialog.set_directory(path); }
            if let Some(path) = dialog.pick_folder() {
                if cache { self.prefs.cache_directory = Some(path); } else { self.prefs.save_directory = Some(path); }
            }
        }
        if pick_test {
            if let Some(path) = self.file_dialog().add_filter("Atlas", &["atlas"]).pick_file() {
                self.prefs.test_project = Some(path);
            }
        }
        if let Some(action)=jump {
            self.prefs_open=false;
            self.run(action);
        }
    }

    /// The start window: a new world, a file, or one opened lately.
    pub fn welcome_window(&mut self, ctx: &egui::Context) {
        if !self.welcome_open {
            return;
        }
        let mut open = true;
        let mut run: Option<Act> = None;
        let mut close = false;
        egui::Window::new(crate::i18n::tr("开始")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0]).min_width(420.0).show(ctx, |ui| {
            ui.label(RichText::new("Atlas Forge").heading());
            ui.weak(crate::i18n::tr("绘制架空世界的地图：海岸与国界、城市与河流、逐年变化的历史、数据与专题图。"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("新建空白世界")).clicked() {
                    run = Some(Act::NewBlank);
                }
                if ui.button(crate::i18n::tr("打开…")).clicked() {
                    run = Some(Act::Open);
                }
                if ui.button(crate::i18n::tr("看看示例世界")).on_hover_text(crate::i18n::tr("已经载入：关闭此窗口即可。")).clicked() {
                    close = true;
                }
                if ui.button(crate::i18n::tr("创作教程")).clicked() {
                    run = Some(Act::Tutorial);
                }
            });
            if !self.prefs.recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(crate::i18n::tr("最近打开")).strong());
                for (i, p) in self.prefs.recent.iter().enumerate() {
                    let (name, folder) = name_and_folder(p);
                    ui.horizontal(|ui| {
                        if ui.link(name).clicked() {
                            run = Some(Act::OpenRecent(i));
                        }
                        ui.weak(folder);
                    });
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.prefs.welcome, crate::i18n::tr("启动时显示"));
                ui.weak(crate::i18n::tr("F1 快捷键一览 · Ctrl+K 命令面板"));
            });
        });
        if let Some(act) = run {
            self.run(act);
            close = true;
        }
        self.welcome_open = open && !close;
    }

    /// Undo or redo several steps at once (negative: undo).
    pub fn history_jump(&mut self, steps: i64) {
        if steps == 0 {
            return;
        }
        self.end_pending();
        let viewing = self.doc.year;
        let board=map_core::world_book::WorldBook::current(&self.doc);
        let mut done = 0;
        for _ in 0..steps.unsigned_abs() {
            let step = if steps < 0 { self.history.undo(&mut self.doc) } else { self.history.redo(&mut self.doc) };
            if step.is_none() {
                break;
            }
            done += 1;
        }
        if done > 0 {
            self.dirty = true;
            self.keep_book_history_view(board,viewing);
            self.after_history_jump();
            self.set_status(crate::trf!("{} {done} 步", if steps < 0 { "已撤销" } else { "已重做" }), false);
        }
    }

    /// The history panel: every step that can be undone or redone; a
    /// click goes straight to the state after that step.
    pub fn history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let undo: Vec<String> = self.history.undo_labels().iter().map(|s| s.to_string()).collect();
        let redo: Vec<String> = self.history.redo_labels().iter().map(|s| s.to_string()).collect();
        let n = undo.len();
        let moved = self.history_seen != self.history.generation();
        self.history_seen = self.history.generation();
        let mut jump: Option<i64> = None;
        let mut open = true;
        egui::Window::new(crate::i18n::tr("历史记录")).open(&mut open).default_width(240.0).default_pos(ctx.content_rect().right_bottom() - egui::vec2(640.0, 420.0)).show(ctx, |ui| {
            ui.weak(crate::trf!("{n} 步可撤销，{} 步可重做 · 单击回到那一步之后", redo.len()));
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    if ui.selectable_label(n == 0, RichText::new(crate::i18n::tr("（最早可回到的状态）")).italics()).clicked() {
                        jump = Some(-(n as i64));
                    }
                    for (i, l) in undo.iter().enumerate() {
                        let r = ui.selectable_label(i + 1 == n, format!("{}  {l}", i + 1));
                        if i + 1 == n && moved {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            jump = Some(i as i64 + 1 - n as i64);
                        }
                    }
                    for (j, l) in redo.iter().enumerate() {
                        if ui.selectable_label(false, RichText::new(format!("{}  {l}", n + j + 1)).weak()).on_hover_text(crate::i18n::tr("已撤销，可重做")).clicked() {
                            jump = Some(j as i64 + 1);
                        }
                    }
                });
            });
        });
        self.history_open = open;
        if let Some(steps) = jump {
            self.history_jump(steps);
        }
    }

    /// Zoom both views about their centres.
    pub fn zoom_views(&mut self, factor: f64) {
        self.cam2d.zoom_at([0.0, 0.0], factor);
        self.globe.zoom(factor);
        self.nav_touched = true;
    }

    /// The flat map's zoom, as a percentage of the whole world fitting the view.
    fn zoom_percent(&self) -> f64 {
        let mut fit = self.cam2d;
        let bounds = self.doc.paper.map_or_else(|| fit.proj.bounds(), |p| p.bounds());
        fit.fit_bounds(self.last_flat_rect.width() as f64, self.last_flat_rect.height() as f64, bounds);
        100.0 * self.cam2d.zoom / fit.zoom.max(1e-9)
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let c = if self.status_err { egui::Color32::from_rgb(220, 70, 60) } else { ui.visuals().text_color() };
            // the right side first, so a long message is cut short, not the numbers
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.doc.paper.is_some() || self.layout != Layout::Globe {
                    ui.label(crate::trf!("缩放 {:.0}%", self.zoom_percent())).on_hover_text(crate::i18n::tr("二维地图的缩放：100% 时整个世界正好放进视图。滚轮或 Ctrl + 加减号缩放，Home 或 Ctrl+0 回到全图。"));
                    ui.separator();
                }
                if let Some(c) = self.cursor {
                    if let Some(paper) = self.doc.paper {
                        let [x, y] = paper.pixel(c);
                        ui.label(format!("x {x:.0} · y {y:.0} px"));
                    } else {
                        let (ns, ew) = (if c.lat < 0.0 { 'S' } else { 'N' }, if c.lon < 0.0 { 'W' } else { 'E' });
                        ui.label(format!("{:.3}°{ns}  {:.3}°{ew}", c.lat.abs(), c.lon.abs()));
                    }
                    ui.separator();
                }
                if self.prefs.perf_in_status {
                    let mem = crate::process_memory().map(|m| format!(" · 内存 {:.0} MB", m as f64 / 1048576.0)).unwrap_or_default();
                    ui.label(crate::trf!(
                        "面 {} · 边 {} · 三角形 {} · 帧 {:.1} ms (UI {:.1}) · 更新 {:.2} ms · 撤销 {} 步/{:.1} MB{}",
                        self.doc.topo.faces.len() - 1,
                        self.doc.topo.edges.len(),
                        self.scene.vertex_count() / 3,
                        self.frame_ms,
                        self.ui_ms,
                        self.last_stats.ms,
                        self.history.len(),
                        self.history.memory_bytes() as f64 / 1048576.0,
                        mem
                    ));
                    ui.separator();
                }
                if let Some(p) = self.prof {
                    ui.label(crate::trf!("派生 {:.1} · 面板 {:.1} · 视图 {:.1} · 其余 {:.1} ms", p[0], p[1], p[2], p[3]));
                    ui.separator();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&self.status).color(c)).truncate()).on_hover_text(&self.status);
                });
            });
        });
    }
}

/// Open `.atlas` files with this program when they are double-clicked:
/// the file type under the current user's classes (no administrator
/// rights), pointing at this executable.
#[cfg(windows)]
fn associate_atlas_files() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let open = format!("\"{}\" \"%1\"", exe.display());
    let keys: [(&str, String); 3] = [
        (r"HKCU\Software\Classes\.atlas", "AtlasForge.Project".into()),
        (r"HKCU\Software\Classes\AtlasForge.Project", "Atlas Forge 工程".into()),
        (r"HKCU\Software\Classes\AtlasForge.Project\shell\open\command", open),
    ];
    for (key, value) in keys {
        let ok = std::process::Command::new("reg").args(["add", key, "/ve", "/d", &value, "/f"]).output().map_err(|e| e.to_string())?.status.success();
        if !ok {
            return Err(format!("写入 {key} 失败"));
        }
    }
    Ok(())
}
