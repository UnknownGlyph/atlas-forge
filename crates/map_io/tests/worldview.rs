use map_core::{
    doc::{EntityKind, FieldKind, Value as MapValue},
    history::History,
    world_book::{Link, Node, TimeLink, WorldBook, ROOT},
    worldview::{
        Aggregate, Catalog, Direction, Evaluation, Field, Input, Kind, Record, Relation,
        RelationType, Rule, Schema, Series, Source, Template, Value,
    },
    Document,
};
use std::{collections::BTreeMap, sync::Arc};

fn number(name: &str) -> Field {
    Field {
        name: name.into(),
        kind: Kind::Number,
        ..Default::default()
    }
}

#[test]
fn project_catalog_reloads_across_boards_and_resolves_timeline_sources() {
    let mut doc = Document::new("World", 6371.0);
    let country = doc.add_entity(EntityKind::Country, "Map country");
    let population = doc
        .add_field("inhabitants", "Population", FieldKind::Number, None)
        .unwrap();
    doc.set_value(country, population, Some(1740), Some(MapValue::Num(8.0)));
    doc.set_value(country, population, Some(1744), Some(MapValue::Num(12.0)));

    let (schema, record, template) = {
        let c = Catalog::edit(&mut doc);
        let schema = c.alloc().unwrap();
        let record = c.alloc().unwrap();
        let template = c.alloc().unwrap();
        let mut derived = number("Weighted population");
        derived.rule = Some(Rule {
            expression: "mapped * factor + (year - 1740)".into(),
            inputs: BTreeMap::from([(
                "mapped".into(),
                Input::MapField {
                    key: "inhabitants".into(),
                    aggregate: Aggregate::Sum,
                },
            )]),
        });
        c.schemas.insert(
            schema,
            Arc::new(Schema {
                name: "Civilization".into(),
                fields: BTreeMap::from([
                    ("factor".into(), number("Factor")),
                    ("derived".into(), derived),
                ]),
                ..Default::default()
            }),
        );
        c.records.insert(
            record,
            Arc::new(Record {
                name: "Example civilization".into(),
                schema,
                values: BTreeMap::from([(
                    "factor".into(),
                    Series {
                        base: Some(Value::Number(2.0)),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            }),
        );
        c.templates.insert(
            template,
            Arc::new(Template {
                name: "City-state".into(),
                schema,
                values: BTreeMap::from([("factor".into(), Value::Number(3.0))]),
                ..Default::default()
            }),
        );
        (schema, record, template)
    };
    let book = WorldBook::edit(&mut doc);
    let board = book.add_map(Document::new("Independent board", 6371.0));
    let article = book.add_article("Article on example civilization".into());
    for node in [
        Node::Entity(ROOT, country.0),
        Node::Map(board),
        Node::Article(article),
    ] {
        let mut link = Link::new(Node::Record(record), node, false);
        link.time = TimeLink::Inherit;
        book.add_link(link).unwrap();
    }
    WorldBook::switch(&mut doc, board, Some(Some(1740))).unwrap();
    let derived = |map: &Document, year| {
        Evaluation::new(map, Some(year))
            .unwrap()
            .get(record, "derived")
            .unwrap()
            .unwrap()
    };
    assert_eq!(derived(&doc, 1740).value, Value::Number(16.0));
    assert_eq!(derived(&doc, 1744).value, Value::Number(28.0));
    let mut history = History::default();
    history
        .apply(&mut doc, "pin", |d| {
            let c = Catalog::edit(d);
            let record = Arc::make_mut(c.records.get_mut(&record).unwrap());
            record.values.entry("derived".into()).or_default().pinned = Some(Value::Number(123.0));
            Ok(())
        })
        .unwrap();
    assert_eq!(derived(&doc, 1744).source, Source::Pinned);
    assert_eq!(derived(&doc, 1744).value, Value::Number(123.0));
    history.undo(&mut doc).unwrap();
    assert_eq!(derived(&doc, 1740).source, Source::Computed);

    let bytes = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 8);
    let doc = map_io::project::from_bytes(&bytes).unwrap();
    assert_eq!(WorldBook::current(&doc), ROOT);
    assert_eq!(derived(&doc, 1744).value, Value::Number(28.0));
    let c = Catalog::get(&doc).unwrap();
    assert_eq!(
        c.from_template(template, "From template".into())
            .unwrap()
            .schema,
        schema
    );
    assert_eq!(
        c.from_template(template, "From template".into())
            .unwrap()
            .values["factor"]
            .base,
        Some(Value::Number(3.0))
    );
    assert_eq!(
        Node::parse(&Node::Record(record).uri()),
        Some(Node::Record(record))
    );
    assert_eq!(
        doc.book.unwrap().connections(Node::Record(record)).count(),
        3
    );
}

#[test]
fn relationship_rule_obeys_dates_and_reports_cross_record_cycles() {
    let mut doc = Document::new("World", 6371.0);
    let c = Catalog::edit(&mut doc);
    let schema = c.alloc().unwrap();
    let kind = c.alloc().unwrap();
    c.relation_types.insert(
        kind,
        Arc::new(RelationType {
            name: "Command".into(),
            ..Default::default()
        }),
    );
    let mut size = number("Total");
    size.default = Some(Value::Number(1.0));
    size.rule = Some(Rule {
        expression: "next + 1".into(),
        inputs: BTreeMap::from([(
            "next".into(),
            Input::RelatedField {
                relation: kind,
                direction: Direction::Outgoing,
                key: "size".into(),
                aggregate: Aggregate::Sum,
            },
        )]),
    });
    c.schemas.insert(
        schema,
        Arc::new(Schema {
            name: "Family".into(),
            fields: BTreeMap::from([("size".into(), size)]),
            ..Default::default()
        }),
    );
    let one = c.alloc().unwrap();
    let two = c.alloc().unwrap();
    for (id, name) in [(one, "A"), (two, "B")] {
        c.records.insert(
            id,
            Arc::new(Record {
                name: name.into(),
                schema,
                ..Default::default()
            }),
        );
    }
    for (from, to, year) in [(one, two, 1800), (two, one, 1850)] {
        let id = c.alloc().unwrap();
        c.relations.insert(
            id,
            Arc::new(Relation {
                kind,
                from,
                to,
                meta: map_core::world_book::Meta {
                    from: Some(year),
                    ..Default::default()
                },
            }),
        );
    }
    c.validate().unwrap();
    assert_eq!(
        c.change_years().into_iter().collect::<Vec<_>>(),
        vec![1800, 1850]
    );
    let mut eval = Evaluation::new(&doc, Some(1830)).unwrap();
    assert_eq!(
        eval.get(one, "size").unwrap().unwrap().value,
        Value::Number(2.0)
    );
    let mut eval = Evaluation::new(&doc, Some(1850)).unwrap();
    assert!(eval.get(one, "size").unwrap_err().contains("循环"));
    let c = Catalog::edit(&mut doc);
    let record = Arc::make_mut(c.records.get_mut(&two).unwrap());
    let value = record.values.entry("size".into()).or_default();
    value.pinned = Some(Value::Number(5.0));
    value.write(Some(1840), Some(Value::Number(6.0)));
    let mut eval = Evaluation::new(&doc, Some(1850)).unwrap();
    assert_eq!(
        eval.get(one, "size").unwrap().unwrap().value,
        Value::Number(6.0)
    );
}
