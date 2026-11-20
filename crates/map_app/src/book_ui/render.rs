use super::{metadata, Action, BookUi};
use eframe::egui::{self, RichText};
use map_core::{
    world_book::{Article, BookId, Node, WorldBook},
    Document,
};
use map_io::dossier::{self, Block, Paragraph, Span};

pub(super) fn article(
    ui: &mut egui::Ui,
    doc: &Document,
    book: &WorldBook,
    id: BookId,
    article: &Article,
    state: &mut BookUi,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.editor, false, crate::i18n::tr("阅读"));
        ui.selectable_value(&mut state.editor, true, crate::i18n::tr("编辑"));
        if ui.button(crate::i18n::tr("导出文档")).clicked() {
            actions.push(Action::Export(vec![id], false));
        }
        if ui.button(crate::i18n::tr("附件单独导出")).clicked() {
            actions.push(Action::ExportAssets(vec![id]));
        }
    });
    if state.editor {
        let mut value = article.clone();
        let mut changed = ui.text_edit_singleline(&mut value.name).changed();
        changed |= metadata(ui, &mut value.meta);
        let mut markup = None;
        ui.horizontal_wrapped(|ui| {
            for (title, before, after, sample) in [
                ("标题", "\n## ", "\n", "小标题"),
                ("粗体", "**", "**", "重点"),
                ("斜体", "*", "*", "文字"),
                ("列表", "\n- ", "\n", "列表项"),
                (
                    "表格",
                    "\n",
                    "\n",
                    "| 项目 | 资料 |\n| --- | --- |\n| 名称 | 内容 |",
                ),
                ("引用", "\n> ", "\n", "引文"),
                ("链接", "[", "](https://example.com)", "链接文字"),
            ] {
                if ui.small_button(title).clicked() {
                    markup = Some((before, after, sample.to_owned()));
                }
            }
            ui.menu_button(crate::i18n::tr("内部链接"), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(250.)
                    .show(ui, |ui| {
                        for (other, label) in super::catalog(book, doc, Node::Article(id), true)
                            .into_iter()
                            .filter(|(node, _)| *node != Node::Article(id))
                        {
                            if ui.button(&label).clicked() {
                                let safe = label.replace(['[', ']', '\r', '\n'], " ");
                                value
                                    .markdown
                                    .push_str(&format!("\n\n[{safe}]({})\n", other.uri()));
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            });
        });
        let out = egui::TextEdit::multiline(&mut value.markdown)
            .id(ui.scope_id().with(("book-edit", id)))
            .desired_width(f32::INFINITY)
            .desired_rows(9)
            .show(ui);
        changed |= out.response.changed();
        if let Some((before, after, sample)) = markup {
            let insert = out
                .cursor_range
                .map(|range| {
                    let sorted = range.sorted_cursors();
                    let to_byte = |index| {
                        value
                            .markdown
                            .char_indices()
                            .nth(index)
                            .map(|p| p.0)
                            .unwrap_or(value.markdown.len())
                    };
                    (
                        to_byte(sorted[0].index.into()),
                        to_byte(sorted[1].index.into()),
                    )
                })
                .unwrap_or((value.markdown.len(), value.markdown.len()));
            let inside = if insert.0 == insert.1 {
                sample
            } else {
                value.markdown[insert.0..insert.1].to_owned()
            };
            value
                .markdown
                .replace_range(insert.0..insert.1, &format!("{before}{inside}{after}"));
            changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(crate::i18n::tr("插入图片或附件…")).clicked() {
                actions.push(Action::Attach(id));
            }
            if ui.button(crate::i18n::tr("插入当前画板截图")).clicked() {
                actions.push(Action::Snapshot(id));
            }
        });
        if changed {
            // Changes must reach the document before any subsequent attachment
            // or navigation action in the same frame.
            actions.insert(0, Action::WriteArticle(id, value));
        }
    } else {
        if !article.meta.description.is_empty() {
            ui.label(&article.meta.description);
        }
        ui.weak(
            [article.meta.category.as_str(), article.meta.tags.as_str()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        );
        if let Some(from) = article.meta.from {
            ui.weak(
                article
                    .meta
                    .until
                    .map_or(from.to_string(), |until| format!("{from}–{until}")),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt(("book-read", id))
            .max_height(470.0)
            .show(ui, |ui| {
                for block in dossier::rich(&article.markdown) {
                    match block {
                        Block::Para(kind, text) => inline(ui, book, state, &text, kind, actions),
                        Block::Rule => {
                            ui.separator();
                        }
                        Block::Table(rows) => {
                            egui::Grid::new(ui.next_auto_id())
                                .striped(true)
                                .show(ui, |ui| {
                                    for (i, row) in rows.iter().enumerate() {
                                        for cell in row {
                                            inline(
                                                ui,
                                                book,
                                                state,
                                                cell,
                                                if i == 0 {
                                                    Paragraph::Heading(5)
                                                } else {
                                                    Paragraph::Text
                                                },
                                                actions,
                                            );
                                        }
                                        ui.end_row();
                                    }
                                });
                        }
                    }
                    ui.add_space(5.0);
                }
            });
    }
    if !article.attachments.is_empty() {
        ui.collapsing("本文档附件", |ui| {
            for asset in &article.attachments {
                if let Some(file) = book.assets.get(asset) {
                    if ui
                        .link(format!(
                            "{} · {:.1} KB",
                            file.name,
                            file.bytes.0.len() as f64 / 1024.0
                        ))
                        .clicked()
                    {
                        actions.push(Action::Open(Node::Asset(*asset), None));
                    }
                }
            }
        });
    }
}
fn inline(
    ui: &mut egui::Ui,
    book: &WorldBook,
    state: &mut BookUi,
    spans: &[Span],
    kind: Paragraph,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for span in spans {
            if span.image {
                let loaded = span
                    .href
                    .as_deref()
                    .and_then(dossier::asset_id)
                    .and_then(|id| texture(ui, book, state, id));
                if let Some(texture) = loaded {
                    let size = texture.size_vec2();
                    let scale = (ui.available_width().max(130.0) / size.x)
                        .min(340.0 / size.y)
                        .min(1.0);
                    ui.image((texture.id(), size * scale));
                    if !span.text.is_empty() {
                        ui.weak(&span.text);
                    }
                } else {
                    ui.weak(crate::trf!("图片 · {}", span.text));
                }
                continue;
            }
            let mut text = RichText::new(&span.text);
            if span.bold {
                text = text.strong();
            }
            if span.italic {
                text = text.italics();
            }
            if span.code {
                text = text.monospace();
            }
            if span.strike {
                text = text.strikethrough();
            }
            match kind {
                Paragraph::Heading(1) => text = text.size(24.0).strong(),
                Paragraph::Heading(2) => text = text.size(20.0).strong(),
                Paragraph::Heading(_) => text = text.size(16.0).strong(),
                Paragraph::Code => text = text.monospace(),
                Paragraph::Quote => text = text.italics(),
                _ => {}
            }
            if let Some(href) = span.href.as_deref() {
                if let Some(node) =
                    Node::parse(href).or_else(|| dossier::asset_id(href).map(Node::Asset))
                {
                    if ui.link(text).clicked() {
                        actions.push(Action::Open(node, None));
                    }
                    continue;
                }
                if dossier::is_web(href) {
                    ui.hyperlink_to(text, href);
                    continue;
                }
            }
            ui.label(text);
        }
    });
}
pub(super) fn texture(
    ui: &egui::Ui,
    book: &WorldBook,
    state: &mut BookUi,
    id: BookId,
) -> Option<egui::TextureHandle> {
    let asset = book.assets.get(&id)?;
    if let Some((seen, texture)) = state.textures.get(&id) {
        if std::sync::Arc::ptr_eq(&seen.0, &asset.bytes.0) {
            return Some(texture.clone());
        }
    }
    let image = image::load_from_memory(&asset.bytes.0).ok()?;
    let img = image.thumbnail(1200, 850).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    let texture = ui.ctx().load_texture(
        format!("book-image-{id}"),
        color,
        egui::TextureOptions::LINEAR,
    );
    state
        .textures
        .insert(id, (asset.bytes.clone(), texture.clone()));
    Some(texture)
}
