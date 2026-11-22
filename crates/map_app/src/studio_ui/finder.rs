//! The studio's lightweight finder. It resolves to existing project nodes.
use crate::{app::App, i18n::tr};
use eframe::egui;
use map_core::{
    doc::EntityKind,
    world_book::{Node, WorldBook},
    Document,
};

#[derive(Default)]
pub(super) struct Finder {
    pub open: bool,
    focus: bool,
    query: String,
    results: Vec<Match>,
    generation: Option<u64>,
}

struct Match {
    node: Node,
    name: String,
    kind: &'static str,
}

impl Finder {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
        self.generation = None;
    }
}

fn search(doc: &Document, input: &str) -> Vec<Match> {
    let query = input.trim().to_lowercase();
    let words: Vec<_> = query.split_whitespace().collect();
    let matches = |text: &str| {
        if words.is_empty() {
            true
        } else {
            let text = text.to_lowercase();
            words.iter().all(|word| text.contains(word))
        }
    };
    let current = WorldBook::current(doc);
    let mut found = Vec::new();
    let mut add = |node, name: &str, kind| {
        if found.len() < 28 && matches(name) {
            found.push(Match {
                node,
                name: name.into(),
                kind,
            });
        }
    };

    add(Node::Map(current), &doc.meta.name, "画板");
    if let Some(book) = &doc.book {
        for (name, id) in book
            .boards
            .keys()
            .filter(|&&id| id != current)
            .filter_map(|&id| book.map(doc, id).map(|board| (&board.meta.name, id)))
            .filter(|(name, _)| matches(name))
            .take(12)
        {
            add(Node::Map(id), name, "画板");
        }
        if !query.is_empty() {
            for (&id, article) in book
                .articles
                .iter()
                .filter(|(_, a)| matches(&a.name))
                .take(8)
            {
                add(Node::Article(id), &article.name, "文档");
            }
        }
    }
    if !query.is_empty() {
        for (&id, entity) in doc
            .entities
            .iter()
            .filter(|(_, e)| e.exists_in(doc.year) && matches(&e.name))
            .take(18)
        {
            let kind = match entity.kind {
                EntityKind::City => "城市",
                EntityKind::Region => "地区",
                EntityKind::Country if entity.parent.is_some() => "地区",
                EntityKind::Country => "国家",
            };
            add(Node::Entity(current, id.0), &entity.name, kind);
        }
    }
    found
}

impl App {
    pub(crate) fn studio_find_input(&mut self, ctx: &egui::Context) -> bool {
        if !self.studio.foreground {
            return false;
        }
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            self.studio.finder.show();
        }
        self.studio.finder.open
    }

    pub(crate) fn studio_find_window(&mut self, ctx: &egui::Context) {
        if !self.studio.foreground || !self.studio.finder.open {
            return;
        }
        let finder = &mut self.studio.finder;
        let mut chosen = None;
        let mut cancelled = false;
        let response = egui::Modal::new(egui::Id::unique("studio-find")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() * 0.80).clamp(260.0, 460.0));
            ui.heading(tr("查找"));
            let field = ui.add(
                egui::TextEdit::singleline(&mut finder.query)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("搜索画板、文档或地图地点")),
            );
            if std::mem::take(&mut finder.focus) {
                field.request_focus();
            }
            let generation = self.history.generation();
            if field.changed() || finder.generation != Some(generation) {
                finder.results = search(&self.doc, &finder.query);
                finder.generation = Some(generation);
            }
            if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                chosen = finder.results.first().map(|row| row.node);
            }
            if finder.results.is_empty() {
                ui.weak(tr("没有找到匹配项。"));
            } else {
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for row in &finder.results {
                            ui.push_id(row.node.uri(), |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(&row.name)
                                                .min_size(egui::vec2(145.0, 31.0)),
                                        )
                                        .clicked()
                                    {
                                        chosen = Some(row.node);
                                    }
                                    ui.weak(tr(row.kind));
                                })
                            });
                        }
                    });
            }
            if ui.button(tr("取消")).clicked() {
                cancelled = true;
            }
        });
        if response.should_close() || cancelled || chosen.is_some() {
            finder.open = false;
        }
        if let Some(target) = chosen {
            match target {
                Node::Entity(_, id) => {
                    self.star_map.open = false;
                    self.studio.show_map();
                    self.set_tool(crate::view::Tool::Select);
                    let id = map_core::EntityId(id);
                    self.selected = Some(id);
                    self.selected_face = None;
                    if let Some(point) = self.doc.entities.get(&id).and_then(|entity| entity.pos) {
                        self.centre_on(point);
                    } else {
                        self.run(crate::actions::Act::ZoomSelection);
                    }
                }
                Node::Map(id) => {
                    self.star_map.open = false;
                    self.studio.show_map();
                    if id != WorldBook::current(&self.doc) {
                        if let Err(error) = self.enter_book_map(id, None, true) {
                            self.set_status(error, true);
                        }
                    }
                }
                Node::Article(_) => self.open_book_node(target, None),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_matches_board_names_and_articles_by_words_case_insensitively() {
        let mut doc = Document::new("示例世界", 6371.0);
        let book = WorldBook::edit(&mut doc);
        let map = book.add_map(Document::new("North Sea", 6371.0));
        let article = book.add_article("North Sea Treaty".into());
        let hits = search(&doc, " sea NORTH ");
        assert_eq!(
            hits.iter().map(|hit| hit.node).collect::<Vec<_>>(),
            [Node::Map(map), Node::Article(article)]
        );
    }
}
