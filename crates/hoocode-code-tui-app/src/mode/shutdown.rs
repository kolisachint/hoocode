//! The exit path's ordered shutdown (`docs/design/concurrency.md` section 4):
//! abort the agent, kill the tool process groups, flush the session, close MCP,
//! release the session, then restore the terminal. This file lists the steps and
//! their deadlines; `hoocode_runtime::run_ordered` times them.

use std::time::Duration;

use hoocode_code_agent_session::AgentSession;
use hoocode_code_session::SESSION_FLUSH_DEADLINE;
use hoocode_code_tool_bash::shell::kill_tracked_tool_children;
use hoocode_runtime::{io_handle, run_ordered, sync_bounded_channel, ShutdownStep, StepReport};

/// Abort the running turn and wait for the agent to go idle.
const ABORT_DEADLINE: Duration = Duration::from_millis(800);
/// Kill the process groups of `Shell` calls still in flight.
const KILL_TOOLS_DEADLINE: Duration = Duration::from_millis(100);
/// Write out the queued session entries.
const FLUSH_DEADLINE: Duration = SESSION_FLUSH_DEADLINE;
/// Stop the MCP servers.
const MCP_DEADLINE: Duration = Duration::from_millis(700);
/// Drop the listeners and the agent connection.
const RELEASE_DEADLINE: Duration = Duration::from_millis(200);

/// The steps in the order they run. Their deadlines sum to 2.8 s; the terminal
/// restore has the last 0.2 s of the 3 s budget.
fn steps(session: &AgentSession) -> Vec<ShutdownStep> {
    let streaming = session.is_streaming();
    let abort = session.clone();
    let flush = session.clone();
    let mcp = session.clone();
    let release = session.clone();
    vec![
        ShutdownStep::new("abort agent", ABORT_DEADLINE, move || {
            if streaming {
                abort_on_io(abort);
            }
        }),
        ShutdownStep::new(
            "kill tool process groups",
            KILL_TOOLS_DEADLINE,
            kill_tracked_tool_children,
        ),
        ShutdownStep::new("flush session", FLUSH_DEADLINE, move || {
            flush.flush_for_shutdown(FLUSH_DEADLINE);
        }),
        ShutdownStep::new("close MCP", MCP_DEADLINE, move || {
            mcp.close_mcp(MCP_DEADLINE);
        }),
        ShutdownStep::new("release session", RELEASE_DEADLINE, move || {
            release.release();
        }),
    ]
}

/// Runs `session.abort()` on the io runtime and waits for it. The step's
/// deadline cuts the wait; an abort that is still going continues on the io
/// runtime and ends with the process.
fn abort_on_io(session: AgentSession) {
    let (done, finished) = sync_bounded_channel::<()>(1);
    io_handle().spawn(async move {
        session.abort().await;
        let _ = done.send(());
    });
    let _ = finished.recv();
}

/// Runs the ordered shutdown and restores the terminal last, through `restore`.
/// Returns one report per step.
pub(super) fn run(session: &AgentSession, restore: impl FnOnce()) -> Vec<StepReport> {
    run_ordered(steps(session), restore)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_runtime::{RESTORE_RESERVE, SHUTDOWN_BUDGET};

    #[test]
    fn the_steps_and_the_restore_fit_the_shutdown_budget() {
        let steps_total =
            ABORT_DEADLINE + KILL_TOOLS_DEADLINE + FLUSH_DEADLINE + MCP_DEADLINE + RELEASE_DEADLINE;
        assert!(steps_total + RESTORE_RESERVE <= SHUTDOWN_BUDGET);
    }
}
