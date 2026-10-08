//! The `hoocode-watchdog` thread: UI heartbeats, stall reports, runtime
//! starvation probes and memory sampling (`docs/design/concurrency.md`
//! sections 1, 3 and 4).
//!
//! The thread is mostly asleep. It wakes every [`TICK`] to read the heartbeat
//! and once a second to sample RSS. Nothing on it blocks the UI.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::memory::{self, MemoryGuard};
use crate::{io_handle, spawn_named_thread};

/// Name of the watchdog thread.
pub const WATCHDOG_THREAD: &str = "hoocode-watchdog";
/// Silence after which the watchdog logs the UI's phase once.
pub const UI_LOG_STALL: Duration = Duration::from_millis(500);
/// Silence after which the footer shows the stall flag.
pub const UI_FOOTER_STALL: Duration = Duration::from_secs(2);
/// How often the starvation probe runs on the io runtime.
pub const PROBE_PERIOD: Duration = Duration::from_secs(1);
/// A probe that wakes this late means the io runtime is starved.
pub const PROBE_LATE: Duration = Duration::from_millis(250);
/// How often the watchdog thread checks the heartbeat.
const TICK: Duration = Duration::from_millis(100);
/// RSS is sampled every this many ticks (once a second).
const SAMPLE_EVERY_TICKS: u32 = 10;

/// Receives the watchdog's report lines.
pub type LogSink = Box<dyn Fn(&str) + Send + Sync>;

static LOG_SINK: Mutex<Option<LogSink>> = Mutex::new(None);
static BEAT: Mutex<Option<Beat>> = Mutex::new(None);
static UI_STALLED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
struct Beat {
    at: Instant,
    phase: &'static str,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets where watchdog lines go. The default is stderr, which the TUI must not
/// use: the interactive mode routes these to its debug log instead.
pub fn set_log_sink(sink: LogSink) {
    *lock(&LOG_SINK) = Some(sink);
}

fn report(line: &str) {
    match lock(&LOG_SINK).as_ref() {
        Some(sink) => sink(line),
        None => eprintln!("{line}"),
    }
}

/// The UI loop's heartbeat. Call it once per loop turn, with the phase it is in.
pub fn ui_beat(phase: &'static str) {
    *lock(&BEAT) = Some(Beat {
        at: Instant::now(),
        phase,
    });
}

/// The UI loop has exited: stop judging its heartbeat (shutdown is not a stall).
pub fn ui_stopped() {
    *lock(&BEAT) = None;
}

/// True while the UI has been silent for [`UI_FOOTER_STALL`] or more.
pub fn ui_stalled() -> bool {
    UI_STALLED.load(Ordering::Relaxed)
}

/// What the stall detector reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallEvent {
    /// Silent for [`UI_LOG_STALL`]: logged once per stall, with the phase.
    Slow {
        phase: &'static str,
        silent: Duration,
    },
    /// Silent for [`UI_FOOTER_STALL`]: the footer flag goes up.
    Stalled {
        phase: &'static str,
        silent: Duration,
    },
    /// A new beat arrived after a stall; `silent` is how long it lasted.
    Recovered { silent: Duration },
}

/// Turns heartbeats into stall events. Pure: the caller supplies the clock.
#[derive(Debug, Default)]
pub struct StallDetector {
    last: Option<(Instant, &'static str)>,
    slow_logged: bool,
    stalled: bool,
}

impl StallDetector {
    /// A detector that has seen no beat yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks the latest beat (`None` when the UI has stopped) at time `now`.
    pub fn check(
        &mut self,
        beat: Option<(Instant, &'static str)>,
        now: Instant,
    ) -> Vec<StallEvent> {
        let mut events = Vec::new();
        let Some((at, phase)) = beat else {
            // The UI stopped: a stall that was open ends quietly.
            if self.stalled {
                self.stalled = false;
                events.push(StallEvent::Recovered {
                    silent: now.saturating_duration_since(self.last.map_or(now, |(at, _)| at)),
                });
            }
            self.last = None;
            self.slow_logged = false;
            return events;
        };
        if self.last != Some((at, phase)) {
            // A new beat: the UI is alive again. The stall lasted from the
            // previous beat until now.
            if self.stalled {
                self.stalled = false;
                let silent = self.last.map_or(Duration::ZERO, |(previous, _)| {
                    now.saturating_duration_since(previous)
                });
                events.push(StallEvent::Recovered { silent });
            }
            self.slow_logged = false;
            self.last = Some((at, phase));
            return events;
        }
        let silent = now.saturating_duration_since(at);
        if silent >= UI_LOG_STALL && !self.slow_logged {
            self.slow_logged = true;
            events.push(StallEvent::Slow { phase, silent });
        }
        if silent >= UI_FOOTER_STALL && !self.stalled {
            self.stalled = true;
            events.push(StallEvent::Stalled { phase, silent });
        }
        events
    }
}

/// True when a probe that was due at `scheduled` woke at `woke` too late.
pub fn probe_is_starved(scheduled: Instant, woke: Instant, late: Duration) -> bool {
    woke.saturating_duration_since(scheduled) > late
}

/// Runs a starvation probe: sleeps `period`, then calls `on_starved` with the
/// lateness whenever the wake-up is more than `late` behind schedule. Runs for
/// `rounds` periods, or forever when `None`. Spawn it on the io runtime.
pub async fn run_starvation_probe(
    period: Duration,
    late: Duration,
    rounds: Option<u32>,
    on_starved: impl Fn(Duration),
) {
    let mut done = 0u32;
    loop {
        if rounds.is_some_and(|max| done >= max) {
            return;
        }
        let scheduled = Instant::now() + period;
        tokio::time::sleep(period).await;
        let woke = Instant::now();
        if probe_is_starved(scheduled, woke, late) {
            on_starved(woke.saturating_duration_since(scheduled));
        }
        done += 1;
    }
}

/// Starts the watchdog thread and the io starvation probe. Idempotent: later
/// calls do nothing. Memory limits are read from [`memory::configured_memory_limits`]
/// on every sample, so settings applied later take effect.
pub fn start() -> io::Result<()> {
    if STARTED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let guard = MemoryGuard::new(Box::new(memory::process_rss_bytes));
    let spawned = spawn_named_thread(WATCHDOG_THREAD, move || run(guard));
    if let Err(error) = spawned {
        STARTED.store(false, Ordering::SeqCst);
        return Err(error);
    }
    io_handle().spawn(run_starvation_probe(
        PROBE_PERIOD,
        PROBE_LATE,
        None,
        |late| {
            report(&format!(
                "[WATCHDOG] io runtime starved: its 1 s probe woke {} ms late. The io lane (hoocode-io: agent stream, MCP, tool pipes) is not getting CPU.",
                late.as_millis()
            ));
        },
    ));
    Ok(())
}

fn run(mut memory: MemoryGuard) {
    let mut detector = StallDetector::new();
    let mut ticks: u32 = 0;
    loop {
        std::thread::sleep(TICK);
        let beat = (*lock(&BEAT)).map(|b| (b.at, b.phase));
        for event in detector.check(beat, Instant::now()) {
            match event {
                StallEvent::Slow { phase, silent } => report(&format!(
                    "[WATCHDOG] UI loop silent for {} ms; last phase: {phase}",
                    silent.as_millis()
                )),
                StallEvent::Stalled { phase, silent } => {
                    UI_STALLED.store(true, Ordering::Relaxed);
                    report(&format!(
                        "[WATCHDOG] UI stalled: silent for {} ms in phase {phase}",
                        silent.as_millis()
                    ));
                }
                StallEvent::Recovered { silent } => {
                    UI_STALLED.store(false, Ordering::Relaxed);
                    report(&format!(
                        "[WATCHDOG] UI loop resumed after {} ms",
                        silent.as_millis()
                    ));
                }
            }
        }
        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(SAMPLE_EVERY_TICKS) {
            let (soft, hard) = memory::configured_memory_limits();
            memory.set_limits(soft, hard);
            let tick = memory.tick();
            for event in &tick.events {
                report(&describe_memory_event(event));
            }
            memory::publish_tick(&tick);
        }
    }
}

fn describe_memory_event(event: &memory::MemoryEvent) -> String {
    use memory::MemoryEvent::*;
    let mib = |b: u64| b / memory::MIB;
    match *event {
        ShedStarted { rss, limit } => format!(
            "[MEMORY] RSS {} MiB above the soft limit {} MiB: shedding load",
            mib(rss),
            mib(limit)
        ),
        ShedEnded { rss, limit } => format!(
            "[MEMORY] RSS {} MiB back under the soft limit {} MiB: load restored",
            mib(rss),
            mib(limit)
        ),
        HardLimit { rss, limit } => format!(
            "[MEMORY] RSS {} MiB above the hard limit {} MiB: aborting the turn",
            mib(rss),
            mib(limit)
        ),
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn stall_detector_starts_quiet_before_any_beat() {
        let mut d = StallDetector::new();
        assert!(d.check(None, Instant::now()).is_empty());
    }
}
