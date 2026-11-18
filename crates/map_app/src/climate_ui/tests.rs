use super::*;
use map_core::FaceKind;

#[test]
fn scheduling_depends_on_geography_and_run_mode() {
    let mut doc = Document {
        climate: Some(Arc::new(Project::default())),
        ..Document::default()
    };
    let mut ui = ClimateUi::default();
    let now = Instant::now();
    let observe =
        |ui: &mut ClimateUi, doc: &Document| ui.observe(doc, doc.climate.as_ref().unwrap(), now);
    observe(&mut ui, &doc);
    assert!(ui.due.is_none());
    doc.add_ring_face(
        &[
            LonLat::new(1., 1.),
            LonLat::new(8., 1.),
            LonLat::new(7., 8.),
        ],
        FaceKind::Land,
    )
    .unwrap();
    observe(&mut ui, &doc);
    assert!(ui.due.is_none());
    Arc::make_mut(doc.climate.as_mut().unwrap()).live = true;
    observe(&mut ui, &doc);
    assert!(ui.due.take().is_some());
    Arc::make_mut(doc.climate.as_mut().unwrap()).field = Field::Rain;
    observe(&mut ui, &doc);
    assert!(ui.due.is_none());
    doc.set_year(Some(1500));
    observe(&mut ui, &doc);
    assert!(ui.due.take().is_some());
    Arc::make_mut(doc.climate.as_mut().unwrap())
        .surface_edits
        .insert(
            1,
            SurfaceEdit {
                height: Some(3000.),
                surface: None,
            },
        );
    observe(&mut ui, &doc);
    assert!(ui.due.is_some());
    Arc::make_mut(doc.climate.as_mut().unwrap()).live = false;
    observe(&mut ui, &doc);
    assert!(ui.due.is_none());
}
