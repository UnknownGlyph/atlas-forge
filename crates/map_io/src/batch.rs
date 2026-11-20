//! One map per year: the document is viewed at each year (ownership,
//! hierarchy and data as of then) and written to `<stem>_<year>.<ext>`.
//! Years are rendered on all cores; progress and cancellation are shared
//! with the caller.

use crate::scene::{self, ExportOptions};
use map_core::doc::Document;
use map_core::theme::FixedScale;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Svg,
    /// Vector PDF, text as glyph outlines.
    Pdf,
    GeoJsonCountries,
    GeoJsonFaces,
    GeoJsonRivers,
    Csv,
    /// Shapefile sets (.shp with .shx, .dbf, .prj, .cpg), as the GeoJSON ones.
    ShpCountries,
    ShpFaces,
    ShpRivers,
}

impl Format {
    pub fn geographic(self) -> bool {
        matches!(self, Self::GeoJsonCountries | Self::GeoJsonFaces | Self::GeoJsonRivers
            | Self::ShpCountries | Self::ShpFaces | Self::ShpRivers)
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Pdf => "pdf",
            Format::GeoJsonCountries | Format::GeoJsonFaces | Format::GeoJsonRivers => "geojson",
            Format::Csv => "csv",
            Format::ShpCountries | Format::ShpFaces | Format::ShpRivers => "shp",
        }
    }
}

/// Write one file of `format` for the document as it is (its own year).
pub fn write_one(doc: &Document, format: Format, opts: &ExportOptions, font: Option<&[u8]>, path: &Path) -> Result<(), String> {
    write_with(doc, format, opts, font, path, map_core::theme::fixed_scale(doc).as_ref())
}

/// As [`write_one`], with a thematic scale shared by all years worked out
/// once by the caller (see [`map_core::theme::fixed_scale`]).
pub(crate) fn write_with(doc: &Document, format: Format, opts: &ExportOptions, font: Option<&[u8]>, path: &Path, fixed: Option<&FixedScale>) -> Result<(), String> {
    if doc.paper.is_some() && format.geographic() {
        return Err("平面画布请导出 PNG、SVG 或 PDF。地理坐标文件适用于地理地图。".into());
    }
    let io = |r: std::io::Result<()>| r.map_err(|e| format!("{}: {e}", path.display()));
    match format {
        Format::Png | Format::Svg | Format::Pdf => {
            let theme = map_core::theme::evaluate_with(doc, fixed);
            let s = scene::build_scene(doc, &theme, opts);
            match format {
                Format::Png => crate::png::save(&s, font, path),
                Format::Pdf => io(std::fs::write(path, crate::pdf::render(&s, font))),
                _ => io(std::fs::write(path, crate::svg::render(&s))),
            }
        }
        Format::GeoJsonCountries => io(std::fs::write(path, crate::geojson::export_countries(doc))),
        Format::GeoJsonFaces => io(std::fs::write(path, crate::geojson::export_faces(doc))),
        Format::GeoJsonRivers => io(std::fs::write(path, crate::geojson::export_rivers(doc))),
        Format::Csv => io(std::fs::write(path, format!("\u{feff}{}", crate::table::export(doc)))),
        Format::ShpCountries => crate::shapefile::write_geojson(&crate::geojson::export_countries(doc), path).map(|_| ()),
        Format::ShpFaces => crate::shapefile::write_geojson(&crate::geojson::export_faces(doc), path).map(|_| ()),
        Format::ShpRivers => crate::shapefile::write_geojson(&crate::geojson::export_rivers(doc), path).map(|_| ()),
    }
}

/// Year as it appears in file names: zero-padded so files sort by year
/// (`0800`, `1900`), negative years as `-0200`.
pub fn year_tag(y: i32) -> String {
    if y < 0 {
        format!("-{:04}", -(y as i64))
    } else {
        format!("{y:04}")
    }
}

/// Title for one year: `{year}` is replaced; a title without it gets the
/// year appended, and an empty one becomes just the year.
pub fn title_for(template: &str, y: i32) -> String {
    let t = template.trim();
    if t.is_empty() {
        format!("{y} 年")
    } else if t.contains("{year}") {
        t.replace("{year}", &y.to_string())
    } else {
        format!("{t}（{y} 年）")
    }
}

/// As [`title_for`], with the years shown in the world's own calendar
/// (when it has one turned on).
pub fn title_in(template: &str, y: i32, cal: &map_core::calendar::Calendar) -> String {
    if !cal.on {
        return title_for(template, y);
    }
    let t = template.trim();
    if t.is_empty() {
        cal.show(y)
    } else if t.contains("{year}") {
        t.replace("{year}", &cal.show_short(y))
    } else {
        format!("{t}（{}）", cal.show(y))
    }
}

/// The years a "change years" export covers: the year before the first
/// change (the starting map) and every change year.
pub fn change_year_list(doc: &Document) -> Vec<i32> {
    let ys = doc.change_years();
    let mut out: Vec<i32> = ys.first().map(|f| vec![f - 1]).unwrap_or_default();
    out.extend(ys);
    out
}

pub fn range_year_list(from: i32, to: i32, step: i32) -> Vec<i32> {
    let step = step.max(1);
    let (a, b) = (from.min(to), from.max(to));
    let mut v: Vec<i32> = (0..).map(|i| a + i * step).take_while(|y| *y <= b).collect();
    if v.last() != Some(&b) {
        v.push(b);
    }
    v
}

pub struct Job {
    pub years: Vec<i32>,
    pub dir: PathBuf,
    pub stem: String,
    pub format: Format,
    /// Map options; `title` is a template (see [`title_for`]).
    pub opts: ExportOptions,
    pub font: Option<Vec<u8>>,
}

/// Run a job. Returns the written files in year order, or the first error.
/// Stops early (returning what was written) when `cancel` is set.
pub fn export_years(doc: &Document, job: &Job, done: &AtomicUsize, cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(&job.dir).map_err(|e| format!("{}: {e}", job.dir.display()))?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(i32, Result<PathBuf, String>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(job.years.len().max(1));
    // one scale for every year when the theme asks for it
    let fixed = map_core::theme::fixed_scale(doc);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&y) = job.years.get(i) else { return };
                let mut d = doc.clone();
                d.set_year(Some(y));
                let mut opts = job.opts.clone();
                if matches!(job.format, Format::Png | Format::Svg | Format::Pdf) {
                    opts.title = title_in(&job.opts.title, y, &doc.calendar);
                }
                let path = job.dir.join(format!("{}_{}.{}", job.stem, year_tag(y), job.format.ext()));
                let r = write_with(&d, job.format, &opts, job.font.as_deref(), &path, fixed.as_ref()).map(|_| path);
                results.lock().unwrap().push((y, r));
                done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    let mut r = results.into_inner().unwrap();
    r.sort_by_key(|x| x.0);
    r.into_iter().map(|x| x.1).collect()
}

/// Animated output of a per-year export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animation {
    /// Animated GIF (always available; 256 colours per frame).
    Gif,
    /// H.264 MP4 through an `ffmpeg` found on PATH.
    Mp4,
}

impl Animation {
    pub fn ext(self) -> &'static str {
        match self {
            Animation::Gif => "gif",
            Animation::Mp4 => "mp4",
        }
    }
}

/// Whether `ffmpeg` can be run (for MP4 export).
pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg").arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

#[derive(Clone)]
pub struct AnimJob {
    pub years: Vec<i32>,
    pub path: PathBuf,
    pub kind: Animation,
    /// Map options; `title` is a template (see [`title_for`]).
    pub opts: ExportOptions,
    pub font: Option<Vec<u8>>,
    /// How long each year is shown; the last year is held twice as long.
    pub frame_ms: u32,
}

/// One year rendered to straight-alpha RGBA.
fn render_frame(doc: &Document, y: i32, job: &AnimJob, fixed: Option<&FixedScale>) -> Result<image::RgbaImage, String> {
    let mut d = doc.clone();
    d.set_year(Some(y));
    if let Some(project) = d.environment.clone().filter(|p| p.live && p.selected.is_none()) {
        // Evaluate with the geometry and rule values of the rendered year.
        // Frozen and manually generated versions stay as authored.
        let current = map_core::environment::generate(&d, &project)?;
        std::sync::Arc::make_mut(d.environment.as_mut().unwrap()).current = Some(std::sync::Arc::new(current));
    }
    if let Some(project) = d.climate.clone().filter(|p| p.live && p.visible && p.selected.is_none()) {
        use map_core::climate::{sample, Scenario};
        let ground = sample(&d, &project.surface_edits, project.background);
        let mut current = Scenario::new(project.settings.clone(), d.year, project.region, &ground)?;
        if let Some(previous) = &project.current { current.retain_locks(previous); }
        std::sync::Arc::make_mut(d.climate.as_mut().unwrap()).current = Some(std::sync::Arc::new(current));
    }
    let mut opts = job.opts.clone();
    opts.title = title_in(&job.opts.title, y, &doc.calendar);
    let theme = map_core::theme::evaluate_with(&d, fixed);
    let pm = crate::png::render(&scene::build_scene(&d, &theme, &opts), job.font.as_deref())?;
    let mut px = Vec::with_capacity(pm.data().len());
    for c in pm.pixels() {
        let c = c.demultiply();
        px.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    image::RgbaImage::from_raw(pm.width(), pm.height(), px).ok_or_else(|| "帧尺寸无效".to_string())
}

/// Where frames go, in year order.
enum Sink {
    Gif(image::codecs::gif::GifEncoder<std::io::BufWriter<std::fs::File>>),
    Ffmpeg(std::process::Child),
}

impl Sink {
    fn open(job: &AnimJob, w: u32, h: u32) -> Result<Sink, String> {
        match job.kind {
            Animation::Gif => {
                let f = std::fs::File::create(&job.path).map_err(|e| format!("{}: {e}", job.path.display()))?;
                let mut enc = image::codecs::gif::GifEncoder::new_with_speed(std::io::BufWriter::new(f), 10);
                enc.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                Ok(Sink::Gif(enc))
            }
            Animation::Mp4 => {
                let child = std::process::Command::new("ffmpeg")
                    .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}")])
                    .args(["-framerate", &format!("1000/{}", job.frame_ms.max(1)), "-i", "-"])
                    // H.264 needs even sizes; 30 fps output so players seek smoothly
                    .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2,fps=30", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
                    .arg(&job.path)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("无法启动 ffmpeg：{e}"))?;
                Ok(Sink::Ffmpeg(child))
            }
        }
    }

    fn push(&mut self, img: &image::RgbaImage, ms: u32, frame_ms: u32) -> Result<(), String> {
        match self {
            Sink::Gif(enc) => {
                let delay = image::Delay::from_numer_denom_ms(ms, 1);
                enc.encode_frame(image::Frame::from_parts(img.clone(), 0, 0, delay)).map_err(|e| format!("GIF 编码失败：{e}"))
            }
            Sink::Ffmpeg(child) => {
                use std::io::Write;
                let stdin = child.stdin.as_mut().ok_or("ffmpeg 输入已关闭")?;
                // a held frame is the same frame repeated
                for _ in 0..(ms / frame_ms.max(1)).max(1) {
                    stdin.write_all(img.as_raw()).map_err(|e| format!("写入 ffmpeg 失败：{e}"))?;
                }
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<(), String> {
        match self {
            Sink::Gif(enc) => {
                drop(enc);
                Ok(())
            }
            Sink::Ffmpeg(mut child) => {
                drop(child.stdin.take());
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(format!("ffmpeg 失败：{}", String::from_utf8_lossy(&out.stderr).trim()))
                }
            }
        }
    }

    fn abort(self) {
        if let Sink::Ffmpeg(mut child) = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Render every year and write one animation, in year order. Frames are
/// rendered a batch at a time on all cores. On cancel or error the partial
/// file is removed; cancel returns Ok(None).
pub fn export_animation(doc: &Document, job: &AnimJob, done: &AtomicUsize, cancel: &AtomicBool) -> Result<Option<PathBuf>, String> {
    write_animation(doc,job,None,done,cancel)
}

pub fn export_scheduled(
    doc: &Document,
    job: &AnimJob,
    schedule: &[map_core::playback::Sample],
    done: &AtomicUsize,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>, String> {
    if schedule.iter().any(|s| s.hold_ms == 0 || s.hold_ms % 40 != 0) {
        return Err("视频时间表的间隔无效。".into());
    }
    let mut job=job.clone();
    job.years=schedule.iter().map(|s|s.year).collect();
    job.frame_ms=40;
    write_animation(doc,&job,Some(schedule),done,cancel)
}

fn write_animation(doc:&Document, output:&AnimJob, schedule:Option<&[map_core::playback::Sample]>,done:&AtomicUsize,cancel:&AtomicBool)->Result<Option<PathBuf>,String>{
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let session = NEXT.fetch_add(1,Ordering::Relaxed);
    let mut job = output.clone();
    job.path = output.path.with_file_name(format!(".atlas-render-{}-{session}.{}",std::process::id(),output.kind.ext()));
    // Keep the requested file intact until the whole export has succeeded.
    let result = render_animation(doc,&job,schedule,done,cancel);
    if matches!(&result,Ok(Some(_))) && !cancel.load(Ordering::Relaxed) {
        let commit = std::fs::rename(&job.path,&output.path).map_err(|e|format!("视频写入失败：{e}"));
        if commit.is_ok() { return Ok(Some(output.path.clone())); }
        let _=std::fs::remove_file(&job.path);
        commit?;
    }
    let _=std::fs::remove_file(&job.path);
    result.map(|_|None)
}

fn render_animation(doc: &Document, job: &AnimJob, schedule: Option<&[map_core::playback::Sample]>, done: &AtomicUsize, cancel: &AtomicBool) -> Result<Option<PathBuf>, String> {
    if job.years.is_empty() {
        return Err("没有要导出的年份".into());
    }
    let pixels = u64::from(job.opts.width).saturating_mul(u64::from(job.opts.height)).max(1);
    let memory_workers = ((128 * 1024 * 1024) / pixels.saturating_mul(12)).clamp(1,8) as usize;
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(memory_workers);
    let fixed = map_core::theme::fixed_scale(doc);
    let fixed = fixed.as_ref();
    let mut sink: Option<Sink> = None;
    let fail = |sink: Option<Sink>, e: String| {
        if let Some(s) = sink {
            s.abort();
        }
        let _ = std::fs::remove_file(&job.path);
        e
    };
    for (ci, chunk) in job.years.chunks(workers).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            fail(sink, String::new());
            return Ok(None);
        }
        let frames: Vec<Result<image::RgbaImage, String>> = std::thread::scope(|s| {
            let hs: Vec<_> = chunk.iter().map(|y| s.spawn(move || render_frame(doc, *y, job, fixed))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("渲染线程意外退出".into()))).collect()
        });
        for (k, f) in frames.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                fail(sink,String::new());
                return Ok(None);
            }
            let img = match f {
                Ok(i) => i,
                Err(e) => return Err(fail(sink, e)),
            };
            if sink.is_none() {
                sink = Some(Sink::open(job, img.width(), img.height()).map_err(|e| fail(None, e))?);
            }
            let index = ci * workers + k;
            let last = index + 1 == job.years.len();
            let ms = schedule.map_or_else(|| if last { job.frame_ms.saturating_mul(2) } else { job.frame_ms }, |s|s[index].hold_ms);
            if let Err(e) = sink.as_mut().unwrap().push(&img, ms, job.frame_ms) {
                return Err(fail(sink, e));
            }
            done.fetch_add(1, Ordering::Relaxed);
        }
    }
    sink.unwrap().finish().map_err(|e| fail(None, e))?;
    Ok(Some(job.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::{EntityKind, FaceKind, LonLat};

    fn world() -> Document {
        let mut d = Document::default();
        let a = d.add_entity(EntityKind::Country, "甲");
        let b = d.add_entity(EntityKind::Country, "乙");
        let f = d.add_ring_face(&[LonLat::new(0.0, 0.0), LonLat::new(20.0, 0.0), LonLat::new(20.0, 10.0), LonLat::new(0.0, 10.0)], FaceKind::Land).unwrap();
        let (x, y) = d.split_face(&[LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0)]).unwrap();
        assert_eq!(x, f);
        d.assign(x, Some(a)).unwrap();
        d.assign(y, Some(a)).unwrap();
        d.set_year(Some(1900));
        d.assign(y, Some(b)).unwrap();
        d.set_year(Some(1950));
        d.assign(x, Some(b)).unwrap();
        d.set_year(None);
        d
    }

    #[test]
    fn animations_have_one_frame_per_year() {
        let d = world();
        let dir = std::env::temp_dir().join(format!("atlas_anim_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = ExportOptions { width: 320, height: 180, labels: false, legend: false, ..Default::default() };
        let mut job = AnimJob { years: change_year_list(&d), path: dir.join("a.gif"), kind: Animation::Gif, opts, font: None, frame_ms: 500 };
        let (done, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
        let out = export_animation(&d, &job, &done, &cancel).unwrap().unwrap();
        assert_eq!(done.load(Ordering::Relaxed), 3);
        // decode: three frames, the last held twice as long, and the frames differ
        use image::AnimationDecoder;
        let dec = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
        let frames: Vec<image::Frame> = dec.into_frames().collect::<Result<_, _>>().unwrap();
        assert_eq!(frames.len(), 3);
        let ms = |f: &image::Frame| {
            let (n, dd) = f.delay().numer_denom_ms();
            n / dd
        };
        assert_eq!((ms(&frames[0]), ms(&frames[2])), (500, 1000));
        assert_eq!(frames[0].buffer().dimensions(), (320, 180));
        assert_ne!(frames[0].buffer(), frames[1].buffer(), "1899 and 1900 must look different");
        // MP4 when ffmpeg is installed
        if ffmpeg_available() {
            job.kind = Animation::Mp4;
            job.path = dir.join("a.mp4");
            let out = export_animation(&d, &job, &done, &cancel).unwrap().unwrap();
            assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        }
        // cancelled: no file left behind
        cancel.store(true, Ordering::Relaxed);
        job.kind = Animation::Gif;
        job.path = dir.join("b.gif");
        assert_eq!(export_animation(&d, &job, &done, &cancel).unwrap(), None);
        assert!(!job.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn year_lists_and_names() {
        let d = world();
        assert_eq!(change_year_list(&d), vec![1899, 1900, 1950]);
        assert_eq!(range_year_list(1900, 1950, 20), vec![1900, 1920, 1940, 1950]);
        assert_eq!(range_year_list(1950, 1900, 25), vec![1900, 1925, 1950]);
        assert_eq!((year_tag(800), year_tag(1900), year_tag(-200)), ("0800".into(), "1900".into(), "-0200".into()));
        assert_eq!(title_for("", 1900), "1900 年");
        assert_eq!(title_for("{year} 年的世界", 1900), "1900 年的世界");
        assert_eq!(title_for("世界", 1900), "世界（1900 年）");
        // in the world's own calendar (off: plain years)
        let mut cal = map_core::calendar::Calendar::default();
        assert_eq!(title_in("世界", 1900, &cal), "世界（1900 年）");
        cal.on = true;
        cal.epoch = 1800;
        assert_eq!(title_in("", 1900, &cal), "新历101年");
        assert_eq!(title_in("{year}的世界", 1900, &cal), "新历101年的世界");
        assert_eq!(title_in("世界", 1799, &cal), "世界（新历前1年）");
    }

    #[test]
    fn one_file_per_year_showing_that_year() {
        let d = world();
        let dir = std::env::temp_dir().join(format!("atlas_years_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut job = Job { years: change_year_list(&d), dir: dir.clone(), stem: "世界".into(), format: Format::GeoJsonCountries, opts: ExportOptions::default(), font: None };
        let (done, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
        let files = export_years(&d, &job, &done, &cancel).unwrap();
        assert_eq!(done.load(Ordering::Relaxed), 3);
        let names: Vec<String> = files.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["世界_1899.geojson", "世界_1900.geojson", "世界_1950.geojson"]);
        // countries present in each year's file
        let countries = |p: &PathBuf| {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            let mut n: Vec<String> = v["features"].as_array().unwrap().iter().map(|f| f["properties"]["name"].as_str().unwrap().to_string()).collect();
            n.sort();
            n
        };
        assert_eq!(countries(&files[0]), vec!["甲"]);
        assert_eq!(countries(&files[1]), vec!["乙", "甲"]);
        assert_eq!(countries(&files[2]), vec!["乙"]);
        // SVG titles carry the year; the document itself is untouched
        job.format = Format::Svg;
        job.opts.title = "{year} 年".into();
        let files = export_years(&d, &job, &done, &cancel).unwrap();
        assert!(std::fs::read_to_string(&files[1]).unwrap().contains("1900 年"));
        assert_eq!(d.year, None);
        // cancelled before starting: nothing written
        cancel.store(true, Ordering::Relaxed);
        job.stem = "取消".into();
        assert!(export_years(&d, &job, &done, &cancel).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
