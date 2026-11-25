//! Streaming performance traces. The editor only submits measurements;
//! filesystem and operating-system sampling run on a dedicated worker.
pub mod report;
mod system;

use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCHEMA: u32 = 1;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Default, Serialize)]
pub struct Frame {
    pub number: u64,
    pub ui_ms: f32,
    pub interval_ms: f32,
    pub input: Input,
    pub phases: Vec<Phase>,
    pub workspace: &'static str,
    pub tool: &'static str,
    pub layout: &'static str,
    pub year: Option<i32>,
    pub viewport: [f32; 2],
    pub pixels_per_point: f32,
    pub zoom: f64,
    pub globe_distance: f64,
    pub faces: usize,
    pub edges: usize,
    pub entities: usize,
    pub triangles: usize,
    pub lines: usize,
    pub history_generation: u64,
    pub dirty: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Input {
    pub wheel: u32,
    pub keys: u32,
    pub pointer: u32,
    pub raw_motion: u32,
    pub touch: u32,
    pub other: u32,
    pub dragging: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Phase {
    pub name: &'static str,
    pub ms: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct SpanRecord {
    pub name: String,
    pub channel: &'static str,
    pub ms: f32,
    pub frame: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Resources {
    /// A fully occupied logical core counts as 100 percent.
    pub cpu_percent: Option<f64>,
    pub resident_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub read_bytes_per_second: Option<f64>,
    pub write_bytes_per_second: Option<f64>,
    /// The busiest available engine for this process, sampled by Windows.
    pub gpu_engine_percent: Option<f64>,
    pub gpu_dedicated_bytes: Option<u64>,
    pub gpu_shared_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Header {
        schema: u32,
        version: &'static str,
        platform: &'static str,
        pid: u32,
        logical_cores: usize,
        graphics: Option<String>,
        started_unix_ms: u128,
    },
    Frame(Frame),
    Span(SpanRecord),
    Resources(Resources),
    Marker {
        label: String,
    },
    End {
        dropped_records: u64,
    },
}

#[derive(Clone, Debug, Serialize)]
struct Entry {
    t_ms: f64,
    #[serde(flatten)]
    record: Record,
}

struct State {
    origin: Instant,
    sender: SyncSender<Option<Entry>>,
    recording: AtomicBool,
    dropped: AtomicU64,
}

#[derive(Clone)]
pub struct Trace(Arc<State>);

impl Trace {
    pub fn elapsed_ms(&self) -> f64 {
        self.0.origin.elapsed().as_secs_f64() * 1000.0
    }

    pub fn emit(&self, record: Record) {
        self.emit_at(self.elapsed_ms(), record);
    }

    fn emit_at(&self, t_ms: f64, record: Record) {
        if self.0.recording.load(Ordering::Relaxed)
            && matches!(
                self.0.sender.try_send(Some(Entry { t_ms, record })),
                Err(TrySendError::Full(_))
            )
        {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn span(&self, channel: &'static str, name: impl Into<String>, frame: Option<u64>) -> Span {
        Span {
            trace: self.clone(),
            name: name.into(),
            channel,
            frame,
            start: Instant::now(),
        }
    }
}

/// Records both successful and failed work, including work on a worker thread.
pub struct Span {
    trace: Trace,
    name: String,
    channel: &'static str,
    frame: Option<u64>,
    start: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let t_ms = self.start.duration_since(self.trace.0.origin).as_secs_f64() * 1000.0;
        self.trace.emit_at(
            t_ms,
            Record::Span(SpanRecord {
                name: std::mem::take(&mut self.name),
                channel: self.channel,
                frame: self.frame,
                ms: elapsed.as_secs_f32() * 1000.0,
            }),
        );
    }
}

pub struct Recording {
    path: PathBuf,
    trace: Trace,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Recording {
    pub fn start(path: &Path, version: &'static str, graphics: Option<String>) -> io::Result<Self> {
        let file = OpenOptions::new().create_new(true).write(true).open(path)?;
        let mut output = BufWriter::new(file);
        write_entry(
            &mut output,
            &Entry {
                t_ms: 0.0,
                record: Record::Header {
                    schema: SCHEMA,
                    version,
                    platform: std::env::consts::OS,
                    pid: std::process::id(),
                    logical_cores: std::thread::available_parallelism().map_or(0, |n| n.get()),
                    graphics,
                    started_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                },
            },
        )?;
        output.flush()?;
        let (sender, receiver) = mpsc::sync_channel(4096);
        let trace = Trace(Arc::new(State {
            origin: Instant::now(),
            sender,
            recording: AtomicBool::new(true),
            dropped: AtomicU64::new(0),
        }));
        let worker_trace = trace.clone();
        let worker = std::thread::Builder::new()
            .name("atlas-perf-writer".into())
            .spawn(move || {
                let result = collect(&worker_trace, receiver, output);
                worker_trace.0.recording.store(false, Ordering::Release);
                result
            })?;
        Ok(Self {
            path: path.to_owned(),
            trace,
            worker: Some(worker),
        })
    }

    pub fn trace(&self) -> Trace {
        self.trace.clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn failed(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
    }

    pub fn finish(mut self) -> io::Result<PathBuf> {
        self.stop_writer();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("performance writer stopped unexpectedly"))??;
        }
        Ok(self.path.clone())
    }
}

impl Recording {
    fn stop_writer(&self) {
        if self.trace.0.recording.swap(false, Ordering::AcqRel) {
            // Wake a recorder that is resting while the application is idle.
            let _ = self.trace.0.sender.try_send(None);
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop_writer();
    }
}

fn write_entry(output: &mut impl Write, entry: &Entry) -> io::Result<()> {
    serde_json::to_writer(&mut *output, entry)?;
    output.write_all(b"\n")
}

fn collect(
    trace: &Trace,
    receiver: Receiver<Option<Entry>>,
    mut output: impl Write,
) -> io::Result<()> {
    let mut sampler = system::Sampler::new();
    let mut due = Instant::now();
    while trace.0.recording.load(Ordering::Acquire) {
        let now = Instant::now();
        if now >= due {
            let sample = sampler.sample();
            write_entry(
                &mut output,
                &Entry {
                    t_ms: trace.elapsed_ms(),
                    record: Record::Resources(sample),
                },
            )?;
            output.flush()?;
            due = now + SAMPLE_INTERVAL;
        }
        match receiver.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(Some(entry)) => write_entry(&mut output, &entry)?,
            Ok(None) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    for entry in receiver.try_iter().flatten() {
        write_entry(&mut output, &entry)?;
    }
    write_entry(
        &mut output,
        &Entry {
            t_ms: trace.elapsed_ms(),
            record: Record::End {
                dropped_records: trace.0.dropped.load(Ordering::Relaxed),
            },
        },
    )?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_wakes_an_idle_writer_and_preserves_a_full_queue() {
        for full in [false, true] {
            let (sender, receiver) = mpsc::sync_channel(1);
            let trace = Trace(Arc::new(State {
                origin: Instant::now(),
                sender,
                recording: AtomicBool::new(true),
                dropped: AtomicU64::new(0),
            }));
            if full {
                trace.emit(Record::Marker {
                    label: "last".into(),
                });
            }
            let recording = Recording {
                path: PathBuf::new(),
                trace: trace.clone(),
                worker: None,
            };
            recording.stop_writer();
            assert!(!trace.0.recording.load(Ordering::Acquire));
            if !full {
                assert!(
                    matches!(receiver.try_recv(), Ok(None)),
                    "idle receiver must wake"
                );
            }
            let mut output = Vec::new();
            collect(&trace, receiver, &mut output).unwrap();
            let records: Vec<serde_json::Value> = serde_json::Deserializer::from_slice(&output)
                .into_iter()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(records.len(), 1 + usize::from(full));
            assert_eq!(records.last().unwrap()["kind"], "end");
            if full {
                assert_eq!(records[0]["label"], "last");
            }
        }
    }
}
