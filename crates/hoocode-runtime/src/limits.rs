//! Caps and limits that the settings and the agent loop share.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Fewest parallel tool calls a turn may run (`performance.maxParallelTools`).
pub const MIN_PARALLEL_TOOLS: usize = 1;
/// Most parallel tool calls a turn may run (`performance.maxParallelTools`).
pub const MAX_PARALLEL_TOOLS: usize = 32;
/// Highest `nice` value for `bash` children (`performance.bashNice`).
pub const MAX_BASH_NICE: u64 = 19;

/// Caps the tool calls of one turn that run at the same time.
///
/// Permits are granted first come, first served (tokio's semaphore is fair),
/// so calls wait in the order they were started. The limit is clamped to
/// [`MIN_PARALLEL_TOOLS`]..=[`MAX_PARALLEL_TOOLS`]. Clones share the same permits.
#[derive(Debug, Clone)]
pub struct ParallelToolLimit {
    permits: Arc<Semaphore>,
    limit: usize,
}

impl ParallelToolLimit {
    /// A limit of `limit` calls, clamped to the allowed range.
    pub fn new(limit: usize) -> Self {
        let limit = limit.clamp(MIN_PARALLEL_TOOLS, MAX_PARALLEL_TOOLS);
        Self {
            permits: Arc::new(Semaphore::new(limit)),
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
    /// when the permit is dropped.
    pub fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.permits).try_acquire_owned().ok()
    }

    /// Waits for a free slot. The permit is released when it is dropped.
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        // The semaphore is never closed (its only handle is private), so this
        // cannot fail.
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .expect("parallel tool semaphore is never closed")
    }
}

/// Physical RAM in bytes, or `None` when the platform does not report it.
///
/// Linux reads `MemTotal` from `/proc/meminfo`. Other platforms return `None`
/// for now; callers then fall back to fixed defaults.
pub fn total_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_mem_total(&text)
    }
    #[cfg(not(target_os = "linux"))]
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
