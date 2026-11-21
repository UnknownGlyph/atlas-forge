//! Saved layouts and atlas export in the export window: save the current
//! export settings as a named layout, apply one, and export several
//! layouts for a list of years as an atlas with a contents page.

use crate::app::App;
use crate::panels::{BatchRun, ExportFormat};
use eframe::egui::{self, RichText};

/// What the layouts section asked for, done after the window is drawn.
#[derive(Default)]
pub struct LayoutAsk {
    pub browse: bool,
    pub run_atlas: bool,
}

impl App {
    pub fn layouts_ui(&mut self, ui: &mut egui::Ui) -> LayoutAsk {
        let mut ask = LayoutAsk::default();
        ui.label(RichText::new(crate::i18n::tr("版式与地图集")).strong());
        // names no longer saved drop out of the atlas
        let names: Vec<String> = self.doc.layouts.iter().map(|l| l.name.clone()).collect();
        self.export.atlas_pages.retain(|n| names.contains(n));
        if self.doc.layouts.is_empty() {
            ui.weak(crate::i18n::tr("还没有版式。版式保存上面的尺寸、投影、范围、标题与图面要素；之后可一键套用，或把多个版式按年份导出为地图集。"));
        }
        let (mut apply, mut delete) = (None, None);
        for l in &self.doc.layouts {
            ui.horizontal(|ui| {
                let mut on = self.export.atlas_pages.contains(&l.name);
                if ui.checkbox(&mut on, "").on_hover_text(crate::i18n::tr("加入地图集")).changed() {
                    if on {
                        self.export.atlas_pages.push(l.name.clone());
                    } else {
                        self.export.atlas_pages.retain(|n| *n != l.name);
                    }
                }
                ui.label(RichText::new(&l.name).strong());
                ui.weak(format!("{}×{}，{}{}", l.width, l.height, l.projection.name(), if l.bounds.is_some() { "，局部范围" } else { "" }));
                if ui.small_button(crate::i18n::tr("套用")).on_hover_text(crate::i18n::tr("把这个版式的设置填入上面")).clicked() {
                    apply = Some(l.clone());
                }
                if ui.small_button(crate::i18n::tr("删除")).clicked() {
                    delete = Some(l.name.clone());
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("当前设置另存为"));
            ui.add(egui::TextEdit::singleline(&mut self.export.layout_name).desired_width(120.0).hint_text(crate::i18n::tr("版式名称")));
            let name = self.export.layout_name.trim().to_string();
            let replace = names.contains(&name);
            if ui.add_enabled(!name.is_empty(), egui::Button::new(if replace { crate::i18n::tr("覆盖版式") } else { crate::i18n::tr("保存版式") })).clicked() {
                let l = self.export_options().to_layout(&name);
                if self.edit("保存版式", |d| d.save_layout(l)).is_some() {
                    self.set_status(crate::trf!("已保存版式“{name}”"), false);
                }
            }
        });
        if let Some(l) = apply {
            let x = &mut self.export;
            x.width = l.width;
            x.height = l.height;
            x.projection = l.projection;
            x.title = l.title.clone();
            x.labels = l.labels;
            x.legend = l.legend;
            x.graticule = l.graticule;
            x.scale_bar = l.scale_bar;
            x.north_arrow = l.north_arrow;
            x.placement = l.placement.clone();
            x.current_view = false;
            x.bounds = l.bounds;
            x.lon0 = Some(l.lon0);
            if !matches!(x.format, ExportFormat::Png | ExportFormat::Svg | ExportFormat::Pdf) {
                x.format = ExportFormat::Png;
            }
            self.set_status(crate::trf!("已套用版式“{}”", l.name), false);
        }
        if let Some(n) = delete {
            self.edit("删除版式", |d| d.delete_layout(&n));
        }
        // the atlas
        let pages = self.export.atlas_pages.len();
        if pages == 0 {
            if !self.doc.layouts.is_empty() {
                ui.weak(crate::i18n::tr("勾选版式即可把它们按年份导出为地图集。"));
            }
            return ask;
        }
        let years = self.atlas_years();
        ui.checkbox(&mut self.export.same_scale, crate::i18n::tr("整本地图集使用同一图例尺度"))
            .on_hover_text(crate::i18n::tr("分级设色时把所有年份的数值放在一起分级，同一颜色在每页、每年代表同样的数值"));
        ui.weak(match (self.export.per_year, years.len()) {
            (true, n) => crate::trf!("年份按下方“逐年导出”的设置：{n} 个。"),
            (false, 1) => crate::trf!("只做当前年份（{} 年）；勾选下方“逐年导出”可选多个年份。", years[0]),
            _ => "地图没有选定年份：每个版式一幅；勾选下方“逐年导出”可选多个年份。".into(),
        });
        // per-year export shows the same folder already
        if !self.export.per_year {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("输出文件夹"));
                ui.add(egui::TextEdit::singleline(&mut self.export.out_dir).desired_width(240.0));
                ask.browse = ui.button(crate::i18n::tr("浏览…")).clicked();
            });
        }
        let picture = matches!(self.export.format, ExportFormat::Png | ExportFormat::Svg | ExportFormat::Pdf);
        let ready = picture && !self.export.out_dir.trim().is_empty() && self.batch.is_none();
        let n = pages * years.len().max(1);
        ask.run_atlas = ui
            .add_enabled(ready, egui::Button::new(crate::trf!("导出地图集（{pages} 个版式，共 {n} 幅）")))
            .on_disabled_hover_text(if picture { crate::i18n::tr("请先选择输出文件夹") } else { crate::i18n::tr("地图集只能导出 PNG 或 SVG") })
            .clicked();
        ui.weak(crate::i18n::tr("文件名：版式名_年份.扩展名；文件夹中另写 index.html 目录页，可在浏览器中翻阅。"));
        ask
    }

    /// The years an atlas covers: the per-year settings when chosen, else
    /// the selected year, else none (one map per page).
    fn atlas_years(&self) -> Vec<i32> {
        let x = &self.export;
        if x.per_year {
            x.year_list(&self.doc)
        } else {
            self.doc.year.into_iter().collect()
        }
    }

    pub fn start_atlas(&mut self) {
        let pages: Vec<map_core::layout::Layout> = self.export.atlas_pages.iter().filter_map(|n| self.doc.layout(n).cloned()).collect();
        let job = map_io::atlas::AtlasJob {
            pages,
            years: self.atlas_years(),
            dir: std::path::PathBuf::from(self.export.out_dir.trim()),
            format: self.export.format,
            font: self.font.as_deref().cloned(),
            same_scale: self.export.same_scale,
        };
        let total = job.len();
        let dir = job.dir.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (d, c, doc) = (done.clone(), cancel.clone(), self.doc.clone());
        let handle = crate::perf_ui::spawn("导出地图集", move || map_io::atlas::export_atlas(&doc, &job, &d, &c));
        self.batch = Some(BatchRun { done, cancel, total, dir, started: std::time::Instant::now(), what: "地图集导出", handle: Some(handle) });
        self.set_status(crate::trf!("正在导出地图集（{total} 幅）…"), false);
    }
}
