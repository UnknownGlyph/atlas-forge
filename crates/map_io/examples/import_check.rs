//! Strict shared-border import of a GeoJSON file, from the command line.
//!
//!   tools\cargo-msvc.cmd run --release -p map_io --example import_check -- <file.geojson> [--repair <metres>] [out.atlas]

use std::time::Instant;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = map_io::geojson::ImportMode::Strict;
    if let Some(i) = args.iter().position(|a| a == "--repair") {
        let snap_m = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(100.0);
        mode = map_io::geojson::ImportMode::Repair(map_core::repair::RepairOptions { snap_m, ..Default::default() });
        args.drain(i..(i + 2).min(args.len()));
    }
    let mut args = args.into_iter();
    let Some(path) = args.next() else {
        eprintln!("usage: import_check <file.geojson> [--repair <metres>] [out.atlas]");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).expect("read input");
    let t = Instant::now();
    match map_io::geojson::import_topology_with(&text, &mode) {
        Ok((doc, rep)) => {
            println!(
                "ok in {:.0} ms: {} countries, {} faces, {} edges, {} lakes, {} fields, {} skipped rings",
                t.elapsed().as_secs_f64() * 1e3,
                rep.countries,
                rep.faces,
                rep.edges,
                rep.lakes,
                rep.fields,
                rep.skipped_rings
            );
            if let Some(r) = &rep.repair {
                println!(
                    "repair: {} vertices snapped (max shift {:.1} m), {} vertices inserted, {} overlaps, {} swallowed, {} slivers filled ({:.3} km²)",
                    r.snapped_vertices, r.max_shift_m, r.inserted_vertices, r.overlaps.len(), r.swallowed.len(), r.slivers_filled, r.sliver_area_km2
                );
                for o in r.overlaps.iter().take(10) {
                    println!("  overlap: feature {} keeps {:.3} km² claimed by feature {} near {:.3}, {:.3}", o.kept_by, o.area_km2, o.removed_from, o.at.lon, o.at.lat);
                }
                for (f, before, after) in r.area_changes.iter().take(5) {
                    println!("  area change: feature {f}: {before:.1} -> {after:.1} km² ({:+.4}%)", (after - before) / before * 100.0);
                }
            }
            let shared = doc.topo.edges.values().filter(|e| e.left != map_core::BACKGROUND && e.right != map_core::BACKGROUND).count();
            println!("edges between two land/water faces (shared borders): {shared}");
            doc.topo.validate().expect("valid topology");
            let areas = doc.areas_by_entity();
            let mut top: Vec<_> = areas.iter().map(|(e, a)| (doc.entities[e].name.clone(), doc.faces_of(*e).len(), *a)).collect();
            top.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
            for (name, parts, a) in top.iter().take(8) {
                println!("  {name:<28} {parts:>3} parts {:>12.0} km²", a);
            }
            if let Some(out) = args.next() {
                map_io::project::save(std::path::Path::new(&out), &doc).expect("save");
                println!("saved {out}");
            }
        }
        Err(rep) => {
            println!("FAILED in {:.0} ms with {} problems:", t.elapsed().as_secs_f64() * 1e3, rep.problems.len());
            for p in &rep.problems {
                println!("  {p}");
            }
            std::process::exit(1);
        }
    }
}
