//! Performance baseline on synthetic worlds. Run with:
//!   tools\cargo-msvc.cmd run -p map_bench --release
//! Results are recorded in docs/BENCHMARKS.md.

use map_core::geom::LonLat;
use map_core::history::History;
use map_core::sample::{grid_world, Rng};
use map_core::theme;
use map_render::tess::SceneCache;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("authoring") {
        return authoring();
    }
    if std::env::args().nth(1).as_deref() == Some("history") {
        let n = |i: usize, d: usize| std::env::args().nth(i).and_then(|s| s.parse().ok()).unwrap_or(d);
        let km = std::env::args().nth(5).and_then(|s| s.parse().ok());
        return history(n(2, 10), n(3, 6), n(4, 240), km);
    }
    println!("| 世界 | 面 | 边 | 顶点 | 生成 ms | 校验 ms | 全量三角化 ms | 三角形 | 单点编辑+增量重建 ms | 撤销 ms | 拾取 µs/次 | 专题图 ms | 保存 ms | 读取 ms | 文件 KB | 标注全量 ms | 标注增量 ms | 变更集 µs | 其中拓扑编辑 ms |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (nx, ny, detail) in [(8, 5, 6), (20, 12, 8), (40, 25, 8), (80, 50, 8)] {
        let t = Instant::now();
        let mut doc = grid_world(nx, ny, detail, 42);
        let gen = ms(t);
        let verts: usize = doc.topo.edges.values().map(|e| e.pts.len()).sum::<usize>() + doc.topo.nodes.len();
        let t = Instant::now();
        doc.topo.validate().expect("valid");
        let val = ms(t);

        let mut cache = SceneCache::default();
        let t = Instant::now();
        cache.update_geometry(&doc, 0.0);
        let tess = ms(t);
        let tris = cache.vertex_count() / 3;

        // interactive edit: move an interior boundary vertex, then incremental rebuild
        let mut labels = map_core::labels::LabelCache::default();
        let t = Instant::now();
        labels.update(&doc);
        let label_full = ms(t);
        let mut hist = History::default();
        let (eid, e) = doc.topo.edges.iter().find(|(_, e)| e.pts.len() > 4 && e.left != map_core::BACKGROUND && e.right != map_core::BACKGROUND).map(|(i, e)| (*i, e.clone())).unwrap();
        let p = e.pts[2];
        let prev = doc.clone();
        let t = Instant::now();
        hist.apply(&mut doc, "move", |d| d.topo.move_vertex(map_core::VertexRef::Pt { edge: eid, idx: 2 }, LonLat::new(p.lon + 0.01, p.lat + 0.01))).unwrap();
        let topo_edit = ms(t);
        let st = cache.update_geometry(&doc, 0.0);
        let edit = ms(t);
        let t = Instant::now();
        let n_labels = labels.update(&doc);
        let label_inc = ms(t);
        assert_eq!(n_labels, 2);
        let t = Instant::now();
        let cs = map_core::change::ChangeSet::between(&prev, &doc);
        let cs_us = ms(t) * 1000.0;
        assert_eq!(cs.faces.len(), 2);
        assert_eq!(st.faces_retessellated, 2);
        let t = Instant::now();
        hist.undo(&mut doc);
        cache.update_geometry(&doc, 0.0);
        let undo = ms(t);

        let mut rng = Rng::new(1);
        let n = 2000;
        let t = Instant::now();
        let mut hits = 0;
        for _ in 0..n {
            let q = LonLat::new(rng.range(-160.0, 160.0), rng.range(-70.0, 80.0));
            if doc.topo.face_at(q) != map_core::BACKGROUND {
                hits += 1;
            }
        }
        let pick = ms(t) * 1000.0 / n as f64;
        assert!(hits > 0);

        doc.theme.mode = theme::ThemeMode::Choropleth;
        doc.theme.field_key = "density".into();
        let t = Instant::now();
        let th = theme::evaluate(&doc);
        let them = ms(t);
        assert!(!th.legend.is_empty());

        let scratch = tempfile::tempdir().expect("benchmark directory");
        let path = scratch.path().join("atlas_bench.atlas");
        let t = Instant::now();
        map_io::project::save(&path, &doc).unwrap();
        let save = ms(t);
        let size = std::fs::metadata(&path).unwrap().len() as f64 / 1024.0;
        let t = Instant::now();
        let back = map_io::project::load(&path).unwrap();
        let load = ms(t);
        assert_eq!(back.topo.faces.len(), doc.topo.faces.len());

        // drawing over existing land: local splice (falls back to a whole
        // rebuild) vs always rebuilding the whole world
        let x = doc.countries()[0];
        let ring = vec![LonLat::new(-140.0, -20.0), LonLat::new(-100.0, -20.0), LonLat::new(-100.0, 10.0), LonLat::new(-140.0, 10.0)];
        let t = Instant::now();
        let (after, orep) = map_core::overlay::apply(&doc, &ring, map_core::overlay::DrawIntent::Take(x)).expect("overlay");
        let overlay_ms = ms(t);
        after.topo.validate().expect("valid after overlay");
        let t = Instant::now();
        let (whole, _) = map_core::overlay::apply_whole_world(&doc, &ring, map_core::overlay::DrawIntent::Take(x)).expect("overlay");
        let whole_ms = ms(t);
        assert_eq!(whole.topo.faces.len(), after.topo.faces.len());
        eprintln!(
            "{nx}x{ny}: overlay (take) {overlay_ms:.1} ms (local={}, {} faces rebuilt{}), whole-world rebuild {whole_ms:.0} ms; faces {} -> {}, taken from {} owners",
            orep.local,
            orep.rebuilt_faces,
            orep.fallback.as_deref().map(|f| format!(", fallback: {f}")).unwrap_or_default(),
            orep.faces_before,
            orep.faces_after,
            orep.taken.len()
        );
        println!(
            "| {nx}x{ny} | {} | {} | {verts} | {gen:.0} | {val:.1} | {tess:.1} | {tris} | {edit:.2} | {undo:.2} | {pick:.1} | {them:.1} | {save:.1} | {load:.1} | {size:.0} | {label_full:.1} | {label_inc:.2} | {cs_us:.0} | {topo_edit:.2} |",
            doc.topo.faces.len() - 1,
            doc.topo.edges.len()
        );
    }
}

/// The test world with a long history: what grows with history, and what
/// a year change costs in the app (time view, colours, geometry, labels).
fn history(nx: usize, ny: usize, events: usize, refine_km: Option<f64>) {
    use map_core::testworld::{history_world, TestWorldSpec};
    let spec = TestWorldSpec { nx, ny, events, refine_km, ..Default::default() };
    let (mut doc, rep) = history_world(&spec);
    let verts: usize = doc.topo.edges.values().map(|e| e.pts.len()).sum::<usize>() + doc.topo.nodes.len();
    let changes: usize = doc.owner_changes.values().map(|c| c.len()).sum();
    let years = doc.change_years();
    println!("## 测试世界 {nx}x{ny}，{events} 次变化{}", refine_km.map(|k| format!("，边界细化到约 {k} km")).unwrap_or_default());
    if let Some(r) = &rep.refine {
        println!("- 细化：顶点 {} → {}，{} 条边（{} 条降低起伏、{} 条只加密），{:.0} ms", r.vertices_before, r.vertices_after, r.edges, r.calmer, r.straight, r.ms);
    }
    println!("- 生成 {:.0} ms，其中绘制 {:.0} ms（{} 次成功、{} 次被拒，{} 次局部重建，最慢 {:.0} ms）；建国 {}、终止 {}、海岸变化 {}、文化层变化 {}", rep.total_ms, rep.overlay_ms, rep.events_applied, rep.events_failed, rep.local, rep.slowest_overlay_ms, rep.founded, rep.ended, rep.coast_changes, rep.culture_events);
    println!("- 面 {} → {}，边 {}，顶点 {verts}；实体 {}；归属变化点 {changes}（{} 个面有历史），海陆变化面 {}；变化年份 {} 个", rep.faces_start, rep.faces_end, doc.topo.edges.len(), doc.entities.len(), doc.owner_changes.len(), doc.kind_changes.len(), years.len());
    doc.topo.validate().expect("valid");

    // a year change as the app does it
    let mut cache = SceneCache::default();
    let t = Instant::now();
    cache.update_geometry(&doc, 0.0);
    let tess = ms(t);
    let mut labels = map_core::labels::LabelCache::default();
    let t = Instant::now();
    labels.update(&doc);
    println!("- 全量三角化 {tess:.0} ms，{} 个三角形；国家标注全量 {:.0} ms", cache.vertex_count() / 3, ms(t));
    let ys: Vec<i32> = years.iter().copied().step_by((years.len() / 40).max(1)).collect();
    let (mut t_year, mut t_theme, mut t_geom, mut t_label, mut worst) = (0.0, 0.0, 0.0, 0.0, 0.0f64);
    doc.theme.mode = theme::ThemeMode::Political;
    for y in &ys {
        let t = Instant::now();
        doc.set_year(Some(*y));
        let a = ms(t);
        let t = Instant::now();
        let _ = theme::evaluate(&doc);
        let b = ms(t);
        let t = Instant::now();
        cache.update_geometry(&doc, 0.0);
        let c = ms(t);
        let t = Instant::now();
        labels.update(&doc);
        let d = ms(t);
        t_year += a;
        t_theme += b;
        t_geom += c;
        t_label += d;
        worst = worst.max(a + b + c + d);
    }
    let k = ys.len() as f64;
    println!("- 切换年份（{} 个年份平均）：时间视图 {:.2} ms，政治配色 {:.2} ms，几何更新 {:.2} ms，标注 {:.2} ms；单次最慢合计 {worst:.1} ms", ys.len(), t_year / k, t_theme / k, t_geom / k, t_label / k);

    // choropleth: per year, and the scale shared by all years
    doc.theme.mode = theme::ThemeMode::Choropleth;
    doc.theme.field_key = "density".into();
    doc.set_year(Some(1500));
    let t = Instant::now();
    let _ = theme::evaluate(&doc);
    let per_year = ms(t);
    doc.theme.scale = theme::ClassScale::AllYears;
    let t = Instant::now();
    let fixed = theme::fixed_scale(&doc);
    let shared = ms(t);
    println!("- 分级设色（人口密度）：单年 {per_year:.1} ms；各年统一分级 {shared:.0} ms（{} 个年份）", fixed.map_or(0, |f| f.years.len()));
    doc.theme.scale = theme::ClassScale::PerYear;

    // data table and exports in a year
    let t = Instant::now();
    let csv = map_io::table::export(&doc);
    println!("- 1500 年数据表 / CSV {:.1} ms（{} 行）", ms(t), csv.lines().count());
    let t = Instant::now();
    let th = theme::evaluate(&doc);
    let s = map_io::scene::build_scene(&doc, &th, &map_io::scene::ExportOptions { width: 1600, height: 900, ..Default::default() });
    let scene_ms = ms(t);
    let t = Instant::now();
    let _ = map_io::svg::render(&s);
    println!("- 1500 年导出：场景 {scene_ms:.1} ms，SVG {:.1} ms", ms(t));

    // an edit late in history: a border drawn in 1990
    doc.set_year(Some(1990));
    let x = *doc.owner.values().next().unwrap();
    let p = doc.topo.edges.values().find(|e| doc.owner.get(&e.left) == Some(&x) && e.right != map_core::BACKGROUND && doc.owner.get(&e.right).is_some_and(|o| *o != x)).map(|e| { let f = doc.topo.edge_full(e); f[f.len() / 2] }).unwrap();
    let ring: Vec<LonLat> = (0..24).map(|i| { let a = i as f64 / 24.0 * std::f64::consts::TAU; LonLat::new(p.lon + 3.0 * a.cos(), p.lat + 2.0 * a.sin()) }).collect();
    let t = Instant::now();
    let (after, orep) = map_core::overlay::apply(&doc, &ring, map_core::overlay::DrawIntent::Take(x)).unwrap();
    let draw = ms(t);
    let t = Instant::now();
    cache.update_geometry(&after, 0.0);
    println!("- 1990 年画一条新边界：{draw:.1} ms（局部={}{}，重建 {} 个面），之后几何更新 {:.1} ms", orep.local, orep.fallback.map(|f| format!("，原因：{f}")).unwrap_or_default(), orep.rebuilt_faces, ms(t));

    // tidying faces whose history became identical
    let mut tidy = doc.clone();
    let t = Instant::now();
    let merged = tidy.tidy_faces();
    println!("- 整理地块：合并 {merged} 个，{:.0} ms", ms(t));

    doc.set_year(None);
    let scratch = tempfile::tempdir().expect("benchmark directory");
    let path = scratch.path().join("atlas_history_bench.atlas");
    let t = Instant::now();
    map_io::project::save(&path, &doc).unwrap();
    let save = ms(t);
    let size = std::fs::metadata(&path).unwrap().len() as f64 / 1024.0;
    let t = Instant::now();
    let back = map_io::project::load(&path).unwrap();
    println!("- 保存 {save:.0} ms，读取 {:.0} ms，文件 {size:.0} KB", ms(t));
    assert_eq!(back.owner_changes, doc.owner_changes);
    // ATLAS_BENCH_KEEP: leave the world there (to open it in the application)
    if std::env::var_os("ATLAS_BENCH_KEEP").is_some() {
        println!("- 世界保留在 {}", scratch.keep().join("atlas_history_bench.atlas").display());
    }
}


fn authoring() {
    use map_core::{environment, visualization, world_book::WorldBook, worldview};
    use std::{collections::BTreeMap, sync::Arc};

    let mut doc = grid_world(80, 50, 8, 42);
    let mut config = environment::Project::default();
    config.settings.resolution = 768;
    let start = Instant::now();
    let generation = environment::generate(&doc, &config).expect("generate environment");
    println!("environment {} × {}: {:.1} ms", generation.grid.width, generation.grid.height, ms(start));
    config.current = Some(Arc::new(generation));
    doc.environment = Some(Arc::new(config));

    let now = Instant::now();
    let schema;
    {
        let catalog = worldview::Catalog::edit(&mut doc);
        schema = catalog.alloc().expect("worldbuilding type id");
        catalog.schemas.insert(schema, Arc::new(worldview::Schema {
            name: "Surveyed site".into(),
            fields: BTreeMap::from([("score".into(), worldview::Field {
                name: "Score".into(), kind: worldview::Kind::Number,
                rule: Some(worldview::Rule { expression: "year * 1.1".into(), inputs: BTreeMap::new() }),
                ..Default::default()
            })]),
            ..Default::default()
        }));
        for i in 0..50_000 {
            let id = catalog.alloc().expect("worldbuilding record id");
            catalog.records.insert(id, Arc::new(worldview::Record {
                schema, name: format!("Site {i}"), ..Default::default()
            }));
        }
        catalog.validate().expect("validate survey catalog");
    }
    println!("worldbuilding create + validate 50,000 records: {:.1} ms", ms(now));
    doc.set_year(Some(2020));
    let chart = visualization::Spec {
        name: "Site survey".into(),
        data: visualization::Data::Property { schema: Some(schema), key: "score".into() },
        ..Default::default()
    };
    let start = Instant::now();
    let output = chart.evaluate(&doc).expect("evaluate chart");
    assert_eq!(output.rows.len(), 50_000);
    println!("chart through 50,000 numeric rules: {:.1} ms", ms(start));
    let start = Instant::now();
    let bytes = map_io::project::to_bytes(&doc).expect("serialize workspace");
    let write = ms(start);
    let start = Instant::now();
    let restored = map_io::project::from_bytes(&bytes).expect("restore workspace");
    assert_eq!(WorldBook::current(&restored), WorldBook::current(&doc));
    println!("workspace {} KiB, serialize {:.1} ms, restore {:.1} ms", bytes.len() / 1024, write, ms(start));
}
