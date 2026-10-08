//! Caps and limits that the settings and the agent loop share.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Fewest parallel tool calls a turn may run (`performance.maxParallelTools`).
pub const MIN_PARALLEL_TOOLS: usize = 1;
/// Most parallel tool calls a turn may run (`performance.maxParallelTools`).
pub const MAX_PARALLEL_TOOLS: usize = 32;
/// Highest `nice` value for `bash` children (`performance.bashNice`).
pub const MAX_BASH_NICE: u64 = 19;

/// One running tool call's share of a [`ParallelToolLimit`]. Dropping it frees
/// the slot (and the shedding gate, when the call held it).
#[derive(Debug)]
pub struct ParallelSlot {
    _permit: OwnedSemaphorePermit,
    _gate: Option<OwnedSemaphorePermit>,
}

/// Caps the tool calls of one turn that run at the same time.
///
/// Permits are granted first come, first served (tokio's semaphore is fair),
/// so calls wait in the order they were started. The limit is clamped to
/// [`MIN_PARALLEL_TOOLS`]..=[`MAX_PARALLEL_TOOLS`]. Clones share the same permits.
///
/// While the process sheds load ([`crate::memory::shedding`]), each new call
/// also takes the one-slot gate, so new calls run one at a time. Calls already
/// running finish as they are. The gate is taken before the slot, in call
/// order, so a waiting call never holds a slot while it waits for the gate.
#[derive(Debug, Clone)]
pub struct ParallelToolLimit {
    permits: Arc<Semaphore>,
    gate: Arc<Semaphore>,
    limit: usize,
}

impl ParallelToolLimit {
    /// A limit of `limit` calls, clamped to the allowed range.
    pub fn new(limit: usize) -> Self {
        let limit = limit.clamp(MIN_PARALLEL_TOOLS, MAX_PARALLEL_TOOLS);
        Self {
            permits: Arc::new(Semaphore::new(limit)),
            gate: Arc::new(Semaphore::new(1)),
            limit,
        }
    }

    /// The effective limit after clamping.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Permits not held by a running call right now.
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Takes a free slot if one is free, without waiting. The slot is released
    /// when the returned value is dropped.
    pub fn try_acquire(&self) -> Option<ParallelSlot> {
        let gate = if crate::memory::shedding() {
            Some(Arc::clone(&self.gate).try_acquire_owned().ok()?)
        } else {
            None
        };
        let permit = Arc::clone(&self.permits).try_acquire_owned().ok()?;
        Some(ParallelSlot {
            _permit: permit,
            _gate: gate,
        })
    }

    /// Waits for a free slot. The slot is released when the returned value is dropped.
    pub async fn acquire(&self) -> ParallelSlot {
        // The semaphores are never closed (their only handles are private), so
        // these cannot fail.
        let gate = if crate::memory::shedding() {
            Some(
                Arc::clone(&self.gate)
                    .acquire_owned()
                    .await
                    .expect("parallel tool gate is never closed"),
            )
        } else {
            None
        };
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .expect("parallel tool semaphore is never closed");
        ParallelSlot {
            _permit: permit,
            _gate: gate,
        }
    }
}

/// Physical RAM in bytes, or `None` when the platform does not report it.
///
/// Linux reads `MemTotal` from `/proc/meminfo`; macOS runs `sysctl -n hw.memsize`
/// once. Other platforms return `None`; callers then use fixed defaults.
pub fn total_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_mem_total(&text)
    }
    #[cfg(target_os = "macos")]
    {
        static TOTAL: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *TOTAL.get_or_init(|| {
            let out = std::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Parses the `MemTotal:` line of `/proc/meminfo` (value in kB).
fn parse_mem_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod unit {
    use super::parse_mem_total;

    #[test]
    fn parses_mem_total_in_kib() {
        let text = "MemTotal:       16318236 kB\nMemFree:  1 kB\n";
        assert_eq!(parse_mem_total(text), Some(16_318_236 * 1024));
        assert_eq!(parse_mem_total("MemFree: 1 kB\n"), None);
    }
}
