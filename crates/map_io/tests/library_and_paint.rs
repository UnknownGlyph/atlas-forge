use std::sync::Arc;

use map_core::world_book::{Attachment, Link, Node, ROOT, WorldBook};
use map_core::{Document, LonLat};
use map_core::area::{AreaMark, Pattern};

#[test]
fn deleted_assets_and_places_disappear_from_links_and_new_project_bytes() {
    let mut doc = Document::new("Library", 6371.0);
    let (asset, article, place) = {
        let book = WorldBook::edit(&mut doc);
        let article = book.add_article("Text stays".into());
        let asset = book.add_asset(Attachment {
            name: "Image".into(),
            bytes: map_core::underlay::Blob(Arc::new(vec![1, 2, 3])),
        });
        Arc::make_mut(book.articles.get_mut(&article).unwrap()).attachments.insert(asset);
        let place = book.add_place("Place".into(), LonLat::new(2.0, 8.0));
        book.add_link(Link::new(Node::Article(article), Node::Asset(asset), false)).unwrap();
        book.add_link(Link::new(Node::Map(ROOT), place, false)).unwrap();
        (asset, article, place)
    };
    let before = doc.clone();
    WorldBook::remove_item(&mut doc, Node::Asset(asset)).unwrap();
    WorldBook::remove_item(&mut doc, place).unwrap();
    let read = map_io::project::from_bytes(&map_io::project::to_bytes(&doc).unwrap()).unwrap();
    let book = read.book.unwrap();
    assert!(book.links.is_empty() && book.assets.is_empty() && book.places.is_empty());
    assert!(book.articles[&article].attachments.is_empty());
    assert_eq!(book.articles[&article].name, "Text stays");
    assert!(before.book.unwrap().assets.contains_key(&asset));
}

#[test]
fn solid_color_areas_keep_colors_in_projects_and_scene_exports() {
    let mut doc = Document::new("Color", 6371.0);
    let id = map_core::doc::LineId(doc.topo.alloc());
    let mut area = AreaMark::new(vec![LonLat::new(0.0, 0.0), LonLat::new(6.0, 0.0), LonLat::new(3.0, 6.0)]);
    let color = [123, 75, 51, 164];
    area.color = color;
    area.pattern = Pattern::Solid;
    doc.areas.insert(id, area);
    let bytes = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 9);
    let read = map_io::project::from_bytes(&bytes).unwrap();
    assert_eq!(read.areas[&id].tint_color(), color);
    let scene = map_io::scene::build_scene(&read, &map_core::theme::evaluate(&read), &Default::default());
    assert!(scene.polys.iter().any(|poly| poly.fill == color));
}
