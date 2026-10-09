//! `core/subagent-pool-instance.ts`: the process-wide [`SubagentPool`].
//!
//! The Agent tool and `/subagent` share one pool, so concurrency limits,
//! lifeguard monitoring and token budgets span every delegation in the
//! session. Created lazily on first use (inside a tokio runtime).

use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

use hoocode_ai_types::Model;
use hoocode_code_models::{resolve_scoped_models, ResolvedScoped};
use hoocode_code_settings::{deep_merge_settings, ScopedModel, SettingsManager};
use hoocode_code_task_store::{task_store, TaskAgentPatch};
use serde_json::Value;

use crate::depth::{pool_concurrency_for_depth, ProcessEnv};
use crate::model_categories::CategorySettings;
use crate::pool::{SubagentPool, SubagentPoolOptions};

#[derive(Default)]
struct Instance {
    pool: Option<SubagentPool>,
    override_pool: Option<SubagentPool>,
    /// Latest non-default skill paths, kept in sync with the resource loader.
    skill_paths: Vec<String>,
    /// The session's scoped models (`--models`), replacing `scopedModels` from
    /// the settings for this run. `None`: read the settings.
    scoped_override: Option<Vec<ScopedModel>>,
}

static INSTANCE: LazyLock<Mutex<Instance>> = LazyLock::new(Mutex::default);

fn instance() -> MutexGuard<'static, Instance> {
    INSTANCE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `getSubagentSpawnCommand`: this executable, no prefix args.
fn spawn_command() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "hoocode".into())
}

/// Sets the scoped models for this process (the `--models` flag): they replace
/// `scopedModels` from the settings. `None` goes back to the settings.
pub fn set_scoped_models_override(models: Option<Vec<ScopedModel>>) {
    instance().scoped_override = models;
}

/// Whether a run-only scope (`--models`) is set. The TUI keeps its picker save
/// to the settings file while it is, so the run's scope stays as the flag set it.
pub fn scoped_models_override_active() -> bool {
    instance().scoped_override.is_some()
}

/// The scope a dispatch uses now: the session override, else `scopedModels`
/// from the settings, resolved against `available_models`. Empty: no scope.
pub fn current_scope(cwd: &Path, available_models: &[Model]) -> Vec<ResolvedScoped> {
    let override_models = instance().scoped_override.clone();
    scope_from(cwd, available_models, override_models)
}

fn scope_from(
    cwd: &Path,
    available_models: &[Model],
    override_models: Option<Vec<ScopedModel>>,
) -> Vec<ResolvedScoped> {
    let entries = override_models
        .or_else(|| SettingsManager::create(cwd, hoocode_code_paths::agent_dir()).scoped_models());
    resolve_scoped_models(&entries.unwrap_or_default(), available_models)
}

/// The shared pool for `cwd`, created on first use. `available_models` (the
/// caller's available models) is snapshotted then, for deriving model
/// categories; later calls reuse the pool.
pub fn get_subagent_pool(cwd: &Path, available_models: &[Model]) -> SubagentPool {
    let mut instance = instance();
    if let Some(pool) = &instance.override_pool {
        return pool.clone();
    }
    if let Some(pool) = &instance.pool {
        return pool.clone();
    }
    let scope = scope_from(cwd, available_models, instance.scoped_override.clone());
    let manager = SettingsManager::create(cwd, hoocode_code_paths::agent_dir());
    // One-level deep merge, the same rule the rest of the codebase uses. A
    // plain `extend` replaced the whole `modelCategories` object, so a project
    // that set only `capable` silently lost the global `fast` and `standard`
    // tiers and every model tier fell back to a derived default.
    let settings = deep_merge_settings(&manager.global_settings(), &manager.project_settings());
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: spawn_command(),
        cwd: Some(cwd.to_path_buf()),
        skill_paths: instance.skill_paths.clone(),
        // Nested pools (depth >= 1) run with the reduced cap.
        max_concurrency: pool_concurrency_for_depth(&ProcessEnv).map(|n| n as usize),
        settings: Some(CategorySettings::from_settings(&settings)),
        available_models: available_models.to_vec(),
        scope,
        ..Default::default()
    });
    wire_progress_to_task_store(&pool);
    instance.pool = Some(pool.clone());
    pool
}

/// Live subagent detail on the task panel's roster row (keyed by the pool task
/// id): which attempt is running, on which model, how long is left, and — when
/// it ends — how it ended.
///
/// The panel used to show only the running tool and a stopwatch, so a retry on
/// a different model, a run that was cut short and a run that failed looked
/// identical from the outside. Everything here comes from pool events, so the
/// row never guesses.
fn wire_progress_to_task_store(pool: &SubagentPool) {
    pool.on(|event| {
        let Some(task_id) = event.data.get("task_id").and_then(Value::as_str) else {
            return;
        };
        let patch = match event.name {
            "task_started" => TaskAgentPatch {
                activity: Some("starting".into()),
                attempt: event
                    .data
                    .get("attempt")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32),
                model: event
                    .data
                    .get("model")
                    .and_then(Value::as_str)
                    .map(String::from),
                deadline_at: event.data.get("deadline_at").and_then(Value::as_u64),
                outcome: None,
                cause: None,
                ..Default::default()
            },
            "task_progress" => {
                let activity = match event.data["event"].get("type").and_then(Value::as_str) {
                    Some("tool_execution_start") => event.data["event"]
                        .get("toolName")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    Some("turn_end") => "thinking".to_string(),
                    Some("tool_execution_end") => String::new(),
                    _ => return,
                };
                TaskAgentPatch {
                    activity: Some(activity),
                    ..Default::default()
                }
            }
            name if name.starts_with("task_") && name != "task_progress" => {
                // Every terminal event: record how it ended so the row keeps
                // saying something useful after the run is gone.
                let status = event
                    .data
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or(match name {
                        "task_done" => "complete",
                        "task_failed" => "failed",
                        "task_stalled" => "stalled",
                        "task_timeout" => "timeout",
                        _ => "cancelled",
                    })
                    .to_string();
                TaskAgentPatch {
                    activity: Some(String::new()),
                    outcome: Some(status),
                    confidence: event.data.get("confidence").and_then(Value::as_f64),
                    cause: event
                        .data
                        .get("cause")
                        .and_then(Value::as_str)
                        .or_else(|| event.data.get("error").and_then(Value::as_str))
                        .map(str::to_string),
                    ..Default::default()
                }
            }
            _ => return,
        };
        task_store().patch_agent(task_id, patch);
    });
}

/// The shared pool if one exists (never creates it).
pub fn peek_subagent_pool() -> Option<SubagentPool> {
    let instance = instance();
    instance
        .override_pool
        .clone()
        .or_else(|| instance.pool.clone())
}

/// Update the skill paths forwarded to every subagent.
pub fn update_subagent_skill_paths(paths: Vec<String>) {
    let mut instance = instance();
    instance.skill_paths = paths.clone();
    if let Some(pool) = &instance.pool {
        pool.update_skill_paths(paths);
    }
}

/// Dispose and clear the shared pool (shutdown, test isolation).
pub fn dispose_subagent_pool() {
    let pool = {
        let mut instance = instance();
        instance.skill_paths.clear();
        instance.pool.take()
    };
    if let Some(pool) = pool {
        pool.dispose();
    }
}

/// Inject a pool for tests; `None` clears the override.
pub fn set_subagent_pool_for_testing(pool: Option<SubagentPool>) {
    instance().override_pool = pool;
}
