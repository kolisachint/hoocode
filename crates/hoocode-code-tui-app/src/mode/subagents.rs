//! Subagent commands: `/subagent`, cancel, retry, stats, and the settle of a finished run.

use hoocode_agent_types::{AgentMessage, CustomMessage};
use hoocode_ai_types::UserContent;
use hoocode_code_resources::agent_registry::{load_agent_registry, LoadAgentRegistryOptions};
use hoocode_code_subagents::instance::get_subagent_pool;
use hoocode_code_subagents::ledger;
use hoocode_code_subagents::pool::DispatchOptions;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::{Spacer, Text};

use super::*;

/// `toLocaleString()` for a count: grouped with commas.
pub(super) fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Mode {
    /// `handleSubagent`: `/subagent <mode> <task>` runs one subagent of that
    /// type off the UI thread; [`Self::finish_subagent`] reports it.
    pub(super) fn handle_subagent_command(&mut self, text: &str) {
        const USAGE: &str = "Usage: /subagent <mode> <task>";
        let args = text.strip_prefix("/subagent ").map_or("", str::trim);
        let Some((mode, task)) = args.split_once(' ') else {
            self.show_status(USAGE);
            return;
        };
        let (mode, task) = (mode.trim().to_string(), task.trim().to_string());
        if task.is_empty() {
            self.show_status(USAGE);
            return;
        }
        let cwd = self.session.cwd().to_path_buf();
        let registry = load_agent_registry(&LoadAgentRegistryOptions::new(
            cwd.to_string_lossy().into_owned(),
        ));
        let valid: Vec<String> = registry.list().iter().map(|a| a.name.clone()).collect();
        if !valid.contains(&mode) {
            self.show_status(&format!(
                "Unknown subagent_type: {mode}. Available: {}",
                valid.join(", ")
            ));
            return;
        }

        self.show_status(&format!("Spawning {mode} subagent..."));
        let available = self.session.get_available_models();
        let model = self.session.model();
        let options = DispatchOptions {
            force_agent: Some(mode.clone()),
            model: model.as_ref().map(|m| m.id.clone()),
            provider: model.as_ref().map(|m| m.provider.clone()),
            ..Default::default()
        };
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            // The pool's lifeguard runs on this runtime, so it is made here.
            let pool = get_subagent_pool(&cwd, &available);
            let outcome = match pool.dispatch(&task, options).await {
                Ok(dispatched) => match dispatched.result {
                    Some(result) if result.ok => Ok(result
                        .result_data
                        .as_ref()
                        .and_then(|data| data.get("summary"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)),
                    result => Err(format!(
                        "Subagent ({mode}) failed: {}",
                        result
                            .and_then(|r| r.error)
                            .unwrap_or_else(|| "unknown error".into())
                    )),
                },
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(AppEvent::SubagentDone(mode, outcome));
        });
    }

    /// `/subagent-cancel [task_id]`: stop the newest running dispatch, or the
    /// named one. The panel could not do this before: a run could only be
    /// cancelled by interrupting the whole turn.
    pub(super) fn cancel_newest_subagent(&mut self, text: &str) {
        let wanted = text
            .strip_prefix("/subagent-cancel")
            .map_or("", str::trim)
            .to_string();
        let cwd = self.session.cwd().to_path_buf();
        let models = self.session.get_available_models();
        let pool = get_subagent_pool(&cwd, &models);
        let status = pool.statuses();
        let candidate = status
            .iter()
            .filter(|(_, state)| state.is_running())
            .filter(|(id, _)| wanted.is_empty() || id.contains(&wanted))
            .max_by_key(|(_, state)| state.since());
        let Some((task_id, _)) = candidate else {
            let message = if wanted.is_empty() {
                "No subagent is running.".to_string()
            } else {
                format!("No running subagent matches \"{wanted}\".")
            };
            self.show_status(&message);
            return;
        };
        if pool.cancel(task_id) {
            self.show_status(&format!("Cancelling subagent {task_id}…"));
        }
    }

    /// `/subagent-retry [agent] [task]`: dispatch again, on the same inputs.
    ///
    /// A transient provider failure — a region rejection, a dead stream, a
    /// deadline — should not cost the work. With no arguments this re-runs the
    /// most recent failed attempt from the ledger, which is the only record of
    /// what it was actually asked to do.
    pub(super) fn retry_newest_subagent(&mut self, text: &str) {
        let args = text
            .strip_prefix("/subagent-retry")
            .map_or("", str::trim)
            .to_string();
        let cwd = self.session.cwd().to_path_buf();
        let last_failed = hoocode_code_subagents::ledger::recent(&cwd, 200)
            .into_iter()
            .rfind(|attempt| !attempt.ok);
        let (forced_agent, task_text) = match args.split_once(' ') {
            Some((agent, rest)) if !agent.is_empty() => {
                (Some(agent.trim().to_string()), rest.trim().to_string())
            }
            Some(_) | None => (None, args.trim().to_string()),
        };
        let agent_type = forced_agent
            .or_else(|| last_failed.as_ref().map(|a| a.agent_type.clone()))
            .unwrap_or_else(|| "explore".into());
        let prompt = if task_text.is_empty() {
            last_failed
                .as_ref()
                .map(|a| {
                    format!(
                        "Retry {}: it failed with {}",
                        a.task_id,
                        a.error.as_deref().unwrap_or("no cause recorded")
                    )
                })
                .unwrap_or_else(|| {
                    "Describe the repository layout and report what is in it.".into()
                })
        } else {
            task_text
        };
        let known: Vec<String> = load_agent_registry(&LoadAgentRegistryOptions::new(
            cwd.to_string_lossy().into_owned(),
        ))
        .list()
        .iter()
        .map(|agent| agent.name.clone())
        .collect();
        if !known.contains(&agent_type) {
            self.show_error(&format!(
                "Unknown subagent type: {agent_type}. Available: {}",
                known.join(", ")
            ));
            return;
        }
        self.show_status(&format!("Re-dispatching {agent_type}…"));
        let models = self.session.get_available_models();
        let model = self.session.model();
        let options = DispatchOptions {
            force_agent: Some(agent_type.clone()),
            model: model.as_ref().map(|m| m.id.clone()),
            provider: model.as_ref().map(|m| m.provider.clone()),
            ..Default::default()
        };
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let pool = get_subagent_pool(&cwd, &models);
            let outcome = pool.dispatch(&prompt, options).await;
            let text = match outcome {
                Ok(dispatched) => match dispatched.result {
                    Some(result) if result.ok => Ok(result
                        .result_data
                        .as_ref()
                        .and_then(|data| data.get("summary"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)),
                    other => Err(format!(
                        "retry failed: {}",
                        other
                            .and_then(|r| r.error)
                            .unwrap_or_else(|| "unknown error".into())
                    )),
                },
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(AppEvent::SubagentDone(agent_type, text));
        });
    }

    /// `/subagent-stats [24h|7d|all]`: what the dispatch ledger says about
    /// reliability. Read from `<cwd>/.hoocode/dispatch/ledger.jsonl` — one
    /// line per attempt — instead of from whatever dispatch dirs survived on
    /// disk, which is the only evidence there was before the ledger.
    pub(super) fn handle_subagent_stats_command(&mut self, text: &str) {
        let arg = text
            .strip_prefix("/subagent-stats")
            .map_or("", str::trim)
            .split_whitespace()
            .next()
            .unwrap_or("24h")
            .to_string();
        let window_ms = match arg.as_str() {
            "24h" | "day" => 24 * 60 * 60 * 1000,
            "7d" | "week" => 7 * 24 * 60 * 60 * 1000,
            "all" | "*" => 0,
            other => {
                self.show_status(&format!(
                    "Unknown window \"{other}\". Usage: /subagent-stats [24h|7d|all]"
                ));
                return;
            }
        };
        let label = match window_ms {
            0 => "all retained attempts",
            _ => "last recorded attempts",
        };
        let now = hoocode_ai_types::now_ms().max(0) as u64;
        let since = (window_ms > 0).then(|| now.saturating_sub(window_ms));
        let cwd = self.session.cwd().to_path_buf();
        let stats = ledger::stats(&cwd, since);
        if stats.attempts == 0 {
            self.show_status(&format!(
                "No subagent attempts in {label}. Ledger: {}",
                ledger::ledger_path(&cwd).display()
            ));
            return;
        }
        let t = theme();
        let dim = |s: &str| t.fg("dim", s);
        let secs = |ms: u64| {
            if ms < 60_000 {
                format!("{}s", ms / 1000)
            } else if ms < 3_600_000 {
                format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000)
            } else {
                format!("{}h{:02}m", ms / 3_600_000, (ms % 3_600_000) / 60_000)
            }
        };
        let mut info = format!("{}\n\n", t.bold("Subagent reliability"));
        info += &format!(
            "{} {} attempts, {} usable ({:.0}%)\n",
            dim("Window:"),
            label,
            stats.usable,
            ledger::success_rate(&stats)
        );
        info += &format!(
            "{} complete {} · partial {} · failed {} · timeout {} · stalled {} · cancelled {}\n",
            dim("Status:"),
            stats.complete,
            stats.partial,
            stats.failed,
            stats.timeout,
            stats.stalled,
            stats.cancelled,
        );
        if stats.other > 0 {
            info += &format!("{} {} unknown\n", dim("Status:"), stats.other);
        }
        info += &format!(
            "{} median {} · p90 {} · max {}\n",
            dim("Wall clock:"),
            secs(stats.median_ms),
            secs(stats.p90_ms),
            secs(stats.max_ms),
        );
        info += &format!(
            "{} {} generated\n",
            dim("Tokens:"),
            group_digits(stats.tokens_generated)
        );
        info += &format!(
            "{} {} attempt(s) on the inherited model\n",
            dim("Fallbacks:"),
            stats.fallback_attempts
        );
        if !stats.by_agent.is_empty() {
            info += &format!("\n{}\n", t.bold("By agent"));
            for (agent, agent_stats) in &stats.by_agent {
                // The mean, not a median: per-agent percentiles would need
                // their own pass and the average is enough to spot an outlier.
                let mean_ms = agent_stats.wall_ms / agent_stats.attempts.max(1) as u64;
                info += &format!(
                    "  {:<16} {} attempt(s) - {} usable - {} avg\n",
                    agent,
                    agent_stats.attempts,
                    agent_stats.usable,
                    secs(mean_ms),
                );
            }
        }
        let failures: Vec<String> = ledger::recent(&cwd, 500)
            .into_iter()
            .filter(|a| since.is_none_or(|s| a.ts >= s) && !a.ok)
            .rev()
            .take(5)
            .map(|a| {
                format!(
                    "  {} · {} · {} · {}{}",
                    a.agent_type,
                    if a.status.is_empty() { "?" } else { &a.status },
                    secs(a.duration_ms),
                    a.task_id,
                    a.error.map(|e| format!(" — {e}")).unwrap_or_default()
                )
            })
            .collect();
        if !failures.is_empty() {
            info += &format!("\n{}\n", t.bold("Recent failures"));
            for line in failures {
                info += &format!("{line}\n");
            }
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(info.trim_end(), 1, 0))));
    }

    /// A `/subagent` run ended. Its answer joins the session as a displayed
    /// custom message (seen when the transcript is next drawn).
    pub(super) fn finish_subagent(&mut self, mode: &str, result: Result<Option<String>, String>) {
        match result {
            Ok(summary) => {
                self.show_status(&format!("{mode} subagent completed"));
                let summary = summary
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "(no output)".into());
                self.session
                    .session_manager()
                    .append_message(AgentMessage::Custom(CustomMessage {
                        custom_type: "subagent".into(),
                        content: UserContent::Text(summary),
                        display: true,
                        details: None,
                        timestamp: hoocode_ai_types::now_ms(),
                    }));
            }
            Err(error) => self.show_error(&error),
        }
    }
}
