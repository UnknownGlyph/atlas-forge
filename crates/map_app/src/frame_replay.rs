//! Replays the desktop UI and tessellates its shapes without waiting for a display.
//! Set ATLAS_BENCH_WORLD to a project and ATLAS_PREFS to isolated preferences.
use crate::app::{App, Layout};
use eframe::{egui, App as _};
use std::{path::PathBuf, sync::Arc, time::Instant};

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
