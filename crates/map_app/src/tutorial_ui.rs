//! The illustrated Ithaer lessons, worked in the Creative Studio on the
//! local three-state project. Instructions and figures occupy separate pages
//! with a chapter index. Launching a tool returns to the map.
use crate::{actions::Act, app::App, view::Tool};
use crate::i18n::{text, tr};
use eframe::egui;
use serde::Deserialize;
use std::collections::HashMap;
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
    /// Pictures already decoded, by name; decoded the first time they are shown.
    pictures: HashMap<&'static str, egui::TextureHandle>,
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
        serde_json::from_str(include_str!("../../../docs/tutorial/ithaer/lessons.json"))
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
    lesson!("studio", "ithaer/01-studio", "查看快捷键一览", Act::Shortcuts),
    lesson!(
        "find",
        "ithaer/02-find",
        "切换二维与三维分屏",
        Act::Layout(crate::app::Layout::Split)
    ),
    lesson!(
        "hierarchy",
        "ithaer/03-hierarchy",
        "选择地图对象",
        Act::Tool(Tool::Select)
    ),
    lesson!(
        "territory",
        "ithaer/04-territory",
        "选择领土画笔",
        Act::Tool(Tool::Assign)
    ),
    lesson!(
        "borders",
        "ithaer/05-borders",
        "选择切分工具",
        Act::Tool(Tool::Split)
    ),
    lesson!(
        "rivers",
        "ithaer/06-rivers",
        "选择河流工具",
        Act::Tool(Tool::River)
    ),
    lesson!(
        "marks",
        "ithaer/07-marks",
        "打开彩色像素画笔",
        Act::PixelBrush
    ),
    lesson!("history", "ithaer/08-history", "打开事件时间线", Act::Events),
    lesson!(
        "data",
        "ithaer/09-data",
        "返回地图打开数据表",
        Act::Tool(Tool::Select)
    ),
    lesson!("library", "ithaer/10-library", "打开世界资料", Act::WorldBook),
    lesson!("export", "ithaer/11-export", "打开导出设置", Act::Export),
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
            // Keep the tutorial above the workspace's floating map controls.
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .default_size([1005.0, 760.0])
            .min_size([530.0, 395.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(crate::i18n::tr("在创作工作室里完成伊瑟尔三国"));
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
                                if let Some(index) = instructions(ui, ctx, &mut state.pictures, chapter, media, body_height) {
                                    state.figure_step = index;
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
                    if ui.button(crate::i18n::tr("打开伊瑟尔三国测试工程")).clicked() {
                        action = Some(Act::NewTestWorld);
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

/// A bundled picture as a texture, decoded once.
fn texture(
    ctx: &egui::Context,
    cache: &mut HashMap<&'static str, egui::TextureHandle>,
    name: &'static str,
    bytes: &'static [u8],
) -> Option<egui::TextureHandle> {
    if !cache.contains_key(name) {
        let image = image::load_from_memory(bytes).ok()?.into_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        cache.insert(name, ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR));
    }
    cache.get(name).cloned()
}

/// A picture in the text, as wide as the column; true when clicked.
fn inline_picture(ui: &mut egui::Ui, texture: &egui::TextureHandle, max_height: f32, caption: &str) -> bool {
    let size = texture.size_vec2();
    let scale = (ui.available_width() / size.x).min(max_height / size.y).min(1.0);
    let clicked = ui
        .add(egui::Image::new((texture.id(), size * scale)).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::ZoomIn)
        .on_hover_text(crate::i18n::tr("点击查看大图"))
        .clicked();
    if !caption.is_empty() {
        ui.label(egui::RichText::new(caption).size(13.5).weak());
    }
    clicked
}

/// The chapter as one column of text and pictures. `Some(step)` asks for
/// the large view of a picture (`None` inside: the chapter's own picture).
fn instructions(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    cache: &mut HashMap<&'static str, egui::TextureHandle>,
    chapter: &Chapter,
    media: &Media,
    height: f32,
) -> Option<Option<usize>> {
    let mut enlarge = None;
    egui::ScrollArea::vertical()
        .id_salt(("tutorial-instructions", &chapter.id))
        .max_height(height)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text(&chapter.goal)).size(16.0));
            ui.add_space(10.0);
            if let Some(picture) = texture(ctx, cache, media.image, media.bytes) {
                if inline_picture(ui, &picture, (height * 0.62).clamp(220.0, 460.0), text(&chapter.caption)) {
                    enlarge = Some(None);
                }
            }
            ui.add_space(14.0);
            for (index, item) in chapter.steps.iter().enumerate() {
                ui.label(
                    egui::RichText::new(format!("{}. {}", index + 1, text(&item.title)))
                        .size(16.0)
                        .strong(),
                );
                ui.label(egui::RichText::new(text(&item.body)).size(15.0));
                // a step with a picture of its own shows it right under its text
                let own = item
                    .figure
                    .as_deref()
                    .and_then(|name| DIAGRAMS.iter().find(|(key, _)| *key == name).copied());
                if let Some((name, bytes)) = own {
                    ui.add_space(6.0);
                    if let Some(picture) = texture(ctx, cache, name, bytes) {
                        if inline_picture(ui, &picture, (height * 0.5).clamp(200.0, 380.0), "") {
                            enlarge = Some(Some(index));
                        }
                    }
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
    enlarge
}

/// One picture at a larger size, with the way back to the text.
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
        if ui.button(crate::i18n::tr("返回教程")).clicked() {
            state.page = Page::Practice;
        }
        ui.separator();
        ui.selectable_value(&mut state.original_size, false, crate::i18n::tr("适合窗口"));
        ui.selectable_value(&mut state.original_size, true, crate::i18n::tr("原始尺寸"));
    });
    let step = state.figure_step.and_then(|i| chapter.steps.get(i));
    let (name, bytes) = step
        .and_then(|s| s.figure.as_deref())
        .and_then(|name| DIAGRAMS.iter().find(|(key, _)| *key == name).copied())
        .unwrap_or((media.image, media.bytes));
    if let Some(picture) = texture(ctx, &mut state.pictures, name, bytes) {
        let size = picture.size_vec2();
        let width = ui.available_width();
        let height = (height - 55.0).max(190.0);
        let scale = if state.original_size {
            1.0
        } else {
            (width / size.x).min(height / size.y).min(1.0)
        };
        let mut back = false;
        egui::ScrollArea::both()
            .id_salt(("tutorial-figure", state.step))
            .max_height(height)
            .max_width(width)
            .show(ui, |ui| {
                back = ui
                    .add(egui::Image::new((picture.id(), size * scale)).sense(egui::Sense::click()))
                    .on_hover_text(crate::i18n::tr("点击返回教程"))
                    .clicked();
            });
        if back {
            state.page = Page::Practice;
        }
    }
    ui.add_space(7.0);
    match step {
        Some(step) => {
            ui.strong(text(&step.title));
        }
        None => {
            ui.label(text(&chapter.caption));
        }
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
