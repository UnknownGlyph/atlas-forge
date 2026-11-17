//! `atlas --self-test <folder>`: the core of the program at work without a
//! window, for checking a package on a machine (or a build): a new world,
//! a partition, roads, saving and reading back, and the three picture
//! exports into the folder. Prints a short report; an error stops it.

use map_core::partition::PartitionSpec;
use map_core::roads::RoadSpec;
use map_core::worldgen::{generate, WorldSpec};
use std::path::Path;
use std::time::Instant;

pub fn run(dir: &Path) -> Result<String, String> {
    let t = Instant::now();
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建输出文件夹 {}：{e}", dir.display()))?;
    let mut out = vec![format!("Atlas Forge {} 自检", env!("CARGO_PKG_VERSION"))];
    // a world
    let (mut d, rep) = generate(&WorldSpec { seed: 7, countries: 10, rivers: 5, ..WorldSpec::default() })?;
    d.topo.validate()?;
    out.push(format!("新世界：{} 个国家，{} 条河，{:.0}% 陆地", rep.countries, rep.rivers, rep.land * 100.0));
    // a partition of the largest country, then roads
    let areas = d.areas_by_entity();
    let big = *areas.iter().max_by(|a, b| a.1.total_cmp(b.1)).ok_or("没有国家")?.0;
    let spec = PartitionSpec { units: 4, ..PartitionSpec::default() };
    let plan = d.partition_plan(big, &spec)?;
    let units = d.apply_partition(&plan, &spec)?;
    d.topo.validate()?;
    out.push(format!("自动划分：{} 块", units.len()));
    let roads = d.road_plan(&RoadSpec::default())?;
    d.apply_roads(&roads)?;
    out.push(format!("道路：{} 条", roads.roads.len()));
    // saved and read back the same
    let file = dir.join("自检.atlas");
    map_io::project::save_with_backups(&file, &d, 1)?;
    let back = map_io::project::load(&file)?;
    if back.topo.faces.len() != d.topo.faces.len() || back.entities.len() != d.entities.len() {
        return Err("保存后读回的世界不同".into());
    }
    out.push(format!("保存与读取：{} KB", std::fs::metadata(&file).map(|m| m.len() / 1024).unwrap_or(0)));
    // the three pictures
    let font = map_io::png::find_system_font();
    match (&font, map_io::fonts::bundled()) {
        (_, Some(_)) => out.push("字体：随附的 Noto Sans SC".into()),
        (Some(_), None) => out.push(format!("字体：系统字体（没有找到随附的 fonts\\{}）", map_io::fonts::BUNDLED)),
        (None, None) => out.push("警告：没有找到中文字体，导出的图片中文字会缺失".into()),
    }
    let theme = map_core::theme::evaluate(&back);
    let opts = map_io::scene::ExportOptions { width: 1200, height: 650, title: "自检".into(), scale_bar: true, north_arrow: true, ..Default::default() };
    let scene = map_io::scene::build_scene(&back, &theme, &opts);
    map_io::png::save(&scene, font.as_deref(), &dir.join("自检.png"))?;
    std::fs::write(dir.join("自检.svg"), map_io::svg::render(&scene)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("自检.pdf"), map_io::pdf::render(&scene, font.as_deref())).map_err(|e| e.to_string())?;
    out.push(format!("导出：PNG、SVG、PDF 到 {}", dir.display()));
    out.push(format!("全部通过，用时 {:.1} s", t.elapsed().as_secs_f64()));
    let report = out.join("
");
    // (a windowed program has no console to print to)
    std::fs::write(dir.join("自检报告.txt"), &report).map_err(|e| e.to_string())?;
    Ok(report)
}
