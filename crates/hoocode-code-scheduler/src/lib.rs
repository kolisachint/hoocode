//! Cron scheduling for the hoocode coding agent: the CronCreate, CronList and
//! CronDelete tools and the task store they share with hoocode-ts.
//!
//! The store is `<cwd>/.agents/scheduled_tasks.json`. The interactive loop
//! calls [`claim_due`] every [`TICK_INTERVAL`] while the agent is idle, and
//! submits each returned prompt as a user message, as `TaskScheduler` does in
//! hoocode-ts (`core/scheduler.ts`).
//!
//! `/loop` is parsed by [`loop_command`]; the TUI runs it. A task due while the
//! agent is busy fires on the next idle tick if that tick is within
//! [`store::LATE_FIRE_MINUTES`] of its due minute, and is skipped otherwise.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local};

pub mod cron;
pub mod loop_command;
pub mod rows;
pub mod store;
pub mod tools;

pub use cron::{is_five_field, matches as cron_matches};
pub use rows::Runs;
pub use store::{
    Claimed, ScheduledTask, TaskStore, Tick, LATE_FIRE_MINUTES, LEGACY_STORE_RELATIVE_PATH,
    STORE_RELATIVE_PATH,
};
pub use tools::{
    create_cron_create_tool_definition, create_cron_delete_tool_definition,
    create_cron_list_tool_definition, create_cron_tool_definitions, CRON_CREATE_TOOL_NAME,
    CRON_DELETE_TOOL_NAME, CRON_LIST_TOOL_NAME,
};

/// `TaskScheduler`'s default tick interval.
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// One tick for the working directory `cwd` at local time `now`: the prompts of
/// the tasks due (see [`TaskStore::claim_due`]), claimed so that no other
/// process fires them again. Callers check that the agent is idle first. A
/// store that cannot be written fires nothing this tick (TS persistence is best
/// effort; the claim is what keeps a task from firing twice).
pub fn claim_due(cwd: &Path, now: &DateTime<Local>) -> Claimed {
    TaskStore::for_cwd(cwd).claim_due(now).unwrap_or_default()
}

/// [`claim_due`] at the current local time.
pub fn claim_due_now(cwd: &Path) -> Claimed {
    claim_due(cwd, &Local::now())
}

/// [`claim_due`] for a tick, with the store's tasks after the claim (for the
/// task panel's Schedule rows; see [`rows`]). Nothing is read when the lock is
/// not taken.
pub fn tick(cwd: &Path, now: &DateTime<Local>) -> Tick {
    TaskStore::for_cwd(cwd)
        .claim_due_tick(now)
        .unwrap_or_default()
}

/// [`tick`] at the current local time.
pub fn tick_now(cwd: &Path) -> Tick {
    tick(cwd, &Local::now())
}
