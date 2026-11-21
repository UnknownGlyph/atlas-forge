//! Pixel stamp editor and the textures shared by placed map symbols.
use crate::{app::App, i18n::tr, view::Tool};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, Vec2};
use map_core::{
    doc::LineId,
    features::FeatureRef,
    pixel_art::{PixelArt, MAX_SIDE},
};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, Weak},
};

const UNDO_LIMIT: usize = 48;

pub(crate) struct PixelTools {
    pub open: bool,
    pub stamp: Option<Arc<PixelArt>>,
    pub placing: bool,
    target: Option<LineId>,
    editor: Editor,
    textures: RefCell<HashMap<*const PixelArt, (Weak<PixelArt>, TextureHandle)>>,
}

impl Default for PixelTools {
    fn default() -> Self {
        Self {
            open: false,
            stamp: None,
            placing: false,
            target: None,
            editor: Editor::new(PixelArt::new(32, 32).unwrap()),
            textures: Default::default(),
        }
    }
}

struct Editor {
    art: PixelArt,
    undo: VecDeque<PixelArt>,
    redo: Vec<PixelArt>,
    brush: Color32,
    eraser: bool,
    width: u16,
    height: u16,
    brush_size: u16,
    zoom: f32,
    grid: bool,
    before: Option<PixelArt>,
    last: Option<[u16; 2]>,
    texture: Option<TextureHandle>,
    changed: bool,
}

impl Editor {
    fn new(art: PixelArt) -> Self {
        let [width, height] = art.size();
        Self {
            art,
            width,
            height,
            undo: VecDeque::new(),
            redo: Vec::new(),
            brush: Color32::from_rgb(200, 57, 70),
            eraser: false,
            brush_size: 1,
            zoom: 12.0,
            grid: true,
            before: None,
            last: None,
            texture: None,
            changed: true,
        }
    }

    fn record(&mut self, before: PixelArt) {
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(before);
        self.redo.clear();
    }

    fn replace(&mut self, next: PixelArt) {
        if self.art != next {
            let before = std::mem::replace(&mut self.art, next);
            self.record(before);
            self.changed = true;
        }
    }

    fn undo(&mut self) {
        if let Some(before) = self.undo.pop_back() {
            self.redo.push(std::mem::replace(&mut self.art, before));
            [self.width, self.height] = self.art.size();
            self.changed = true;
        }
    }

    fn redo(&mut self) {
        if let Some(after) = self.redo.pop() {
            self.undo.push_back(std::mem::replace(&mut self.art, after));
            [self.width, self.height] = self.art.size();
            self.changed = true;
        }
    }

    fn finish_stroke(&mut self) {
        self.last = None;
        if let Some(before) = self.before.take().filter(|before| before != &self.art) {
            self.record(before);
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.label(tr("地图预览"));
        for background in [Color32::from_gray(230), Color32::from_gray(44)] {
            let (panel, _) = ui.allocate_exact_size(egui::vec2(112.0, 64.0), Sense::hover());
            ui.painter().rect_filled(panel, 4.0, background);
            if let Some(texture) = &self.texture {
                for (centre, size) in [(0.31, 52.0), (0.78, 24.0)] {
                    let [w, h] = self.art.drawn_size(size);
                    let rect = Rect::from_center_size(
                        egui::pos2(panel.left() + centre * panel.width(), panel.center().y),
                        egui::vec2(w, h),
                    );
                    ui.painter()
                        .image(texture.id(), rect, uv_rect(), Color32::WHITE);
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.set_width(112.0);
            for color in [
                Color32::from_rgb(200, 57, 70),
                Color32::from_rgb(230, 148, 30),
                Color32::from_rgb(46, 135, 78),
                Color32::from_rgb(40, 101, 205),
                Color32::from_rgb(116, 74, 175),
                Color32::from_rgb(95, 61, 41),
                Color32::WHITE,
                Color32::from_gray(26),
            ] {
                let label = format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b());
                let mut button = egui::Button::new(" ")
                    .fill(color)
                    .min_size(Vec2::splat(17.0));
                if color == self.brush && !self.eraser {
                    button =
                        button.stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                }
                if ui.add(button).on_hover_text(label).clicked() {
                    self.brush = color;
                    self.eraser = false;
                }
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let [w, h] = self.art.size().map(f32::from);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(w, h) * self.zoom, Sense::click_and_drag());
        let zoom = self.zoom;
        let at = |pos: Pos2| {
            if rect.contains(pos) {
                let p = (pos - rect.min) / zoom;
                (p.x >= 0.0 && p.y >= 0.0 && p.x < w && p.y < h).then_some([p.x as u16, p.y as u16])
            } else {
                None
            }
        };
        let color = if self.eraser {
            [0; 4]
        } else {
            self.brush.to_srgba_unmultiplied()
        };
        enum StrokeEvent {
            Begin,
            At(Option<[u16; 2]>),
            End,
        }
        let (samples, ended) = ui.input(|i| {
            let mut active = self.before.is_some();
            let mut samples = Vec::new();
            for event in &i.events {
                match *event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } if at(pos).is_some()
                        && (response.contains_pointer()
                            || response.is_pointer_button_down_on()) =>
                    {
                        active = true;
                        samples.push(StrokeEvent::Begin);
                        samples.push(StrokeEvent::At(at(pos)));
                    }
                    egui::Event::PointerMoved(pos) if active => {
                        samples.push(StrokeEvent::At(at(pos)))
                    }
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    } if active => {
                        samples.push(StrokeEvent::At(at(pos)));
                        samples.push(StrokeEvent::End);
                        active = false;
                    }
                    _ => {}
                }
            }
            (samples, !i.pointer.primary_down())
        });
        for event in samples {
            match event {
                StrokeEvent::Begin => {
                    self.finish_stroke();
                    self.before = Some(self.art.clone());
                }
                StrokeEvent::At(spot) => {
                    if let Some(cell) = spot {
                        self.changed |= self.art.stroke(
                            self.last.unwrap_or(cell),
                            cell,
                            self.brush_size,
                            color,
                        );
                    }
                    self.last = spot;
                }
                StrokeEvent::End => self.finish_stroke(),
            }
        }
        if ended {
            self.finish_stroke();
        }
        if response.secondary_clicked() {
            if let Some(color) = response
                .interact_pointer_pos()
                .and_then(at)
                .and_then(|p| self.art.pixel(p))
            {
                if color[3] != 0 {
                    self.brush = rgba(color);
                    self.eraser = false;
                }
            }
        }
        if self.changed {
            let image = color_image(&self.art);
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "pixel draft",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
            self.changed = false;
        }
        let painter = ui.painter_at(rect);
        let clip = rect.intersect(ui.clip_rect());
        if !clip.is_positive() {
            return;
        }
        let tile = (self.zoom * 2.0).max(8.0);
        let (left, right) = (
            ((clip.left() - rect.left()) / tile).floor() as u32,
            ((clip.right() - rect.left()) / tile).ceil() as u32,
        );
        let (top, bottom) = (
            ((clip.top() - rect.top()) / tile).floor() as u32,
            ((clip.bottom() - rect.top()) / tile).ceil() as u32,
        );
        for y in top..bottom {
            for x in left..right {
                let point = rect.min + Vec2::new(x as f32, y as f32) * tile;
                let color = if (x + y) % 2 == 0 {
                    Color32::from_gray(196)
                } else {
                    Color32::from_gray(148)
                };
                painter.rect_filled(Rect::from_min_size(point, Vec2::splat(tile)), 0.0, color);
            }
        }
        if let Some(texture) = &self.texture {
            painter.image(texture.id(), rect, uv_rect(), Color32::WHITE);
        }
        if self.grid && self.zoom >= 7.0 {
            let line = egui::Stroke::new(1.0, Color32::from_black_alpha(50));
            for x in 0..=self.art.size()[0] {
                let x = rect.left() + f32::from(x) * self.zoom;
                if (clip.left()..=clip.right()).contains(&x) {
                    painter.vline(x, clip.y_range(), line);
                }
            }
            for y in 0..=self.art.size()[1] {
                let y = rect.top() + f32::from(y) * self.zoom;
                if (clip.top()..=clip.bottom()).contains(&y) {
                    painter.hline(clip.x_range(), y, line);
                }
            }
        }
    }
}

fn rgba(color: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

fn color_image(art: &PixelArt) -> egui::ColorImage {
    egui::ColorImage::new(
        art.size().map(usize::from),
        art.pixels().iter().copied().map(rgba).collect(),
    )
}

fn uv_rect() -> Rect {
    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0))
}

impl PixelTools {
    pub fn paint(&self, painter: &egui::Painter, art: &Arc<PixelArt>, rect: Rect, fade: f32) {
        let key = Arc::as_ptr(art);
        let mut textures = self.textures.borrow_mut();
        let (_, image) = textures.entry(key).or_insert_with(|| {
            (
                Arc::downgrade(art),
                painter.ctx().load_texture(
                    "pixel mark",
                    color_image(art),
                    egui::TextureOptions::NEAREST,
                ),
            )
        });
        painter.image(
            image.id(),
            rect,
            uv_rect(),
            Color32::WHITE.gamma_multiply(fade),
        );
    }

    pub fn close_target(&mut self) {
        self.target = None;
        self.open = false;
        self.editor.finish_stroke();
        self.textures.get_mut().clear();
    }
}

impl App {
    pub(crate) fn open_pixel_editor(&mut self, target: Option<LineId>) {
        if let Some(id) = target {
            let Some(symbol) = self.doc.symbols.get(&id) else {
                return;
            };
            if self.doc.locked_in_effect(FeatureRef::Symbol(id)) {
                self.set_status(tr("请先解锁这个符号。"), true);
                return;
            }
            if let Some(art) = &symbol.pixels {
                self.pixel_tools.editor = Editor::new((**art).clone());
            }
        }
        self.pixel_tools.target = target;
        self.pixel_tools.open = true;
    }

    pub(crate) fn pixel_window(&mut self, ctx: &egui::Context) {
        if ctx.cumulative_frame_nr().is_multiple_of(128) {
            self.pixel_tools
                .textures
                .get_mut()
                .retain(|_, (art, _)| art.strong_count() > 0);
        }
        if self.pixel_tools.open && ctx.input(|i| i.viewport().close_requested()) {
            self.pixel_tools.editor.finish_stroke();
            self.pixel_tools.open = false;
            self.pixel_tools.target = None;
        }
        if !self.pixel_tools.open {
            return;
        }
        let tools = &mut self.pixel_tools;
        let target = tools.target;
        let editor = &mut tools.editor;
        if !ctx.egui_wants_keyboard_input() {
            use egui::{Key, KeyboardShortcut, Modifiers};
            let command = |key, modifiers| {
                ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
            };
            if command(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)
                || command(Key::Y, Modifiers::COMMAND)
            {
                editor.redo();
            } else if command(Key::Z, Modifiers::COMMAND) {
                editor.undo();
            }
        }
        let (mut place, mut apply, mut export, mut import, mut close) =
            (false, false, false, false, false);
        let result = egui::Modal::new(egui::Id::unique("pixel-stamp-editor")).show(ctx, |ui| {
            ui.set_width(ui.ctx().content_rect().width().clamp(320.0, 720.0) * 0.90);
            ui.heading(tr("彩色像素画笔"));
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut editor.eraser, false, tr("画笔"));
                ui.selectable_value(&mut editor.eraser, true, tr("橡皮"));
                ui.color_edit_button_srgba(&mut editor.brush);
                ui.add(
                    egui::DragValue::new(&mut editor.brush_size)
                        .range(1..=32)
                        .suffix(" px"),
                )
                .on_hover_text(tr("画笔宽度"));
                ui.checkbox(&mut editor.grid, tr("网格"));
                ui.add(egui::Slider::new(&mut editor.zoom, 2.0..=24.0).text(tr("画布缩放")));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("画布尺寸"));
                ui.add(egui::DragValue::new(&mut editor.width).range(1..=MAX_SIDE));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut editor.height).range(1..=MAX_SIDE));
                if ui.button(tr("调整尺寸")).clicked() {
                    if let Ok(art) = editor.art.resized(editor.width, editor.height) {
                        editor.replace(art);
                    }
                }
                if ui
                    .add_enabled(!editor.undo.is_empty(), egui::Button::new(tr("撤销")))
                    .clicked()
                {
                    editor.undo();
                }
                if ui
                    .add_enabled(!editor.redo.is_empty(), egui::Button::new(tr("重做")))
                    .clicked()
                {
                    editor.redo();
                }
                if ui
                    .add_enabled(editor.art.has_ink(), egui::Button::new(tr("清空画布")))
                    .clicked()
                {
                    let [w, h] = editor.art.size();
                    editor.replace(PixelArt::new(w, h).unwrap());
                }
            });
            ui.weak(tr("拖动绘制，右键吸色。方格表示透明背景。"));
            let height = (ui.ctx().content_rect().height() - 188.0).clamp(156.0, 460.0);
            if ui.available_width() >= 456.0 {
                ui.horizontal_top(|ui| {
                    let width = ui.available_width() - 136.0;
                    egui::ScrollArea::both()
                        .id_salt("pixel-stamp-canvas")
                        .max_width(width)
                        .max_height(height)
                        .show(ui, |ui| editor.canvas(ui));
                    ui.vertical(|ui| editor.preview(ui));
                });
            } else {
                egui::ScrollArea::both()
                    .id_salt("pixel-stamp-canvas")
                    .max_height(height)
                    .show(ui, |ui| editor.canvas(ui));
            }
            ui.horizontal_wrapped(|ui| {
                let has_ink = editor.art.has_ink();
                place = ui
                    .add_enabled(has_ink, egui::Button::new(tr("作为新标志放置")))
                    .clicked();
                if target.is_some() {
                    apply = ui
                        .add_enabled(has_ink, egui::Button::new(tr("更新所选标志")))
                        .clicked();
                }
                export = ui
                    .add_enabled(has_ink, egui::Button::new(tr("导出透明 PNG…")))
                    .clicked();
                import = ui.button(tr("打开 PNG…")).clicked();
                close = ui.button(tr("关闭")).clicked();
            });
        });
        if import {
            if let Some(path) = self.file_dialog().add_filter("PNG", &["png"]).pick_file() {
                match map_io::png::load_pixel_art(&path) {
                    Ok(art) => {
                        let editor = &mut self.pixel_tools.editor;
                        [editor.width, editor.height] = art.size();
                        editor.replace(art);
                    }
                    Err(error) => self.set_status(error, true),
                }
            }
        }
        if export {
            if let Some(path) = self
                .file_dialog()
                .add_filter("PNG", &["png"])
                .set_file_name("pixel-mark.png")
                .save_file()
            {
                match map_io::png::save_pixel_art(&self.pixel_tools.editor.art, &path) {
                    Ok(()) => self.set_status(tr("像素图已导出。"), false),
                    Err(error) => self.set_status(error, true),
                }
            }
        }
        if place || apply {
            let art = Arc::new(self.pixel_tools.editor.art.clone());
            if place {
                self.set_tool(Tool::Symbol);
                self.studio.show_map();
                self.pixel_tools.stamp = Some(art);
                self.pixel_tools.placing = true;
                self.set_status(tr("单击地图或画板放置像素标志。"), false);
            } else if let Some(id) = target {
                self.edit("更新像素标志", |doc| {
                    if doc.locked_in_effect(FeatureRef::Symbol(id)) {
                        return Err(tr("请先解锁这个符号。").to_owned());
                    }
                    doc.symbols.get_mut(&id).ok_or("符号不存在")?.pixels = Some(art);
                    Ok(())
                });
            }
        }
        if place || apply || close || result.should_close() {
            self.pixel_tools.editor.finish_stroke();
            self.pixel_tools.open = false;
            self.pixel_tools.target = None;
        }
    }
}
