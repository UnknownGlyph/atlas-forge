use map_core::{
    world_book::{Attachment, Link, Node, TimeLink, WorldBook, ROOT},
    Document, LonLat,
};
use map_io::{dossier, project};
use std::{
    fs,
    io::{Cursor, Read},
    sync::Arc,
};

#[test]
fn project_roundtrip_keeps_shared_links_and_independent_timeline_state() {
    let mut project = Document::new("世界", 6371.0);
    let event = project.add_event("条约签订", 1701, Some(1705));
    let treaty = Node::Event(ROOT, event);
    let book = WorldBook::edit(&mut project);
    let article = book.add_article("条约原文".into());
    let point = book.add_place("会场".into(), LonLat::new(15.0, 40.0));
    let hub = book.add_map(Document::new("城镇", 6371.0));
    let mut origin = ROOT;
    let mut chain = vec![ROOT];
    for depth in 1..=24 {
        let id = book.add_map(Document::new(&format!("画板 {depth}"), 6371.0));
        book.add_link(Link::new(Node::Map(origin), Node::Map(id), true))
            .unwrap();
        origin = id;
        chain.push(id);
    }
    let deep = origin;
    let mut link = Link::new(treaty, Node::Map(deep), true);
    link.time = TimeLink::Fixed(1702);
    link.from_year = Some(1701);
    link.until_year = Some(1705);
    let jump = book.add_link(link.clone()).unwrap();
    book.add_link(Link::new(Node::Map(hub), Node::Map(deep), true))
        .unwrap();
    assert!(book
        .add_link(Link::new(Node::Map(deep), Node::Map(hub), true))
        .is_err());
    book.add_link(Link::new(Node::Map(deep), Node::Map(hub), false))
        .unwrap();
    book.add_link(Link::new(point, treaty, false)).unwrap();
    for from in [point, treaty, Node::Map(deep)] {
        book.add_link(Link::new(from, Node::Article(article), false))
            .unwrap();
    }
    assert_eq!(book.descendants(ROOT).len(), chain.len());
    assert_eq!(book.connections(Node::Article(article)).count(), 3);
    assert!(!link.visible(Some(1700)));
    assert!(link.visible(Some(1703)));
    project.set_year(Some(1740));
    let year = link.year(project.year);
    WorldBook::switch(&mut project, deep, year).unwrap();
    assert_eq!(project.year, Some(1702));
    project.meta.name = "战地".into();

    let bytes = project::to_bytes(&project).unwrap();
    assert_eq!(&bytes[8..12], &5_u32.to_le_bytes());
    let mut opened = project::from_bytes(&bytes).unwrap();
    assert_eq!(WorldBook::current(&opened), ROOT);
    assert_eq!(opened.meta.name, "世界");
    assert_eq!(opened.year, Some(1740));
    let book = opened.book.as_ref().unwrap();
    assert_eq!(Node::parse(&treaty.uri()), Some(treaty));
    assert_eq!(book.label(&opened, treaty).as_deref(), Some("条约签订"));
    assert_eq!(book.connections(Node::Article(article)).count(), 3);
    assert_eq!(book.links.get(&jump), Some(&link));
    WorldBook::switch(&mut opened, deep, None).unwrap();
    assert_eq!(opened.meta.name, "战地");
    assert_eq!(opened.year, Some(1702));
    opened.topo.validate().unwrap();

    let book = WorldBook::edit(&mut opened);
    book.links.remove(&jump);
    assert!(book.boards.contains_key(&deep));
    assert!(book.articles.contains_key(&article));
    WorldBook::switch(&mut opened, ROOT, None).unwrap();
    opened.remove_event(event).unwrap();
    assert_ne!(opened.add_event("后来发生的事件", 1720, None), event);
}

#[test]
fn face_link_survives_a_merge_and_project_reload() {
    let mut doc = Document::new("海湾", 6371.0);
    let island = [
        LonLat::new(0.0, 0.0),
        LonLat::new(10.0, 0.0),
        LonLat::new(10.0, 10.0),
        LonLat::new(0.0, 10.0),
    ];
    doc.add_ring_face(&island, map_core::FaceKind::Land)
        .unwrap();
    let (a, b) = doc
        .split_face(&[
            LonLat::new(5.0, 0.0),
            LonLat::new(5.0, 5.0),
            LonLat::new(5.0, 10.0),
        ])
        .unwrap();
    let original = Node::Face(ROOT, b.0);
    WorldBook::register_anchor(&mut doc, original);
    let book = WorldBook::edit(&mut doc);
    let article = book.add_article("海湾介绍".into());
    book.add_link(Link::new(original, Node::Article(article), false))
        .unwrap();
    assert_eq!(doc.merge_faces(a, b).unwrap(), a);
    let loaded = project::from_bytes(&project::to_bytes(&doc).unwrap()).unwrap();
    let book = loaded.book.as_ref().unwrap();
    assert_eq!(book.resolve(&loaded, original), Node::Face(ROOT, a.0));
    let (_, recovered) = book
        .connections_at(&loaded, Node::Face(ROOT, a.0))
        .next()
        .unwrap();
    assert_eq!(recovered.to, Node::Article(article));
}

#[test]
fn document_exports_include_real_images_tables_and_related_maps() {
    let tmp = std::env::temp_dir().join(format!("atlas_book_formats_{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    let mut doc = Document::new("世界图", 6371.0);
    let event = doc.add_event("独立条约", 1701, None);
    let book = WorldBook::edit(&mut doc);
    let art = book.add_article("第一篇".into());
    let second = book.add_article("第二篇".into());
    let map = book.add_map(Document::new("城墙", 6371.0));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(12, 10)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let bytes = png.into_inner();
    let image_id = book.add_asset(Attachment {
        name: "旗帜.png".into(),
        bytes: map_core::underlay::Blob(Arc::new(bytes.clone())),
    });
    let article = Arc::make_mut(book.articles.get_mut(&art).unwrap());
    article.markdown = format!(
        "## 历史\n\n一段 **重要** 说明。\n\n| 年份 | 地点 |\n| --- | --- |\n| 1701 | 西港 |\n\n![旗帜](atlas-asset:{image_id})\n\n[下一篇](atlas-doc:{second})"
    );
    article.attachments.insert(image_id);
    Arc::make_mut(book.articles.get_mut(&second).unwrap()).markdown =
        (0..100).map(|i| format!("第 {i} 条记录。\n\n")).collect();
    book.add_link(Link::new(
        Node::Article(art),
        Node::Event(ROOT, event),
        false,
    ))
    .unwrap();
    book.add_link(Link::new(Node::Article(art), Node::Map(map), false))
        .unwrap();
    let options = dossier::OptionsForExport {
        events: true,
        maps: false,
        attachments: true,
    };
    for format in dossier::Format::ALL {
        let path = tmp.join(format!("册.{}", format.extension()));
        dossier::export(&doc, &[art, second], format, options, &path).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 100);
    }
    let plain = fs::read_to_string(tmp.join("册.txt")).unwrap();
    assert!(plain.contains("年份\t地点"));
    assert!(plain.contains("独立条约"));
    assert!(plain.contains("第二篇"));
    let md = fs::read_to_string(tmp.join("册.md")).unwrap();
    assert!(md.contains("册.assets/"));
    assert!(md.contains("| 年份 | 地点 |"));
    let html = fs::read_to_string(tmp.join("册.html")).unwrap();
    assert!(html.contains("<table>"));
    assert!(html.contains("data:image/png;base64,"));
    assert!(html.contains(&format!("href=\"#doc-{second}\"")));
    let mut word = zip::ZipArchive::new(fs::File::open(tmp.join("册.docx")).unwrap()).unwrap();
    let mut xml = String::new();
    word.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains("<w:tbl>"));
    assert!(xml.contains("<w:drawing>"));
    assert!(xml.contains(&format!("w:anchor=\"doc_{second}\"")));
    assert!(word.by_name("word/media/picture1.png").unwrap().size() > 30);
    let pdf = fs::read(tmp.join("册.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let pdf = String::from_utf8_lossy(&pdf);
    assert!(pdf.contains("/Subtype /Image"));
    assert!(pdf.matches("/Type /Page ").count() > 1);
    assert_eq!(dossier::export_attachments(&doc, &[art], &tmp).unwrap(), 1);
    assert_eq!(
        fs::read(tmp.join(dossier::unique_name("旗帜.png", image_id))).unwrap(),
        bytes
    );
    let maps = dossier::export_maps(&doc, &[ROOT, map], map_io::batch::Format::Svg, &tmp).unwrap();
    assert_eq!(maps.len(), 2);
    assert!(fs::read_to_string(&maps[1]).unwrap().contains("<svg"));
    let path = tmp.join("截图.html");
    dossier::export(
        &doc,
        &[art],
        dossier::Format::Html,
        dossier::OptionsForExport {
            maps: true,
            ..options
        },
        &path,
    )
    .unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("地图截图"));
}
