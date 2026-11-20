//! The program's frame around the map: preferences and the files opened
//! lately (kept between runs, see `map_io::prefs`), the start window, the
//! history panel and the status bar.

use crate::actions::Act;
use crate::app::{App, Layout};
use eframe::egui::{self, RichText};
use map_io::prefs::{self, Prefs};
use std::path::Path;

impl App {
    pub fn about_window(&mut self, ctx: &egui::Context) {
        use crate::i18n::tr;
        if !self.about_open { return; }
        egui::Window::new(tr("关于 Atlas Forge"))
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
    /// Each frame: the preferences follow the interface, take effect, and
    /// are stored when they changed (once the pointer is let go, so a
    /// slider being dragged does not write a file per frame).
    pub fn sync_prefs(&mut self, ctx: &egui::Context) {
        self.prefs.layout = layout_name(self.layout).into();
        self.prefs.graticule = self.graticule;
        self.prefs.furniture = self.furniture;
        if self.prefs == self.prefs_saved || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.history.limit = self.prefs.undo_limit;
        if let Some(e) = self.autosave.configure(&self.prefs) {
            self.set_status(format!("整理自动保存历史失败：{e}"),true);
        }
        if (ctx.zoom_factor() - self.prefs.ui_scale).abs() > 1e-3 {
            ctx.set_zoom_factor(self.prefs.ui_scale);
        }
        if let Err(e) = prefs::save(&self.prefs_path, &self.prefs) {
            self.set_status(format!("偏好设置未能保存：{e}"), true);
        }
        self.prefs_saved = self.prefs.clone();
    }

    /// A world was opened from or saved to this file.
    pub fn note_recent(&mut self, p: &Path) {
        // another window may have added files meanwhile
        self.prefs.recent = prefs::load(&self.prefs_path).recent;
        self.prefs.note_recent(p);
    }

    /// File menu: the files opened lately.
    pub fn recent_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("最近打开", |ui| {
            if self.prefs.recent.is_empty() {
                ui.weak("（还没有打开或保存过文件）");
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
            if ui.button("清空列表").clicked() {
                self.prefs.recent.clear();
            }
            if let Some(i) = open {
                self.run(Act::OpenRecent(i));
            }
        });
    }

    pub fn prefs_window(&mut self, ctx: &egui::Context) {
        if !self.prefs_open {
            return;
        }
        let mut open = true;
        egui::Window::new(crate::i18n::tr("偏好设置")).open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            let p = &mut self.prefs;
            egui::Grid::new("prefs").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(crate::i18n::tr("界面语言"));
                {
                    let mut lang = crate::i18n::Lang::from_code(&p.language);
                    egui::ComboBox::from_id_salt("pref-lang").selected_text(lang.name()).show_ui(ui, |ui| {
                        for l in crate::i18n::Lang::ALL {
                            ui.selectable_value(&mut lang, l, l.name());
                        }
                    })
                    .response
                    .on_hover_text("命令、菜单与工具已有英文；其余部分暂为中文。Commands, menus and tools are translated; the rest is in Chinese for now.");
                    if lang.code() != p.language {
                        p.language = lang.code().to_string();
                        crate::i18n::set(lang);
                    }
                }
                ui.end_row();
                ui.label("界面缩放");
                ui.add(egui::Slider::new(&mut p.ui_scale, 0.75..=2.0).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text("文字、按钮与面板的大小；松开鼠标后生效。地图本身用滚轮或 Ctrl + 加减号缩放。");
                ui.end_row();
                ui.label("自动保存");
                ui.checkbox(&mut p.autosave_on,"后台保存版本");
                ui.end_row();
                ui.label("自动保存间隔");
                ui.add_enabled(p.autosave_on,egui::DragValue::new(&mut p.autosave_secs).range(5..=3600).suffix(" 秒"));
                ui.end_row();
                ui.label("每编辑多少步也保存");
                ui.add_enabled(p.autosave_on,egui::DragValue::new(&mut p.autosave_steps).range(0..=1000).suffix(" 步")).on_hover_text("0 表示只按时间触发。撤销和重做也计入编辑历史。");
                ui.end_row();
                ui.label("保留多少份自动版本");
                ui.add_enabled(p.autosave_on,egui::DragValue::new(&mut p.autosave_keep).range(1..=1000).suffix(" 份")).on_hover_text("只整理自动版本，命名保存点独立保留。");
                ui.end_row();
                ui.label("撤销步数上限");
                ui.add(egui::DragValue::new(&mut p.undo_limit).range(20..=5000).suffix(" 步")).on_hover_text("超出后最早的步骤被丢弃。撤销记录另有 512 MB 的内存上限。");
                ui.end_row();
                ui.label("启动时");
                ui.vertical(|ui| {
                    ui.checkbox(&mut p.welcome, "显示开始窗口");
                    ui.checkbox(&mut p.reopen_last, "打开上次的文件");
                });
                ui.end_row();
                ui.label("状态栏");
                ui.checkbox(&mut p.perf_in_status, "显示性能数据（帧时间、三角形数、内存）");
                ui.end_row();
                #[cfg(windows)]
                {
                    ui.label("文件关联");
                    if ui.button("关联 .atlas 文件（当前用户）").on_hover_text("双击 .atlas 文件时用这个程序打开（写入当前用户的注册表，不需要管理员权限；移动程序后再点一次）").clicked() {
                        let msg = match associate_atlas_files() {
                            Ok(()) => "已关联 .atlas 文件".to_string(),
                            Err(e) => format!("关联失败：{e}"),
                        };
                        self.status = msg;
                    }
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("恢复默认").clicked() {
                    *p = Prefs { recent: std::mem::take(&mut p.recent), ..Default::default() };
                }
                ui.weak(format!("保存在 {}", self.prefs_path.display()));
            });
        });
        self.prefs_open = open;
    }

    /// The start window: a new world, a file, or one opened lately.
    pub fn welcome_window(&mut self, ctx: &egui::Context) {
        if !self.welcome_open {
            return;
        }
        let mut open = true;
        let mut run: Option<Act> = None;
        let mut close = false;
        egui::Window::new("开始").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0]).min_width(420.0).show(ctx, |ui| {
            ui.label(RichText::new("Atlas Forge").heading());
            ui.weak("绘制架空世界的地图：海岸与国界、城市与河流、逐年变化的历史、数据与专题图。");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("新建空白世界").clicked() {
                    run = Some(Act::NewBlank);
                }
                if ui.button("打开…").clicked() {
                    run = Some(Act::Open);
                }
                if ui.button("看看示例世界").on_hover_text("已经载入：关闭此窗口即可。").clicked() {
                    close = true;
                }
                if ui.button("创作教程").clicked() {
                    run = Some(Act::Tutorial);
                }
            });
            if !self.prefs.recent.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("最近打开").strong());
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
                ui.checkbox(&mut self.prefs.welcome, "启动时显示");
                ui.weak("F1 快捷键一览 · Ctrl+K 命令面板");
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
            self.set_status(format!("{} {done} 步", if steps < 0 { "已撤销" } else { "已重做" }), false);
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
        egui::Window::new("历史记录").open(&mut open).default_width(240.0).default_pos(ctx.content_rect().right_bottom() - egui::vec2(640.0, 420.0)).show(ctx, |ui| {
            ui.weak(format!("{n} 步可撤销，{} 步可重做 · 单击回到那一步之后", redo.len()));
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    if ui.selectable_label(n == 0, RichText::new("（最早可回到的状态）").italics()).clicked() {
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
                        if ui.selectable_label(false, RichText::new(format!("{}  {l}", n + j + 1)).weak()).on_hover_text("已撤销，可重做").clicked() {
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
                    ui.label(format!("缩放 {:.0}%", self.zoom_percent())).on_hover_text("二维地图的缩放：100% 时整个世界正好放进视图。滚轮或 Ctrl + 加减号缩放，Home 或 Ctrl+0 回到全图。");
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
                    ui.label(format!(
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
                    ui.label(format!("派生 {:.1} · 面板 {:.1} · 视图 {:.1} · 其余 {:.1} ms", p[0], p[1], p[2], p[3]));
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
