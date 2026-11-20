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
    // A saved reference can be inspected immediately in both map views.
    use map_core::climate::{self, Project, Scenario, Surface};
    let settings = climate::preset(0);
    let ground = climate::sample(&d, &Default::default(), Surface::Unknown);
    let reference = Scenario::new(settings.clone(), d.year, Default::default(), &ground)?;
    let means = reference.summary(reference.region);
    out.push(format!("气候：12 个轨道月份，区域年平均 {:.1} °C、年降水 {:.0} mm", means[0], means[1]));
    d.climate = Some(std::sync::Arc::new(Project {
        settings, current: Some(std::sync::Arc::new(reference)), ..Project::default()
    }));
    // saved and read back the same
    let file = dir.join("自检.atlas");
    map_io::project::save_with_backups(&file, &d, 1)?;
    let back = map_io::project::load(&file)?;
    if back.climate != d.climate { return Err("气候参考未能完整读回".into()); }
    let climate = back.climate.as_ref().unwrap();
    let (w,h,rgba) = climate.current.as_ref().unwrap().raster(climate.field,climate.period);
    image::save_buffer(dir.join("气候参考.png"), &rgba, w as u32, h as u32, image::ColorType::Rgba8).map_err(|e|e.to_string())?;
    if back.topo.faces.len() != d.topo.faces.len() || back.entities.len() != d.entities.len() {
        return Err("保存后读回的世界不同".into());
    }
    out.push(format!("保存与读取：{} KB", std::fs::metadata(&file).map(|m| m.len() / 1024).unwrap_or(0)));
    use map_io::checkpoints::{Kind, Store};
    let history = Store::for_project(&file);
    let pinned = history.capture(&back, Kind::Manual, "制作前", 2)?;
    let mut changed = back.clone();
    for i in 1..=3 {
        changed.meta.name = format!("自动版本 {i}");
        history.capture(&changed, Kind::Automatic, "", 2)?;
    }
    let versions = Store::for_project(&file).list()?;
    if versions.iter().filter(|v| v.kind == Kind::Automatic).count() != 2
        || history.load(&pinned.id)?.meta.name != back.meta.name
        || map_io::project::load(&file)?.meta.name != back.meta.name
    {
        return Err("保存点、自动轮转或主工程校验失败".into());
    }
    out.push("保存点：保留 1 个命名保存点与最新 2 个自动版本，原工程可重读".into());
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
