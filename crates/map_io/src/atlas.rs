//! An atlas: several saved layouts (pages), each made for a list of years,
//! written to one folder with an `index.html` contents page. With
//! `same_scale` a choropleth uses one set of classes for every page and
//! year, so a colour means the same value throughout the atlas.

use crate::batch::{title_for, write_with, year_tag, Format};
use crate::markup::escape as esc;
use crate::scene::ExportOptions;
use map_core::doc::Document;
use map_core::layout::Layout;
use map_core::theme::{ClassScale, ThemeMode};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct AtlasJob {
    pub pages: Vec<Layout>,
    /// Years every page is made for; empty: each page once, as the
    /// document is (a map without years).
    pub years: Vec<i32>,
    pub dir: PathBuf,
    /// PNG or SVG.
    pub format: Format,
    pub font: Option<Vec<u8>>,
    /// One legend scale for the whole atlas (choropleth only).
    pub same_scale: bool,
}

impl AtlasJob {
    /// How many maps the job makes.
    pub fn len(&self) -> usize {
        self.pages.len() * self.years.len().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A layout name made safe for a file name.
pub fn file_stem(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    if s.is_empty() {
        "page".into()
    } else {
        s
    }
}

/// One map of the atlas: page `p` in `year`.
struct Item {
    page: usize,
    year: Option<i32>,
    file: String,
    title: String,
}

fn items(job: &AtlasJob, ext: &str) -> Vec<Item> {
    let mut out = Vec::new();
    // pages may share a name only by accident: number the repeats
    let mut stems: Vec<String> = Vec::new();
    for (p, l) in job.pages.iter().enumerate() {
        let mut stem = file_stem(&l.name);
        let n = stems.iter().filter(|s| **s == stem || s.starts_with(&format!("{stem}~"))).count();
        if n > 0 {
            stem = format!("{stem}~{}", n + 1);
        }
        stems.push(stem.clone());
        if job.years.is_empty() {
            out.push(Item { page: p, year: None, file: format!("{stem}.{ext}"), title: l.title.replace("{year}", "").trim().to_string() });
        }
        for y in &job.years {
            out.push(Item { page: p, year: Some(*y), file: format!("{stem}_{}.{ext}", year_tag(*y)), title: title_for(&l.title, *y) });
        }
    }
    out
}

/// Make every page for every year, on all cores, then write `index.html`.
/// Returns the files written (the index last), or the first error; stops
/// early when `cancel` is set (no index is written then).
pub fn export_atlas(doc: &Document, job: &AtlasJob, done: &AtomicUsize, cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
    if !matches!(job.format, Format::Png | Format::Svg | Format::Pdf) {
        return Err("地图集只能导出 PNG、SVG 或 PDF".into());
    }
    if job.pages.is_empty() {
        return Err("地图集没有版式".into());
    }
    std::fs::create_dir_all(&job.dir).map_err(|e| format!("{}: {e}", job.dir.display()))?;
    let list = items(job, job.format.ext());
    // one scale for every page and year
    let fixed = {
        let mut d = doc.clone();
        if job.same_scale && d.theme.mode == ThemeMode::Choropleth {
            d.theme.scale = ClassScale::AllYears;
        }
        map_core::theme::fixed_scale(&d)
    };
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Result<PathBuf, String>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(list.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(it) = list.get(i) else { return };
                let mut d = doc.clone();
                if let Some(y) = it.year {
                    d.set_year(Some(y));
                }
                let mut opts = ExportOptions::from_layout(&job.pages[it.page]);
                opts.title = it.title.clone();
                let path = job.dir.join(&it.file);
                let r = write_with(&d, job.format, &opts, job.font.as_deref(), &path, fixed.as_ref()).map(|_| path);
                results.lock().unwrap().push((i, r));
                done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    let mut r = results.into_inner().unwrap();
    r.sort_by_key(|x| x.0);
    let mut files: Vec<PathBuf> = r.into_iter().map(|x| x.1).collect::<Result<_, _>>()?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(files);
    }
    let index = job.dir.join("index.html");
    std::fs::write(&index, index_html(doc, job, &list)).map_err(|e| format!("{}: {e}", index.display()))?;
    files.push(index);
    Ok(files)
}

/// The contents page: one section per layout, its maps in year order.
fn index_html(doc: &Document, job: &AtlasJob, list: &[Item]) -> String {
    let name = esc(&doc.meta.name);
    let years = match (job.years.first(), job.years.last()) {
        (Some(a), Some(b)) if a != b => format!("{a}–{b} 年，{} 个年份", job.years.len()),
        (Some(a), _) => format!("{a} 年"),
        _ => "不分年份".into(),
    };
    let mut h = format!(
        "<!doctype html>\n<html lang=\"zh\">\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{name} 地图集</title>\n<style>\nbody{{font-family:system-ui,\"Microsoft YaHei\",sans-serif;margin:24px auto;max-width:1200px;padding:0 16px;color:#222;background:#fff}}\nh1{{margin-bottom:4px}}\nnav a{{margin-right:16px}}\nfigure{{margin:0 0 32px}}\nimg{{max-width:100%;border:1px solid #ccc}}\nfigcaption{{color:#555;margin-top:4px}}\n</style>\n<h1>{name} 地图集</h1>\n<p>{} 个版式，{years}，共 {} 幅。</p>\n<nav>",
        job.pages.len(),
        list.len()
    );
    for (p, l) in job.pages.iter().enumerate() {
        h.push_str(&format!("<a href=\"#p{p}\">{}</a>", esc(&l.name)));
    }
    h.push_str("</nav>\n");
    for (p, l) in job.pages.iter().enumerate() {
        h.push_str(&format!("<h2 id=\"p{p}\">{}</h2>\n", esc(&l.name)));
        for it in list.iter().filter(|it| it.page == p) {
            let cap = match (it.title.is_empty(), it.year) {
                (false, _) => esc(&it.title),
                (true, Some(y)) => format!("{y} 年").into(),
                (true, None) => esc(&l.name),
            };
            h.push_str(&format!("<figure><img src=\"{}\" alt=\"{cap}\" loading=\"lazy\"><figcaption>{cap}</figcaption></figure>\n", esc(&it.file)));
        }
    }
    h.push_str("</html>\n");
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::doc::{FieldKind, Value};
    use map_core::{EntityKind, FaceKind, LonLat};

    /// Two countries; the eastern square passes from B to A in 1930;
    /// population recorded in 1900 and 1950.
    fn world() -> Document {
        let mut d = Document::new("测试世界", 6371.0);
        let sq = |x: f64| vec![LonLat::new(x, 0.0), LonLat::new(x + 10.0, 0.0), LonLat::new(x + 10.0, 10.0), LonLat::new(x, 10.0)];
        let f: Vec<_> = [0.0, 20.0, 40.0].iter().map(|x| d.add_ring_face(&sq(*x), FaceKind::Land).unwrap()).collect();
        let (a, b) = (d.add_entity(EntityKind::Country, "甲"), d.add_entity(EntityKind::Country, "乙"));
        d.assign(f[0], Some(a)).unwrap();
        d.assign(f[1], Some(b)).unwrap();
        d.assign(f[2], Some(b)).unwrap();
        d.set_year(Some(1930));
        d.assign(f[2], Some(a)).unwrap();
        d.set_year(None);
        let pop = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
        for (e, v0, v1) in [(a, 100.0, 1000.0), (b, 10.0, 500.0)] {
            d.set_value(e, pop, Some(1900), Some(Value::Num(v0)));
            d.set_value(e, pop, Some(1950), Some(Value::Num(v1)));
        }
        d.theme.mode = ThemeMode::Choropleth;
        d.theme.field_key = "pop".into();
        d.theme.method = map_core::theme::ClassMethod::EqualInterval;
        d.theme.classes = 2;
        d
    }

    fn page(name: &str, title: &str) -> Layout {
        ExportOptions { width: 400, height: 240, title: title.into(), ..Default::default() }.to_layout(name)
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn pages_times_years_with_contents() {
        let d = world();
        let dir = tmp("svg");
        let job = AtlasJob {
            pages: vec![page("全图", "{year} 年人口"), page("东部: 细节", "")],
            years: vec![1900, 1950],
            dir: dir.clone(),
            format: Format::Svg,
            font: None,
            same_scale: true,
        };
        let (done, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
        let files = export_atlas(&d, &job, &done, &cancel).unwrap();
        assert_eq!(done.load(Ordering::Relaxed), 4);
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["全图_1900.svg", "全图_1950.svg", "东部_ 细节_1900.svg", "东部_ 细节_1950.svg", "index.html"]);
        let index = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(index.contains("2 个版式，1900–1950 年，2 个年份，共 4 幅"));
        assert!(index.contains("src=\"全图_1950.svg\"") && index.contains("1950 年人口"));
        // one legend scale: both years' legends cover 10–1000
        for y in ["1900", "1950"] {
            let svg = std::fs::read_to_string(dir.join(format!("全图_{y}.svg"))).unwrap();
            assert!(svg.contains("10 – 505") && svg.contains("505 – 1000"), "{y}");
            assert!(svg.contains("各年统一分级"), "{y}");
        }
        // without it each year's legend is its own
        let job = AtlasJob { same_scale: false, dir: tmp("own"), ..job };
        export_atlas(&d, &job, &done, &cancel).unwrap();
        let svg = std::fs::read_to_string(job.dir.join("全图_1900.svg")).unwrap();
        assert!(!svg.contains("10 – 505") && !svg.contains("各年统一分级"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&job.dir);
    }

    #[test]
    fn a_map_without_years_gets_one_map_per_page() {
        let mut d = world();
        d.theme.mode = ThemeMode::Political;
        let dir = tmp("noyear");
        let job = AtlasJob { pages: vec![page("A", "标题 {year}"), page("A", "")], years: vec![], dir: dir.clone(), format: Format::Png, font: None, same_scale: true };
        assert_eq!(job.len(), 2);
        let files = export_atlas(&d, &job, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["A.png", "A~2.png", "index.html"]);
        let index = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(index.contains("不分年份") && index.contains("<figcaption>标题</figcaption>"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_picture_formats_and_some_pages() {
        let d = world();
        let job = AtlasJob { pages: vec![], years: vec![1900], dir: tmp("none"), format: Format::Png, font: None, same_scale: true };
        assert!(export_atlas(&d, &job, &AtomicUsize::new(0), &AtomicBool::new(false)).is_err());
        let job = AtlasJob { pages: vec![page("A", "")], format: Format::Csv, ..job };
        assert!(export_atlas(&d, &job, &AtomicUsize::new(0), &AtomicBool::new(false)).is_err());
    }
}
