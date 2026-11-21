//! Convert a trace, including an interrupted recording, to an offline HTML report.
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

#[derive(Default, Serialize)]
struct Stats {
    count: usize,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Stats {
    fn compute(values: &mut [f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable_by(f64::total_cmp);
        let percentile =
            |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
        Self {
            count: values.len(),
            p50: percentile(0.50),
            p95: percentile(0.95),
            p99: percentile(0.99),
            max: values[values.len() - 1],
        }
    }
}

#[derive(Serialize)]
struct Report {
    header: Value,
    duration: f64,
    frames: Stats,
    intervals: Stats,
    interacting: Stats,
    slow_frames: [usize; 3],
    discarded_lines: usize,
    dropped_records: Option<u64>,
    series: BTreeMap<String, Vec<[f64; 2]>>,
    operations: BTreeMap<String, Stats>,
    worst: Vec<Value>,
    markers: Vec<MarkerReport>,
    input_backlog: Vec<Stall>,
}

const STALL_MS: f64 = 50.0;
const MARKER_LOOKBACK_MS: f64 = 10_000.0;

#[derive(Clone, Serialize)]
struct Stall {
    at_ms: f64,
    frame: u64,
    ui_ms: f64,
    interval_ms: f64,
    workspace: String,
    layout: String,
    pointer: u64,
    raw_motion: Option<u64>,
    other: u64,
    active: bool,
    burst: bool,
}

impl Stall {
    fn from_frame(value: &Value, time: f64, ui_ms: f64) -> Option<Self> {
        let frame = value["number"].as_u64().unwrap_or(0);
        // The first egui interval can begin before recording was enabled.
        let interval_ms = if frame > 1 {
            number(value, "interval_ms").unwrap_or(0.0)
        } else {
            0.0
        };
        if ui_ms < STALL_MS && interval_ms < STALL_MS {
            return None;
        }
        let input = &value["input"];
        let count = |key: &str| input[key].as_u64().unwrap_or(0);
        let pointer = count("pointer");
        let raw_motion = input["raw_motion"].as_u64();
        let other = count("other");
        let burst = pointer >= 20 || raw_motion.is_some_and(|n| n >= 40);
        let active = value["focused"].as_bool().unwrap_or(true)
            && (burst
                || input["dragging"].as_bool() == Some(true)
                || count("wheel") > 0
                || count("touch") > 1);
        Some(Self {
            at_ms: time,
            frame,
            ui_ms,
            interval_ms,
            workspace: value["workspace"].as_str().unwrap_or_default().to_owned(),
            layout: value["layout"].as_str().unwrap_or_default().to_owned(),
            pointer,
            raw_motion,
            other,
            active,
            burst,
        })
    }
}

#[derive(Serialize)]
struct MarkerReport {
    at_ms: f64,
    label: String,
    nearest: Option<Stall>,
    longest_interval: Option<Stall>,
    longest_ui: Option<Stall>,
}

impl MarkerReport {
    fn from_trace(at_ms: f64, label: String, stalls: &[Stall]) -> Self {
        let start = stalls.partition_point(|s| s.at_ms < at_ms - MARKER_LOOKBACK_MS);
        let end = stalls.partition_point(|s| s.at_ms <= at_ms);
        let slice = &stalls[start..end];
        let gaps = || {
            slice
                .iter()
                .filter(|s| s.active && s.interval_ms >= STALL_MS)
        };
        Self {
            at_ms,
            label,
            // A backlog is stronger evidence than the first frame after idle.
            nearest: gaps()
                .rfind(|s| s.burst)
                .or_else(|| gaps().next_back())
                .cloned(),
            longest_interval: gaps()
                .max_by(|a, b| a.interval_ms.total_cmp(&b.interval_ms))
                .cloned(),
            longest_ui: slice
                .iter()
                .filter(|s| s.ui_ms >= STALL_MS)
                .max_by(|a, b| a.ui_ms.total_cmp(&b.ui_ms))
                .cloned(),
        }
    }
}

fn number(value: &Value, field: &str) -> Option<f64> {
    value
        .get(field)?
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
}

const CHART_BUCKETS: usize = 3500;
const CHART_LIMIT: usize = CHART_BUCKETS * 4 + 4;

fn chart_point(
    series: &mut BTreeMap<String, Vec<[f64; 2]>>,
    name: &str,
    t: f64,
    value: Option<f64>,
) {
    if let Some(value) = value {
        let points = if let Some(existing) = series.get_mut(name) {
            existing
        } else {
            series.entry(name.to_owned()).or_default()
        };
        points.push([t, value]);
        if points.len() >= CHART_LIMIT {
            let latest = points.iter().map(|point| point[0]).fold(0.0_f64, f64::max);
            downsample(points, latest);
        }
    }
}

/// Keep both extrema in each time bucket so a single long frame survives
/// even when the report compresses an hours-long trace.
fn downsample(points: &mut Vec<[f64; 2]>, duration: f64) {
    points.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    if points.len() <= CHART_BUCKETS * 2 {
        return;
    }
    let mut reduced = Vec::with_capacity(CHART_BUCKETS * 2);
    let slot = |p: [f64; 2]| ((p[0] / duration.max(1.0)) * CHART_BUCKETS as f64) as usize;
    let mut pos = 0;
    while pos < points.len() {
        let bucket = slot(points[pos]);
        let mut low = points[pos];
        let mut high = low;
        while pos < points.len() && slot(points[pos]) == bucket {
            if points[pos][1] < low[1] {
                low = points[pos];
            }
            if points[pos][1] > high[1] {
                high = points[pos];
            }
            pos += 1;
        }
        if high[0] < low[0] {
            std::mem::swap(&mut high, &mut low);
        }
        reduced.push(low);
        if high != low {
            reduced.push(high);
        }
    }
    *points = reduced;
}

fn read(input: impl BufRead) -> io::Result<Report> {
    let mut interacting = Vec::new();
    let (mut frames, mut intervals, mut worst) = (Vec::new(), Vec::new(), Vec::<Value>::new());
    let (mut series, mut operation_ms) = (BTreeMap::new(), BTreeMap::<String, Vec<f64>>::new());
    let mut markers = Vec::new();
    let mut stalls = Vec::new();
    let (mut duration, mut discarded_lines, mut header, mut dropped_records) =
        (0.0f64, 0, None, None);
    let mut slow_frames = [0; 3];
    for line in input.split(b'\n') {
        let line = line?;
        if line.trim_ascii().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            discarded_lines += 1;
            continue;
        };
        let Some(t) = number(&value, "t_ms") else {
            discarded_lines += 1;
            continue;
        };
        duration = duration.max(t);
        match value["kind"].as_str() {
            Some("header") => {
                if value["schema"].as_u64() != Some(1) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unsupported Atlas performance trace schema",
                    ));
                }
                header = Some(value);
            }
            Some("frame") => {
                let Some(ui) = number(&value, "ui_ms") else {
                    discarded_lines += 1;
                    continue;
                };
                frames.push(ui);
                if let Some(stall) = Stall::from_frame(&value, t, ui) {
                    if stall.active && stall.interval_ms >= STALL_MS {
                        chart_point(&mut series, "交互长间隔 · ms", t, Some(stall.interval_ms));
                    }
                    stalls.push(stall);
                }
                let input = &value["input"];
                if input["dragging"].as_bool() == Some(true)
                    || ["wheel", "keys", "pointer", "touch"]
                        .iter()
                        .any(|key| input[key].as_u64().unwrap_or(0) > 0)
                {
                    interacting.push(ui);
                }
                for (i, limit) in [16.67, 33.33, 70.0].iter().enumerate() {
                    slow_frames[i] += usize::from(ui > *limit);
                }
                chart_point(&mut series, "UI · ms", t, Some(ui));
                if let Some(interval) = number(&value, "interval_ms") {
                    if interval > 0.0 && value["number"].as_u64() != Some(1) {
                        intervals.push(interval);
                        chart_point(&mut series, "帧间隔 · ms", t, Some(interval));
                    }
                }
                if let Some(phases) = value["phases"].as_array() {
                    for phase in phases {
                        if let Some(name) = phase["name"].as_str() {
                            if let Some(ms) = number(phase, "ms") {
                                chart_point(&mut series, name, t, Some(ms));
                                operation_ms
                                    .entry(format!("界面阶段 / {name}"))
                                    .or_default()
                                    .push(ms);
                            }
                        }
                    }
                }
                let insert =
                    worst.partition_point(|v| v["ui_ms"].as_f64().unwrap_or_default() >= ui);
                if insert < 100 {
                    worst.insert(insert, value);
                    worst.truncate(100);
                }
            }
            Some("span") => {
                if let (Some(name), Some(ms)) = (value["name"].as_str(), number(&value, "ms")) {
                    let channel = value["channel"].as_str().unwrap_or("operation");
                    operation_ms
                        .entry(format!("{channel} / {name}"))
                        .or_default()
                        .push(ms);
                    if matches!(channel, "绘制回调" | "窗口重绘") {
                        chart_point(&mut series, name, t, Some(ms));
                    } else if ms >= 8.0 {
                        chart_point(&mut series, "较长操作 · ms", t, Some(ms));
                    }
                }
            }
            Some("resources") => {
                for (key, name, divisor) in [
                    ("cpu_percent", "进程 CPU · %（单核＝100）", 1.0),
                    ("gpu_engine_percent", "最忙 GPU 引擎 · %", 1.0),
                    ("resident_bytes", "常驻内存 · MiB", 1048576.0),
                    ("private_bytes", "专用提交内存 · MiB", 1048576.0),
                    ("gpu_dedicated_bytes", "专用显存 · MiB", 1048576.0),
                    ("gpu_shared_bytes", "共享显存 · MiB", 1048576.0),
                    ("read_bytes_per_second", "进程读取 · MiB/s", 1048576.0),
                    ("write_bytes_per_second", "进程写入 · MiB/s", 1048576.0),
                ] {
                    chart_point(
                        &mut series,
                        name,
                        t,
                        number(&value, key).map(|n| n / divisor),
                    );
                }
            }
            Some("marker") => {
                if let Some(label) = value["label"].as_str() {
                    markers.push((t, label.to_owned()));
                }
            }
            Some("end") => dropped_records = value["dropped_records"].as_u64(),
            _ => discarded_lines += 1,
        }
    }
    let Some(header) = header else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an Atlas Forge performance recording",
        ));
    };
    for points in series.values_mut() {
        downsample(points, duration);
    }
    stalls.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));
    markers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let markers = markers
        .into_iter()
        .map(|(at, label)| MarkerReport::from_trace(at, label, &stalls))
        .collect();
    let mut input_backlog: Vec<_> = stalls
        .into_iter()
        .filter(|s| s.active && s.interval_ms >= STALL_MS)
        .collect();
    input_backlog.sort_by(|a, b| b.interval_ms.total_cmp(&a.interval_ms));
    input_backlog.truncate(100);
    Ok(Report {
        header,
        duration,
        frames: Stats::compute(&mut frames),
        intervals: Stats::compute(&mut intervals),
        interacting: Stats::compute(&mut interacting),
        slow_frames,
        worst,
        markers,
        input_backlog,
        discarded_lines,
        dropped_records,
        series,
        operations: operation_ms
            .into_iter()
            .map(|(key, mut samples)| (key, Stats::compute(&mut samples)))
            .collect(),
    })
}

pub fn default_path(input: &Path) -> PathBuf {
    input.with_extension("html")
}

/// Creates a new report file. The JSONL remains the authoritative raw data.
pub fn write(input: &Path, output: &Path) -> io::Result<()> {
    let report = read(BufReader::new(File::open(input)?))?;
    let data = serde_json::to_string(&report)?.replace('<', "\\u003c");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?;
    let mut output = BufWriter::new(file);
    let (start, end) = include_str!("report.html")
        .split_once("__ATLAS_TRACE_DATA__")
        .expect("trace HTML data slot");
    output.write_all(start.as_bytes())?;
    output.write_all(data.as_bytes())?;
    output.write_all(end.as_bytes())?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_keeps_a_slow_frame_and_an_interrupted_log() {
        let log = concat!(
            "{\"t_ms\":0,\"kind\":\"header\",\"schema\":1}\n",
            "{\"t_ms\":16,\"kind\":\"frame\",\"ui_ms\":4,\"interval_ms\":16}\n",
            "{\"t_ms\":160,\"kind\":\"frame\",\"ui_ms\":91,\"interval_ms\":144}\n",
            "{\"t_ms\":170,\"kind\":\"marker\",\"label\":\"缩放\"}\n",
            "{\"t_ms\":172,\"kind\":\"fr"
        );
        let bytes = [log.as_bytes(), &[0xe4, 0xb8]].concat();
        let report = read(bytes.as_slice()).unwrap();
        assert_eq!(report.frames.max, 91.0);
        assert_eq!(report.slow_frames, [1; 3]);
        assert_eq!(report.worst[0]["ui_ms"], 91);
        assert_eq!(report.discarded_lines, 1);
        assert_eq!(report.dropped_records, None);
    }

    #[test]
    fn a_delayed_marker_points_back_to_interaction_and_skips_idle_gaps() {
        let log = [
            serde_json::json!({"t_ms":0,"kind":"header","schema":1}),
            serde_json::json!({"t_ms":20,"kind":"frame","number":1,"ui_ms":2,"interval_ms":60_000,"input":{"pointer":100}}),
            serde_json::json!({"t_ms":300,"kind":"frame","number":2,"ui_ms":150,"interval_ms":20}),
            serde_json::json!({"t_ms":700,"kind":"frame","number":3,"ui_ms":3,"interval_ms":200,"input":{"pointer":71}}),
            serde_json::json!({"t_ms":1000,"kind":"frame","number":4,"ui_ms":4,"interval_ms":900,"input":{"pointer":1}}),
            serde_json::json!({"t_ms":1300,"kind":"marker","label":"late"}),
        ].into_iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n");
        let report = read(log.as_bytes()).unwrap();
        let marker = &report.markers[0];
        assert_eq!(marker.at_ms, 1300.0);
        assert_eq!(marker.nearest.as_ref().map(|s| s.frame), Some(3));
        assert_eq!(marker.longest_interval.as_ref().map(|s| s.frame), Some(3));
        assert_eq!(marker.longest_ui.as_ref().map(|s| s.frame), Some(2));
        assert_eq!(report.input_backlog.len(), 1);
    }

    #[test]
    fn growing_charts_are_bounded_while_the_log_is_read() {
        let mut series = BTreeMap::new();
        for i in 0..100_000 {
            let value = if i == 100 { 400.0 } else { 3.0 };
            chart_point(&mut series, "UI", i as f64, Some(value));
            assert!(series["UI"].len() <= CHART_LIMIT);
        }
        assert!(series["UI"].contains(&[100.0, 400.0]));
    }

    #[test]
    fn downsampling_keeps_both_extrema() {
        let mut points: Vec<_> = (0..15_000).map(|i| [i as f64, 1.0]).collect();
        points[930] = [930.0, 300.0];
        points[931] = [931.0, 0.0];
        downsample(&mut points, 15_000.0);
        assert!(points.len() <= 7000);
        assert!(points.contains(&[930.0, 300.0]));
        assert!(points.contains(&[931.0, 0.0]));
    }
}
