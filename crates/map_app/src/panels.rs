//! Tool palette, inspector tabs and the export dialog.

use crate::app::App;
use crate::view::Tool;
use eframe::egui::{self, Color32, RichText};
use map_core::doc::{format_num, EntityKind, FieldKind, Value, AREA_KEY};
use map_core::projection::Projection;
use map_core::theme::{ClassMethod, ClassScale, ThemeMode, RAMPS};
use map_core::EntityId;
use map_io::scene::ExportOptions;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Countries,
    Data,
    Fields,
    Theme,
    Style,
    Underlay,
    /// Every drawn feature (text, symbols, lines, areas).
    Features,
}

pub use map_io::batch::Format as ExportFormat;

/// Which years a per-year export covers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum YearSource {
    /// The starting map and every year where something changes.
    Changes,
    /// From, to, step.
    Range,
}

/// A per-year export running in the background.
pub struct BatchRun {
    pub done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub total: usize,
    pub dir: std::path::PathBuf,
    pub started: std::time::Instant,
    /// What is being exported, for the status line ("逐年导出", "地图集").
    pub what: &'static str,
    pub handle: Option<std::thread::JoinHandle<Result<Vec<std::path::PathBuf>, String>>>,
}

pub struct ExportState {
    pub open: bool,
    pub format: ExportFormat,
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
    pub current_view: bool,
    pub labels: bool,
    pub legend: bool,
    pub graticule: bool,
    pub scale_bar: bool,
    pub north_arrow: bool,
    pub title: String,
    /// Where the furniture sits and the inset maps (the layout view).
    pub placement: map_core::layout::Placement,
    /// One file per year instead of one for the selected year.
    pub per_year: bool,
    pub years: YearSource,
    pub range: [i32; 3],
    /// Output folder of per-year exports (kept for the next run).
    pub out_dir: String,
    /// One animation instead of one file per year.
    pub anim: Option<map_io::batch::Animation>,
    /// Seconds each year is shown in an animation.
    pub frame_s: f32,
    /// Whether ffmpeg was found (checked once, when first needed).
    pub ffmpeg: Option<bool>,
    /// Region and central meridian of an applied layout (None: the whole
    /// world / the 2D view's meridian).
    pub bounds: Option<[f64; 4]>,
    pub lon0: Option<f64>,
    /// Name typed for saving a layout.
    pub layout_name: String,
    /// Layouts in the atlas, by name, in the order they were ticked.
    pub atlas_pages: Vec<String>,
    /// One legend scale for the whole atlas.
    pub same_scale: bool,
}

impl ExportState {
    /// The years a per-year export covers.
    pub fn year_list(&self, doc: &map_core::Document) -> Vec<i32> {
        match self.years {
            YearSource::Changes => map_io::batch::change_year_list(doc),
            YearSource::Range => map_io::batch::range_year_list(self.range[0], self.range[1], self.range[2]),
        }
    }

    /// Returns true when the folder "browse" button was clicked.
    fn per_year_ui(&mut self, ui: &mut egui::Ui, doc: &map_core::Document) -> bool {
        let mut browse = false;
        ui.checkbox(&mut self.per_year, crate::i18n::tr("逐年导出（每年一个文件，按该年的归属、隶属与数据）"));
        if !self.per_year {
            return false;
        }
        let changes = map_io::batch::change_year_list(doc);
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.years, YearSource::Changes, crate::trf!("变化年份（{} 个，含最初状态）", changes.len()));
            ui.radio_value(&mut self.years, YearSource::Range, crate::i18n::tr("年份区间"));
        });
        if self.years == YearSource::Range {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("从"));
                ui.add(egui::DragValue::new(&mut self.range[0]).speed(1.0));
                ui.label(crate::i18n::tr("到"));
                ui.add(egui::DragValue::new(&mut self.range[1]).speed(1.0));
                ui.label(crate::i18n::tr("每"));
                ui.add(egui::DragValue::new(&mut self.range[2]).range(1..=1000).speed(0.2).suffix(crate::i18n::tr(" 年")));
            });
            let n = map_io::batch::range_year_list(self.range[0], self.range[1], self.range[2]).len();
            ui.weak(crate::trf!("共 {n} 年"));
        }
        use map_io::batch::Animation;
        let ffmpeg = *self.ffmpeg.get_or_insert_with(map_io::batch::ffmpeg_available);
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("输出"));
            ui.radio_value(&mut self.anim, None, crate::i18n::tr("每年一个文件"));
            ui.radio_value(&mut self.anim, Some(Animation::Gif), crate::i18n::tr("动画 GIF"));
            ui.add_enabled_ui(ffmpeg, |ui| {
                ui.radio_value(&mut self.anim, Some(Animation::Mp4), crate::i18n::tr("视频 MP4")).on_disabled_hover_text(crate::i18n::tr("需要在 PATH 中安装 ffmpeg"));
            });
        });
        if self.anim.is_some() {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("每年显示"));
                ui.add(egui::DragValue::new(&mut self.frame_s).range(0.1..=10.0).speed(0.05).suffix(crate::i18n::tr(" 秒")));
                ui.weak(crate::i18n::tr("（最后一年停留两倍时间；动画按地图格式选项渲染，建议宽度 1200–1920）"));
            });
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("输出文件夹"));
            ui.add(egui::TextEdit::singleline(&mut self.out_dir).desired_width(260.0));
            browse = ui.button(crate::i18n::tr("浏览…")).clicked();
        });
        if matches!(self.format, ExportFormat::Png | ExportFormat::Svg | ExportFormat::Pdf) || self.anim.is_some() {
            ui.weak(crate::i18n::tr("标题中的 {year} 会替换为年份；不含 {year} 时在标题后加上年份。文件名：工程名_年份.扩展名（动画为 工程名.gif / .mp4）。"));
        }
        browse
    }
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            open: false,
            format: ExportFormat::Png,
            width: 3000,
            height: 1600,
            projection: Projection::EqualEarth,
            current_view: false,
            labels: true,
            legend: true,
            graticule: true,
            scale_bar: false,
            north_arrow: false,
            placement: Default::default(),
            title: String::new(),
            per_year: false,
            years: YearSource::Changes,
            range: [1800, 2000, 10],
            out_dir: String::new(),
            anim: None,
            frame_s: 1.0,
            ffmpeg: None,
            bounds: None,
            lon0: None,
            layout_name: String::new(),
            atlas_pages: Vec::new(),
            same_scale: true,
        }
    }
}

pub(crate) fn swatch(ui: &mut egui::Ui, c: [u8; 4]) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
}

fn color_edit(ui: &mut egui::Ui, c: &mut [u8; 4]) -> bool {
    let mut col = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let ch = egui::color_picker::color_edit_button_srgba(ui, &mut col, egui::color_picker::Alpha::OnlyBlend).changed();
    if ch {
        *c = col.to_srgba_unmultiplied();
    }
    ch
}

impl App {
    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (t, n) in [(Tab::Countries, "国家"), (Tab::Data, "数据表"), (Tab::Fields, "字段"), (Tab::Theme, "专题图"), (Tab::Style, "样式"), (Tab::Underlay, "参考图"), (Tab::Features, "图层")] {
                ui.selectable_value(&mut self.tab, t, crate::i18n::tr(n));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
            Tab::Countries => {
                // (a scope of its own, there or not: what follows keeps its ids,
                // so the keyboard focus does not jump when the selection changes)
                ui.push_id("selection", |ui| self.selection_panel(ui));
                self.countries_tab(ui)
            }
            Tab::Data => self.data_tab(ui),
            Tab::Fields => self.fields_tab(ui),
            Tab::Theme => self.theme_tab(ui),
            Tab::Style => self.style_tab(ui),
            Tab::Underlay => self.underlay_tab(ui),
            Tab::Features => self.features_tab(ui),
        });
    }

    /// Layer selector and management (top of the countries tab).
    fn layer_bar(&mut self, ui: &mut egui::Ui) {
        use map_core::doc::POLITICAL;
        let cur = self.doc.active_layer;
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::i18n::tr("分区层")).strong());
            let label = |d: &map_core::Document, l| {
                let mut n = d.layer_name(l);
                if l == POLITICAL && n == "政治归属" { n = crate::i18n::tr("政治归属").into(); }
                if d.layer_kind(l) == map_core::doc::LayerKind::Shares {
                    format!("{n} · {}", crate::i18n::tr("比例层"))
                } else {
                    n
                }
            };
            egui::ComboBox::from_id_salt("layer").selected_text(label(&self.doc, cur)).show_ui(ui, |ui| {
                for l in self.doc.layer_ids() {
                    if ui.selectable_label(l == cur, label(&self.doc, l)).clicked() {
                        pick = Some(l);
                    }
                }
            });
            for (text, kind, hint) in [
                ("+ 独占层", map_core::doc::LayerKind::Exclusive, "每块地最多属于一个单位，如实际控制"),
                ("+ 比例层", map_core::doc::LayerKind::Shares, "每块地按比例分属多个单位，如文化、宗教"),
            ] {
                if ui.button(crate::i18n::tr(text)).on_hover_text(crate::i18n::tr(hint)).clicked() {
                    let n = self.doc.layer_ids().len();
                    if let Some(l) = self.edit("新建分区层", |d| Ok(d.add_layer_of(&format!("分区层 {n}"), kind))) {
                        pick = Some(l);
                    }
                }
            }
            if cur != POLITICAL && ui.button(crate::i18n::tr("删除此层")).clicked() {
                self.edit("删除分区层", |d| d.delete_layer(cur));
            }
        });
        if cur != POLITICAL && self.doc.layers.contains_key(&cur) {
            let name = self.doc.layer_name(cur);
            let entry = self.layer_name_edit.get_or_insert_with(|| (cur, name.clone()));
            if entry.0 != cur {
                *entry = (cur, name.clone());
            }
            let response = ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("层名称"));
                ui.add(egui::TextEdit::singleline(&mut entry.1)
                    .id_salt(("partition-name", cur)).desired_width(185.0))
            }).inner;
            let cancel = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if cancel {
                entry.1 = name;
                response.surrender_focus();
            } else if response.lost_focus() && entry.1.trim() != name {
                let draft = entry.1.clone();
                self.layer_name_edit = None;
                self.edit("重命名分区层", |d| d.rename_layer(cur, &draft));
            } else if !response.has_focus() {
                entry.1 = name;
            }
            if self.doc.active_is_shares() {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("画笔比例"));
                    ui.add(egui::Slider::new(&mut self.share_pct, 1.0..=100.0).suffix(" %"));
                });
                ui.weak(crate::i18n::tr("比例层：领土画笔把画笔单位在地块中的比例设为上值，其他单位按比例缩小；右键清空该地块。地图按比例混合各单位颜色，面积按比例计算，标注和叠加线取比例最大的单位。"));
            } else {
                ui.weak(crate::i18n::tr("每一层都是同一组地块的一种独占划分（如实际控制、文化区），有自己的单位和时间线；切分、合并、绘制会同步保持所有层。"));
            }
        }
        // another layer's boundaries over this one
        if self.doc.layer_ids().len() > 1 {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("叠加显示"));
                let cur_ov = self.doc.style.overlay_layer.filter(|l| *l != cur);
                let name = cur_ov.map_or_else(|| crate::i18n::tr("（无）").to_owned(), |l|
                    format!("{} · {}", self.doc.layer_name(l), crate::i18n::tr("边界")));
                let mut ov = None;
                egui::ComboBox::from_id_salt("overlay_layer").selected_text(name).show_ui(ui, |ui| {
                    if ui.selectable_label(cur_ov.is_none(), crate::i18n::tr("（无）")).clicked() {
                        ov = Some(None);
                    }
                    for l in self.doc.layer_ids().into_iter().filter(|l| *l != cur) {
                        if ui.selectable_label(cur_ov == Some(l), self.doc.layer_name(l)).clicked() {
                            ov = Some(Some(l));
                        }
                    }
                });
                if let Some(v) = ov {
                    self.doc.style.overlay_layer = v;
                }
                if cur_ov.is_some() {
                    color_edit(ui, &mut self.doc.style.overlay_color);
                    ui.add(egui::Slider::new(&mut self.doc.style.overlay_width, 0.5..=6.0).suffix(" px"));
                }
            });
        }
        if let Some(l) = pick {
            self.layer_name_edit = None;
            if let Err(e) = self.doc.set_active_layer(l) {
                self.set_status(e, true);
            }
            // selections and the brush belong to a layer
            let ok = |d: &map_core::Document, e: &EntityId| d.entities.get(e).is_some_and(|x| x.layer == d.active_layer);
            if !self.selected.is_some_and(|e| ok(&self.doc, &e)) {
                self.selected = None;
            }
            if !self.brush.is_some_and(|e| ok(&self.doc, &e)) {
                self.brush = None;
            }
            if matches!(self.draw_mode, crate::view::DrawMode::Province(_)) {
                self.clear_draft();
                self.draw_mode = crate::view::DrawMode::AddLand;
            }
        }
        ui.separator();
    }

    fn subdivision_panel(&mut self, ui: &mut egui::Ui, parent: EntityId, areas: &std::collections::HashMap<EntityId, f64>) {
        let children: Vec<_> = self.doc.entities.iter()
            .filter(|(_, e)| e.parent == Some(parent) && e.kind == EntityKind::Country && e.exists_in(self.doc.year))
            .map(|(id, e)| (*id, e.name.clone())).collect();
        let parent_info = self.doc.entities[&parent].parent
            .and_then(|id| self.doc.entities.get(&id).map(|e| (id, e.name.clone())));
        if let Some((id, name)) = &parent_info {
            if ui.link(crate::trf!("上级 · {name}")).clicked() {
                self.selected = Some(*id);
                self.selected_face = None;
            }
        }
        let label = if parent_info.is_none() { "省份" } else { "下级地区" };
        let name = format!("新{label}{}", children.len() + 1);
        ui.horizontal_wrapped(|ui| {
            if ui.button(crate::trf!("绘制{label}")).on_hover_text(crate::i18n::tr("画一个多边形，仅划入上级直接管辖的陆地")).clicked() {
                self.clear_draft();
                self.set_tool(Tool::DrawLand);
                self.brush_paint = false;
                self.draw_mode = crate::view::DrawMode::Province(parent);
                self.set_status(crate::trf!("在地图上勾画{label}，按回车完成。只划入该单位直属陆地。"), false);
            }
            if ui.button(crate::trf!("自动划分{label}…")).clicked() {
                self.open_partition(parent);
                self.partition.spec.provinces = true;
            }
            if ui.small_button(crate::i18n::tr("批量设为下级…")).clicked() {
                self.partition.adopt = Some((parent, String::new(), Default::default()));
            }
            if let Some(face) = self.selected_face.filter(|f| self.doc.owner.get(f) == Some(&parent)) {
                if ui.button(crate::trf!("用选中地块建{label}")).clicked() {
                    if let Some(child) = self.edit("用地块创建下级地区", |d| d.subdivision_on_face(parent, &name, face)) {
                        self.selected = Some(child);
                    }
                }
            }
        });
        if !children.is_empty() {
            egui::CollapsingHeader::new(format!("{label} · {}", children.len())).id_salt(("children", parent)).default_open(true).show(ui, |ui| {
                for (id, name) in children {
                    ui.horizontal(|ui| {
                        if ui.link(name).clicked() {
                            self.selected = Some(id);
                            self.selected_face = None;
                        }
                        if let Some(area) = areas.get(&id) {
                            ui.weak(format!("{} km²", format_num(area.round())));
                        }
                    });
                }
            });
        }
    }

    fn countries_tab(&mut self, ui: &mut egui::Ui) {
        self.layer_bar(ui);
        let layer_is_political = self.doc.active_layer == map_core::doc::POLITICAL;
        let unit = crate::i18n::tr(if layer_is_political { "+ 新建国家" } else { "+ 新建单位" });
        ui.horizontal(|ui| {
            if ui.button(crate::i18n::tr("自动配色")).on_hover_text(crate::i18n::tr("相邻的国家（任何年份中相邻）用不同的颜色，颜色使用均衡；可撤销")).clicked() {
                let pal = map_core::colouring::palette();
                if let Some(n) = self.edit("自动配色", |d| Ok(d.apply_auto_colours(&pal))) {
                    self.set_status(crate::trf!("自动配色：{n} 个国家换了颜色，相邻国家颜色都不同"), false);
                }
            }
            if ui.button(unit).clicked() {
                let n = self.doc.countries().len() + 1;
                if let Some(id) = self.edit("新建国家", |d| Ok(d.add_entity(EntityKind::Country, &format!("{}{n}", if layer_is_political { "新国家" } else { "新单位" })))) {
                    self.selected = Some(id);
                    self.brush = Some(id);
                    self.set_tool(Tool::Assign);
                }
            }
            if let Some(f) = self.selected_face.filter(|f| !self.doc.owner.contains_key(f)) {
                if self.doc.topo.faces.get(&f).is_some_and(|x| x.kind == map_core::FaceKind::Land) && ui.button(crate::i18n::tr("用所选陆地新建国家")).clicked() {
                    let n = self.doc.countries().len() + 1;
                    if let Some(id) = self.edit("新建国家", |d| {
                        let id = d.add_entity(EntityKind::Country, &format!("{}{n}", if layer_is_political { "新国家" } else { "新单位" }));
                        d.assign(f, Some(id))?;
                        Ok(id)
                    }) {
                        self.selected = Some(id);
                        self.brush = Some(id);
                    }
                }
            }
        });
        ui.add_space(4.0);
        let areas = self.detail_cache.areas.clone();
        let layer = self.doc.active_layer;
        if let Some(id) = self.selected.filter(|id| self.doc.entities.get(id).is_some_and(|ent|
            ent.kind == EntityKind::Country && ent.layer == layer && ent.exists_in(self.doc.year)
        )) {
            if !self.doc.active_is_shares() {
                ui.label(RichText::new(&self.doc.entities[&id].name).strong());
                self.subdivision_panel(ui, id, &areas);
                ui.separator();
            }
        }
        let ids: Vec<EntityId> = self.doc.entities.iter()
            .filter(|(_, e)| e.layer == layer && e.kind == EntityKind::Country && e.parent.is_none())
            .map(|(id, _)| *id).collect();
        egui::Grid::new("countries").striped(true).num_columns(3).show(ui, |ui| {
            for id in ids {
                let e = &self.doc.entities[&id];
                swatch(ui, e.color);
                if ui.selectable_label(self.selected == Some(id), &e.name).clicked() {
                    self.selected = Some(id);
                    self.selected_face = None;
                }
                match self.presence_note(id, areas.get(&id).copied()) {
                    Some(note) if note != "无领土" => ui.weak(format!("（{note}）")),
                    Some(note) => ui.weak(note),
                    None => ui.label(areas.get(&id).map(|a| format!("{} km²", format_num(a.round()))).unwrap_or_default()),
                };
                ui.end_row();
            }
        });
        let cities: Vec<_> = self.doc.entities.iter()
            .filter(|(_, e)| e.layer == layer && e.kind == EntityKind::City)
            .map(|(&id, _)| id).collect();
        if !cities.is_empty() {
            egui::CollapsingHeader::new(crate::trf!("城市（{}）", cities.len()))
                .id_salt("countries-city-list")
                .default_open(cities.len() <= 20)
                .show(ui, |ui| {
                    let search_id = ui.scope_id().with("city-filter");
                    let mut query = ui.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut query).hint_text(crate::i18n::tr("按城市名称或编号筛选"))).changed() {
                        ui.data_mut(|d| d.insert_temp(search_id, query.clone()));
                    }
                    let query = query.trim().to_lowercase();
                    egui::ScrollArea::vertical().id_salt("country-city-rows").max_height(220.0).show(ui, |ui| {
                        for id in cities {
                            let city = &self.doc.entities[&id];
                            if !query.is_empty() && !city.name.to_lowercase().contains(&query) && !city.code.to_lowercase().contains(&query) {
                                continue;
                            }
                            if ui.selectable_label(self.selected == Some(id), &city.name).clicked() {
                                self.selected = Some(id);
                                self.selected_face = None;
                            }
                        }
                    });
                });
        }
        self.lifespan_conflicts_section(ui);
        self.river_panel(ui);
        self.area_panel(ui);
        self.history_section(ui);
        let Some(sel) = self.selected.filter(|s| self.doc.entities.contains_key(s)) else {
            ui.separator();
            ui.weak(crate::i18n::tr("在地图上单击（选择工具）或在列表中选择一个国家以编辑。"));
            return;
        };
        ui.separator();
        let ent = self.doc.entities[&sel].clone();
        ui.label(RichText::new(if ent.kind == EntityKind::City { crate::i18n::tr("城市属性") }
            else if ent.parent.is_some() { crate::i18n::tr("下级地区属性") } else { crate::i18n::tr("国家属性") }).strong());
        egui::Grid::new("entity").num_columns(2).show(ui, |ui| {
            for (k, label) in [(0u8, "名称"), (1, "简称"), (2, "编号 code")] {
                ui.label(label);
                let cur = match k {
                    0 => ent.name.clone(),
                    1 => ent.short.clone(),
                    _ => ent.code.clone(),
                };
                let buf = self.text_edit.entry((sel, k)).or_insert(cur.clone());
                let mut random = false;
                let r = ui
                    .horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(if k == 0 { 150.0 } else { 200.0 }));
                        if k == 0 {
                            random = ui.small_button(crate::i18n::tr("随机")).on_hover_text(crate::i18n::tr("按名称生成器的风格起一个新名字（工具菜单“名称生成器”可选风格）")).clicked();
                        }
                        r
                    })
                    .inner;
                if random {
                    self.random_name(sel);
                    ui.end_row();
                    continue;
                }
                if r.lost_focus() && *buf != cur {
                    let v = buf.clone();
                    self.text_edit.remove(&(sel, k));
                    self.edit("修改属性", |d| {
                        if k == 2 && d.entity_by_code(&v).is_some_and(|o| o != sel) {
                            return Err(format!("编号 {v} 已被使用"));
                        }
                        let e = d.entities.get_mut(&sel).unwrap();
                        match k {
                            0 => e.name = v,
                            1 => e.short = v,
                            _ => e.code = v,
                        }
                        Ok(())
                    });
                } else if !r.has_focus() && *buf != cur {
                    *buf = cur;
                }
                ui.end_row();
            }
            // a name style of its own (a culture's lends itself to the places it covers)
            ui.label(crate::i18n::tr("名称风格"));
            {
                let own = self.doc.name_styles.get(&sel).copied();
                let lent = if own.is_none() { self.doc.name_style_for(sel) } else { None };
                let shown = match (own, lent) {
                    (Some(s), _) => s.name().to_string(),
                    (None, Some(s)) => format!("（随所在：{}）", s.name()),
                    (None, None) => "（不指定）".to_string(),
                };
                let mut pick = own;
                egui::ComboBox::from_id_salt("entity-name-style").selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, crate::i18n::tr("（不指定）"));
                    for s in map_core::names::NameStyle::ALL {
                        ui.selectable_value(&mut pick, Some(s), s.name());
                    }
                })
                .response
                .on_hover_text(crate::i18n::tr("给这个单位一种名称风格：随机名称、自动划分出的新单位都按它起名。文化等分区层的单位设了风格后，位于其中的城市、大部分国土在其中的国家也随它（自己设了风格的除外）。"));
                if pick != own {
                    self.edit("名称风格", move |d| d.set_name_style(sel, pick));
                }
            }
            ui.end_row();
            if ent.kind != EntityKind::City {
                ui.label(crate::i18n::tr("划分"));
                if ui.add_enabled(!self.doc.active_is_shares(), egui::Button::new(crate::i18n::tr("自动划分…")).small())
                    .on_hover_text(crate::i18n::tr("把这个国家的领土划成几块：新的国家，或它的下级地区（省）；可预览，可撤销")).clicked() {
                    self.open_partition(sel);
                    self.partition.spec.provinces = false;
                }
                ui.end_row();
                ui.label(crate::i18n::tr("颜色"));
                let mut c = ent.color;
                if color_edit(ui, &mut c) {
                    self.doc.entities.get_mut(&sel).unwrap().color = c;
                    self.dirty = true;
                }
                ui.end_row();
                ui.label(crate::i18n::tr("面积"));
                ui.label(format!("{} km²", format_num(areas.get(&sel).copied().unwrap_or(0.0).round())));
                ui.end_row();
                ui.label(crate::i18n::tr("领土块数"));
                ui.label(format!("{}", self.doc.faces_of_tree(sel).len()));
                ui.end_row();
                ui.label(crate::i18n::tr("首都"));
                let cities: Vec<(EntityId, String)> =
                    self.doc.entities.iter().filter(|(_, e)| e.kind == EntityKind::City).map(|(id, e)| (*id, e.name.clone())).collect();
                let cur = ent.capital.and_then(|c| self.doc.entities.get(&c)).map(|e| e.name.clone()).unwrap_or("（无）".into());
                let mut pick = None;
                egui::ComboBox::from_id_salt("capital").selected_text(cur).show_ui(ui, |ui| {
                    if ui.selectable_label(ent.capital.is_none(), crate::i18n::tr("（无）")).clicked() {
                        pick = Some(None);
                    }
                    for (id, n) in &cities {
                        if ui.selectable_label(ent.capital == Some(*id), n).clicked() {
                            pick = Some(Some(*id));
                        }
                    }
                });
                if let Some(p) = pick {
                    self.edit("设置首都", |d| {
                        d.entities.get_mut(&sel).unwrap().capital = p;
                        Ok(())
                    });
                }
                ui.end_row();
                ui.label(crate::i18n::tr("上级"));
                let cands: Vec<(EntityId, String)> = self
                    .doc
                    .entities
                    .iter()
                    .filter(|(id, e)| **id != sel && e.kind != EntityKind::City && e.layer == ent.layer)
                    .map(|(id, e)| (*id, e.name.clone()))
                    .collect();
                let cur = ent.parent.and_then(|c| self.doc.entities.get(&c)).map(|e| e.name.clone()).unwrap_or("（无，顶层）".into());
                let mut pick = None;
                egui::ComboBox::from_id_salt("parent").selected_text(cur).show_ui(ui, |ui| {
                    if ui.selectable_label(ent.parent.is_none(), crate::i18n::tr("（无，顶层）")).clicked() {
                        pick = Some(None);
                    }
                    for (id, n) in &cands {
                        if ui.selectable_label(ent.parent == Some(*id), n).clicked() {
                            pick = Some(Some(*id));
                        }
                    }
                });
                if let Some(p) = pick {
                    let label = if self.doc.year.is_some() { "从该年起改变上级" } else { "设置上级" };
                    self.edit(label, |d| d.record_parent(sel, p));
                }
                ui.end_row();
            }
        });
        if ent.kind == EntityKind::City {
            if let Some(p) = ent.pos {
                ui.weak(crate::trf!("位置 {:.3}°, {:.3}°（用“添加城市”工具拖动可移动）", p.lon, p.lat));
                // which country it belongs to, and the one it stands in
                let name = |e: Option<EntityId>| e.and_then(|e| self.doc.entities.get(&e)).map_or("无归属".to_string(), |x| x.name.clone());
                let here = self.doc.owner.get(&self.doc.topo.face_at(p)).copied();
                ui.label(crate::trf!("所属：{}", name(ent.parent)));
                if here != ent.parent {
                    let when = self.doc.year.map(|y| format!("从 {y} 年起")).unwrap_or_else(|| "所有年份".into());
                    if ui.button(crate::trf!("改为所在的 {}（{when}）", name(here))).clicked() {
                        self.edit("城市归属", |d| d.record_parent(sel, here));
                    }
                }
            }
        }
        self.lifespan_panel(ui, sel);
        if ent.kind != EntityKind::City {
            self.label_panel(ui, sel);
            let year = self.doc.year;
            let what = year.map_or("当前疆域".to_string(), |y| format!("{y} 年疆域"));
            let exists = self.doc.exists_in(sel, year);
            if ui
                .add_enabled(exists, egui::Button::new(crate::trf!("把{what}标为斜线区域")))
                .on_disabled_hover_text(crate::i18n::tr("这个实体在该年不存续，没有疆域"))
                .on_hover_text(crate::i18n::tr("把这个国家（含下属地区）在当前年份的轮廓复制为一个斜线区域，例如在后来的地图上标出旧疆域或主张范围。区域只是标记，不改变领土；之后国界变化时它保持该年的轮廓。"))
                .clicked()
            {
                if let Some(id) = self.edit("国家轮廓转为斜线区域", |d| d.area_from_entity(sel, year)) {
                    self.selected = None;
                    self.selected_face = None;
                    self.selected_area = Some(id);
                    self.set_status(crate::trf!("已把{what}标为斜线区域：可在下方修改名称、图案、年份；拖动边线移动"), false);
                }
            }
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("设为画笔")).clicked() {
                    self.brush = Some(sel);
                    self.set_tool(Tool::Assign);
                }
                if ui.button(crate::i18n::tr("删除")).clicked() {
                    self.edit("删除国家", |d| {
                        d.delete_entity(sel);
                        Ok(())
                    });
                    self.selected = None;
                }
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("合并到："));
                let others: Vec<(EntityId, String)> =
                    self.doc.countries().into_iter().filter(|c| *c != sel).map(|c| (c, self.doc.entities[&c].name.clone())).collect();
                egui::ComboBox::from_id_salt("merge_into").selected_text(crate::i18n::tr("选择国家…")).show_ui(ui, |ui| {
                    for (id, n) in others {
                        if ui.selectable_label(false, n).clicked() {
                            if self.edit("合并国家", |d| d.merge_entities(sel, id)).is_some() {
                                self.selected = Some(id);
                            }
                        }
                    }
                });
            });
            if let Some(f) = self.selected_face.filter(|f| self.doc.owner.get(f) == Some(&sel)) {
                if ui.button(crate::i18n::tr("将所选地块独立为新国家（分裂）")).clicked() {
                    let name = format!("{}（分离）", ent.name);
                    if let Some(id) = self.edit("分裂国家", |d| {
                        let id = d.add_entity(EntityKind::Country, &name);
                        d.assign(f, Some(id))?;
                        Ok(id)
                    }) {
                        self.selected = Some(id);
                    }
                }
            }
        } else if ui.button(crate::i18n::tr("删除城市")).clicked() {
            self.edit("删除城市", |d| {
                d.delete_entity(sel);
                Ok(())
            });
            self.selected = None;
        }
        ui.label(crate::i18n::tr("备注"));
        let mut note = ent.note.clone();
        if ui.text_edit_multiline(&mut note).changed() {
            self.doc.entities.get_mut(&sel).unwrap().note = note;
            self.dirty = true;
        }
    }

    fn year_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut use_year = self.doc.year.is_some();
            if ui.checkbox(&mut use_year, crate::i18n::tr("按年份查看/编辑")).changed() {
                self.set_year(use_year.then_some(self.timeline_last_year));
            }
            if let Some(mut y) = self.doc.year {
                if ui.add(egui::DragValue::new(&mut y).speed(1.0).suffix(crate::i18n::tr(" 年"))).changed() {
                    self.set_year(Some(y));
                }
            }
        });
    }

    fn data_tab(&mut self, ui: &mut egui::Ui) {
        self.year_control(ui);
        if self.table_panel {
            ui.label(crate::i18n::tr("数据表显示在地图下方的面板中，可拖动面板上边缘调整高度。"));
            if ui.button(crate::i18n::tr("关闭数据表面板")).clicked() {
                self.table_panel = false;
            }
        } else if ui.button(crate::i18n::tr("在地图下方打开数据表")).clicked() {
            self.table_panel = true;
        }
    }

    /// Wide, scrollable attribute table (bottom panel).
    pub fn data_table(&mut self, ui: &mut egui::Ui) {
        let table_kind = ui.scope_id().with("data-table-kind");
        let mut cities = ui.data(|data| data.get_temp::<bool>(table_kind)).unwrap_or(false);
        let filter_id = ui.scope_id().with("data-table-search");
        let mut filter = ui.data(|data| data.get_temp::<String>(filter_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.strong(crate::i18n::tr("数据表"));
            ui.selectable_value(&mut cities, false, crate::i18n::tr("辖区"));
            ui.selectable_value(&mut cities, true, crate::i18n::tr("城市"));
            ui.data_mut(|data| data.insert_temp(table_kind, cities));
            if ui.add(egui::TextEdit::singleline(&mut filter).hint_text(crate::i18n::tr("按名称或编号筛选")).desired_width(150.0)).changed() {
                ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            }
            ui.weak(crate::i18n::tr("回车、Tab 或点击别处提交；斜体为派生字段。"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(crate::i18n::tr("关闭")).clicked() {
                    self.table_panel = false;
                }
            });
        });
        let doc_now = self.doc.clone();
        // areas of this and other years only change with the map itself
        let same = self.stats_cache.as_ref().is_some_and(|(d, _, _)| {
            d.topo.faces.ptr_eq(&doc_now.topo.faces)
                && d.owner.ptr_eq(&doc_now.owner)
                && d.owner_base.ptr_eq(&doc_now.owner_base)
                && d.owner_changes.ptr_eq(&doc_now.owner_changes)
                && d.layers.ptr_eq(&doc_now.layers)
                && d.kind_changes.ptr_eq(&doc_now.kind_changes)
                && d.year == doc_now.year
                && d.active_layer == doc_now.active_layer
        });
        let (cur, other) = match self.stats_cache.take() {
            Some((_, a, o)) if same => (a, o),
            _ => (doc_now.areas_by_entity(), Default::default()),
        };
        let stats = map_core::stats::Stats::with_cache(&doc_now, cur.clone(), other);
        let areas = stats.areas().clone();
        let mut total_areas = areas.clone();
        for (&id, &area) in &areas {
            for parent in doc_now.ancestors(id).skip(1) {
                *total_areas.entry(parent).or_default() += area;
            }
        }
        let fields: Vec<_> = self.doc.fields.iter().map(|(id, f)| (*id, f.clone())).collect();
        let filter = filter.trim().to_lowercase();
        let ents: Vec<_> = self.doc.entities.iter()
            .filter(|(_, e)| e.layer == doc_now.active_layer && (e.kind == EntityKind::City) == cities)
            .filter(|(_, e)| filter.is_empty() || e.name.to_lowercase().contains(&filter) || e.code.to_lowercase().contains(&filter))
            .map(|(&id, _)| id).collect();
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("data").striped(true).min_col_width(96.0).show(ui, |ui| {
                ui.label(RichText::new(crate::i18n::tr("名称")).strong());
                if !cities {
                    ui.label(RichText::new(crate::i18n::tr("直属面积 km²")).strong());
                    ui.label(RichText::new(crate::i18n::tr("辖区总面积 km²")).strong()).on_hover_text(crate::i18n::tr("直属几何面积加上各级下属辖区。"));
                }
                for (_, f) in &fields {
                    ui.label(RichText::new(&f.name).strong()).on_hover_text(crate::trf!("键：{}{}", f.key, f.expr.as_ref().map(|e| format!("\n= {e}")).unwrap_or_default()));
                }
                ui.end_row();
                for e in ents {
                    let name = self.doc.entities[&e].name.clone();
                    if ui.selectable_label(self.selected == Some(e), name).clicked() {
                        self.selected = Some(e);
                    }
                    if !cities {
                        ui.label(format_num(areas.get(&e).copied().unwrap_or(0.0).round()));
                        ui.label(format_num(total_areas.get(&e).copied().unwrap_or(0.0).round()));
                    }
                    for (fid, f) in &fields {
                        if f.expr.is_some() {
                            let st = stats.stat(e, &f.key);
                            let text = st.value.map(format_num).unwrap_or("—".into());
                            let mark = if st.borders == map_core::stats::Borders::Selected { "" } else { " *" };
                            ui.label(RichText::new(format!("{text}{mark}")).italics()).on_hover_text(st.label());
                            continue;
                        }
                        // edit only what is stored for the selected year; show what
                        // the map uses (and where it comes from) as a hint
                        let cur = self.doc.stored_value(e, *fid).map(|v| v.display()).unwrap_or_default();
                        let (shown, src) = self.doc.value_with_source(e, *fid);
                        let hint = match (&shown, src) {
                            (_, map_core::doc::Source::Unknown) => "未知".to_string(),
                            (Some(v), _) => v.display(),
                            (None, _) => String::new(),
                        };
                        let buf = self.cell_edit.entry((e, *fid)).or_insert(cur.clone());
                        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(96.0).hint_text(RichText::new(hint).italics()));
                        let r = if self.doc.year.is_some() && cur.is_empty() { r.on_hover_text(src.label()) } else { r };
                        if r.lost_focus() && *buf != cur {
                            let v = Value::parse(f.kind, buf);
                            let ok = buf.trim().is_empty() || v.is_some();
                            let year = self.doc.year;
                            let attempted = self.cell_edit.remove(&(e, *fid)).unwrap_or_default();
                            if ok {
                                self.edit("编辑数据", |d| {
                                    d.set_value(e, *fid, year, v);
                                    Ok(())
                                });
                            } else {
                                self.set_status(crate::trf!("“{}” 不是有效的{}", attempted, f.name), true);
                            }
                        } else if !r.has_focus() && *buf != cur {
                            *buf = cur;
                        }
                    }
                    ui.end_row();
                }
            });
        });
        let other = stats.into_cache();
        self.stats_cache = Some((doc_now, cur, other));
    }

    fn fields_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(crate::i18n::tr("自定义字段")).strong());
        let fields: Vec<_> = self.doc.fields.iter().map(|(id, f)| (*id, f.clone())).collect();
        // the table is wider than the panel: it scrolls sideways instead of
        // widening the panel (which then stayed wide on every other tab)
        egui::ScrollArea::horizontal().id_salt("fields_scroll").show(ui, |ui| {
            egui::Grid::new("fields").striped(true).num_columns(6).show(ui, |ui| {
                ui.label(RichText::new(crate::i18n::tr("名称")).strong());
                ui.label(RichText::new(crate::i18n::tr("键")).strong());
                ui.label(RichText::new(crate::i18n::tr("类型/表达式")).strong());
                ui.label(RichText::new(crate::i18n::tr("年份继承")).strong()).on_hover_text(crate::i18n::tr("选择年份而该年没有记录时如何取值"));
                ui.label(RichText::new(crate::i18n::tr("基础值年份")).strong()).on_hover_text(crate::i18n::tr("基础值实际对应的年份（例如“最新数据 = 2020 年”）；不设则基础值不属于任何年份"));
                ui.label("");
                ui.end_row();
                ui.label(crate::i18n::tr("面积 (内置)"));
                ui.monospace(AREA_KEY);
                ui.label(crate::i18n::tr("由几何计算"));
                ui.label("");
                ui.label("");
                ui.label("");
                ui.end_row();
                for (id, f) in fields {
                    ui.label(&f.name);
                    ui.monospace(&f.key);
                    ui.label(match (&f.expr, f.kind) {
                        (Some(e), _) => format!("= {e}"),
                        (None, FieldKind::Number) => "数值".into(),
                        (None, FieldKind::Text) => "文本".into(),
                        (None, FieldKind::Bool) => "布尔".into(),
                        (None, FieldKind::Category) => "分类".into(),
                    });
                    if f.expr.is_none() {
                        use map_core::doc::Inherit;
                        let default = Inherit::default_for(f.kind);
                        let text = |p: Option<Inherit>| match p {
                            None => format!("默认（{}）", default.name()),
                            Some(p) => p.name().to_string(),
                        };
                        let mut pick = None;
                        egui::ComboBox::from_id_salt(("inherit", id.0)).selected_text(text(f.inherit)).show_ui(ui, |ui| {
                            if ui.selectable_label(f.inherit.is_none(), text(None)).clicked() {
                                pick = Some(None);
                            }
                            for p in Inherit::ALL {
                                if p == Inherit::Interpolate && f.kind != FieldKind::Number {
                                    continue;
                                }
                                if ui.selectable_label(f.inherit == Some(p), p.name()).clicked() {
                                    pick = Some(Some(p));
                                }
                            }
                        });
                        if let Some(p) = pick {
                            self.edit("修改年份继承", |d| {
                                d.fields.get_mut(&id).unwrap().inherit = p;
                                Ok(())
                            });
                        }
                        let mut by = f.base_year;
                        let mut on = by.is_some();
                        ui.horizontal(|ui| {
                            let mut changed = ui.checkbox(&mut on, "").changed();
                            if on {
                                let mut y = by.unwrap_or(2020);
                                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.2).suffix(crate::i18n::tr(" 年"))).changed();
                                by = Some(y);
                            } else {
                                by = None;
                            }
                            if changed {
                                self.edit("修改基础值年份", |d| {
                                    d.fields.get_mut(&id).unwrap().base_year = by;
                                    Ok(())
                                });
                            }
                        });
                    } else {
                        ui.label("");
                        ui.label("");
                    }
                    if ui.small_button(crate::i18n::tr("删除")).clicked() {
                        self.edit("删除字段", |d| {
                            d.fields.remove(&id);
                            let keys: Vec<_> = d.values.keys().filter(|(_, f)| *f == id).copied().collect();
                            for k in keys {
                                d.values.remove(&k);
                            }
                            Ok(())
                        });
                    }
                    ui.end_row();
                }
            });
        });
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("新建字段")).strong());
        let (key, name, kind, expr) = &mut self.new_field;
        egui::Grid::new("newfield").num_columns(2).show(ui, |ui| {
            ui.label(crate::i18n::tr("键 (英文)"));
            ui.text_edit_singleline(key);
            ui.end_row();
            ui.label(crate::i18n::tr("显示名称"));
            ui.text_edit_singleline(name);
            ui.end_row();
            ui.label(crate::i18n::tr("类型"));
            egui::ComboBox::from_id_salt("fkind")
                .selected_text(match kind {
                    FieldKind::Number => crate::i18n::tr("数值"),
                    FieldKind::Text => crate::i18n::tr("文本"),
                    FieldKind::Bool => crate::i18n::tr("布尔"),
                    FieldKind::Category => crate::i18n::tr("分类"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(kind, FieldKind::Number, crate::i18n::tr("数值"));
                    ui.selectable_value(kind, FieldKind::Text, crate::i18n::tr("文本"));
                    ui.selectable_value(kind, FieldKind::Bool, crate::i18n::tr("布尔"));
                    ui.selectable_value(kind, FieldKind::Category, crate::i18n::tr("分类"));
                });
            ui.end_row();
            ui.label(crate::i18n::tr("派生表达式"));
            ui.add(egui::TextEdit::singleline(expr).hint_text(crate::i18n::tr("可选，如 gdp / population")));
            ui.end_row();
        });
        if ui.button(crate::i18n::tr("添加字段")).clicked() {
            let (k, n, kd, e) = self.new_field.clone();
            let n = if n.trim().is_empty() { k.clone() } else { n };
            let e = (!e.trim().is_empty()).then_some(e);
            if self.edit("新建字段", |d| d.add_field(&k, &n, kd, e)).is_some() {
                self.new_field = (String::new(), String::new(), FieldKind::Number, String::new());
            }
        }
        ui.weak(crate::i18n::tr("表达式支持 + - * / ( )、min max abs sqrt ln log10 pow，可引用其他字段的键和 area_km2。"));
    }

    fn theme_tab(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.doc.theme;
        let before = t.clone();
        ui.horizontal(|ui| {
            ui.radio_value(&mut t.mode, ThemeMode::Political, crate::i18n::tr("政治地图"));
            ui.radio_value(&mut t.mode, ThemeMode::Choropleth, crate::i18n::tr("分级设色"));
        });
        if t.mode == ThemeMode::Choropleth {
            let mut keys: Vec<(String, String)> = vec![(AREA_KEY.into(), "面积 (km²)".into())];
            keys.extend(self.doc.fields.values().filter(|f| f.kind == FieldKind::Number).map(|f| (f.key.clone(), f.name.clone())));
            let cur = keys.iter().find(|k| k.0 == t.field_key).map(|k| k.1.clone()).unwrap_or(t.field_key.clone());
            egui::Grid::new("theme").num_columns(2).show(ui, |ui| {
                ui.label(crate::i18n::tr("指标"));
                egui::ComboBox::from_id_salt("tfield").selected_text(cur).show_ui(ui, |ui| {
                    for (k, n) in &keys {
                        ui.selectable_value(&mut t.field_key, k.clone(), n);
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("分级方法"));
                egui::ComboBox::from_id_salt("tmethod")
                    .selected_text(match t.method {
                        ClassMethod::EqualInterval => crate::i18n::tr("等间距"),
                        ClassMethod::Quantile => crate::i18n::tr("分位数"),
                        ClassMethod::Manual => crate::i18n::tr("手动阈值"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut t.method, ClassMethod::EqualInterval, crate::i18n::tr("等间距"));
                        ui.selectable_value(&mut t.method, ClassMethod::Quantile, crate::i18n::tr("分位数"));
                        ui.selectable_value(&mut t.method, ClassMethod::Manual, crate::i18n::tr("手动阈值"));
                    });
                ui.end_row();
                if t.method == ClassMethod::Manual {
                    ui.label(crate::i18n::tr("阈值（逗号分隔）"));
                    let mut s = t.manual_breaks.iter().map(|v| format_num(*v)).collect::<Vec<_>>().join(", ");
                    if ui.text_edit_singleline(&mut s).changed() {
                        t.manual_breaks = s.split([',', '，', ' ']).filter_map(|x| x.trim().parse().ok()).collect();
                    }
                } else {
                    ui.label(crate::i18n::tr("分级数"));
                    ui.add(egui::Slider::new(&mut t.classes, 2..=9));
                }
                ui.end_row();
                ui.label(crate::i18n::tr("跨年份"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut t.scale, ClassScale::PerYear, crate::i18n::tr("逐年分级"))
                        .on_hover_text(crate::i18n::tr("按所选年份的数值分级：每年的颜色各自拉开，但不同年份同一颜色的数值不同"));
                    ui.radio_value(&mut t.scale, ClassScale::AllYears, crate::i18n::tr("各年统一"))
                        .on_hover_text(crate::i18n::tr("把所有年份的数值放在一起分级：同一颜色在每一年代表同样的数值，便于比较年份、逐年导出与动画"));
                });
                ui.end_row();
                ui.label(crate::i18n::tr("色带"));
                ui.vertical(|ui| {
                    for (name, ramp) in RAMPS {
                        ui.horizontal(|ui| {
                            for c in ramp.iter() {
                                swatch(ui, *c);
                            }
                            if ui.selectable_label(t.ramp == *ramp, *name).clicked() {
                                t.ramp = ramp.to_vec();
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("无数据颜色"));
                color_edit(ui, &mut t.missing);
                ui.end_row();
            });
        }
        if *t != before {
            self.dirty = true;
        }
        if self.doc.theme.mode == ThemeMode::Choropleth {
            if self.doc.theme.scale == ClassScale::AllYears {
                ui.weak(match &self.fixed_scale {
                    Some(f) if f.years.len() > 1 => crate::trf!(
                        "统一分级依据 {} 个年份（{}–{} 年：变化年份与数据记录年份），范围 {} – {}。",
                        f.years.len(),
                        f.years[0],
                        f.years[f.years.len() - 1],
                        format_num(f.lo),
                        format_num(f.hi)
                    ),
                    Some(_) => "只有一个年份，统一分级与逐年分级相同。".into(),
                    None => "地图还没有年份（没有随年份的变化或数据记录），按当前数值分级。".into(),
                });
            }
            self.year_control(ui);
        }
        ui.separator();
        ui.label(RichText::new(crate::trf!("图例：{}", self.theme.title)).strong());
        for item in self.theme.legend.clone() {
            ui.horizontal(|ui| {
                swatch(ui, item.color);
                ui.label(item.label);
            });
        }
    }

    fn style_tab(&mut self, ui: &mut egui::Ui) {
        // named styles of drawn features and the palette
        ui.push_id("feature-styles", |ui| self.styles_section(ui));
        let before = (self.doc.style.clone(), self.doc.meta.clone());
        ui.label(RichText::new(crate::i18n::tr("世界")).strong());
        egui::Grid::new("world").num_columns(2).show(ui, |ui| {
            ui.label(crate::i18n::tr("名称"));
            ui.text_edit_singleline(&mut self.doc.meta.name);
            ui.end_row();
            if let Some(paper) = self.doc.paper {
                ui.label(crate::i18n::tr("画布尺寸"));
                ui.label(format!("{} × {} px", paper.width, paper.height));
                ui.end_row();
            } else {
                ui.label(crate::i18n::tr("星球半径"));
                ui.add(egui::DragValue::new(&mut self.doc.meta.planet_radius_km).range(100.0..=1.0e6).speed(10.0).suffix(" km"));
                ui.end_row();
            }
            // the world's own calendar: only how years are shown
            let c = &mut self.doc.calendar;
            ui.label(crate::i18n::tr("纪年"));
            ui.checkbox(&mut c.on, crate::i18n::tr("使用自定义纪年")).on_hover_text(crate::i18n::tr("只改变年份的显示（时间轴、历史、导出标题）；内部年份与已有数据不变"));
            ui.end_row();
            if c.on {
                ui.label(crate::i18n::tr("纪元名"));
                ui.add(egui::TextEdit::singleline(&mut c.era).desired_width(120.0));
                ui.end_row();
                ui.label(crate::i18n::tr("元年"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut c.epoch).speed(0.5)).on_hover_text(crate::i18n::tr("这个内部年份显示为元年（第 1 年）"));
                    ui.weak(crate::i18n::tr("（内部年份）"));
                });
                ui.end_row();
                ui.label(crate::i18n::tr("之前的年份"));
                ui.add(egui::TextEdit::singleline(&mut c.before).desired_width(60.0)).on_hover_text(crate::i18n::tr("元年之前的年份写在纪元名后，如“前”"));
                ui.end_row();
                ui.label(crate::i18n::tr("格式"));
                ui.add(egui::TextEdit::singleline(&mut c.pattern).desired_width(120.0)).on_hover_text(crate::i18n::tr("{era} 为纪元名，{n} 为第几年，如“{era}{n}年”"));
                ui.end_row();
                ui.label(crate::i18n::tr("示例"));
                let y = self.doc.year.unwrap_or(c.epoch + 99);
                ui.weak(format!("{y} → {}，{} → {}", c.show(y), c.epoch - 1, c.show(c.epoch - 1)));
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            use map_core::FaceKind;
            ui.label(crate::i18n::tr("背景（未被任何面覆盖的区域）"));
            let cur = self.doc.background_kind();
            for (k, n) in [(FaceKind::Water, "海洋"), (FaceKind::Land, "陆地")] {
                if ui.radio(cur == k, n).clicked() && cur != k {
                    let label = if k == FaceKind::Land { "背景改为陆地" } else { "背景改为海洋" };
                    self.edit(label, move |d| {
                        d.set_background_kind(k);
                        Ok(())
                    });
                }
            }
        });
        if self.doc.background_kind() == map_core::FaceKind::Land {
            ui.weak(crate::i18n::tr("背景是陆地时：可用领土画笔把它分给国家；用“绘制湖泊”或“变为水域”画出海洋；跨越边界的绘制会整体重建。"));
        }
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("样式")).strong());
        // map themes: the whole base map's look in one click
        ui.horizontal_wrapped(|ui| {
            let cur = map_core::basemap::current(&self.doc.style).map(|t| t.name);
            for t in &map_core::basemap::THEMES {
                if theme_card(ui, t.name, &t.style(), cur == Some(t.name)).on_hover_text(t.about).clicked() {
                    // its font: the first of its list this computer has
                    let all = map_io::fonts::families();
                    let font = t.fonts.iter().find(|f| all.iter().any(|x| x.family == **f)).map(|f| f.to_string());
                    self.edit(&format!("地图主题：{}", t.name), |d| {
                        t.apply(&mut d.style, font);
                        Ok(())
                    });
                }
            }
        });
        if map_core::basemap::current(&self.doc.style).is_none() {
            ui.weak(crate::i18n::tr("（自定义：已在主题之上改动）"));
        }
        let has_terrain = self.doc.terrain.is_some();
        let mut remove_terrain = false;
        let s = &mut self.doc.style;
        egui::Grid::new("style").num_columns(3).show(ui, |ui| {
            ui.label(crate::i18n::tr("海洋（背景）"));
            color_edit(ui, &mut s.ocean);
            ui.label("");
            ui.end_row();
            ui.label(crate::i18n::tr("无主陆地"));
            color_edit(ui, &mut s.land);
            ui.label("");
            ui.end_row();
            ui.label(crate::i18n::tr("湖泊"));
            color_edit(ui, &mut s.lake);
            ui.label("");
            ui.end_row();
            for (n, c, w) in [
                ("海岸线", &mut s.coast, &mut s.coast_width),
                ("国界", &mut s.border, &mut s.border_width),
                ("内部边界", &mut s.internal, &mut s.internal_width),
                ("河流", &mut s.river, &mut s.river_width),
            ] {
                ui.label(n);
                color_edit(ui, c);
                ui.add(egui::Slider::new(w, 0.0..=6.0).suffix(" px"));
                ui.end_row();
            }
            ui.label(crate::i18n::tr("标注文字"));
            color_edit(ui, &mut s.label);
            ui.label("");
            ui.end_row();
            ui.label(crate::i18n::tr("国家颜色色调")).on_hover_text(crate::i18n::tr("政治地图上国家的颜色向这个颜色靠近（0 为原色）"));
            color_edit(ui, &mut s.tone);
            ui.add(egui::Slider::new(&mut s.tone_mix, 0.0..=0.9).fixed_decimals(2));
            ui.end_row();
            ui.label(crate::i18n::tr("海岸晕带")).on_hover_text(crate::i18n::tr("沿海岸在海中画出由深到浅的三道色带（0 为不画）"));
            color_edit(ui, &mut s.coast_glow);
            ui.add(egui::Slider::new(&mut s.coast_glow_width, 0.0..=24.0).suffix(" px"));
            ui.end_row();
            if has_terrain {
                ui.label(crate::i18n::tr("地形晕渲")).on_hover_text(crate::i18n::tr("新世界向导生成的地形，以西北光照的山体阴影显示在陆地上（0 为不显示）"));
                ui.add(egui::Slider::new(&mut s.hillshade, 0.0..=1.0).fixed_decimals(2));
                remove_terrain = ui.small_button(crate::i18n::tr("删除地形")).on_hover_text(crate::i18n::tr("删除高程数据（可撤销）；陆地与国家不变")).clicked();
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("国家名称字体"));
            crate::fonts_ui::font_picker(ui, "labels", &mut s.label_font, &mut s.label_bold);
        });
        ui.checkbox(&mut s.show_labels, crate::i18n::tr("显示国家名称"));
        ui.checkbox(&mut s.admin_typography, crate::i18n::tr("行政区分级字体"))
            .on_hover_text(crate::i18n::tr("国名用黑体，省级用宋体，市县级用楷体，基层使用较小的无衬线字。二维、三维与导出共享此设置。"));
        ui.checkbox(&mut s.show_cities, crate::i18n::tr("显示城市"));
        if remove_terrain {
            self.edit("删除地形", |d| {
                d.terrain = None;
                Ok(())
            });
        }
        if (self.doc.style.clone(), self.doc.meta.clone()) != before {
            self.dirty = true;
        }
    }

    pub fn export_window(&mut self, ctx: &egui::Context) {
        let mut open = self.export.open;
        let mut run = false;
        let mut browse = false;
        let mut run_atlas = false;
        let mut open_page = false;
        egui::Window::new(crate::i18n::tr("导出")).open(&mut open).resizable(false).show(ctx, |ui| {
            let x = &mut self.export;
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut x.format, ExportFormat::Png, crate::i18n::tr("PNG 图片"));
                ui.radio_value(&mut x.format, ExportFormat::Svg, crate::i18n::tr("SVG 矢量图"));
                ui.radio_value(&mut x.format, ExportFormat::Pdf, "PDF").on_hover_text(crate::i18n::tr("矢量 PDF：文字转为字形轮廓，不需要字体，任何阅读器与打印机看到的都一样"));
                if self.doc.paper.is_none() {
                    ui.radio_value(&mut x.format, ExportFormat::GeoJsonCountries, crate::i18n::tr("GeoJSON（国家）"));
                    ui.radio_value(&mut x.format, ExportFormat::GeoJsonFaces, crate::i18n::tr("GeoJSON（地块）"));
                    ui.radio_value(&mut x.format, ExportFormat::GeoJsonRivers, crate::i18n::tr("GeoJSON（河流与线路）"));
                }
                ui.radio_value(&mut x.format, ExportFormat::Csv, crate::i18n::tr("CSV 数据"));
            });
            if self.doc.paper.is_none() { ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut x.format, ExportFormat::ShpCountries, crate::i18n::tr("Shapefile（国家）"));
                ui.radio_value(&mut x.format, ExportFormat::ShpFaces, crate::i18n::tr("Shapefile（地块）"));
                ui.radio_value(&mut x.format, ExportFormat::ShpRivers, crate::i18n::tr("Shapefile（河流与线路）"));
            }); }
            if matches!(x.format, ExportFormat::ShpCountries | ExportFormat::ShpFaces | ExportFormat::ShpRivers) {
                ui.weak(crate::i18n::tr("经纬度（WGS 84），属性为 UTF-8；同名的 .shx、.dbf、.prj、.cpg 一并写出。字段名超过 10 字节会被截短。"));
            }
            if matches!(x.format, ExportFormat::Png | ExportFormat::Svg | ExportFormat::Pdf) || (x.per_year && x.anim.is_some()) {
                let proj = x.projection;
                let min_side = if self.doc.paper.is_some() { 64 } else { 200 };
                egui::Grid::new("exp").num_columns(2).show(ui, |ui| {
                    ui.label(crate::i18n::tr("宽 × 高 (px)"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut x.width).range(min_side..=20000));
                        ui.label("×");
                        ui.add_enabled(!x.current_view, egui::DragValue::new(&mut x.height).range(min_side..=20000));
                    });
                    ui.end_row();
                    if self.doc.paper.is_none() {
                        ui.label(crate::i18n::tr("投影"));
                        egui::ComboBox::from_id_salt("eproj").selected_text(x.projection.name()).show_ui(ui, |ui| {
                            for p in Projection::ALL {
                                ui.selectable_value(&mut x.projection, p, p.name());
                            }
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("范围"));
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut x.current_view, crate::i18n::tr("仅当前二维视图范围")).changed() && x.current_view {
                                x.bounds = None;
                                x.lon0 = None;
                            }
                            if x.bounds.is_some() && !x.current_view {
                                ui.weak(crate::i18n::tr("版式的局部范围"));
                                if ui.small_button(crate::i18n::tr("改为全图")).clicked() {
                                    x.bounds = None;
                                }
                            }
                        });
                        ui.end_row();
                    }
                    ui.label(crate::i18n::tr("标题"));
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut x.title);
                        if ui.button(crate::i18n::tr("版式视图…")).on_hover_text(crate::i18n::tr("按导出的样子显示整页：拖动标题、图例、比例尺、指北针，添加插图框")).clicked() {
                            open_page = true;
                        }
                    });
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut x.labels, crate::i18n::tr("标注"));
                    ui.checkbox(&mut x.legend, crate::i18n::tr("图例"));
                    if self.doc.paper.is_none() {
                        ui.checkbox(&mut x.graticule, crate::i18n::tr("经纬网"));
                        ui.checkbox(&mut x.scale_bar, crate::i18n::tr("比例尺")).on_hover_text(crate::i18n::tr("按图中心的比例；全球图上各处比例不同，会注明“仅适用于图中心附近”"));
                        ui.checkbox(&mut x.north_arrow, crate::i18n::tr("指北针")).on_hover_text(crate::i18n::tr("指向图中心处的北方"));
                    }
                });
                if self.doc.paper.is_none() {
                    let mut skip = self.doc.style.hide_face_seams_in_export;
                    if ui.checkbox(&mut skip, crate::i18n::tr("导出时隐藏同属地块接缝")).changed() {
                        self.doc.style.hide_face_seams_in_export = skip;
                    }
                }
                // a layout's region is in its own projection's units
                if x.projection != proj {
                    x.bounds = None;
                }
            }
            ui.separator();
            browse = x.per_year_ui(ui, &self.doc);
            ui.separator();
            let ask = self.layouts_ui(ui);
            browse |= ask.browse;
            run_atlas = ask.run_atlas;
            ui.separator();
            let x = &mut self.export;
            if let Some(b) = &self.batch {
                let n = b.done.load(std::sync::atomic::Ordering::Relaxed);
                ui.add(egui::ProgressBar::new(n as f32 / b.total.max(1) as f32).text(crate::trf!("{n} / {} 年", b.total)));
                if ui.button(crate::i18n::tr("取消")).clicked() {
                    b.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            } else if ui.add_enabled(!x.per_year || !x.out_dir.trim().is_empty(), egui::Button::new(if x.per_year { crate::i18n::tr("开始逐年导出") } else { crate::i18n::tr("导出到文件…") })).clicked() {
                run = true;
            }
        });
        self.export.open = open;
        if open_page {
            self.page.open = true;
        }
        if browse {
            if let Some(d) = self.file_dialog().set_title(crate::i18n::tr("选择输出文件夹")).pick_folder() {
                self.export.out_dir = d.display().to_string();
            }
        }
        if run_atlas {
            self.start_atlas();
        }
        if run {
            if self.export.per_year {
                self.start_batch();
            } else {
                self.run_export();
            }
        }
        self.poll_batch(ctx);
    }

    pub fn export_options(&self) -> ExportOptions {
        let x = &self.export;
        let mut opts = ExportOptions {
            width: x.width,
            height: x.height,
            projection: x.projection,
            lon0: x.lon0.unwrap_or(self.cam2d.lon0),
            bounds: x.bounds,
            labels: x.labels,
            legend: x.legend,
            graticule: x.graticule,
            background: [255, 255, 255, 255],
            title: x.title.clone(),
            scale_bar: x.scale_bar,
            north_arrow: x.north_arrow,
            placement: x.placement.clone(),
        };
        if x.current_view {
            // visible extent of the 2D view (same projection as the view)
            let rect = self.last_flat_rect;
            let c = &self.cam2d;
            let (hw, hh) = (rect.width() as f64 * 0.5 / c.zoom, rect.height() as f64 * 0.5 / c.zoom);
            opts.bounds = Some([c.center[0] - hw, c.center[1] - hh, c.center[0] + hw, c.center[1] + hh]);
            opts.projection = c.proj;
            opts.height = ((x.width as f64) * hh / hw).round().max(1.0) as u32;
        }
        opts
    }

    fn run_export(&mut self) {
        let format = self.export.format;
        let ext = format.ext();
        let Some(path) = self.file_dialog().add_filter(ext.to_uppercase(), &[ext]).set_file_name(format!("{}.{ext}", self.doc.meta.name)).save_file() else {
            return;
        };
        let t = std::time::Instant::now();
        let opts = self.export_options();
        match map_io::batch::write_one(&self.doc, format, &opts, self.font.as_deref().map(|v| v.as_slice()), &path) {
            Ok(()) => self.set_status(crate::trf!("已导出 {}（{:.0} ms）", path.display(), t.elapsed().as_secs_f64() * 1e3), false),
            Err(e) => self.set_status(crate::trf!("导出失败：{e}"), true),
        }
    }

    fn start_batch(&mut self) {
        let x = &self.export;
        let years = x.year_list(&self.doc);
        if years.is_empty() {
            self.set_status(crate::i18n::tr("没有可导出的年份：还没有任何随年份的变化，请改用年份区间"), true);
            return;
        }
        let dir = std::path::PathBuf::from(x.out_dir.trim());
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(kind) = x.anim {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.set_status(crate::trf!("无法创建输出文件夹：{e}"), true);
                return;
            }
            let job = map_io::batch::AnimJob {
                path: dir.join(format!("{}.{}", self.doc.meta.name, kind.ext())),
                years,
                kind,
                opts: self.export_options(),
                font: self.font.as_deref().cloned(),
                frame_ms: (x.frame_s * 1000.0).round().max(50.0) as u32,
            };
            let (d, c, doc) = (done.clone(), cancel.clone(), self.doc.clone());
            let total = job.years.len();
            let handle = std::thread::spawn(move || map_io::batch::export_animation(&doc, &job, &d, &c).map(|p| p.into_iter().collect()));
            self.batch = Some(BatchRun { done, cancel, total, dir, started: std::time::Instant::now(), what: "动画导出", handle: Some(handle) });
            self.set_status(crate::trf!("正在生成 {total} 帧动画…"), false);
            return;
        }
        let job = map_io::batch::Job {
            years,
            dir: dir.clone(),
            stem: self.doc.meta.name.clone(),
            format: x.format,
            opts: self.export_options(),
            font: self.font.as_deref().cloned(),
        };
        let (d, c, doc) = (done.clone(), cancel.clone(), self.doc.clone());
        let total = job.years.len();
        let handle = std::thread::spawn(move || map_io::batch::export_years(&doc, &job, &d, &c));
        self.batch = Some(BatchRun { done, cancel, total, dir, started: std::time::Instant::now(), what: "逐年导出", handle: Some(handle) });
        self.set_status(crate::trf!("逐年导出 {total} 年…"), false);
    }

    fn poll_batch(&mut self, ctx: &egui::Context) {
        let finished = self.batch.as_ref().is_some_and(|b| b.handle.as_ref().is_some_and(|h| h.is_finished()));
        if !finished {
            // keep looking while it runs, also with the export window closed
            if self.batch.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            return;
        }
        let mut b = self.batch.take().unwrap();
        let cancelled = b.cancel.load(std::sync::atomic::Ordering::Relaxed);
        let secs = b.started.elapsed().as_secs_f64();
        match b.handle.take().unwrap().join() {
            Ok(Ok(files)) if cancelled => self.set_status(crate::trf!("已取消{}：已写出 {} 个文件到 {}", b.what, files.len(), b.dir.display()), true),
            Ok(Ok(files)) => self.set_status(crate::trf!("{}完成：{} 个文件写到 {}（{secs:.1} s）", b.what, files.len(), b.dir.display()), false),
            Ok(Err(e)) => self.set_status(crate::trf!("{}失败：{e}", b.what), true),
            Err(_) => self.set_status(crate::trf!("{}线程意外退出", b.what), true),
        }
    }
}

/// A map theme as a small picture: its sea with a coast band, two
/// countries in its tone with a border, and its name in its label colour.
fn theme_card(ui: &mut egui::Ui, name: &str, st: &map_core::doc::Style, selected: bool) -> egui::Response {
    use egui::{pos2, vec2, Color32, Rect, Stroke};
    let c = |x: [u8; 4]| Color32::from_rgba_unmultiplied(x[0], x[1], x[2], x[3]);
    let (rect, resp) = ui.allocate_exact_size(vec2(66.0, 46.0), egui::Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, c(st.ocean));
    let land = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 8.0), pos2(rect.right() - 10.0, rect.bottom() - 16.0));
    if st.coast_glow_width > 0.0 {
        let b = st.glow_bands();
        for (i, w) in [5.0, 3.5, 2.0].into_iter().enumerate() {
            p.rect_filled(land.expand(w), 4.0, c(b[i]));
        }
    }
    let mid = land.center().x;
    p.rect_filled(Rect::from_min_max(land.min, pos2(mid, land.max.y)), 2.0, c(st.toned([231, 160, 140, 255])));
    p.rect_filled(Rect::from_min_max(pos2(mid, land.min.y), land.max), 2.0, c(st.toned([150, 190, 140, 255])));
    p.line_segment([pos2(mid, land.min.y), pos2(mid, land.max.y)], Stroke::new(st.border_width.min(2.0), c(st.border)));
    p.rect_stroke(land, 2.0, Stroke::new(st.coast_width.min(2.0), c(st.coast)), egui::StrokeKind::Middle);
    p.text(pos2(rect.center().x, rect.bottom() - 7.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(10.5), c(st.label));
    let frame = if selected { Stroke::new(2.0, ui.visuals().selection.stroke.color) } else if resp.hovered() { Stroke::new(1.0, ui.visuals().widgets.hovered.fg_stroke.color) } else { Stroke::new(1.0, Color32::from_gray(90)) };
    p.rect_stroke(rect, 4.0, frame, egui::StrokeKind::Inside);
    resp
}
