//! Replays the desktop UI and tessellates its shapes without waiting for a display.
//! Set ATLAS_BENCH_WORLD to a project and ATLAS_PREFS to isolated preferences.
use crate::app::{App, Layout};
use eframe::{egui, App as _};
use std::{path::PathBuf, sync::Arc, time::Instant};

#[test]
fn switching_boards_and_tools_discards_old_drag_targets() {
    use map_core::{doc::{LineFeature, LineId}, world_book::{WorldBook, ROOT}, Document, EntityId, LonLat};
    // App startup installs process-wide localization and crash handling.
    // Run this case with its own process and preferences.
    const CHILD: &str = "ATLAS_TEST_BOARD_GESTURES";
    if std::env::var_os(CHILD).is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let prefs = map_io::prefs::Prefs {cache_directory: Some(dir.path().into()), autosave_on:false, welcome:false, ..Default::default()};
        map_io::prefs::save(&path, &prefs).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "frame_replay::switching_boards_and_tools_discards_old_drag_targets", "--nocapture"])
            .env(CHILD, "1").env("ATLAS_PREFS", path).env_remove("ATLAS_PERF_LOG")
            .output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        return;
    }
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = App::new(&cc, None, None);
    let make_board = |name: &str| {
        let mut doc = Document::default();
        let id = LineId(doc.topo.alloc());
        doc.rivers.insert(id, LineFeature { name: name.into(), pts: vec![LonLat::new(0.0, 0.0), LonLat::new(1.0, 1.0)],
            legend: String::new(), route: None, show_name: false, hidden: false });
        (doc, id)
    };
    let (parent, river) = make_board("parent river");
    let (child, child_river) = make_board("child river");
    assert_eq!(river, child_river);
    app.doc = parent;
    let board = WorldBook::edit(&mut app.doc).add_map(child);
    app.gestures.city = Some(EntityId(1));
    app.begin_pending("move city");
    app.doc.add_event("finished edit", 5, None);
    app.pending.as_mut().unwrap().changed = true;
    app.set_tool(crate::view::Tool::River);
    assert!(app.gestures.city.is_none() && app.history.can_undo());

    app.gestures.river_point = Some((river, 0));
    app.gestures.brush_stroke.push(egui::Pos2::ZERO);
    app.ann_path = Some((LineId(1), vec![LonLat::new(0.0, 0.0)]));
    app.measure_pts.push(LonLat::new(1.0, 1.0));
    app.pixel_tools.open = true;
    app.pixel_tools.placing = true;
    app.enter_book_map(board, None, true).unwrap();
    assert!(app.gestures.river_point.is_none() && app.gestures.brush_stroke.is_empty());
    assert!(app.ann_path.is_none() && app.measure_pts.is_empty());
    assert!(!app.pixel_tools.open && !app.pixel_tools.placing);
    assert_eq!(app.doc.rivers[&child_river].name, "child river");
    app.gestures.brush_stroke.push(egui::Pos2::ZERO);
    let mut frame = eframe::Frame::_new_kittest();
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true,
        repeat: false, modifiers: egui::Modifiers::NONE };
    let _ = cc.egui_ctx.run_ui(egui::RawInput { events: vec![escape], ..Default::default() }, |ui| app.ui(ui, &mut frame));
    assert!(app.gestures.brush_stroke.is_empty());
    app.enter_book_map(ROOT, None, true).unwrap();
    assert!(app.doc.events.iter().any(|event| event.name == "finished edit"));
    app.on_exit();
    crate::shutdown::finish();
}

#[test]
#[ignore = "manual desktop frame timing with an external project"]
fn desktop_pan_and_rotate() {
    let world = PathBuf::from(std::env::var_os("ATLAS_BENCH_WORLD").expect("set ATLAS_BENCH_WORLD"));
    let prefs = PathBuf::from(std::env::var_os("ATLAS_PREFS").expect("set isolated ATLAS_PREFS"));
    let settings = map_io::prefs::load(&prefs);
    assert!(settings.cache_directory.is_some() && !settings.autosave_on, "use a private cache and disable autosave");
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut frame = eframe::Frame::_new_kittest();
    let mut app = App::new(&cc, map_io::png::find_system_font().map(Arc::new), Some(world));
    assert!(app.path.is_some(), "project did not load");
    app.layout = Layout::Split;
    app.set_status("", false);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500.0, 900.0));
    let mut times = Vec::new();
    let mut counts = (0, 0);
    for i in 0..108 {
        if i >= 12 {
            app.cam2d.pan_px([2.5, 0.2]);
            app.globe.center_lon += 0.75;
        }
        let input = egui::RawInput { screen_rect: Some(rect), time: Some(i as f64 / 60.0), max_texture_side: Some(8192), ..Default::default() };
        let start = Instant::now();
        let output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if i >= 12 {
            times.push(elapsed);
            for item in meshes {
                if let egui::epaint::Primitive::Mesh(mesh) = item.primitive {
                    counts.0 += mesh.vertices.len();
                    counts.1 += mesh.indices.len();
                }
            }
        }
    }
    times.sort_by(f64::total_cmp);
    println!("desktop CPU ({} frames) median {:.3} ms, p95 {:.3} ms; vertices {}, indices {}", times.len(), times[times.len() / 2], times[times.len() * 95 / 100], counts.0, counts.1);
    app.on_exit();
    crate::shutdown::finish();
}
