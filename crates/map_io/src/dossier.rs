//! Formatted documents and portable dossiers. The same Markdown parse is
//! used for desktop previews, Word/PDF and the exported web reader.
mod office;

use base64::Engine;
use map_core::{
    world_book::{Article, Attachment, BookId, Node, WorldBook},
    Document,
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub href: Option<String>,
    pub image: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Paragraph {
    #[default]
    Text,
    Heading(u8),
    Quote,
    List,
    Code,
}
#[derive(Clone, Debug)]
pub enum Block {
    Para(Paragraph, Vec<Span>),
    Table(Vec<Vec<Vec<Span>>>),
    Rule,
}
pub fn rich(markdown: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = Vec::new();
    let mut line = Vec::new();
    let mut style = Paragraph::Text;
    let mut paint = Span::default();
    let mut link: Option<String> = None;
    let mut photo: Option<String> = None;
    let mut photo_caption = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut table = Vec::<Vec<Vec<Span>>>::new();
    let mut row = Vec::<Vec<Span>>::new();
    let mut in_table = false;
    let flush = |out: &mut Vec<Block>, line: &mut Vec<Span>, style| {
        if !line.is_empty() {
            out.push(Block::Para(style, std::mem::take(line)));
        }
    };
    for event in Parser::new_ext(markdown, opts) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut out, &mut line, style);
                style = Paragraph::Heading(level as u8);
            }
            Event::Start(Tag::BlockQuote(_)) => style = Paragraph::Quote,
            Event::Start(Tag::CodeBlock(_)) => {
                style = Paragraph::Code;
                paint.code = true;
            }
            Event::Start(Tag::List(n)) => lists.push(n),
            Event::Start(Tag::Item) => {
                flush(&mut out, &mut line, style);
                style = Paragraph::List;
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n = n.saturating_add(1);
                        marker
                    }
                    _ => "• ".into(),
                };
                line.push(Span {
                    text: format!("{}{}", "  ".repeat(lists.len().saturating_sub(1)), marker),
                    ..Default::default()
                });
            }
            Event::Start(Tag::Strong) => paint.bold = true,
            Event::Start(Tag::Emphasis) => paint.italic = true,
            Event::Start(Tag::Strikethrough) => paint.strike = true,
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.into_string()),
            Event::Start(Tag::Image { dest_url, .. }) => {
                photo = Some(dest_url.into_string());
                photo_caption.clear();
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut out, &mut line, style);
                in_table = true;
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut line)),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                table.push(std::mem::take(&mut row))
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
                out.push(Block::Table(std::mem::take(&mut table)));
            }
            Event::End(TagEnd::Strong) => paint.bold = false,
            Event::End(TagEnd::Emphasis) => paint.italic = false,
            Event::End(TagEnd::Strikethrough) => paint.strike = false,
            Event::End(TagEnd::Link) => link = None,
            Event::End(TagEnd::Image) => line.push(Span {
                text: std::mem::take(&mut photo_caption),
                href: photo.take(),
                image: true,
                ..Default::default()
            }),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                style = if lists.is_empty() {
                    Paragraph::Text
                } else {
                    Paragraph::List
                };
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::Item,
            ) if !in_table => {
                flush(&mut out, &mut line, style);
                if !matches!(style, Paragraph::Quote) {
                    style = if lists.is_empty() {
                        Paragraph::Text
                    } else {
                        Paragraph::List
                    };
                }
                paint.code = false;
            }
            Event::End(TagEnd::BlockQuote(_)) => style = Paragraph::Text,
            Event::Rule => out.push(Block::Rule),
            Event::SoftBreak | Event::HardBreak => line.push(Span {
                text: if matches!(event, Event::HardBreak) {
                    "\n".into()
                } else {
                    " ".into()
                },
                ..paint.clone()
            }),
            Event::TaskListMarker(done) => line.push(Span {
                text: if done { "☑ ".into() } else { "☐ ".into() },
                ..Default::default()
            }),
            Event::Text(ref t)
            | Event::Code(ref t)
            | Event::Html(ref t)
            | Event::InlineHtml(ref t) => {
                if photo.is_some() {
                    photo_caption.push_str(&t);
                } else {
                    line.push(Span {
                        text: t.to_string(),
                        href: link.clone(),
                        code: paint.code || matches!(event, Event::Code(_)),
                        ..paint.clone()
                    });
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut line, style);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Plain,
    Markdown,
    Word,
    Pdf,
    Html,
}
impl Format {
    pub const ALL: [Self; 5] = [
        Self::Plain,
        Self::Markdown,
        Self::Word,
        Self::Pdf,
        Self::Html,
    ];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Plain => "txt",
            Self::Markdown => "md",
            Self::Word => "docx",
            Self::Pdf => "pdf",
            Self::Html => "html",
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct OptionsForExport {
    pub events: bool,
    pub maps: bool,
    pub attachments: bool,
}
#[derive(Default)]
pub(crate) struct Publication {
    pub title: String,
    pub blocks: Vec<Block>,
    pub anchors: BTreeMap<usize, BookId>,
    pub assets: BTreeMap<BookId, Attachment>,
    pub source: String,
}
pub fn is_web(link: &str) -> bool {
    link.starts_with("https://") || link.starts_with("http://") || link.starts_with("mailto:")
}
pub fn asset_id(uri: &str) -> Option<BookId> {
    uri.strip_prefix("atlas-asset:")?.parse().ok()
}
fn escape(text: &str) -> std::borrow::Cow<'_, str> {
    let escaped = crate::markup::escape(text);
    if escaped.contains('\'') {
        escaped.replace('\'', "&#39;").into()
    } else {
        escaped
    }
}
fn title_text(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
        .replace(']', "\\]")
        .replace('[', "\\[")
}
pub fn unique_name(title: &str, id: BookId) -> String {
    let name: String = title
        .chars()
        .take(65)
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches([' ', '.']);
    format!("{id}-{}", if name.is_empty() { "资料" } else { name })
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::recovery::write_atomic(path, bytes)
        .map_err(|e| format!("无法写入 {}：{e}", path.display()))
}
fn image_ref(id: BookId, name: &str) -> String {
    format!("![{}](atlas-asset:{id})", title_text(name))
}
fn meta_text(a: &Article) -> String {
    let mut fields = Vec::new();
    if !a.meta.category.is_empty() {
        fields.push(a.meta.category.clone());
    }
    if !a.meta.tags.is_empty() {
        fields.push(a.meta.tags.clone());
    }
    if let Some(y) = a.meta.from {
        fields.push(
            a.meta
                .until
                .map_or(y.to_string(), |end| format!("{y}–{end}")),
        );
    }
    fields.join(" · ")
}
fn compose(
    doc: &Document,
    ids: &[BookId],
    options: OptionsForExport,
) -> Result<Publication, String> {
    use std::fmt::Write;
    let book = doc.book.as_ref().ok_or("工程没有资料库")?;
    if ids.is_empty() {
        return Err("请选择资料文档".into());
    }
    let mut pubn = Publication {
        title: if ids.len() == 1 {
            book.articles
                .get(&ids[0])
                .ok_or("文档已不存在")?
                .name
                .clone()
        } else {
            "世界资料集".into()
        },
        ..Default::default()
    };
    let mut next = book.assets.keys().copied().max().unwrap_or(0) + 1;
    for &id in ids {
        let a = book.articles.get(&id).ok_or("文档已不存在")?;
        let start = pubn.source.len();
        let _ = writeln!(
            pubn.source,
            "# {}\n\n{}\n\n{}\n\n{}\n",
            title_text(&a.name),
            meta_text(a),
            a.meta.description,
            a.markdown
        );
        let mut maps = BTreeSet::new();
        let mut events = BTreeSet::new();
        for (_, link) in book.connections(Node::Article(id)) {
            let outgoing = link.from == Node::Article(id);
            let other = if outgoing { link.to } else { link.from };
            if let Some(label) = book.label(doc, other) {
                let _ = writeln!(pubn.source, "- [{}]({})", title_text(&label), other.uri());
            }
            if let Node::Event(b, e) = other {
                events.insert((b, e));
            }
            if let Some(b) = other.board() {
                let year = if outgoing && matches!(other, Node::Map(_)) {
                    link.year(doc.year)
                } else {
                    None
                };
                maps.insert((b, year));
            }
        }
        if options.events {
            if !events.is_empty() {
                pubn.source.push_str("\n## 关联事件\n\n");
            }
            for (b, e) in events {
                if let Some(event) = book.map(doc, b).and_then(|m| m.event(e)) {
                    let _ = writeln!(
                        pubn.source,
                        "### {} · {}–{}\n\n{}\n",
                        title_text(&event.name),
                        event.year,
                        event.last(),
                        event.note
                    );
                }
            }
        }
        if options.maps {
            if !maps.is_empty() {
                pubn.source.push_str("\n## 地图截图\n\n");
            }
            let font = crate::png::find_system_font();
            for (id, year) in maps {
                let Some(base) = book.map(doc, id) else {
                    continue;
                };
                let mut target = base.clone();
                if let Some(year) = year {
                    target.set_year(year);
                }
                let opts = crate::scene::ExportOptions {
                    width: 1100,
                    height: 650,
                    title: target.meta.name.clone(),
                    ..Default::default()
                };
                let scene =
                    crate::scene::build_scene(&target, &map_core::theme::evaluate(&target), &opts);
                let bytes = crate::png::render(&scene, font.as_deref())?
                    .encode_png()
                    .map_err(|e| e.to_string())?;
                let name = format!("{}.png", target.meta.name);
                pubn.assets.insert(
                    next,
                    Attachment {
                        name: name.clone(),
                        bytes: map_core::underlay::Blob(std::sync::Arc::new(bytes)),
                    },
                );
                let _ = writeln!(pubn.source, "{}\n", image_ref(next, &name));
                next += 1;
            }
        }
        if options.attachments {
            for asset in &a.attachments {
                if let Some(data) = book.assets.get(asset) {
                    pubn.assets.insert(*asset, data.clone());
                    let _ = writeln!(
                        pubn.source,
                        "\n[附件 · {}](atlas-asset:{asset})",
                        title_text(&data.name)
                    );
                }
            }
        }
        pubn.source.push_str("\n\n---\n\n");
        pubn.anchors.insert(pubn.blocks.len(), id);
        pubn.blocks.extend(rich(&pubn.source[start..]));
    }
    for block in &pubn.blocks {
        let spans = match block {
            Block::Para(_, v) => v.iter().collect::<Vec<_>>(),
            Block::Table(rows) => rows.iter().flatten().flatten().collect(),
            Block::Rule => vec![],
        };
        for span in spans {
            if let Some(id) = span.href.as_deref().and_then(asset_id) {
                if let Some(asset) = book.assets.get(&id) {
                    pubn.assets.entry(id).or_insert_with(|| asset.clone());
                }
            }
        }
    }
    Ok(pubn)
}
impl Publication {
    pub fn destination(&self, uri: &str) -> Option<BookId> {
        let Node::Article(id) = Node::parse(uri)? else {
            return None;
        };
        self.anchors.values().any(|a| *a == id).then_some(id)
    }
}
pub fn export(
    doc: &Document,
    ids: &[BookId],
    format: Format,
    options: OptionsForExport,
    path: &Path,
) -> Result<(), String> {
    let pubn = compose(doc, ids, options)?;
    match format {
        Format::Plain => write(path, plain(&pubn.blocks).as_bytes()),
        Format::Markdown => {
            let mut source = pubn.source;
            if !pubn.assets.is_empty() {
                let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("资料");
                let dir = format!("{stem}.assets");
                let assets = path.with_file_name(&dir);
                std::fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
                // Replace only destinations, never a number inside a different ID.
                for (&id, a) in &pubn.assets {
                    let name = unique_name(&a.name, id);
                    write(&assets.join(&name), &a.bytes.0)?;
                    source = source
                        .replace(&format!("(atlas-asset:{id})"), &format!("(<{dir}/{name}>)"));
                }
            }
            write(path, source.as_bytes())
        }
        Format::Word => write(path, &office::docx(&pubn)?),
        Format::Pdf => write(path, &crate::pdf::dossier_pdf(&pubn)?),
        Format::Html => write(path, html(&pubn).as_bytes()),
    }
}
pub fn export_attachments(doc: &Document, ids: &[BookId], dir: &Path) -> Result<usize, String> {
    let publication = compose(
        doc,
        ids,
        OptionsForExport {
            attachments: true,
            ..Default::default()
        },
    )?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (&id, asset) in &publication.assets {
        write(&dir.join(unique_name(&asset.name, id)), &asset.bytes.0)?;
    }
    Ok(publication.assets.len())
}
pub fn export_maps(
    doc: &Document,
    ids: &[BookId],
    format: crate::batch::Format,
    dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let font = crate::png::find_system_font();
    let mut out = Vec::new();
    for &id in ids {
        let base = WorldBook::active_map(doc, id).ok_or("画板已不存在")?;
        let path = dir.join(format!(
            "{}.{}",
            unique_name(&base.meta.name, id),
            format.ext()
        ));
        let (width, height) = base.paper.map_or_else(
            || { let default = crate::scene::ExportOptions::default(); (default.width, default.height) },
            |p| (p.width, p.height),
        );
        crate::batch::write_one(
            base,
            format,
            &crate::scene::ExportOptions {
                width,
                height,
                title: base.meta.name.clone(),
                ..Default::default()
            },
            font.as_deref(),
            &path,
        )?;
        out.push(path);
    }
    Ok(out)
}
fn plain(blocks: &[Block]) -> String {
    let inline = |s: &[Span]| {
        s.iter()
            .map(|s| {
                if s.image {
                    format!("[图片：{}]", s.text)
                } else {
                    s.text.clone()
                }
            })
            .collect::<String>()
    };
    blocks
        .iter()
        .map(|b| match b {
            Block::Para(_, s) => inline(s),
            Block::Table(rows) => rows
                .iter()
                .map(|r| r.iter().map(|s| inline(s)).collect::<Vec<_>>().join("\t"))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Rule => "────────────────".into(),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn inline_html(s: &[Span], pubn: &Publication) -> String {
    s.iter()
        .map(|span| {
            let value = escape(&span.text).replace('\n', "<br>");
            let data = span
                .href
                .as_deref()
                .and_then(asset_id)
                .and_then(|id| pubn.assets.get(&id));
            if let Some(asset) = data {
                let mime = if span.image {
                    image::guess_format(&asset.bytes.0)
                        .ok()
                        .map(|f| f.to_mime_type())
                        .unwrap_or("application/octet-stream")
                } else {
                    "application/octet-stream"
                };
                let uri = format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(asset.bytes.0.as_slice())
                );
                return if span.image {
                    format!("<img src=\"{uri}\" alt=\"{}\">", escape(&span.text))
                } else {
                    format!(
                        "<a download=\"{}\" href=\"{uri}\">{value}</a>",
                        escape(&asset.name)
                    )
                };
            }
            let mut formatted = value;
            for (enabled, tag) in [
                (span.bold, "strong"),
                (span.italic, "em"),
                (span.strike, "s"),
                (span.code, "code"),
            ] {
                if enabled {
                    formatted = format!("<{tag}>{formatted}</{tag}>");
                }
            }
            if let Some(href) = span.href.as_deref() {
                if let Some(id) = pubn.destination(href) {
                    formatted = format!("<a href=\"#doc-{id}\">{formatted}</a>");
                } else if is_web(href) || Node::parse(href).is_some() {
                    formatted = format!("<a href=\"{}\">{formatted}</a>", escape(href));
                }
            }
            formatted
        })
        .collect()
}
fn html(pubn: &Publication) -> String {
    let mut out=format!("<!doctype html><html lang=\"zh\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>body{{font:16px/1.65 system-ui,sans-serif;max-width:58rem;margin:2rem auto;padding:0 1rem;color:#18212b}}img{{max-width:100%;height:auto}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #a5adb4;padding:.45rem;text-align:left}}pre{{white-space:pre-wrap}}a{{color:#0a5a85}}blockquote{{border-left:3px solid #a5adb4;margin-left:0;padding-left:1rem}}</style><body>",escape(&pubn.title));
    for (index, b) in pubn.blocks.iter().enumerate() {
        match b {
            Block::Para(kind, spans) => {
                let tag = match kind {
                    Paragraph::Heading(1) => "h1",
                    Paragraph::Heading(2) => "h2",
                    Paragraph::Heading(3) => "h3",
                    Paragraph::Heading(_) => "h4",
                    Paragraph::Code => "pre",
                    Paragraph::Quote => "blockquote",
                    _ => "p",
                };
                let id = pubn
                    .anchors
                    .get(&index)
                    .map_or_else(String::new, |id| format!(" id=\"doc-{id}\""));
                out.push_str(&format!("<{tag}{id}>{}</{tag}>", inline_html(spans, pubn)));
            }
            Block::Rule => out.push_str("<hr>"),
            Block::Table(rows) => {
                out.push_str("<table>");
                for (i, row) in rows.iter().enumerate() {
                    out.push_str("<tr>");
                    let tag = if i == 0 { "th" } else { "td" };
                    for cell in row {
                        out.push_str(&format!("<{tag}>{}</{tag}>", inline_html(cell, pubn)));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
        }
    }
    out.push_str("</body></html>");
    out
}
