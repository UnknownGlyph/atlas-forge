//! The illustrated Yuanyang lessons. Instructions and figures occupy
//! separate pages with a chapter index. Launching a tool returns to the map.
use crate::{actions::Act, app::App, view::Tool};
use crate::i18n::{text, tr};
use eframe::egui;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Practice,
    Figure,
}

#[derive(Default)]
pub(crate) struct Tutorial {
    pub open: bool,
    step: usize,
    page: Page,
    original_size: bool,
    figure_step: Option<usize>,
    picture: Option<(&'static str, egui::TextureHandle)>,
}

#[derive(Deserialize)]
struct Instruction {
    title: String,
    body: String,
    #[serde(default)]
    figure: Option<String>,
}

#[derive(Deserialize)]
struct Chapter {
    id: String,
    title: String,
    image: String,
    goal: String,
    caption: String,
    steps: Vec<Instruction>,
    check: String,
    help: String,
}

fn chapters() -> &'static [Chapter] {
    static CHAPTERS: OnceLock<Vec<Chapter>> = OnceLock::new();
    CHAPTERS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../docs/tutorial/lessons.json"))
            .expect("bundled tutorial chapters")
    })
}

struct Media {
    id: &'static str,
    image: &'static str,
    bytes: &'static [u8],
    entry: &'static str,
    action: Act,
}
macro_rules! lesson {
    ($id:literal, $name:literal, $entry:literal, $act:expr) => {
        Media {
            id: $id,
            image: $name,
            bytes: include_bytes!(concat!("../../../docs/tutorial/", $name, ".png")),
            entry: $entry,
            action: $act,
        }
    };
}

const MEDIA: &[Media] = &[
    lesson!("paper", "01-blank", "打开世界资料", Act::WorldBook),
    lesson!(
        "coast",
        "02-coast",
        "选择绘制陆地",
        Act::Tool(Tool::DrawLand)
    ),
    lesson!(
        "countries",
        "03-countries",
        "选择领土画笔",
        Act::Tool(Tool::Assign)
    ),
    lesson!(
        "provinces",
        "04-provinces",
        "选择地图对象",
        Act::Tool(Tool::Select)
    ),
    lesson!(
        "regions",
        "05-regions",
        "选择分区画笔",
        Act::Tool(Tool::Assign)
    ),
    lesson!("water", "06-water", "选择河流工具", Act::Tool(Tool::River)),
    lesson!("cities", "07-cities", "选择添加城市", Act::Tool(Tool::City)),
    lesson!("history", "08-history", "打开事件时间线", Act::Events),
    lesson!("diplomacy", "09-diplomacy", "打开外交关系", Act::Diplomacy),
    lesson!("board", "10-board", "打开世界资料", Act::WorldBook),
    lesson!("export", "11-atlas", "打开导出设置", Act::Export),
    lesson!(
        "districts",
        "12-districts",
        "选择地图对象",
        Act::Tool(Tool::Select)
    ),
    lesson!(
        "navigation",
        "13-navigation",
        "切换二维与三维分屏",
        Act::Layout(crate::app::Layout::Split)
    ),
    lesson!(
        "checkpoints",
        "14-checkpoints",
        "打开保存点",
        Act::Checkpoints
    ),
    lesson!(
        "imports",
        "15-imports",
        "选择配准参考图",
        Act::Tool(Tool::Align)
    ),
    lesson!(
        "data",
        "16-data",
        "返回地图打开数据表",
        Act::Tool(Tool::Select)
    ),
    lesson!("features", "17-tools", "选择钢笔工具", Act::Tool(Tool::Pen)),
    lesson!("climate", "18-climate", "打开行星环境与气候", Act::Climate),
    lesson!("stars", "19-stars", "打开星图", Act::StarChart),
    lesson!("library", "20-library", "打开世界资料", Act::WorldBook),
    lesson!("automation", "21-automation", "打开本地插件", Act::Plugins),
    lesson!("final-review", "22-review", "打开版式视图", Act::PageView),
];

macro_rules! diagrams {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../docs/tutorial/", $name, ".png")))),*]
    };
}

const DIAGRAMS: &[(&str, &[u8])] = diagrams!(
    "14-preferences",
    "14-restore",
    "14-autosave",
    "16-cities",
    "18-planet",
    "19-lunar",
    "19-galaxy",
    "20-document",
);

impl App {
    pub(crate) fn tutorial_window(&mut self, ctx: &egui::Context) {
        if !self.tutorial.open {
            return;
        }
        let state = &mut self.tutorial;
        let mut open = true;
        let mut action = None;
        let chapters = chapters();
        let body_height = (ctx.content_rect().height() - 264.0).clamp(240.0, 590.0);
        egui::Window::new(crate::i18n::tr("创作教程"))
            .id(egui::Id::unique("creative-tutorial"))
            .open(&mut open)
            .default_size([1005.0, 760.0])
            .min_size([530.0, 395.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(crate::i18n::tr("从零绘制远洋人民共和国与四邻国"));
                ui.add_space(8.0);
                let sidebar = ui.available_width() >= 760.0;
                let previous_chapter = state.step;
                if !sidebar {
                    egui::ComboBox::from_id_salt("tutorial-chapter")
                        .selected_text(text(&chapters[state.step].title))
                        .show_ui(ui, |ui| {
                            for (i, chapter) in chapters.iter().enumerate() {
                                ui.selectable_value(&mut state.step, i, text(&chapter.title));
                            }
                        });
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut state.page, Page::Practice, crate::i18n::tr("操作与验收"));
                    if ui
                        .selectable_label(
                            state.page == Page::Figure && state.figure_step.is_none(),
                            crate::i18n::tr("阶段大图"),
                        )
                        .clicked()
                    {
                        state.figure_step = None;
                        state.page = Page::Figure;
                    }
                });
                ui.separator();

                ui.horizontal_top(|ui| {
                    if sidebar {
                        ui.vertical(|ui| {
                            ui.set_width(216.0);
                            ui.strong(crate::i18n::tr("教程目录"));
                            ui.add_space(8.0);
                            egui::ScrollArea::vertical()
                                .id_salt("tutorial-navigation")
                                .max_height(body_height)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    for (i, chapter) in chapters.iter().enumerate() {
                                        let title = format!("{:02}　{}", i + 1, text(&chapter.title));
                                        if ui.selectable_label(state.step == i, title).clicked() {
                                            state.step = i;
                                            state.page = Page::Practice;
                                        }
                                    }
                                });
                        });
                        ui.add_space(14.0);
                    }
                    if state.step != previous_chapter {
                        state.page = Page::Practice;
                        state.figure_step = None;
                    }
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        let chapter = &chapters[state.step];
                        let media = &MEDIA[state.step];
                        ui.heading(text(&chapter.title));
                        ui.add_space(6.0);
                        match state.page {
                            Page::Practice => {
                                if let Some(index) = instructions(ui, chapter, body_height) {
                                    state.figure_step = Some(index);
                                    state.page = Page::Figure;
                                }
                            }
                            Page::Figure => figure(ui, ctx, state, chapter, media, body_height),
                        }
                    });
                });
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(state.step > 0, egui::Button::new(crate::i18n::tr("上一节")))
                        .clicked()
                    {
                        state.step -= 1;
                        state.page = Page::Practice;
                        state.figure_step = None;
                    }
                    if ui
                        .add_enabled(state.step + 1 < chapters.len(), egui::Button::new(crate::i18n::tr("下一节")))
                        .clicked()
                    {
                        state.step += 1;
                        state.page = Page::Practice;
                        state.figure_step = None;
                    }
                    ui.weak(crate::trf!(
                        "第 {} 节，共 {} 节",
                        state.step + 1,
                        chapters.len()
                    ));
                    ui.add_space(8.0);
                    if ui.button(tr(MEDIA[state.step].entry)).clicked() {
                        action = Some(MEDIA[state.step].action);
                    }
                    if ui.button(crate::i18n::tr("打开教程成品")).clicked() {
                        action = Some(Act::TutorialExample);
                    }
                });
            });
        self.tutorial.open = open;
        if let Some(act) = action.filter(|act| self.can(*act)) {
            self.tutorial.open = false;
            self.run(act);
        }
    }
}

fn instructions(ui: &mut egui::Ui, chapter: &Chapter, height: f32) -> Option<usize> {
    let mut picture = None;
    egui::ScrollArea::vertical()
        .id_salt(("tutorial-instructions", &chapter.id))
        .max_height(height)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text(&chapter.goal)).size(16.0));
            ui.add_space(13.0);
            for (index, item) in chapter.steps.iter().enumerate() {
                ui.label(
                    egui::RichText::new(format!("{}. {}", index + 1, text(&item.title)))
                        .size(16.0)
                        .strong(),
                );
                ui.label(egui::RichText::new(text(&item.body)).size(15.0));
                if ui.link(crate::i18n::tr("查看本步配图")).clicked() {
                    picture = Some(index);
                }
                ui.add_space(11.0);
            }
            ui.separator();
            ui.strong(crate::i18n::tr("验收"));
            ui.label(egui::RichText::new(text(&chapter.check)).size(15.0));
            ui.add_space(10.0);
            ui.strong(crate::i18n::tr("遇到问题"));
            ui.label(egui::RichText::new(text(&chapter.help)).size(15.0));
        });
    picture
}

fn figure(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut Tutorial,
    chapter: &Chapter,
    media: &Media,
    height: f32,
) {
    debug_assert_eq!(chapter.image, media.image);
    debug_assert_eq!(chapter.id, media.id);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.original_size, false, crate::i18n::tr("适合窗口"));
        ui.selectable_value(&mut state.original_size, true, crate::i18n::tr("原始尺寸"));
    });
    let step = state.figure_step.and_then(|i| chapter.steps.get(i));
    let (name, bytes) = step
        .and_then(|s| s.figure.as_deref())
        .and_then(|name| DIAGRAMS.iter().find(|(key, _)| *key == name).copied())
        .unwrap_or((media.image, media.bytes));
    if state.picture.as_ref().is_none_or(|(key, _)| *key != name) {
        state.picture = image::load_from_memory(bytes).ok().map(|image| {
            let image = image.into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            (
                name,
                ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR),
            )
        });
    }
    if let Some((_, texture)) = &state.picture {
        let size = texture.size_vec2();
        let width = ui.available_width();
        let height = (height - 55.0).max(190.0);
        let scale = if state.original_size {
            1.0
        } else {
            (width / size.x).min(height / size.y).min(1.0)
        };
        egui::ScrollArea::both()
            .id_salt(("tutorial-figure", state.step))
            .max_height(height)
            .max_width(width)
            .show(ui, |ui| {
                ui.image((texture.id(), size * scale));
            });
    }
    ui.add_space(7.0);
    if let Some(step) = step {
        ui.strong(text(&step.title));
        if name == media.image {
            ui.label(text(&chapter.caption));
        }
        if ui.link(crate::i18n::tr("返回操作步骤")).clicked() {
            state.page = Page::Practice;
        }
    } else {
        ui.label(text(&chapter.caption));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_chapter_has_workable_steps_and_its_own_illustration() {
        assert_eq!(chapters().len(), MEDIA.len());
        for (chapter, media) in chapters().iter().zip(MEDIA) {
            assert_eq!((&*chapter.id, &*chapter.image), (media.id, media.image));
            assert!(chapter.steps.len() >= 5);
            assert!(!chapter.check.is_empty() && !chapter.help.is_empty());
            image::load_from_memory(media.bytes).unwrap();
            for name in chapter
                .steps
                .iter()
                .filter_map(|step| step.figure.as_deref())
            {
                let picture = DIAGRAMS.iter().find(|(key, _)| *key == name).unwrap();
                image::load_from_memory(picture.1).unwrap();
            }
        }
    }
}
