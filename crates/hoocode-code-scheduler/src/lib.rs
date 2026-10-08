//! Cron scheduling for the hoocode coding agent: the CronCreate, CronList and
//! CronDelete tools and the task store they share with hoocode-ts.
//!
//! The store is `<cwd>/.agents/scheduled_tasks.json`. The interactive loop
//! calls [`claim_due`] every [`TICK_INTERVAL`] while the agent is idle, and
//! submits each returned prompt as a user message, as `TaskScheduler` does in
//! hoocode-ts (`core/scheduler.ts`).
//!
//! Not here yet: `/loop` (the slash command and `/loop auto`), and firing
//! while a turn is running (a due task waits for the next idle tick in the
//! same minute only, as in hoocode-ts).

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local};

pub mod cron;
pub mod store;
pub mod tools;

pub use cron::{is_five_field, matches as cron_matches};
pub use store::{ScheduledTask, TaskStore, LEGACY_STORE_RELATIVE_PATH, STORE_RELATIVE_PATH};
pub use tools::{
    create_cron_create_tool_definition, create_cron_delete_tool_definition,
    create_cron_list_tool_definition, create_cron_tool_definitions, CRON_CREATE_TOOL_NAME,
    CRON_DELETE_TOOL_NAME, CRON_LIST_TOOL_NAME,
};

/// `TaskScheduler`'s default tick interval.
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// One tick for the working directory `cwd` at local time `now`: the prompts of
/// the tasks due in this minute, claimed so that no other process fires them
/// again. Callers check that the agent is idle first. A store that cannot be
/// written fires nothing this tick (TS persistence is best effort; the claim
/// is what keeps a task from firing twice).
pub fn claim_due(cwd: &Path, now: &DateTime<Local>) -> Vec<String> {
    TaskStore::for_cwd(cwd).claim_due(now).unwrap_or_default()
}

/// [`claim_due`] at the current local time.
pub fn claim_due_now(cwd: &Path) -> Vec<String> {
    claim_due(cwd, &Local::now())
}
