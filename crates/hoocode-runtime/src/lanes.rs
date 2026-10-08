//! Scheduling lanes and the OS priority of their threads
//! (`docs/design/concurrency.md` section 2).
//!
//! Priority only goes down. `High` is the normal priority the process already
//! has, so it is never raised; `Medium` and `Low` are lowered. Lowering needs no
//! privilege. Failing to set a priority is logged once and the thread carries on.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crate::threads::spawn_named_thread;

/// A scheduling lane. Each lane has its own threads, queue and cap, so low
/// priority work never runs on a high priority thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Keys, frames, terminal output and the agent's LLM streams. Normal priority.
    High,
    /// Tool bodies, session writes and `Shell` I/O. Lowered a little.
    Medium,
    /// Subagent work and housekeeping (`hoocode-bg`). Lowered the most.
    Low,
}

impl Lane {
    /// The Linux niceness of a thread in this lane: 0, +5 or +10.
    pub const fn linux_nice(self) -> i32 {
        match self {
            Lane::High => 0,
            Lane::Medium => 5,
            Lane::Low => 10,
        }
    }
}

static PRIORITY_LOGGED: AtomicBool = AtomicBool::new(false);

/// Sets the OS priority of the calling thread to `lane`. A failure is logged
/// once for the whole process, then ignored.
pub fn apply_current_thread_lane(lane: Lane) {
    if let Err(reason) = set_current_thread_lane(lane) {
        if !PRIORITY_LOGGED.swap(true, Ordering::Relaxed) {
            eprintln!("hoocode: could not set thread priority ({lane:?} lane): {reason}");
        }
    }
}

/// Starts a named OS thread in `lane` (see [`spawn_named_thread`]).
pub fn spawn_lane_thread<F, T>(name: &str, lane: Lane, f: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    spawn_named_thread(name, move || {
        apply_current_thread_lane(lane);
        f()
    })
}

/// The `thread-priority` crate maps 0..=99 onto niceness -20..=19 as
/// `trunc(39 * (1 - p / 99)) - 20`. This picks the value whose niceness is
/// `nice`, taking the middle of the range that maps to it so rounding cannot
/// land one step off. The unit test and the Linux `/proc` tests check it.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn crossplatform_value_for_nice(nice: i32) -> u8 {
    let k = f64::from(nice + 20);
    (99.0 * (1.0 - (k + 0.5) / 39.0)).round() as u8
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_current_thread_lane(lane: Lane) -> Result<(), String> {
    use thread_priority::{set_current_thread_priority, ThreadPriority, ThreadPriorityValue};

    let target = lane.linux_nice();
    if target == 0 {
        return Ok(());
    }
    // Never raise: a process started at a higher niceness (a subagent child)
    // keeps it. Linux keeps niceness per thread, so this reads the calling thread.
    // SAFETY: getpriority only reads the niceness of the given id.
    let current = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    if current >= target {
        return Ok(());
    }
    let value = ThreadPriorityValue::try_from(crossplatform_value_for_nice(target))
        .map_err(|e| format!("{e:?}"))?;
    set_current_thread_priority(ThreadPriority::Crossplatform(value)).map_err(|e| format!("{e:?}"))
}

#[cfg(windows)]
fn set_current_thread_lane(lane: Lane) -> Result<(), String> {
    use thread_priority::{set_current_thread_priority, ThreadPriority, WinAPIThreadPriority};

    let priority = match lane {
        Lane::High => WinAPIThreadPriority::Normal,
        Lane::Medium => WinAPIThreadPriority::BelowNormal,
        Lane::Low => WinAPIThreadPriority::Lowest,
    };
    set_current_thread_priority(ThreadPriority::Os(priority.into())).map_err(|e| format!("{e:?}"))
}

#[cfg(target_os = "macos")]
fn set_current_thread_lane(lane: Lane) -> Result<(), String> {
    // QoS classes from <sys/qos.h>. The thread-priority crate has no QoS API.
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    const QOS_CLASS_UTILITY: u32 = 0x11;
    const QOS_CLASS_BACKGROUND: u32 = 0x09;
    extern "C" {
        fn pthread_set_qos_class_self_np(
            qos_class: u32,
            relative_priority: libc::c_int,
        ) -> libc::c_int;
    }
    let qos = match lane {
        Lane::High => QOS_CLASS_USER_INTERACTIVE,
        Lane::Medium => QOS_CLASS_UTILITY,
        Lane::Low => QOS_CLASS_BACKGROUND,
    };
    // SAFETY: sets the calling thread's own QoS class; no pointers are passed.
    let ret = unsafe { pthread_set_qos_class_self_np(qos, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(format!("pthread_set_qos_class_self_np returned {ret}"))
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
fn set_current_thread_lane(_lane: Lane) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::Lane;

    #[test]
    fn lanes_map_to_the_linux_niceness_table() {
        assert_eq!(Lane::High.linux_nice(), 0);
        assert_eq!(Lane::Medium.linux_nice(), 5);
        assert_eq!(Lane::Low.linux_nice(), 10);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn crossplatform_values_map_back_to_their_niceness() {
        // thread-priority's own formula, applied to the value picked for each nice.
        for nice in [0, 5, 10] {
            let p = f64::from(super::crossplatform_value_for_nice(nice));
            let back = (39.0 * (1.0 - p / 99.0)).trunc() as i32 - 20;
            assert_eq!(back, nice);
        }
    }
}
