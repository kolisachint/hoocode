//! `SessionSpawn`: hands the work to a fresh, detached hoocode session (SA-12).
//!
//! The model passes a brief with four sections. The harness adds the facts of
//! the checkout, starts `hoocode` on a new session file, and returns at once:
//! the parent gets the command that opens the child, never its result. The
//! child runs unattended with the subagent limits, flushes every entry, and
//! records how it ended in a `detached_exit` entry.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use hoocode_agent_types::{AgentMessage, AgentToolResult};
use hoocode_ai_types::{Content, StopReason, TextContent};
use hoocode_code_session::{SessionManager, SESSION_FLUSH_DEADLINE};
use hoocode_code_subagents::depth::SUBAGENT_DEPTH_ENV;
use hoocode_code_subagents::lifeguard::SUBAGENT_DEADLINE_MS;
use hoocode_code_subagents::DEFAULT_SUBAGENT_MAX_TURNS;
use hoocode_code_tool_api::{ToolContext, ToolDefinition, ToolError, ToolOrigin};
use serde_json::{json, Value};

/// The model-facing name of the tool.
pub const SESSION_SPAWN_TOOL: &str = "SessionSpawn";

/// Set in the environment of every detached run (and read by the run itself).
pub const DETACHED_ENV: &str = "HOOCODE_DETACHED";

/// The section names a brief must have, in the order the errors name them.
pub const SECTIONS: [&str; 4] = ["Goal", "Decisions", "Dead ends", "Next step"];

/// The longest goal line used as the child's session name.
const GOAL_MAX_CHARS: usize = 60;

/// Dirty-file lines listed in the facts before the "… and N more" line.
const DIRTY_LIMIT: usize = 50;

/// The task-identity variables the subagent machinery sets on its children. A
/// detached session is a root run, so it must not inherit them.
const AGENT_IDENTITY_ENV: [&str; 4] = [
    "HOOCODE_AGENT_ID",
    "HOOCODE_AGENT_TOKEN",
    "HOOCODE_AGENT_PARENT",
    "HOOCODE_AGENT_DEPTH",
];

/// Prefix of the tier C helper socket variables, which a child must not inherit.
const HELPER_SOCKET_PREFIX: &str = "HOOCODE_HELPER_SOCKET_";

/// Whether this process is a detached run.
pub fn is_detached_run() -> bool {
    std::env::var_os(DETACHED_ENV).is_some()
}

/// Whether `SessionSpawn` is offered: never inside a detached run, never in a
/// subagent tree (depth above 0), never for a task child.
pub fn session_spawn_registered(detached: bool, subagent_depth: u32, task_child: bool) -> bool {
    !detached && subagent_depth == 0 && !task_child
}

/// Why a brief is not usable. Its message is what the model sees.
pub fn brief_error(brief: &str) -> Result<(), String> {
    let missing: Vec<&str> = SECTIONS
        .iter()
        .enumerate()
        .filter(|(index, _)| find_section(brief, *index).is_none())
        .map(|(_, name)| *name)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "SessionSpawn: the brief is missing: {}",
            missing.join(", ")
        ))
    }
}

/// The goal line of a brief: the text after `Goal:`, else the next non-empty
/// line after the Goal marker, trimmed and capped at 60 characters. Empty when
/// the Goal section has no text.
pub fn goal_line(brief: &str) -> String {
    let Some((line_index, rest)) = find_section(brief, 0) else {
        return String::new();
    };
    if !rest.is_empty() {
        return cap_chars(&rest);
    }
    for line in brief.lines().skip(line_index + 1) {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if section_marker(text).is_some() {
            return String::new();
        }
        return cap_chars(text);
    }
    String::new()
}

fn cap_chars(text: &str) -> String {
    text.trim()
        .chars()
        .take(GOAL_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The first line that marks section `index`: its line number and the text
/// after the marker on that line (empty for a bare heading).
fn find_section(brief: &str, index: usize) -> Option<(usize, String)> {
    brief.lines().enumerate().find_map(|(line_index, line)| {
        let (found, rest) = section_marker(line.trim())?;
        (found == index).then_some((line_index, rest))
    })
}

/// Which section a line marks, with the text after the marker. A markdown
/// heading (`#`..`######`) marks a section by its text (`## Dead ends`, or
/// `## Dead ends:`); any other line marks one only as `<Name>:`. Case-insensitive.
fn section_marker(line: &str) -> Option<(usize, String)> {
    let (body, is_heading) = match line.strip_prefix('#') {
        Some(_) => {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            let after = &line[hashes..];
            let heading = (1..=6).contains(&hashes) && after.starts_with([' ', '\t']);
            if !heading {
                return None;
            }
            (after.trim(), true)
        }
        None => (line, false),
    };
    SECTIONS.iter().enumerate().find_map(|(index, name)| {
        let head = body.get(..name.len())?;
        if !head.eq_ignore_ascii_case(name) {
            return None;
        }
        let after = &body[name.len()..];
        if let Some(rest) = after.strip_prefix(':') {
            return Some((index, rest.trim().to_string()));
        }
        (is_heading && after.trim().is_empty()).then(|| (index, String::new()))
    })
}

/// What the harness knows about the checkout, taken from git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitState {
    /// The branch, or `(detached)`.
    pub branch: String,
    /// The short commit id of HEAD, when there is one.
    pub head: Option<String>,
    /// `git status --porcelain` lines.
    pub dirty: Vec<String>,
}

/// The git state of `cwd`; `None` when it is not a work tree (or git fails).
pub fn gather_git(cwd: &Path) -> Option<GitState> {
    let inside = git_output(cwd, &["rev-parse", "--is-inside-work-tree"])?;
    if inside.trim() != "true" {
        return None;
    }
    let branch = git_output(cwd, &["symbolic-ref", "--short", "-q", "HEAD"])
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "(detached)".to_string());
    let head = git_output(cwd, &["rev-parse", "--short", "HEAD"])
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    let dirty = git_output(cwd, &["status", "--porcelain"])
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    Some(GitState {
        branch,
        head,
        dirty,
    })
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The `## Facts (added by hoocode)` block. Pure: the git state and the
/// parent session are passed in.
pub fn format_facts(cwd: &str, git: Option<&GitState>, parent: Option<&str>) -> String {
    let mut lines = vec![
        "## Facts (added by hoocode)".to_string(),
        format!("cwd: {cwd}"),
    ];
    match git {
        None => lines.push("branch: none".to_string()),
        Some(git) => {
            lines.push(format!("branch: {}", git.branch));
            if let Some(head) = &git.head {
                lines.push(format!("HEAD: {head}"));
            }
            if git.dirty.is_empty() {
                lines.push("dirty files: none".to_string());
            } else {
                lines.push("dirty files:".to_string());
                for line in git.dirty.iter().take(DIRTY_LIMIT) {
                    lines.push(format!("  {line}"));
                }
                if git.dirty.len() > DIRTY_LIMIT {
                    lines.push(format!("  … and {} more", git.dirty.len() - DIRTY_LIMIT));
                }
            }
        }
    }
    lines.push(format!("parent session: {}", parent.unwrap_or("none")));
    lines.join("\n")
}

/// The child's first prompt: the brief and the facts, with what to do with them.
pub fn child_prompt(brief: &str, facts: &str) -> String {
    format!(
        "You are continuing work handed over from another hoocode session. The brief below was approved by the user. Do the Next step; do not redo what Decisions settled or retry anything under Dead ends.\n\n{}\n\n{facts}",
        brief.trim()
    )
}

/// The command that runs the child: `-p <prompt> --session <path>` with the
/// subagent turn and deadline limits, and the parent's model when it has one.
/// The child gets `HOOCODE_DETACHED=1`, no subagent depth and no task identity,
/// and no stdin. Its stdio is set by [`spawn_detached`].
pub fn child_command(
    exe: &Path,
    session: &Path,
    prompt: &str,
    cwd: &Path,
    model: Option<(&str, &str)>,
) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("-p")
        .arg(prompt)
        .arg("--session")
        .arg(session)
        .arg("--max-turns")
        .arg(DEFAULT_SUBAGENT_MAX_TURNS.to_string())
        .arg("--deadline-ms")
        .arg(SUBAGENT_DEADLINE_MS.to_string());
    if let Some((provider, id)) = model {
        command
            .arg("--provider")
            .arg(provider)
            .arg("--model")
            .arg(id);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env(DETACHED_ENV, "1");
    for prefix in hoocode_code_paths::ENV_PREFIXES {
        command.env_remove(format!("{prefix}{SUBAGENT_DEPTH_ENV}"));
    }
    for name in AGENT_IDENTITY_ENV {
        command.env_remove(name);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with(HELPER_SOCKET_PREFIX) {
            command.env_remove(key);
        }
    }
    command
}

/// The log file of a detached session: `<session file>.log`.
pub fn log_path_for(session: &Path) -> PathBuf {
    let mut name = session.as_os_str().to_owned();
    name.push(".log");
    PathBuf::from(name)
}

/// Starts `command` detached (its own session, `SIGHUP` ignored) with stdout and
/// stderr appended to `log`. The child is not killed when this process exits.
pub fn spawn_detached(command: &mut Command, log: &Path) -> std::io::Result<Child> {
    let file = OpenOptions::new().create(true).append(true).open(log)?;
    let copy = file.try_clone()?;
    command.stdout(Stdio::from(file)).stderr(Stdio::from(copy));
    hoocode_runtime::spawn_detached_session(command);
    command.spawn()
}

/// Creates the child's session file in `dir` and records its parent, name and
/// goal. The file exists when this returns; the manager (and its lock) is dropped.
pub fn create_child_session(
    dir: &Path,
    cwd: &Path,
    parent: Option<&Path>,
    goal: &str,
) -> Result<PathBuf, String> {
    let mut manager = SessionManager::create(cwd.display().to_string(), Some(dir.to_path_buf()));
    let path = manager
        .session_file()
        .map(Path::to_path_buf)
        .ok_or("SessionSpawn: could not name the new session file")?;
    manager.set_flush_each_entry(true);
    if let Some(parent) = parent {
        manager.set_parent_session(parent.display().to_string());
    }
    manager.append_auto_session_name(goal);
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .map_err(|e| format!("SessionSpawn: could not write the new session: {e:?}"))?;
    drop(manager);
    if !path.is_file() {
        return Err(format!(
            "SessionSpawn: the new session file was not written: {}",
            path.display()
        ));
    }
    Ok(path)
}

/// The `detached_exit` record of a run that just ended: failed when the run
/// returned an error or its last message stopped with one, else done.
pub fn detached_exit_data(run_error: Option<&str>, messages: &[AgentMessage]) -> Value {
    if let Some(error) = run_error {
        return json!({"status": "failed", "reason": error});
    }
    if let Some(AgentMessage::Assistant(message)) = messages.last() {
        if message.stop_reason == StopReason::Error {
            let reason = message
                .error_message
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "error".to_string());
            return json!({"status": "failed", "reason": reason});
        }
    }
    json!({"status": "done", "reason": null})
}

/// The `SessionSpawn` tool definition. `execute` makes the child and returns the
/// command that opens it.
pub fn create_session_spawn_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: SESSION_SPAWN_TOOL.into(),
        origin: ToolOrigin::BuiltIn,
        render: None,
        label: SESSION_SPAWN_TOOL.into(),
        description: [
            "Hands the work to a fresh session with its own context. That session runs detached: it outlives this one and returns nothing here.",
            "Use it when the user asks for a handover, or when the remaining work is a clean next step and this context is long.",
            "The brief must have the sections Goal, Decisions, Dead ends, Next step.",
            "After the call, stop working on that task.",
        ]
        .join("\n"),
        prompt_snippet: Some(
            "Hand a clean next step to a fresh, detached session (returns nothing here)".into(),
        ),
        prompt_guidelines: Vec::new(),
        parameters: json!({
            "type": "object",
            "required": ["brief"],
            "properties": {
                "brief": {
                    "type": "string",
                    "description": "Markdown with the sections Goal, Decisions, Dead ends and Next step.",
                }
            },
        }),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        background_placeholder: None,
        ordered_start: false,
        abort_grace: None,
        execute: Arc::new(move |_id, args, _signal, _on_update, ctx| {
            run_session_spawn(&args, ctx)
        }),
    }
}

/// The tool's `execute`: validates the brief, creates the child session and
/// starts it. The result names the session; the child's outcome is not awaited.
fn run_session_spawn(
    args: &Value,
    ctx: Option<&ToolContext>,
) -> Result<AgentToolResult, ToolError> {
    let brief = args
        .get("brief")
        .and_then(Value::as_str)
        .ok_or("SessionSpawn: the brief is required")?;
    brief_error(brief)?;
    let goal = goal_line(brief);
    if goal.is_empty() {
        return Err("SessionSpawn: the Goal section is empty".into());
    }
    let cwd = match ctx.and_then(|c| c.cwd.clone()) {
        Some(cwd) => cwd,
        None => std::env::current_dir()?,
    };
    let parent = ctx.and_then(|c| c.session_file.clone()).map(|p| {
        if p.is_absolute() {
            p.clone()
        } else {
            cwd.join(p)
        }
    });
    let dir = match parent.as_deref().and_then(Path::parent) {
        Some(dir) => dir.to_path_buf(),
        None => hoocode_code_session::default_session_dir(&cwd.display().to_string()),
    };
    let session = create_child_session(&dir, &cwd, parent.as_deref(), &goal)?;

    let facts = format_facts(
        &cwd.display().to_string(),
        gather_git(&cwd).as_ref(),
        parent
            .as_deref()
            .map(|p| p.display().to_string())
            .as_deref(),
    );
    let prompt = child_prompt(brief, &facts);
    let model = ctx.and_then(|c| c.model.as_ref());
    let exe = std::env::current_exe()?;
    let mut command = child_command(
        &exe,
        &session,
        &prompt,
        &cwd,
        model.map(|m| (m.provider.as_str(), m.id.as_str())),
    );
    let child = spawn_detached(&mut command, &log_path_for(&session))?;
    // Reaped off this thread, so the child never lingers as a zombie while
    // this process lives. A dedicated thread, not the blocking pool: a wait
    // for hours must not hold a pool slot, nor delay this process's exit.
    let mut child = child;
    let _ = hoocode_runtime::spawn_named_thread("hoocode-detached-reaper", move || {
        let _ = child.wait();
    });

    let text = format!(
        "Started a detached session: {goal}\n\nOpen it with:\n\n```\nhoocode --session {}\n```\n\nStatus: running. It works on its own and returns nothing here. Stop working on this task.",
        session.display()
    );
    Ok(AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text,
        })],
        details: json!({ "sessionFile": session.display().to_string(), "status": "running" }),
        terminate: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments of `command`, as text.
    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    const BRIEF: &str = "## Goal\nShip the parser fix\n\n## Decisions\n- use nom\n\n## Dead ends\n- regex approach\n\n## Next step\nWrite the tests";

    #[test]
    fn all_sections_present_is_valid() {
        assert_eq!(brief_error(BRIEF), Ok(()));
    }

    #[test]
    fn colon_form_is_accepted() {
        let brief =
            "Goal: fix it\nDecisions: keep the API\nDead ends: none yet\nNext step: test it";
        assert_eq!(brief_error(brief), Ok(()));
    }

    #[test]
    fn section_names_match_case_insensitively_and_with_heading_levels() {
        let brief = "# GOAL\nx\n###### decisions:\ny\n## dead ends\nz\n#### Next step\nw";
        assert_eq!(brief_error(brief), Ok(()));
    }

    #[test]
    fn a_plain_line_without_a_colon_is_not_a_section() {
        let brief = "Goal\nDecisions\nDead ends\nNext step";
        assert_eq!(
            brief_error(brief),
            Err("SessionSpawn: the brief is missing: Goal, Decisions, Dead ends, Next step".into())
        );
    }

    #[test]
    fn hashes_without_a_space_are_not_a_heading() {
        assert_eq!(section_marker("#Goal"), None);
        assert_eq!(section_marker("####### Goal"), None);
    }

    #[test]
    fn missing_sections_are_named_in_order() {
        let brief = "## Next step\nrun\n## Goal\ngo";
        assert_eq!(
            brief_error(brief),
            Err("SessionSpawn: the brief is missing: Decisions, Dead ends".into())
        );
        assert_eq!(
            brief_error("nothing here"),
            Err("SessionSpawn: the brief is missing: Goal, Decisions, Dead ends, Next step".into())
        );
    }

    #[test]
    fn goal_is_the_text_after_the_colon_or_the_next_line() {
        assert_eq!(
            goal_line("Goal: fix the lexer\nDecisions:"),
            "fix the lexer"
        );
        assert_eq!(goal_line(BRIEF), "Ship the parser fix");
        assert_eq!(
            goal_line("## Goal\n\n  spaced out  \n## Decisions"),
            "spaced out"
        );
    }

    #[test]
    fn goal_stops_at_the_next_section() {
        assert_eq!(goal_line("## Goal\n## Decisions\nx"), "");
        assert_eq!(goal_line("nothing"), "");
    }

    #[test]
    fn goal_is_capped_on_a_char_boundary() {
        let long = "é".repeat(80);
        let goal = goal_line(&format!("Goal: {long}\nDecisions:"));
        assert_eq!(goal.chars().count(), GOAL_MAX_CHARS);
        assert!(goal.chars().all(|c| c == 'é'));
    }

    #[test]
    fn facts_inside_a_git_repo_list_branch_head_and_dirty_files() {
        let git = GitState {
            branch: "main".into(),
            head: Some("abc1234".into()),
            dirty: vec![" M src/lib.rs".into(), "?? notes.txt".into()],
        };
        assert_eq!(
            format_facts("/work/repo", Some(&git), Some("/s/parent.jsonl")),
            "## Facts (added by hoocode)\ncwd: /work/repo\nbranch: main\nHEAD: abc1234\ndirty files:\n   M src/lib.rs\n  ?? notes.txt\nparent session: /s/parent.jsonl"
        );
    }

    #[test]
    fn clean_checkout_says_no_dirty_files() {
        let git = GitState {
            branch: "(detached)".into(),
            head: None,
            dirty: vec![],
        };
        let text = format_facts("/w", Some(&git), None);
        assert!(text.contains("branch: (detached)\ndirty files: none\n"));
        assert!(!text.contains("HEAD:"));
        assert!(text.ends_with("parent session: none"));
    }

    #[test]
    fn dirty_files_are_capped_with_a_count() {
        let git = GitState {
            branch: "main".into(),
            head: None,
            dirty: (0..53).map(|i| format!("?? f{i}")).collect(),
        };
        let text = format_facts("/w", Some(&git), None);
        assert!(text.contains("  ?? f49\n  … and 3 more\n"));
        assert!(!text.contains("f50"));
    }

    #[test]
    fn facts_outside_a_repo_say_branch_none_and_nothing_more() {
        let text = format_facts("/tmp/plain", None, None);
        assert_eq!(
            text,
            "## Facts (added by hoocode)\ncwd: /tmp/plain\nbranch: none\nparent session: none"
        );
    }

    #[test]
    fn child_prompt_carries_the_brief_and_the_facts() {
        let prompt = child_prompt("  ## Goal\nx  ", "FACTS");
        assert!(
            prompt.starts_with("You are continuing work handed over from another hoocode session.")
        );
        assert!(prompt.contains("\n\n## Goal\nx\n\nFACTS"));
    }

    #[test]
    fn child_command_runs_a_detached_root_with_the_limits() {
        let exe = Path::new("/bin/hoocode");
        let session = Path::new("/abs/sessions/2026_x.jsonl");
        let cwd = Path::new("/work/repo");
        let command = child_command(exe, session, "PROMPT", cwd, Some(("anthropic", "claude-x")));
        assert_eq!(
            args_of(&command),
            vec![
                "-p",
                "PROMPT",
                "--session",
                "/abs/sessions/2026_x.jsonl",
                "--max-turns",
                "200",
                "--deadline-ms",
                "14400000",
                "--provider",
                "anthropic",
                "--model",
                "claude-x",
            ]
        );
        assert_eq!(command.get_current_dir(), Some(cwd));
        let envs: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let env = |name: &str| {
            envs.iter()
                .find(|(k, _)| k == name)
                .and_then(|(_, v)| v.clone())
        };
        assert_eq!(env(DETACHED_ENV).as_deref(), Some("1"));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "HOOCODE_SUBAGENT_DEPTH" && v.is_none()));
        for name in AGENT_IDENTITY_ENV {
            assert!(envs.iter().any(|(k, v)| k == name && v.is_none()), "{name}");
        }
    }

    #[test]
    fn child_command_without_a_model_has_no_model_flags() {
        let command = child_command(
            Path::new("/bin/h"),
            Path::new("/a.jsonl"),
            "p",
            Path::new("/"),
            None,
        );
        assert!(!args_of(&command).iter().any(|a| a == "--model"));
    }

    #[test]
    fn log_file_sits_beside_the_session_file() {
        assert_eq!(
            log_path_for(Path::new("/s/2026_ab.jsonl")),
            PathBuf::from("/s/2026_ab.jsonl.log")
        );
    }

    #[test]
    fn registration_needs_no_detached_run_no_subagent_tree_and_no_task() {
        assert!(session_spawn_registered(false, 0, false));
        assert!(!session_spawn_registered(true, 0, false));
        assert!(!session_spawn_registered(false, 1, false));
        assert!(!session_spawn_registered(false, 0, true));
    }

    #[test]
    fn detached_exit_is_done_unless_the_run_or_its_last_answer_failed() {
        assert_eq!(
            detached_exit_data(None, &[]),
            json!({"status": "done", "reason": null})
        );
        assert_eq!(
            detached_exit_data(Some("boom"), &[]),
            json!({"status": "failed", "reason": "boom"})
        );
    }

    #[test]
    fn detached_exit_reads_the_last_assistant_error() {
        use hoocode_ai_types::{AssistantMessage, Message};
        let failed = AgentMessage::from_message(Message::Assistant(AssistantMessage {
            content: vec![],
            stop_reason: StopReason::Error,
            error_message: Some("rate limited".into()),
            ..Default::default()
        }));
        assert_eq!(
            detached_exit_data(None, &[failed]),
            json!({"status": "failed", "reason": "rate limited"})
        );
    }
}
