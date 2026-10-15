mod agent_log_ts;
mod depth;
mod dispatch_evaluator;
mod events;
mod hardening;
mod inbox;
mod ledger;
mod lifeguard;
mod live;
mod model_categories;
mod model_selection;
mod output_verifier;
mod pool;
mod result;
mod runner;
mod shipped_prose;
mod subagent_skills;
mod subagent_spawn_audit_ts;
mod token_budget;
mod tools;
mod warm;

/// One test at a time touches the process-wide inbox, task store and pool slot.
/// Every module that does takes this lock: per-module locks let two of them race.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The process env as a root pool sees it: any inherited depth is dropped.
/// Without this, a test run from inside a subagent (`HOOCODE_SUBAGENT_DEPTH=1`)
/// makes the pool's children depth 2.
pub fn root_pool_env() -> std::collections::HashMap<String, String> {
    let depth_vars: Vec<String> = hoocode_code_paths::ENV_PREFIXES
        .iter()
        .map(|prefix| format!("{prefix}SUBAGENT_DEPTH"))
        .collect();
    std::env::vars()
        .filter(|(key, _)| !depth_vars.contains(key))
        .collect()
}
