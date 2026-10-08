//! Memory limits: the soft/hard state machine, default limit math and the
//! guard over an injected RSS source. No test allocates real memory.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hoocode_runtime::{
    default_memory_limits_mb, MemoryEvent, MemoryGuard, MemoryMonitor, CHILD_RSS_LIMIT_BYTES, MIB,
};

const SOFT: u64 = 1000 * MIB;
const HARD: u64 = 2000 * MIB;

fn mib(n: u64) -> u64 {
    n * MIB
}

#[test]
fn soft_limit_sheds_and_recovers_with_hysteresis() {
    let mut m = MemoryMonitor::new(SOFT, 0);
    assert!(m.observe(mib(900)).is_empty());
    assert!(!m.shedding());

    assert_eq!(
        m.observe(mib(1100)),
        vec![MemoryEvent::ShedStarted {
            rss: mib(1100),
            limit: SOFT
        }]
    );
    assert!(m.shedding());

    // Dipping just under the limit does not lift shedding: it must be 10% under.
    assert!(m.observe(mib(950)).is_empty());
    assert!(m.shedding());
    assert!(m.observe(mib(901)).is_empty());
    assert!(m.shedding());

    assert_eq!(
        m.observe(mib(900)),
        vec![MemoryEvent::ShedEnded {
            rss: mib(900),
            limit: SOFT
        }]
    );
    assert!(!m.shedding());
}

#[test]
fn shedding_starts_once_per_crossing() {
    let mut m = MemoryMonitor::new(SOFT, 0);
    assert_eq!(m.observe(mib(1100)).len(), 1);
    assert!(m.observe(mib(1500)).is_empty(), "still above: no new event");
    assert!(m.observe(mib(1200)).is_empty());
}

#[test]
fn hard_limit_fires_once_and_rearms_after_recovery() {
    let mut m = MemoryMonitor::new(0, HARD);
    assert!(m.observe(mib(1900)).is_empty());
    assert_eq!(
        m.observe(mib(2100)),
        vec![MemoryEvent::HardLimit {
            rss: mib(2100),
            limit: HARD
        }]
    );
    assert!(m.observe(mib(2200)).is_empty(), "one event per crossing");
    // 1900 MiB is above the 10%-under mark (1800 MiB): still tripped.
    assert!(m.observe(mib(1900)).is_empty());
    assert!(
        m.observe(mib(1800)).is_empty(),
        "re-armed, no event on recovery"
    );
    assert_eq!(m.observe(mib(2500)).len(), 1, "a new crossing fires again");
}

#[test]
fn soft_and_hard_are_independent_and_both_can_fire_on_one_sample() {
    let mut m = MemoryMonitor::new(SOFT, HARD);
    let events = m.observe(mib(2100));
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], MemoryEvent::ShedStarted { .. }));
    assert!(matches!(events[1], MemoryEvent::HardLimit { .. }));
}

#[test]
fn zero_limits_are_off() {
    let mut m = MemoryMonitor::new(0, 0);
    assert!(m.observe(mib(100_000)).is_empty());
    assert!(!m.shedding());
}

#[test]
fn turning_the_soft_limit_off_lifts_shedding() {
    let mut m = MemoryMonitor::new(SOFT, 0);
    m.observe(mib(1100));
    assert!(m.shedding());
    m.set_limits(0, 0);
    assert_eq!(
        m.observe(mib(1100)),
        vec![MemoryEvent::ShedEnded {
            rss: mib(1100),
            limit: 0
        }]
    );
    assert!(!m.shedding());
}

#[test]
fn default_limits_follow_ram() {
    // 16 GiB machine: soft is 25% = 4096 MB capped at 2048; hard is 50% = 8192 capped at 4096.
    assert_eq!(
        default_memory_limits_mb(Some(16 * 1024 * MIB)),
        (2048, 4096)
    );
    // 4 GiB machine: soft 1024 (25%), hard 2048 (50%), both under the caps.
    assert_eq!(default_memory_limits_mb(Some(4 * 1024 * MIB)), (1024, 2048));
    // 1 GiB machine: 25% is 256, the floor; 50% is 512.
    assert_eq!(default_memory_limits_mb(Some(1024 * MIB)), (256, 512));
    // Tiny machine: both floor at 256, so the hard limit is still above the soft one.
    assert_eq!(default_memory_limits_mb(Some(512 * MIB)), (256, 256));
    // RAM unknown: the caps.
    assert_eq!(default_memory_limits_mb(None), (2048, 4096));
}

#[test]
fn child_rss_limit_is_two_gib() {
    assert_eq!(CHILD_RSS_LIMIT_BYTES, 2048 * MIB);
}

#[test]
fn guard_samples_the_injected_source_and_reports_ticks() {
    let samples = Arc::new(Mutex::new(VecDeque::from(vec![
        Some(mib(500)),
        None,
        Some(mib(1200)),
        Some(mib(2300)),
    ])));
    let feed = Arc::clone(&samples);
    let mut guard = MemoryGuard::new(Box::new(move || feed.lock().unwrap().pop_front().flatten()));
    guard.set_limits(SOFT, HARD);

    let first = guard.tick();
    assert_eq!(first.rss, Some(mib(500)));
    assert!(!first.shedding && first.events.is_empty());

    let unknown = guard.tick();
    assert_eq!(unknown.rss, None);
    assert!(unknown.events.is_empty(), "an unknown RSS changes nothing");

    let soft = guard.tick();
    assert!(soft.shedding);
    assert_eq!(soft.events.len(), 1);

    let hard = guard.tick();
    assert!(hard.shedding);
    assert_eq!(
        hard.events,
        vec![MemoryEvent::HardLimit {
            rss: mib(2300),
            limit: HARD
        }]
    );
}

#[test]
fn guard_limits_can_change_between_samples() {
    let mut guard = MemoryGuard::new(Box::new(|| Some(mib(1500))));
    assert!(
        guard.tick().events.is_empty(),
        "no limits: nothing to judge"
    );
    guard.set_limits(SOFT, 0);
    let tick = guard.tick();
    assert!(tick.shedding);
    assert_eq!(tick.events.len(), 1);
}
