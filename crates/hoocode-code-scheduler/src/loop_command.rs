//! `/loop`: what its arguments mean, and the state of `/loop auto`. Nothing
//! here touches the UI or the store; the TUI app carries the command out.
//!
//! Forms:
//!
//! ```text
//! /loop "<cron>" <prompt>          recurring, at a 5-field cron (local time)
//! /loop 5m|2h|1d <prompt>          recurring, at an interval
//! /loop once <schedule> <prompt>   one fire, then the task is removed
//! /loop list
//! /loop delete <id>
//! /loop stop
//! /loop auto [--max-turns N] <task>
//! ```

use crate::cron::validate as validate_cron;

/// The reply token that ends `/loop auto`. The same token as
/// `AUTO_LOOP_DONE_TOKEN` in hoocode-code-modes.
pub const DONE_TOKEN: &str = "LOOP_DONE";
/// The turn cap of `/loop auto` without `--max-turns`.
pub const DEFAULT_MAX_TURNS: u64 = 10;

/// The usage line, shown for an argument that does not parse.
pub const USAGE: &str = "Usage: /loop \"<cron>\" <prompt> | /loop <5m|2h|1d> <prompt> | /loop once <schedule> <prompt> | /loop list | /loop delete <id> | /loop stop | /loop auto [--max-turns N] <task>";

/// A parsed `/loop` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopCommand {
    /// Create a scheduled task.
    Create { schedule: Schedule, prompt: String },
    /// Show the scheduled tasks.
    List,
    /// Remove one scheduled task.
    Delete(String),
    /// End `/loop auto` and remove every scheduled task.
    Stop,
    /// Run `task` until it is done or the turn cap is reached.
    Auto { max_turns: u64, task: String },
}

/// When a task fires: a 5-field cron expression, repeated or once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub cron: String,
    pub recurring: bool,
}

/// Parse the text after `/loop`. An `Err` is the message to show.
pub fn parse_loop_args(args: &str) -> Result<LoopCommand, String> {
    let (head, rest) = split_word(args);
    match head {
        "" => Err(USAGE.into()),
        "list" if rest.is_empty() => Ok(LoopCommand::List),
        "stop" if rest.is_empty() => Ok(LoopCommand::Stop),
        "delete" => {
            let (id, extra) = split_word(rest);
            if id.is_empty() || !extra.is_empty() {
                return Err("Usage: /loop delete <id>".into());
            }
            Ok(LoopCommand::Delete(id.into()))
        }
        "auto" => parse_auto(rest),
        "once" => {
            let (cron, prompt) = parse_schedule(rest)?;
            Ok(LoopCommand::Create {
                schedule: Schedule {
                    cron,
                    recurring: false,
                },
                prompt,
            })
        }
        _ => {
            let (cron, prompt) = parse_schedule(args)?;
            Ok(LoopCommand::Create {
                schedule: Schedule {
                    cron,
                    recurring: true,
                },
                prompt,
            })
        }
    }
}

/// `"<cron>" <prompt>` or `<N>m|<N>h|<N>d <prompt>`: the cron expression and the
/// prompt that follows it.
fn parse_schedule(text: &str) -> Result<(String, String), String> {
    let text = text.trim_start();
    let (cron, rest) = match text.strip_prefix('"') {
        Some(quoted) => {
            let Some((cron, rest)) = quoted.split_once('"') else {
                return Err("The cron expression needs a closing \".".into());
            };
            let cron = cron.trim();
            validate_cron(cron)?;
            (cron.to_string(), rest.trim())
        }
        None => {
            let (token, rest) = split_word(text);
            match interval_cron(token) {
                Some(cron) => (cron?, rest),
                None => return Err(USAGE.into()),
            }
        }
    };
    if rest.is_empty() {
        return Err("Missing the prompt to run.".into());
    }
    Ok((cron, rest.to_string()))
}

/// The cron expression for an interval token: `5m`, `2h` or `1d`. `None` when
/// the token is not an interval at all.
fn interval_cron(token: &str) -> Option<Result<String, String>> {
    let unit = token.chars().last()?;
    let digits = token.strip_suffix(unit)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let even = |span: u64, step: u64| span.is_multiple_of(step);
    Some(match unit {
        'm' if (1..=59).contains(&n) && even(60, n) => Ok(format!("*/{n} * * * *")),
        'h' if (1..=23).contains(&n) && even(24, n) => Ok(format!("0 */{n} * * *")),
        'd' if (1..=31).contains(&n) => Ok(format!("0 0 */{n} * *")),
        'm' | 'h' => Err(format!(
            "The interval {token} does not divide evenly; use a cron expression, e.g. \"*/10 * * * *\"."
        )),
        _ => return None,
    })
}

/// `/loop auto [--max-turns N] <task>`.
fn parse_auto(rest: &str) -> Result<LoopCommand, String> {
    let mut max_turns = DEFAULT_MAX_TURNS;
    let mut task = rest.trim();
    if let Some(after) = task.strip_prefix("--max-turns") {
        let (value, remaining) = split_word(after);
        max_turns = value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| "--max-turns needs a whole number of at least 1.".to_string())?;
        task = remaining;
    }
    if task.is_empty() {
        return Err("Usage: /loop auto [--max-turns N] <task>".into());
    }
    Ok(LoopCommand::Auto {
        max_turns,
        task: task.into(),
    })
}

/// The first whitespace-separated word, and the trimmed rest.
fn split_word(text: &str) -> (&str, &str) {
    match text.trim().split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (text.trim(), ""),
    }
}

/// The first prompt of `/loop auto`.
pub fn auto_task_prompt(task: &str) -> String {
    format!("Work autonomously toward this task:\n\n{task}")
}

/// The prompt sent after each turn of `/loop auto` that did not end the loop.
pub fn auto_continue_prompt(task: &str) -> String {
    format!(
        "Keep working toward the task:\n\n{task}\n\nReply with {DONE_TOKEN} only when it is genuinely complete — not when it is merely close."
    )
}

/// How a `/loop auto` run ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoEnd {
    /// A reply contained [`DONE_TOKEN`].
    Done,
    /// The turn cap was reached without [`DONE_TOKEN`].
    TurnCap,
    /// An `AskUserQuestion` had no recommended answer; the reason says which.
    Halted(String),
    /// A turn was aborted or failed.
    Interrupted,
    /// `/loop stop`.
    Stopped,
}

/// What the loop does after a turn ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoNext {
    /// Send this prompt as the next turn.
    Prompt(String),
    /// End the loop.
    Stop(AutoEnd),
}

/// One `/loop auto` run: counts turns and watches the replies for [`DONE_TOKEN`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoLoop {
    task: String,
    max_turns: u64,
    turns: u64,
    reply_done: bool,
}

impl AutoLoop {
    /// Start a run. Returns its state and the first prompt, which is turn 1.
    pub fn start(task: &str, max_turns: u64) -> (Self, String) {
        let state = Self {
            task: task.to_string(),
            max_turns: max_turns.max(1),
            turns: 1,
            reply_done: false,
        };
        (state, auto_task_prompt(task))
    }

    /// Turns started so far, the first prompt included.
    pub fn turns(&self) -> u64 {
        self.turns
    }

    /// The turn cap.
    pub fn max_turns(&self) -> u64 {
        self.max_turns
    }

    /// Note the text of an assistant message of the current turn.
    pub fn observe_reply(&mut self, text: &str) {
        if text.contains(DONE_TOKEN) {
            self.reply_done = true;
        }
    }

    /// The current turn has ended without an error: say what comes next.
    /// [`DONE_TOKEN`] ends the loop first; then the cap.
    pub fn turn_ended(&mut self) -> AutoNext {
        if std::mem::take(&mut self.reply_done) {
            return AutoNext::Stop(AutoEnd::Done);
        }
        if self.turns >= self.max_turns {
            return AutoNext::Stop(AutoEnd::TurnCap);
        }
        self.turns += 1;
        AutoNext::Prompt(auto_continue_prompt(&self.task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(cron: &str, prompt: &str, recurring: bool) -> LoopCommand {
        LoopCommand::Create {
            schedule: Schedule {
                cron: cron.into(),
                recurring,
            },
            prompt: prompt.into(),
        }
    }

    #[test]
    fn a_quoted_cron_expression_then_the_prompt_recurs() {
        assert_eq!(
            parse_loop_args(r#""*/10 * * * *" check the build"#),
            Ok(create("*/10 * * * *", "check the build", true))
        );
        // The prompt keeps its inner spacing.
        assert_eq!(
            parse_loop_args(r#"  "0 9 * * 1-5"   say  good   morning  "#),
            Ok(create("0 9 * * 1-5", "say  good   morning", true))
        );
    }

    #[test]
    fn a_quoted_cron_must_have_five_fields_and_be_closed() {
        assert!(parse_loop_args(r#""* * * *" x"#)
            .unwrap_err()
            .contains("5 fields"));
        assert!(parse_loop_args(r#""*/5 * * * * *" x"#).is_err());
        assert!(parse_loop_args(r#""*/5 * * * * x"#)
            .unwrap_err()
            .contains("closing"));
        assert!(parse_loop_args(r#""*/5 * * * *""#)
            .unwrap_err()
            .contains("Missing the prompt"));
    }

    #[test]
    fn a_quoted_cron_whose_fields_cannot_match_is_refused() {
        // Such a task would be stored and never fire.
        assert!(parse_loop_args(r#""60 * * * *" x"#)
            .unwrap_err()
            .contains("minute field"));
        assert!(parse_loop_args(r#""0 9 * * foo" x"#).is_err());
        assert!(parse_loop_args(r#""*/0 * * * *" x"#).is_err());
        assert!(parse_loop_args(r#""x * * * *" x"#).is_err());
        assert!(parse_loop_args(r#""0 9 * * 1-7" x"#).is_ok());
    }

    #[test]
    fn an_interval_becomes_a_cron_expression() {
        assert_eq!(
            parse_loop_args("5m check the build"),
            Ok(create("*/5 * * * *", "check the build", true))
        );
        assert_eq!(
            parse_loop_args("2h water the plants"),
            Ok(create("0 */2 * * *", "water the plants", true))
        );
        assert_eq!(
            parse_loop_args("1d weekly digest"),
            Ok(create("0 0 */1 * *", "weekly digest", true))
        );
        assert_eq!(
            parse_loop_args("1m tick"),
            Ok(create("*/1 * * * *", "tick", true))
        );
    }

    #[test]
    fn an_interval_must_be_in_range_and_even() {
        assert!(parse_loop_args("0m x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("7m x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("60m x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("4h x").is_ok());
        assert!(parse_loop_args("5h x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("7h x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("24h x")
            .unwrap_err()
            .contains("does not divide"));
        assert!(parse_loop_args("0d x").is_err());
        assert!(parse_loop_args("32d x").is_err());
    }

    #[test]
    fn once_makes_a_one_shot_task_with_either_schedule_form() {
        assert_eq!(
            parse_loop_args("once 30m remind me"),
            Ok(create("*/30 * * * *", "remind me", false))
        );
        assert_eq!(
            parse_loop_args("once 5m remind me"),
            Ok(create("*/5 * * * *", "remind me", false))
        );
        assert_eq!(
            parse_loop_args("once soon remind me"),
            Err(USAGE.to_string())
        );
        assert_eq!(
            parse_loop_args(r#"once "0 17 * * *" close the tab"#),
            Ok(create("0 17 * * *", "close the tab", false))
        );
        assert!(parse_loop_args("once").is_err());
        assert!(parse_loop_args("once 5m").unwrap_err().contains("prompt"));
    }

    #[test]
    fn a_bare_word_that_is_not_a_form_is_a_usage_error() {
        assert_eq!(parse_loop_args(""), Err(USAGE.to_string()));
        assert_eq!(parse_loop_args("   "), Err(USAGE.to_string()));
        assert_eq!(parse_loop_args("soon do it"), Err(USAGE.to_string()));
        assert_eq!(parse_loop_args("5 do it"), Err(USAGE.to_string()));
    }

    #[test]
    fn list_delete_and_stop_take_their_arguments() {
        assert_eq!(parse_loop_args("list"), Ok(LoopCommand::List));
        assert_eq!(parse_loop_args("  list  "), Ok(LoopCommand::List));
        assert!(parse_loop_args("list all").is_err());
        assert_eq!(
            parse_loop_args("delete task_abc1"),
            Ok(LoopCommand::Delete("task_abc1".into()))
        );
        assert!(parse_loop_args("delete").unwrap_err().contains("<id>"));
        assert!(parse_loop_args("delete a b").unwrap_err().contains("<id>"));
        assert_eq!(parse_loop_args("stop"), Ok(LoopCommand::Stop));
        assert!(parse_loop_args("stop now").is_err());
    }

    #[test]
    fn auto_takes_an_optional_turn_cap_then_the_task() {
        assert_eq!(
            parse_loop_args("auto fix the flaky test"),
            Ok(LoopCommand::Auto {
                max_turns: DEFAULT_MAX_TURNS,
                task: "fix the flaky test".into(),
            })
        );
        assert_eq!(
            parse_loop_args("auto --max-turns 3 fix the flaky test"),
            Ok(LoopCommand::Auto {
                max_turns: 3,
                task: "fix the flaky test".into(),
            })
        );
        assert_eq!(DEFAULT_MAX_TURNS, 10);
    }

    #[test]
    fn auto_rejects_a_bad_cap_or_no_task() {
        for bad in [
            "auto",
            "auto --max-turns 3",
            "auto --max-turns",
            "auto --max-turns 0 task",
            "auto --max-turns -2 task",
            "auto --max-turns many task",
        ] {
            assert!(parse_loop_args(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_done_token_ends_a_run_on_the_turn_that_says_it() {
        let (mut run, first) = AutoLoop::start("write the report", 10);
        assert!(first.contains("write the report"));
        assert_eq!(run.turns(), 1);
        assert!(matches!(run.turn_ended(), AutoNext::Prompt(p) if p.contains(DONE_TOKEN)));
        assert_eq!(run.turns(), 2);
        run.observe_reply("Still drafting the conclusion.");
        assert!(matches!(run.turn_ended(), AutoNext::Prompt(_)));
        run.observe_reply("The report is complete. LOOP_DONE");
        assert_eq!(run.turn_ended(), AutoNext::Stop(AutoEnd::Done));
        assert_eq!(run.turns(), 3);
    }

    #[test]
    fn the_done_token_must_be_in_the_reply_of_the_turn_that_ended() {
        let (mut run, _) = AutoLoop::start("task", 10);
        run.observe_reply("LOOP_DONE");
        // Consumed by the turn it was seen in.
        assert_eq!(run.turn_ended(), AutoNext::Stop(AutoEnd::Done));
        let (mut run, _) = AutoLoop::start("task", 10);
        run.observe_reply("not yet, LOOP_DON");
        assert!(matches!(run.turn_ended(), AutoNext::Prompt(_)));
    }

    #[test]
    fn the_cap_stops_a_run_after_exactly_max_turns() {
        let (mut run, _) = AutoLoop::start("never done", DEFAULT_MAX_TURNS);
        let mut prompts = 1;
        loop {
            match run.turn_ended() {
                AutoNext::Prompt(_) => prompts += 1,
                AutoNext::Stop(end) => {
                    assert_eq!(end, AutoEnd::TurnCap);
                    break;
                }
            }
        }
        assert_eq!(prompts, 10, "ten turns in all, the first prompt included");
        assert_eq!(run.turns(), 10);
    }

    #[test]
    fn a_cap_of_one_runs_the_task_once() {
        let (mut run, _) = AutoLoop::start("quick check", 1);
        assert_eq!(run.turn_ended(), AutoNext::Stop(AutoEnd::TurnCap));
    }

    #[test]
    fn done_on_the_last_turn_is_done_not_the_cap() {
        let (mut run, _) = AutoLoop::start("task", 2);
        assert!(matches!(run.turn_ended(), AutoNext::Prompt(_)));
        run.observe_reply("LOOP_DONE");
        assert_eq!(run.turn_ended(), AutoNext::Stop(AutoEnd::Done));
    }
}
