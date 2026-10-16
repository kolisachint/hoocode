//! Subagent run limits: the turn cap, the advisory token budget, the wall-clock
//! deadline, the progress stall threshold and the child resident-memory cap.
//!
//! Each limit resolves in the same order: the `HOOCODE_` environment variable,
//! then the settings value, then the default. A value that is missing,
//! non-numeric, non-finite, or below 1 falls through to the next source (the
//! same rule as `depth.rs`). Agent frontmatter (`maxTurns`, `tokenBudget`) sits
//! above both and is applied with [`SubagentLimits::turns_for`] and
//! [`SubagentLimits::token_budget_for`].
//!
//! | limit | env (wins) | setting | unit | default |
//! |---|---|---|---|---|
//! | max turns | `HOOCODE_SUBAGENT_MAX_TURNS` | `subagentMaxTurns` | turns | 200 |
//! | token budget | `HOOCODE_SUBAGENT_TOKEN_BUDGET` | `subagentTokenBudget` | tokens | per type |
//! | deadline | `HOOCODE_SUBAGENT_DEADLINE_MS` | `subagentDeadlineMinutes` | ms / min | 4 h |
//! | stall | `HOOCODE_SUBAGENT_STALL_MS` | `subagentStallSeconds` | ms / s | 20 min |
//! | child RSS | `HOOCODE_SUBAGENT_MAX_RSS_MB` | `subagentMaxRssMb` | MiB | 4096 MiB |

use std::collections::HashMap;

use hoocode_code_settings::SettingsManager;
use hoocode_runtime::{CHILD_RSS_LIMIT_BYTES, MIB};

use crate::lifeguard::{
    max_run_budget_ms, PROGRESS_STALL_THRESHOLD_MS, STALL_THRESHOLD_ENV, SUBAGENT_DEADLINE_MS,
};
use crate::pool::DEFAULT_SUBAGENT_MAX_TURNS;
use crate::token_budget::get_default_budget;

/// Environment override for the turn cap.
pub const MAX_TURNS_ENV: &str = "HOOCODE_SUBAGENT_MAX_TURNS";
/// Environment override for the advisory token budget (every type).
pub const TOKEN_BUDGET_ENV: &str = "HOOCODE_SUBAGENT_TOKEN_BUDGET";
/// Environment override for the wall-clock deadline, in milliseconds.
pub const DEADLINE_ENV: &str = "HOOCODE_SUBAGENT_DEADLINE_MS";
/// Environment override for the child resident-memory cap, in MiB.
pub const MAX_RSS_ENV: &str = "HOOCODE_SUBAGENT_MAX_RSS_MB";

/// The `subagent*` settings, as read from the settings files. `None` is unset.
/// Each value is in the setting's own unit (see the module table).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentLimitSettings {
    pub max_turns: Option<f64>,
    pub token_budget: Option<f64>,
    pub deadline_minutes: Option<f64>,
    pub stall_seconds: Option<f64>,
    pub max_rss_mb: Option<f64>,
}

impl SubagentLimitSettings {
    /// The limit settings from the settings files.
    pub fn from_manager(manager: &SettingsManager) -> Self {
        Self {
            max_turns: manager.subagent_max_turns(),
            token_budget: manager.subagent_token_budget(),
            deadline_minutes: manager.subagent_deadline_minutes(),
            stall_seconds: manager.subagent_stall_seconds(),
            max_rss_mb: manager.subagent_max_rss_mb(),
        }
    }
}

/// The resolved limits a pool runs with: env, then setting, then default.
/// Frontmatter is applied per agent by the accessor methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentLimits {
    /// Turn cap for an agent without its own `maxTurns`.
    pub max_turns: u64,
    /// Token budget for every type, or `None` to keep each type's default.
    pub token_budget_override: Option<u64>,
    /// Base wall-clock deadline for every run, in milliseconds.
    pub deadline_ms: u64,
    /// No-forward-progress threshold, in milliseconds (before load scaling).
    pub stall_ms: u64,
    /// Resident-memory cap for one child process, in bytes.
    pub max_rss_bytes: u64,
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self {
            max_turns: DEFAULT_SUBAGENT_MAX_TURNS,
            token_budget_override: None,
            deadline_ms: SUBAGENT_DEADLINE_MS,
            stall_ms: PROGRESS_STALL_THRESHOLD_MS,
            max_rss_bytes: CHILD_RSS_LIMIT_BYTES,
        }
    }
}

/// A value usable as a limit: finite and at least 1, floored.
fn whole(value: f64) -> Option<u64> {
    (value.is_finite() && value >= 1.0).then(|| value.floor() as u64)
}

/// The first usable value of `env_key` (parsed as a number), else `setting`.
/// `setting` is already in the same unit as the env value.
fn pick(env: &HashMap<String, String>, env_key: &str, setting: Option<f64>) -> Option<u64> {
    env.get(env_key)
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .and_then(whole)
        .or_else(|| setting.and_then(whole))
}

impl SubagentLimits {
    /// Resolve every limit from `env` (the pool's environment) and `settings`.
    pub fn resolve(settings: &SubagentLimitSettings, env: &HashMap<String, String>) -> Self {
        let defaults = Self::default();
        Self {
            max_turns: pick(env, MAX_TURNS_ENV, settings.max_turns).unwrap_or(defaults.max_turns),
            token_budget_override: pick(env, TOKEN_BUDGET_ENV, settings.token_budget),
            deadline_ms: pick(
                env,
                DEADLINE_ENV,
                settings.deadline_minutes.map(|m| m * 60_000.0),
            )
            .unwrap_or(defaults.deadline_ms),
            stall_ms: pick(
                env,
                STALL_THRESHOLD_ENV,
                settings.stall_seconds.map(|s| s * 1_000.0),
            )
            .unwrap_or(defaults.stall_ms),
            max_rss_bytes: pick(env, MAX_RSS_ENV, settings.max_rss_mb)
                .map(|mb| mb.saturating_mul(MIB))
                .unwrap_or(defaults.max_rss_bytes),
        }
    }

    /// Turn cap for an agent: its `maxTurns` frontmatter when set (and above 0),
    /// else the resolved cap.
    pub fn turns_for(&self, frontmatter: Option<u64>) -> u64 {
        frontmatter.filter(|n| *n > 0).unwrap_or(self.max_turns)
    }

    /// Advisory token budget for an agent: its `tokenBudget` frontmatter when
    /// set, else the env or setting override, else the type's default.
    pub fn token_budget_for(&self, agent_type: &str, frontmatter: Option<u64>) -> u64 {
        frontmatter
            .filter(|n| *n > 0)
            .or(self.token_budget_override)
            .unwrap_or_else(|| get_default_budget(agent_type))
    }

    /// The longest a run's budget can get under load with this deadline. The
    /// `AgentOutput` reconcile age is derived from it.
    pub fn max_run_budget_ms(&self) -> u64 {
        max_run_budget_ms(self.deadline_ms)
    }
}
