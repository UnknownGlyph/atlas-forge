//! The timeline bar under the map and ownership-history views (M3, ADR-7).

use crate::app::App;
use crate::view::Tool;
use eframe::egui::{self, RichText};
use map_core::{EntityId, FaceKind};

type StudyKey = (u64, u64, Option<u32>);
pub(crate) struct StudyYearCache {
    key: StudyKey,
    pub(crate) years: std::sync::Arc<std::collections::BTreeSet<i32>>,
}

impl App {
    pub(crate) fn study_years(&self) -> std::sync::Arc<std::collections::BTreeSet<i32>> {
        let key = (self.history.generation(), map_core::world_book::WorldBook::current(&self.doc), self.focus);
        let mut cache = self.study_year_cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.key != key) {
            *cache = Some(StudyYearCache {
                key, years: std::sync::Arc::new(map_core::playback::study_years(&self.doc, self.active_focus())),
            });
        }
        cache.as_ref().unwrap().years.clone()
    }

    /// Select a year (None = the latest state, edits for all years).
    pub fn set_year(&mut self, y: Option<i32>) {
        if let Some(y) = y {
            self.timeline_last_year = y;
        }
        self.doc.set_year(y);
        self.cell_edit.clear();
    }

    pub fn entity_name(&self, e: Option<EntityId>) -> String {
        e.and_then(|e| self.doc.entities.get(&e)).map(|x| x.name.clone()).unwrap_or_else(|| "无主".into())
    }

    pub fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let years = self.study_years();
        ui.horizontal(|ui| {
            let mut on = self.doc.year.is_some();
            if ui.checkbox(&mut on, RichText::new(crate::i18n::tr("时间轴")).strong()).changed() {
                self.set_year(on.then_some(self.timeline_last_year));
            }
            let Some(y) = self.doc.year else {
                ui.weak(crate::trf!("关闭：显示最新状态，归属修改对所有年份生效（已有历史的面需先选年份）。{} 个变化年份。", years.len()));
                return;
            };
            let lo = years.first().copied().unwrap_or(y).min(y).min(1800) - 10;
            let hi = years.last().copied().unwrap_or(y).max(y).max(2025) + 10;
            // a local view with years: the timeline runs over them
            let span = self.active_focus().and_then(|f| f.years.map(|ys| (f.name.clone(), ys)));
            let (lo, hi) = match &span {
                Some((name, (a, b))) => {
                    ui.label(RichText::new(crate::trf!("局部：{name}")).color(crate::focus_ui::ACCENT).strong()).on_hover_text(crate::i18n::tr("时间轴只走这个局部视图的年份；“上一变化/下一变化”只看这一地区"));
                    if ui.small_button(crate::i18n::tr("退出")).clicked() {
                        self.exit_focus();
                    }
                    (*a, (*b).max(*a + 1))
                }
                None => (lo, hi),
            };
            let (prev, next) = self.change_neighbours();
            if ui.add_enabled(prev.is_some(), egui::Button::new(crate::i18n::tr("< 上一变化"))).clicked() {
                self.set_year(prev);
            }
            let mut v = y;
            let w = (ui.available_width() - 520.0).clamp(160.0, 900.0);
            ui.spacing_mut().slider_width = w;
            let slider = ui.add(egui::Slider::new(&mut v, lo..=hi).integer().show_value(false));
            if slider.changed() {
                self.set_year(Some(v));
            }
            // the events' markers over it
            self.event_markers(ui, slider.rect, lo, hi);
            if ui.add_enabled(next.is_some(), egui::Button::new(crate::i18n::tr("下一变化 >"))).clicked() {
                self.set_year(next);
            }
            let mut v = y;
            if ui.add(egui::DragValue::new(&mut v).speed(0.2).suffix(crate::i18n::tr(" 年"))).changed() {
                self.set_year(Some(v));
            }
            // the world's own calendar
            if self.doc.calendar.on {
                ui.label(RichText::new(format!("＝ {}", self.doc.calendar.show(y))).strong()).on_hover_text(crate::i18n::tr("自定义纪年（样式标签“世界”中设置）；内部仍按左边的年份保存"));
            }
            let hint = match self.tool {
                Tool::EditVertex if self.vertex_dated => crate::trf!("拖动/删除国界顶点 = 从 {y} 年起移交土地，更早的地图不变（工具栏可改为修改所有年份的形状）。"),
                Tool::EditVertex => crate::i18n::tr("编辑顶点修正的是所有年份共用的形状（会改变每个年份的地图）。").to_string(),
                Tool::Assign => crate::trf!("归属修改从 {y} 年起生效，更早的年份不变。"),
                Tool::DrawLand | Tool::DrawLake => crate::trf!("画出的陆地/水域从 {y} 年起出现（之前保持原样）；修正所有年份的海岸请关闭时间轴。"),
                _ => crate::trf!("{} 个变化年份", years.len()),
            };
            ui.weak(hint);
        });
    }

    /// Ownership history of the selected face and territorial change years
    /// of the selected country (click a year to go there).
    pub fn history_section(&mut self, ui: &mut egui::Ui) {
        let mut goto = None;
        // land/water of the selected face through time
        if let Some(f) = self.selected_face.filter(|f| *f != map_core::topo::BACKGROUND && self.doc.topo.faces.contains_key(f)) {
            let (base, changes) = self.doc.kind_history(f);
            let name = |k: FaceKind| if k == FaceKind::Land { "陆地" } else { "水域" };
            ui.separator();
            ui.label(RichText::new(crate::i18n::tr("所选地块的海陆")).strong());
            if !changes.is_empty() {
                ui.label(crate::trf!("起初：{}", name(base)));
                for (y, k) in &changes {
                    ui.horizontal(|ui| {
                        if ui.small_button(self.doc.calendar.show(*y)).clicked() {
                            goto = Some(*y);
                        }
                        ui.label(crate::trf!("起：{}", name(*k)));
                    });
                }
            }
            let now = self.doc.topo.faces[&f].kind;
            let other = if now == FaceKind::Land { FaceKind::Water } else { FaceKind::Land };
            let label = match self.doc.year {
                Some(y) => format!("从 {y} 年起变为{}", name(other)),
                None => format!("改为{}（所有年份）", name(other)),
            };
            if ui.add_enabled(self.doc.year.is_some() || changes.is_empty(), egui::Button::new(label)).on_disabled_hover_text(crate::i18n::tr("海陆随年份变化：请先在时间轴选择年份")).clicked() {
                let what = if other == FaceKind::Land { "变为陆地" } else { "变为水域" };
                self.edit(what, |d| d.record_kind(f, other));
            }
            // reshape this piece's borders (shared with neighbours, all years):
            // previewed first, applied only on confirmation
            ui.horizontal(|ui| {
                use map_core::shape::LineStyle;
                ui.label(crate::i18n::tr("边界："));
                for (style, text) in [(LineStyle::Smooth, "平滑…"), (LineStyle::Natural, "自然化…")] {
                    if ui.small_button(text).on_hover_text(crate::i18n::tr("预览这块地全部边界的新形状（与邻国共享的边界一起改变，所有年份共用）；确认后才应用")).clicked() {
                        self.start_reshape(f, style);
                    }
                }
            });
        }
        // shares of the selected face on a share layer
        if let Some(f) = self.selected_face.filter(|_| self.doc.active_is_shares()) {
            let sh = self.doc.shares(f);
            ui.separator();
            ui.label(RichText::new(crate::i18n::tr("所选地块的比例")).strong());
            if sh.is_empty() {
                ui.weak(crate::i18n::tr("（未设置）"));
            }
            let total: u32 = sh.values().map(|v| *v as u32).sum();
            for (e, bp) in &sh {
                ui.label(format!("{}：{:.1}%", self.entity_name(Some(*e)), *bp as f64 / 100.0));
            }
            if total < map_core::doc::FULL_SHARE as u32 && !sh.is_empty() {
                ui.weak(crate::trf!("未知 / 其他：{:.1}%", (map_core::doc::FULL_SHARE as u32 - total) as f64 / 100.0));
            }
            let (_, changes) = self.doc.share_history(f);
            if !changes.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::i18n::tr("变化年份"));
                    for (y, _) in changes {
                        if ui.small_button(format!("{y}")).clicked() {
                            goto = Some(y);
                        }
                    }
                });
            }
        }
        if let Some(f) = self.selected_face.filter(|f| self.doc.topo.faces.get(f).is_some_and(|x| x.kind == FaceKind::Land)) {
            let (base, changes) = self.doc.face_history(f);
            if !changes.is_empty() {
                ui.separator();
                ui.label(RichText::new(crate::i18n::tr("所选地块的归属历史")).strong());
                ui.label(crate::trf!("起初：{}", self.entity_name(base)));
                for (y, o) in changes {
                    ui.horizontal(|ui| {
                        if ui.small_button(self.doc.calendar.show(y)).clicked() {
                            goto = Some(y);
                        }
                        ui.label(crate::trf!("起：{}", self.entity_name(o)));
                    });
                }
            }
        }
        if let Some(e) = self.selected.filter(|e| self.doc.entities.contains_key(e)) {
            let (base, changes) = self.doc.parent_history(e);
            if !changes.is_empty() {
                ui.separator();
                ui.label(RichText::new(crate::i18n::tr("上级关系历史")).strong());
                ui.label(crate::trf!("起初：{}", base.map_or("（顶层）".to_string(), |p| self.entity_name(Some(p)))));
                for (y, p) in changes {
                    ui.horizontal(|ui| {
                        if ui.small_button(self.doc.calendar.show(y)).clicked() {
                            goto = Some(y);
                        }
                        ui.label(crate::trf!("起：{}", p.map_or("（顶层）".to_string(), |p| self.entity_name(Some(p)))));
                    });
                }
            }
        }
        if let Some(e) = self.selected.filter(|e| self.doc.entities.get(e).is_some_and(|x| x.kind != map_core::EntityKind::City)) {
            let ys = self.doc.entity_change_years(e);
            if !ys.is_empty() {
                ui.separator();
                ui.label(RichText::new(crate::i18n::tr("领土变化年份")).strong());
                ui.horizontal_wrapped(|ui| {
                    for y in ys {
                        if ui.selectable_label(self.doc.year == Some(y), format!("{y}")).clicked() {
                            goto = Some(y);
                        }
                    }
                });
            }
        }
        if let Some(y) = goto {
            self.set_year(Some(y));
        }
    }
}
