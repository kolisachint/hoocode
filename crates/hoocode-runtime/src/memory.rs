//! Process memory limits: the soft limit sheds load, the hard limit aborts the
//! turn (`docs/design/concurrency.md` section 3). Nothing here kills hoocode.
//!
//! The state machine ([`MemoryMonitor`]) is pure: it takes RSS samples and
//! returns events. [`MemoryGuard`] wraps it with an RSS source, so tests inject
//! numbers instead of allocating. The watchdog thread samples the real process.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// One mebibyte in bytes.
pub const MIB: u64 = 1024 * 1024;
/// A subagent child that goes above this is reaped by the lifeguard.
pub const CHILD_RSS_LIMIT_BYTES: u64 = 2048 * MIB;
/// Shedding and the hard trip both lift once RSS is this many percent under the limit.
pub const RECOVER_MARGIN_PERCENT: u64 = 10;
/// Events kept for the UI between two of its loop turns. Transitions are rare,
/// so the cap only matters when the UI is stuck; the oldest event goes first.
const EVENT_QUEUE_MAX: usize = 16;

/// A limit transition. Each one fires once per crossing, not once per sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEvent {
    /// RSS went above the soft limit: shedding starts.
    ShedStarted { rss: u64, limit: u64 },
    /// RSS fell 10% under the soft limit (or the limit was turned off): shedding lifts.
    ShedEnded { rss: u64, limit: u64 },
    /// RSS went above the hard limit: the UI aborts the turn and flushes the session.
    HardLimit { rss: u64, limit: u64 },
}

/// Floor of a default limit, in MB.
pub const DEFAULT_FLOOR_MB: u64 = 256;
/// Largest default soft limit, in MB.
pub const DEFAULT_SOFT_CAP_MB: u64 = 2048;
/// Largest default hard limit, in MB.
pub const DEFAULT_HARD_CAP_MB: u64 = 4096;

/// Default `(soft, hard)` limits in MB for a machine with `total_ram` bytes
/// (`None` when RAM is unknown). Soft: the lower of 2048 and 25% of RAM. Hard:
/// the lower of 4096 and 50% of RAM. Both are at least 256.
pub fn default_memory_limits_mb(total_ram: Option<u64>) -> (u64, u64) {
    match total_ram {
        Some(bytes) => {
            let mb = bytes / MIB;
            (
                (mb / 4).clamp(DEFAULT_FLOOR_MB, DEFAULT_SOFT_CAP_MB),
                (mb / 2).clamp(DEFAULT_FLOOR_MB, DEFAULT_HARD_CAP_MB),
            )
        }
        None => (DEFAULT_SOFT_CAP_MB, DEFAULT_HARD_CAP_MB),
    }
}

/// RSS at which a limit of `limit` bytes counts as recovered.
pub fn recover_below(limit: u64) -> u64 {
    limit - limit * RECOVER_MARGIN_PERCENT / 100
}

/// The soft and hard limit state machine. A limit of 0 is off.
#[derive(Debug, Clone, Default)]
pub struct MemoryMonitor {
    soft: u64,
    hard: u64,
    shedding: bool,
    hard_tripped: bool,
}

impl MemoryMonitor {
    /// A monitor with the given limits in bytes (0 = off).
    pub fn new(soft_bytes: u64, hard_bytes: u64) -> Self {
        Self {
            soft: soft_bytes,
            hard: hard_bytes,
            ..Self::default()
        }
    }

    /// Changes the limits. The state is kept; the next sample corrects it.
    pub fn set_limits(&mut self, soft_bytes: u64, hard_bytes: u64) {
        self.soft = soft_bytes;
        self.hard = hard_bytes;
    }

    /// True while load is being shed.
    pub fn shedding(&self) -> bool {
        self.shedding
    }

    /// Feeds one RSS sample and returns the transitions it caused.
    pub fn observe(&mut self, rss: u64) -> Vec<MemoryEvent> {
        let mut events = Vec::new();
        if self.soft == 0 {
            if self.shedding {
                self.shedding = false;
                events.push(MemoryEvent::ShedEnded {
                    rss,
                    limit: self.soft,
                });
            }
        } else if !self.shedding && rss > self.soft {
            self.shedding = true;
            events.push(MemoryEvent::ShedStarted {
                rss,
                limit: self.soft,
            });
        } else if self.shedding && rss <= recover_below(self.soft) {
            self.shedding = false;
            events.push(MemoryEvent::ShedEnded {
                rss,
                limit: self.soft,
            });
        }
        if self.hard == 0 {
            self.hard_tripped = false;
        } else if !self.hard_tripped && rss > self.hard {
            self.hard_tripped = true;
            events.push(MemoryEvent::HardLimit {
                rss,
                limit: self.hard,
            });
        } else if self.hard_tripped && rss <= recover_below(self.hard) {
            self.hard_tripped = false;
        }
        events
    }
}

/// What one sampling step found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    /// The RSS read, or `None` when the platform could not say.
    pub rss: Option<u64>,
    /// Shedding state after this sample.
    pub shedding: bool,
    /// Transitions caused by this sample.
    pub events: Vec<MemoryEvent>,
}

/// Source of the process's RSS in bytes.
pub type RssSource = Box<dyn FnMut() -> Option<u64> + Send>;

/// A [`MemoryMonitor`] fed by an RSS source. The watchdog owns one.
pub struct MemoryGuard {
    monitor: MemoryMonitor,
    source: RssSource,
}

impl MemoryGuard {
    /// A guard with no limits (off) that reads RSS from `source`.
    pub fn new(source: RssSource) -> Self {
        Self {
            monitor: MemoryMonitor::default(),
            source,
        }
    }

    /// Sets the limits the next samples are judged against.
    pub fn set_limits(&mut self, soft_bytes: u64, hard_bytes: u64) {
        self.monitor.set_limits(soft_bytes, hard_bytes);
    }

    /// Reads RSS once and runs the state machine on it. An unknown RSS changes nothing.
    pub fn tick(&mut self) -> Tick {
        let rss = (self.source)();
        let events = rss.map(|r| self.monitor.observe(r)).unwrap_or_default();
        Tick {
            rss,
            shedding: self.monitor.shedding(),
            events,
        }
    }
}

static SHEDDING: AtomicBool = AtomicBool::new(false);
/// Last RSS sample in bytes; 0 when none has been read yet.
static LAST_RSS: AtomicU64 = AtomicU64::new(0);
static SOFT_LIMIT: AtomicU64 = AtomicU64::new(0);
static HARD_LIMIT: AtomicU64 = AtomicU64::new(0);
static EVENTS: Mutex<VecDeque<MemoryEvent>> = Mutex::new(VecDeque::new());

/// Sets the limits the watchdog judges against, in bytes (0 = off).
pub fn configure_memory_limits(soft_bytes: u64, hard_bytes: u64) {
    SOFT_LIMIT.store(soft_bytes, Ordering::Relaxed);
    HARD_LIMIT.store(hard_bytes, Ordering::Relaxed);
}

/// The limits set by [`configure_memory_limits`]: `(soft, hard)` in bytes.
pub fn configured_memory_limits() -> (u64, u64) {
    (
        SOFT_LIMIT.load(Ordering::Relaxed),
        HARD_LIMIT.load(Ordering::Relaxed),
    )
}

/// True while load is being shed: no new subagents, one parallel tool call at a time.
pub fn shedding() -> bool {
    SHEDDING.load(Ordering::Relaxed)
}

/// The last RSS the watchdog read, in bytes, or `None` before the first read.
pub fn last_rss_bytes() -> Option<u64> {
    match LAST_RSS.load(Ordering::Relaxed) {
        0 => None,
        rss => Some(rss),
    }
}

/// Publishes a sampling step to the process-wide state (called by the watchdog).
pub fn publish_tick(tick: &Tick) {
    SHEDDING.store(tick.shedding, Ordering::Relaxed);
    if let Some(rss) = tick.rss {
        LAST_RSS.store(rss.max(1), Ordering::Relaxed);
    }
    if tick.events.is_empty() {
        return;
    }
    let mut queue = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    for event in &tick.events {
        if queue.len() == EVENT_QUEUE_MAX {
            queue.pop_front();
        }
        queue.push_back(*event);
    }
}

/// Takes the transitions published since the last call, oldest first.
pub fn take_memory_events() -> Vec<MemoryEvent> {
    let mut queue = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    queue.drain(..).collect()
}

/// This process's resident set size in bytes, or `None` if the platform cannot say.
pub fn process_rss_bytes() -> Option<u64> {
    memory_stats::memory_stats().map(|stats| stats.physical_mem as u64)
}

/// Resident set size of a child process in bytes, or `None` if it is gone or
/// the platform cannot say. Linux reads `/proc/<pid>/statm`; macOS asks
/// `proc_pid_rusage`.
pub fn child_rss_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        parse_statm_resident(&text, page_size())
    }
    #[cfg(target_os = "macos")]
    {
        macos::resident_bytes(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Resident pages (the second field of `statm`) times the page size.
#[cfg(any(target_os = "linux", test))]
fn parse_statm_resident(text: &str, page_size: u64) -> Option<u64> {
    let resident: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    resident.checked_mul(page_size)
}

#[cfg(target_os = "linux")]
fn page_size() -> u64 {
    // SAFETY: sysconf with a valid name has no preconditions and no side effects.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).ok().filter(|s| *s > 0).unwrap_or(4096)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::os::raw::{c_int, c_void};

    const RUSAGE_INFO_V2: c_int = 2;

    /// `struct rusage_info_v2` from `<sys/resource.h>`. Only `ri_resident_size`
    /// is read; the rest keeps the layout right.
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV2 {
        ri_uuid: [u8; 16],
        ri_user_time: u64,
        ri_system_time: u64,
        ri_pkg_idle_wkups: u64,
        ri_interrupt_wkups: u64,
        ri_pageins: u64,
        ri_wired_size: u64,
        ri_resident_size: u64,
        ri_phys_footprint: u64,
        ri_proc_start_abstime: u64,
        ri_proc_exit_abstime: u64,
        ri_child_user_time: u64,
        ri_child_system_time: u64,
        ri_child_pkg_idle_wkups: u64,
        ri_child_interrupt_wkups: u64,
        ri_child_pageins: u64,
        ri_child_elapsed_abstime: u64,
        ri_diskio_bytesread: u64,
        ri_diskio_byteswritten: u64,
    }

    extern "C" {
        fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
    }

    pub fn resident_bytes(pid: u32) -> Option<u64> {
        let pid = c_int::try_from(pid).ok()?;
        let mut info = RusageInfoV2::default();
        // SAFETY: `info` is a `#[repr(C)]` struct laid out as `rusage_info_v2`,
        // and the kernel writes at most that many bytes into it.
        let status = unsafe {
            proc_pid_rusage(
                pid,
                RUSAGE_INFO_V2,
                (&mut info as *mut RusageInfoV2).cast::<c_void>(),
            )
        };
        (status == 0).then_some(info.ri_resident_size)
    }
}

#[cfg(test)]
mod unit {
    use super::parse_statm_resident;

    #[test]
    fn statm_resident_is_pages_times_page_size() {
        assert_eq!(
            parse_statm_resident("1000 250 90 1 0 300 0\n", 4096),
            Some(250 * 4096)
        );
        assert_eq!(parse_statm_resident("junk", 4096), None);
    }
}
