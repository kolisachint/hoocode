//! Subagent orchestration for the hoocode coding agent (hoocode's
//! `core/subagent-*.ts`, `lifeguard.ts`, `dispatch-evaluator.ts`,
//! `token-budget.ts`, `output-verifier.ts`, `model-categories.ts`).
//!
//! [`pool::SubagentPool`] runs each subagent as a child process
//! (`hoocode --mode json --task-id <id> ...`): progress events and
//! `{"ping":true}` heartbeats on its stdout, a verified `result.json` in the
//! task's dispatch dir settles it. [`lifeguard`] reaps silent or overdue
//! children. The Task/AgentOutput tools and the warm (RPC) pool build on this.
//!
//! [`ledger`] is the measurement layer: one append-only line per dispatch
//! attempt, so "how reliable are subagents?" has an answer that is not
//! reconstructed from whatever dispatch dirs happened to survive on disk.

pub mod agent_log;
pub mod depth;
pub mod dispatch;
pub mod events;
pub mod inbox;
pub mod instance;
pub mod ledger;
pub mod lifeguard;
pub mod model_categories;
pub mod output_verifier;
pub mod pool;
pub mod result;
pub mod runner;
pub mod token_budget;
pub mod tools;
pub mod warm;

pub use model_categories::{scoped_models_prompt_section, ModelRequest, ModelSelection};

pub use pool::{
    DispatchOptions, PoolError, PoolEvent, SubagentPool, SubagentPoolOptions, SubagentPoolTask,
    SubagentResult, TaskResult, DEFAULT_SUBAGENT_MAX_TURNS,
};
