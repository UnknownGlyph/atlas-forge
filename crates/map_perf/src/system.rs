use crate::Resources;
use std::time::Instant;

#[derive(Default)]
struct Counters {
    cpu_seconds: Option<f64>,
    resident_bytes: Option<u64>,
    private_bytes: Option<u64>,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

pub struct Sampler {
    prior: Option<(Instant, Counters)>,
    #[cfg(windows)]
    gpu: Option<windows::Gpu>,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            prior: None,
            #[cfg(windows)]
            gpu: windows::Gpu::new(),
        }
    }

    pub fn sample(&mut self) -> Resources {
        let now = Instant::now();
        let counters = counters();
        let mut resource = Resources {
            resident_bytes: counters.resident_bytes,
            private_bytes: counters.private_bytes,
            ..Default::default()
        };
        if let Some((then, old)) = &self.prior {
            let seconds = now.duration_since(*then).as_secs_f64();
            if seconds > 0.0 {
                let rate = |current: Option<u64>, previous: Option<u64>| {
                    current
                        .zip(previous)
                        .and_then(|(a, b)| a.checked_sub(b))
                        .map(|n| n as f64 / seconds)
                };
                resource.cpu_percent = counters
                    .cpu_seconds
                    .zip(old.cpu_seconds)
                    .map(|(a, b)| ((a - b) / seconds * 100.0).max(0.0));
                resource.read_bytes_per_second = rate(counters.read_bytes, old.read_bytes);
                resource.write_bytes_per_second = rate(counters.write_bytes, old.write_bytes);
            }
        }
        self.prior = Some((now, counters));
        #[cfg(windows)]
        if let Some(gpu) = &mut self.gpu {
            gpu.sample(&mut resource);
        }
        resource
    }
}

#[cfg(windows)]
fn counters() -> Counters {
    windows::counters()
}

#[cfg(target_os = "linux")]
fn counters() -> Counters {
    let field = |file: &str, key: &str| {
        let text = std::fs::read_to_string(file).ok()?;
        let row = text.lines().find_map(|line| line.strip_prefix(key))?;
        row.split_whitespace().next()?.parse::<u64>().ok()
    };
    Counters {
        resident_bytes: field("/proc/self/status", "VmRSS:").map(|v| v * 1024),
        read_bytes: field("/proc/self/io", "read_bytes:"),
        write_bytes: field("/proc/self/io", "write_bytes:"),
        ..Default::default()
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn counters() -> Counters {
    Counters::default()
}

#[cfg(windows)]
mod windows {
    use super::{Counters, Resources};
    use std::{
        ffi::c_void,
        mem::{self, MaybeUninit},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            Performance::{
                PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
                PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_CSTATUS_NEW_DATA,
                PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA,
            },
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS},
        },
    };

    pub(super) fn counters() -> Counters {
        // Win32 fills these plain structures synchronously, using the
        // pseudo-handle for the current process. It does not need closing.
        unsafe {
            let process = GetCurrentProcess();
            let mut created: FILETIME = mem::zeroed();
            let mut exited = created;
            let mut kernel = created;
            let mut user = created;
            let cpu =
                GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0;
            let ticks =
                |v: FILETIME| (u64::from(v.dwHighDateTime) << 32) | u64::from(v.dwLowDateTime);
            let mut memory: PROCESS_MEMORY_COUNTERS_EX = mem::zeroed();
            memory.cb = mem::size_of_val(&memory) as u32;
            let memory_ok = GetProcessMemoryInfo(
                process,
                &mut memory as *mut PROCESS_MEMORY_COUNTERS_EX as *mut _,
                memory.cb,
            ) != 0;
            let mut io: IO_COUNTERS = mem::zeroed();
            let io_ok = GetProcessIoCounters(process, &mut io) != 0;
            Counters {
                cpu_seconds: cpu.then(|| (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0),
                resident_bytes: memory_ok.then_some(memory.WorkingSetSize as u64),
                private_bytes: memory_ok.then_some(memory.PrivateUsage as u64),
                read_bytes: io_ok.then_some(io.ReadTransferCount),
                write_bytes: io_ok.then_some(io.WriteTransferCount),
            }
        }
    }

    pub(super) struct Gpu {
        query: *mut c_void,
        counters: [*mut c_void; 3],
        prefix: Vec<u16>,
    }

    impl Gpu {
        pub(super) fn new() -> Option<Self> {
            // All PDH calls are confined to the background recorder thread.
            unsafe {
                let mut query = ptr::null_mut();
                if PdhOpenQueryW(ptr::null(), 0, &mut query) != 0 {
                    return None;
                }
                let counters = [
                    r"\GPU Engine(*)\Utilization Percentage",
                    r"\GPU Process Memory(*)\Dedicated Usage",
                    r"\GPU Process Memory(*)\Shared Usage",
                ]
                .map(|path| {
                    let path: Vec<_> = path.encode_utf16().chain([0]).collect();
                    let mut counter = ptr::null_mut();
                    if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) == 0 {
                        counter
                    } else {
                        ptr::null_mut()
                    }
                });
                Some(Self {
                    query,
                    counters,
                    prefix: format!("pid_{}_", std::process::id())
                        .encode_utf16()
                        .collect(),
                })
            }
        }

        pub(super) fn sample(&mut self, out: &mut Resources) {
            if unsafe { PdhCollectQueryData(self.query) } != 0 {
                return;
            }
            out.gpu_engine_percent = self.values(0, f64::max);
            out.gpu_dedicated_bytes = self.values(1, |a, b| a + b).map(|n| n as u64);
            out.gpu_shared_bytes = self.values(2, |a, b| a + b).map(|n| n as u64);
        }

        fn values(&self, index: usize, combine: impl Fn(f64, f64) -> f64) -> Option<f64> {
            let counter = self.counters[index];
            if counter.is_null() {
                return None;
            }
            // Get the required size, then allocate suitably aligned storage.
            // Instance names returned by PDH point into this same buffer.
            unsafe {
                let mut bytes = 0;
                let mut count = 0;
                if PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut bytes,
                    &mut count,
                    ptr::null_mut(),
                ) != PDH_MORE_DATA
                {
                    return None;
                }
                let capacity =
                    (bytes as usize).div_ceil(mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
                let mut storage =
                    vec![MaybeUninit::<PDH_FMT_COUNTERVALUE_ITEM_W>::uninit(); capacity];
                let data = storage.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                if PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut bytes,
                    &mut count,
                    data,
                ) != 0
                    || count as usize > capacity
                {
                    return None;
                }
                std::slice::from_raw_parts(data, count as usize)
                    .iter()
                    .filter_map(|item| {
                        if item.szName.is_null()
                            || !self
                                .prefix
                                .iter()
                                .enumerate()
                                .all(|(i, &value)| *item.szName.add(i) == value)
                            || !matches!(
                                item.FmtValue.CStatus,
                                PDH_CSTATUS_NEW_DATA | PDH_CSTATUS_VALID_DATA
                            )
                        {
                            return None;
                        }
                        let value = item.FmtValue.Anonymous.doubleValue;
                        (value.is_finite() && value >= 0.0).then_some(value)
                    })
                    .reduce(combine)
            }
        }
    }

    impl Drop for Gpu {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}
