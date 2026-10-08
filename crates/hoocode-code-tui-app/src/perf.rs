//! Phase 0 measurement of the UI loop (`docs/design/concurrency.md` §5): frame
//! build and write time, keystroke-to-frame latency, loop stalls, threads and
//! RSS. `/perf` shows them; `--perf-log <file>` appends one JSON line a second.
//!
//! The UI thread only records. Each metric is a fixed ring of `Duration`s, so a
//! sample costs one store and no allocation or lock. Once a second the UI hands
//! its numbers to a sampler thread, which reads `/proc` and writes the log line,
//! so the UI thread does no file I/O for measurement.

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hoocode_tui_render::FrameTiming;
use serde_json::{json, Value};

/// Samples kept per metric: the rolling window the percentiles are taken over.
pub const WINDOW: usize = 1024;

/// A loop iteration longer than this is a stall.
pub const STALL_THRESHOLD: Duration = Duration::from_millis(500);

/// Keys waiting for their frame. Past this cap the key is not timed (the
/// queue is bounded; a paste of more keys than this in one iteration is rare).
const PENDING_KEY_CAP: usize = 1024;

/// How often the UI publishes its numbers and the sampler writes a line.
const TICK: Duration = Duration::from_secs(1);

/// A fixed-size ring of durations. The newest `WINDOW` samples are kept.
pub struct Ring {
    samples: Vec<Duration>,
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(WINDOW),
            next: 0,
        }
    }

    /// Store one sample, overwriting the oldest once the ring is full.
    pub fn push(&mut self, sample: Duration) {
        if self.samples.len() < WINDOW {
            self.samples.push(sample);
        } else {
            self.samples[self.next] = sample;
        }
        self.next = (self.next + 1) % WINDOW;
    }

    /// Min, p50 and p99 of the kept samples; `None` when there are none.
    pub fn dist(&self) -> Option<Dist> {
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        Some(Dist {
            count: sorted.len(),
            min: *sorted.first()?,
            p50: percentile(&sorted, 50)?,
            p99: percentile(&sorted, 99)?,
        })
    }
}

/// Nearest-rank percentile of an ascending slice. `pct` is 0 to 100.
pub fn percentile(sorted: &[Duration], pct: usize) -> Option<Duration> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (pct * sorted.len()).div_ceil(100).clamp(1, sorted.len());
    sorted.get(rank - 1).copied()
}

/// The summary of one metric's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dist {
    pub count: usize,
    pub min: Duration,
    pub p50: Duration,
    pub p99: Duration,
}

/// Everything the UI thread measures, summarised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub frames: u64,
    pub stalls: u64,
    pub frame_build: Option<Dist>,
    pub frame_write: Option<Dist>,
    pub key_latency: Option<Dist>,
    pub loop_iteration: Option<Dist>,
}

/// Process-wide numbers read from `/proc`. `None` where the platform has no
/// such file (everywhere but Linux, for now).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcSample {
    pub threads: Option<usize>,
    pub rss_kib: Option<u64>,
}

/// One reading of everything, as `/perf` and the log line show it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub proc: ProcSample,
    pub stats: Stats,
}

/// The UI thread's recorder. Single-threaded: it lives on the UI thread and is
/// shared with the frame observer through an `Rc`.
pub struct Recorder {
    frame_build: Ring,
    frame_write: Ring,
    key_latency: Ring,
    loop_iteration: Ring,
    frames: u64,
    stalls: u64,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            frame_build: Ring::new(),
            frame_write: Ring::new(),
            key_latency: Ring::new(),
            loop_iteration: Ring::new(),
            frames: 0,
            stalls: 0,
        }
    }

    pub fn record_frame(&mut self, timing: FrameTiming) {
        self.frame_build.push(timing.build);
        self.frame_write.push(timing.write);
        self.frames += 1;
    }

    pub fn record_key(&mut self, latency: Duration) {
        self.key_latency.push(latency);
    }

    /// One loop iteration, from the moment its input arrived to the end of its
    /// frame. Longer than [`STALL_THRESHOLD`] counts as a stall.
    pub fn record_iteration(&mut self, took: Duration) {
        self.loop_iteration.push(took);
        if took > STALL_THRESHOLD {
            self.stalls += 1;
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            frames: self.frames,
            stalls: self.stalls,
            frame_build: self.frame_build.dist(),
            frame_write: self.frame_write.dist(),
            key_latency: self.key_latency.dist(),
            loop_iteration: self.loop_iteration.dist(),
        }
    }
}

/// Read the thread count and RSS of this process.
pub fn sample_process() -> ProcSample {
    #[cfg(target_os = "linux")]
    {
        ProcSample {
            threads: count_threads(),
            rss_kib: read_rss_kib(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        ProcSample::default()
    }
}

#[cfg(target_os = "linux")]
fn count_threads() -> Option<usize> {
    let tasks = std::fs::read_dir("/proc/self/task").ok()?;
    Some(tasks.filter_map(Result::ok).count())
}

/// Resident set from `/proc/self/statm` (field 2, in pages).
#[cfg(target_os = "linux")]
fn read_rss_kib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions; it only reads a runtime constant.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size).ok().filter(|size| *size > 0)?;
    Some(resident_pages * page_size / 1024)
}

/// The sampler's shared slot: the UI publishes `stats`, the sampler publishes `proc`.
#[derive(Default)]
struct Shared {
    proc: ProcSample,
    stats: Stats,
}

/// The sampler thread: once a second it reads `/proc`, takes the UI's latest
/// stats and appends one JSON line to the log, if there is one. Dropping it
/// stops the thread.
pub struct Sampler {
    shared: Arc<Mutex<Shared>>,
    _stop: mpsc::Sender<()>,
}

impl Sampler {
    /// Start the thread, appending to `log` when given. Fails if the log cannot
    /// be opened or the thread cannot start.
    pub fn start(log: Option<&Path>) -> Result<Self, String> {
        let file = match log {
            Some(path) => Some(
                File::options()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|error| {
                        format!("Failed to open perf log {}: {error}", path.display())
                    })?,
            ),
            None => None,
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (stop, stopped) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        hoocode_runtime::spawn_named_thread("hoocode-perf", move || {
            sample_loop(&thread_shared, file, &stopped)
        })
        .map_err(|error| format!("Failed to start the perf sampler: {error}"))?;
        Ok(Self {
            shared,
            _stop: stop,
        })
    }

    /// The UI's latest numbers, for the next log line.
    pub fn publish(&self, stats: Stats) {
        lock(&self.shared).stats = stats;
    }

    /// The process numbers from the last sample (at most a second old).
    pub fn proc(&self) -> ProcSample {
        lock(&self.shared).proc
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn sample_loop(shared: &Mutex<Shared>, mut file: Option<File>, stopped: &mpsc::Receiver<()>) {
    loop {
        let proc = sample_process();
        let stats = {
            let mut slot = lock(shared);
            slot.proc = proc;
            slot.stats
        };
        if let Some(file) = file.as_mut() {
            let line = json_line(unix_millis(), &Snapshot { proc, stats });
            // A failed write (full disk, log removed) is not worth stopping the UI for.
            let _ = writeln!(file, "{line}");
        }
        match stopped.recv_timeout(TICK) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The UI side of the measurement, owned by the interactive mode.
pub struct Perf {
    recorder: Rc<RefCell<Recorder>>,
    pending_keys: Vec<Instant>,
    published_at: Instant,
    sampler: Option<Sampler>,
    startup_error: Option<String>,
}

impl Perf {
    /// Start the sampler. If the log cannot be opened the sampler still runs
    /// without it, and the error is left for [`Perf::take_startup_error`].
    pub fn new(log: Option<&Path>) -> Self {
        let (sampler, startup_error) = match Sampler::start(log) {
            Ok(sampler) => (Some(sampler), None),
            Err(error) => (Sampler::start(None).ok(), Some(error)),
        };
        Self {
            recorder: Rc::new(RefCell::new(Recorder::new())),
            pending_keys: Vec::new(),
            published_at: Instant::now(),
            sampler,
            startup_error,
        }
    }

    /// The observer the TUI calls with every frame's timing.
    pub fn frame_observer(&self) -> Box<dyn FnMut(FrameTiming)> {
        let recorder = Rc::clone(&self.recorder);
        Box::new(move |timing| recorder.borrow_mut().record_frame(timing))
    }

    /// A key the terminal received at `arrived`. Its latency is taken when the
    /// current loop iteration's frame is out.
    pub fn key_arrived(&mut self, arrived: Instant) {
        if self.pending_keys.len() < PENDING_KEY_CAP {
            self.pending_keys.push(arrived);
        }
    }

    /// The end of one loop iteration that began at `started`, after its frame.
    pub fn end_iteration(&mut self, started: Instant) {
        let now = Instant::now();
        {
            let mut recorder = self.recorder.borrow_mut();
            recorder.record_iteration(now.duration_since(started));
            for arrived in self.pending_keys.drain(..) {
                recorder.record_key(now.duration_since(arrived));
            }
        }
        if now.duration_since(self.published_at) >= TICK {
            self.published_at = now;
            if let Some(sampler) = &self.sampler {
                sampler.publish(self.recorder.borrow().stats());
            }
        }
    }

    /// The numbers `/perf` shows now.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            proc: self.sampler.as_ref().map(Sampler::proc).unwrap_or_default(),
            stats: self.recorder.borrow().stats(),
        }
    }

    pub fn take_startup_error(&mut self) -> Option<String> {
        self.startup_error.take()
    }
}

fn ms(duration: Duration) -> String {
    format!("{:.2}", duration.as_secs_f64() * 1000.0)
}

fn dist_line(label: &str, dist: Option<Dist>) -> String {
    match dist {
        Some(d) => format!(
            "{label:<24} n={:<5} min {}  p50 {}  p99 {}",
            d.count,
            ms(d.min),
            ms(d.p50),
            ms(d.p99)
        ),
        None => format!("{label:<24} no samples yet"),
    }
}

fn rss_text(rss_kib: Option<u64>) -> String {
    match rss_kib {
        Some(kib) => format!("{:.1} MiB", kib as f64 / 1024.0),
        None => "n/a".into(),
    }
}

/// The `/perf` notice: one value per line, times in milliseconds.
pub fn format_report(snapshot: &Snapshot) -> String {
    let stats = &snapshot.stats;
    let threads = snapshot
        .proc
        .threads
        .map_or_else(|| "n/a".to_string(), |count| count.to_string());
    [
        "Performance (the last 1024 samples of each)".to_string(),
        format!("threads: {threads}"),
        format!("rss: {}", rss_text(snapshot.proc.rss_kib)),
        format!(
            "frames: {}  stalls (loop > 500 ms): {}",
            stats.frames, stats.stalls
        ),
        dist_line("frame build (ms)", stats.frame_build),
        dist_line("frame write (ms)", stats.frame_write),
        dist_line("keystroke to frame (ms)", stats.key_latency),
        dist_line("loop iteration (ms)", stats.loop_iteration),
    ]
    .join("\n")
}

fn dist_json(dist: Option<Dist>) -> Value {
    match dist {
        Some(d) => json!({
            "n": d.count,
            "min": d.min.as_secs_f64() * 1000.0,
            "p50": d.p50.as_secs_f64() * 1000.0,
            "p99": d.p99.as_secs_f64() * 1000.0,
        }),
        None => Value::Null,
    }
}

/// One `--perf-log` line (no trailing newline).
pub fn json_line(ts_ms: u64, snapshot: &Snapshot) -> String {
    let stats = &snapshot.stats;
    json!({
        "ts_ms": ts_ms,
        "threads": snapshot.proc.threads,
        "rss_kib": snapshot.proc.rss_kib,
        "frames": stats.frames,
        "stalls": stats.stalls,
        "frame_build_ms": dist_json(stats.frame_build),
        "frame_write_ms": dist_json(stats.frame_write),
        "key_latency_ms": dist_json(stats.key_latency),
        "loop_iteration_ms": dist_json(stats.loop_iteration),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms_u64(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let sorted: Vec<Duration> = (1..=100).map(ms_u64).collect();
        assert_eq!(percentile(&sorted, 0), Some(ms_u64(1)));
        assert_eq!(percentile(&sorted, 50), Some(ms_u64(50)));
        assert_eq!(percentile(&sorted, 99), Some(ms_u64(99)));
        assert_eq!(percentile(&sorted, 100), Some(ms_u64(100)));
        // Four samples: rank ceil(0.5 * 4) = 2, the second smallest.
        let four = [ms_u64(1), ms_u64(2), ms_u64(3), ms_u64(10)];
        assert_eq!(percentile(&four, 50), Some(ms_u64(2)));
        assert_eq!(percentile(&four, 99), Some(ms_u64(10)));
    }

    #[test]
    fn percentile_of_one_sample_and_of_none() {
        assert_eq!(percentile(&[ms_u64(7)], 99), Some(ms_u64(7)));
        assert_eq!(percentile(&[], 50), None);
    }

    #[test]
    fn ring_keeps_the_newest_window() {
        let mut ring = Ring::new();
        assert_eq!(ring.dist(), None);
        for i in 0..(WINDOW as u64 + 476) {
            ring.push(ms_u64(i));
        }
        // Kept: 476..=1499 (1024 values). Rank of p50 is 512 (value 987),
        // rank of p99 is 1014 (value 1489).
        let dist = ring.dist().expect("samples");
        assert_eq!(dist.count, WINDOW);
        assert_eq!(dist.min, ms_u64(476));
        assert_eq!(dist.p50, ms_u64(987));
        assert_eq!(dist.p99, ms_u64(1489));
    }

    #[test]
    fn stall_is_an_iteration_longer_than_500_ms() {
        let mut recorder = Recorder::new();
        recorder.record_iteration(ms_u64(500));
        assert_eq!(recorder.stats().stalls, 0);
        recorder.record_iteration(ms_u64(501));
        assert_eq!(recorder.stats().stalls, 1);
        assert_eq!(recorder.stats().loop_iteration.map(|d| d.count), Some(2));
    }

    #[test]
    fn frames_count_build_and_write_separately() {
        let mut recorder = Recorder::new();
        recorder.record_frame(FrameTiming {
            build: ms_u64(2),
            write: ms_u64(1),
        });
        let stats = recorder.stats();
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.frame_build.map(|d| d.p50), Some(ms_u64(2)));
        assert_eq!(stats.frame_write.map(|d| d.p50), Some(ms_u64(1)));
        assert_eq!(stats.key_latency, None);
    }

    fn sample_snapshot() -> Snapshot {
        let dist = Dist {
            count: 3,
            min: Duration::from_micros(1_000),
            p50: Duration::from_micros(2_000),
            p99: Duration::from_micros(9_500),
        };
        Snapshot {
            proc: ProcSample {
                threads: Some(7),
                rss_kib: Some(2048),
            },
            stats: Stats {
                frames: 3,
                stalls: 1,
                frame_build: Some(dist),
                frame_write: None,
                key_latency: Some(dist),
                loop_iteration: Some(dist),
            },
        }
    }

    #[test]
    fn report_shows_every_value() {
        let text = format_report(&sample_snapshot());
        assert!(text.contains("threads: 7"), "{text}");
        assert!(text.contains("rss: 2.0 MiB"), "{text}");
        assert!(
            text.contains("frames: 3  stalls (loop > 500 ms): 1"),
            "{text}"
        );
        assert!(
            text.contains("keystroke to frame (ms)")
                && text.contains("min 1.00  p50 2.00  p99 9.50"),
            "{text}"
        );
        assert!(
            text.contains("frame write (ms)") && text.contains("no samples yet"),
            "{text}"
        );
    }

    #[test]
    fn report_says_n_a_when_the_platform_has_no_proc() {
        let text = format_report(&Snapshot::default());
        assert!(text.contains("threads: n/a"), "{text}");
        assert!(text.contains("rss: n/a"), "{text}");
        assert!(
            text.contains("frames: 0  stalls (loop > 500 ms): 0"),
            "{text}"
        );
    }

    #[test]
    fn json_line_is_one_object_with_null_for_missing_values() {
        let line = json_line(1_700_000_000_000, &sample_snapshot());
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(value["ts_ms"], 1_700_000_000_000u64);
        assert_eq!(value["threads"], 7);
        assert_eq!(value["rss_kib"], 2048);
        assert_eq!(value["stalls"], 1);
        assert_eq!(value["frame_build_ms"]["n"], 3);
        assert_eq!(value["frame_build_ms"]["p99"], 9.5);
        assert!(value["frame_write_ms"].is_null());

        let empty: Value = serde_json::from_str(&json_line(0, &Snapshot::default())).unwrap();
        assert!(empty["threads"].is_null());
        assert!(empty["key_latency_ms"].is_null());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_threads_and_rss() {
        let sample = sample_process();
        assert!(sample.threads.is_some_and(|count| count >= 1), "{sample:?}");
        assert!(sample.rss_kib.is_some_and(|kib| kib > 0), "{sample:?}");
    }

    #[test]
    fn sampler_appends_a_line_each_tick() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("perf.jsonl");
        let sampler = Sampler::start(Some(&log)).expect("start");
        sampler.publish(Stats {
            frames: 5,
            ..Stats::default()
        });
        // The first line is written at once; wait for a second one to prove the tick.
        let deadline = Instant::now() + Duration::from_secs(5);
        let lines = loop {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.lines().count() >= 2 || Instant::now() > deadline {
                break text;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        drop(sampler);
        let lines: Vec<&str> = lines.lines().collect();
        assert!(lines.len() >= 2, "{lines:?}");
        let last: Value = serde_json::from_str(lines[lines.len() - 1]).expect("json line");
        assert_eq!(last["frames"], 5);
    }

    #[test]
    fn unopenable_log_is_reported_and_sampler_still_starts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("missing-dir").join("perf.jsonl");
        let mut perf = Perf::new(Some(&log));
        let error = perf.take_startup_error().expect("error");
        assert!(error.contains("Failed to open perf log"), "{error}");
        assert_eq!(perf.take_startup_error(), None);
    }
}
