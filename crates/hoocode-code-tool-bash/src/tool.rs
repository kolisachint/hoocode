//! The `bash` tool (`createBashToolDefinition` in core/tools/bash.ts).
//! Interactive rendering (`renderCall`/`renderResult`) arrives with phase 11.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_agent_types::{AgentTool, AgentToolResult, AgentToolUpdateCallback};
use hoocode_ai_types::{Content, TextContent};
use hoocode_code_tool_api::{
    format_size, js_number, wrap_tool_definition, ToolContextFactory, ToolDefinition, ToolError,
    TruncatedBy, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};
use serde_json::{json, Map, Value};

use crate::accumulator::{OutputAccumulator, OutputAccumulatorOptions, OutputSnapshot};
use crate::jobs::JobRegistry;
use crate::operations::{BashExecOptions, BashOperations, LocalBashOperations};
use crate::shell::{get_shell_env, ShellEnv};

/// `BashSpawnContext`: what a spawn hook may adjust.
#[derive(Debug, Clone)]
pub struct BashSpawnContext {
    pub command: String,
    pub cwd: PathBuf,
    pub env: ShellEnv,
}

/// `BashSpawnHook`.
pub type BashSpawnHook = Arc<dyn Fn(BashSpawnContext) -> BashSpawnContext + Send + Sync>;

/// `BashToolOptions`.
#[derive(Clone, Default)]
pub struct BashToolOptions {
    /// How commands run. Default: the local shell.
    pub operations: Option<Arc<dyn BashOperations>>,
    /// Prepended to every command on its own line (the `shellCommandPrefix`
    /// setting).
    pub command_prefix: Option<String>,
    /// Explicit shell (the `shellPath` setting).
    pub shell_path: Option<String>,
    pub spawn_hook: Option<BashSpawnHook>,
    /// Regex patterns; when set, a command must match one of them.
    pub allowed_commands: Vec<String>,
    /// Regex patterns; a matching command is blocked (checked first).
    pub denied_commands: Vec<String>,
    /// Byte cap on the returned output (tail kept). Default 32 KiB.
    pub max_output_bytes: Option<usize>,
    /// Line cap on the returned output (tail kept). Default 800.
    pub max_output_lines: Option<usize>,
    /// `performance.bashNice`: niceness of the commands' child processes. 0
    /// (the default) runs them at normal priority.
    pub nice: u64,
    /// The session's background jobs. When set, `Shell` takes
    /// `run_in_background` and the schema says so; without it (the default)
    /// the schema is unchanged.
    pub jobs: Option<JobRegistry>,
}

const BASH_UPDATE_THROTTLE: Duration = Duration::from_millis(100);

/// Timeout in seconds that a subagent gets when the model omits `timeout` (or
/// passes a negative or non-finite value). A command still running at the
/// deadline has its whole process group killed, and its partial output is
/// returned. The main session has no default limit. An explicit positive
/// `timeout` is honored in both contexts; an explicit `0` means no limit.
pub const DEFAULT_BASH_TIMEOUT_SECS: f64 = 1200.0;

/// Env var that overrides [`DEFAULT_BASH_TIMEOUT_SECS`] in a subagent (a positive number of seconds).
pub const DEFAULT_BASH_TIMEOUT_ENV: &str = "HOOCODE_BASH_DEFAULT_TIMEOUT_SECS";

/// Env var the subagent pool sets in every child (`HOOCODE_SUBAGENT_DEPTH`,
/// see `depth.rs` in `hoocode-code-subagents`). A process with depth 1 or more
/// is a subagent. This crate sits below the subagents crate, so the name is
/// repeated here rather than imported.
pub const SUBAGENT_DEPTH_ENV: &str = "HOOCODE_SUBAGENT_DEPTH";

/// Whether this process is a subagent, from the raw depth variable.
fn is_subagent_context(depth: Option<&str>) -> bool {
    depth
        .and_then(|v| v.trim().parse::<i64>().ok())
        .is_some_and(|n| n > 0)
}

/// The default timeout for this context: `None` (no limit) in the main
/// session; in a subagent, the env override when it is a positive number, else
/// [`DEFAULT_BASH_TIMEOUT_SECS`].
fn default_timeout_for_context(subagent: bool, override_raw: Option<&str>) -> Option<f64> {
    subagent.then(|| parse_default_timeout(override_raw))
}

fn parse_default_timeout(raw: Option<&str>) -> f64 {
    raw.and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|t| t.is_finite() && *t > 0.0)
        .unwrap_or(DEFAULT_BASH_TIMEOUT_SECS)
}

/// Maps the model's `timeout` argument to the limit used for the run.
///
/// Returns `(limit, defaulted)`: `limit` is `None` for no limit, and
/// `defaulted` is true when the limit came from the context default rather
/// than the model. An explicit `0` is unlimited and a positive value is used
/// as given. Omitted, negative and non-finite values get the default, which
/// is no limit when `default_timeout` is `None` (the main session).
fn resolve_timeout(requested: Option<f64>, default_timeout: Option<f64>) -> (Option<f64>, bool) {
    match requested {
        Some(0.0) => (None, false),
        Some(t) if t.is_finite() && t > 0.0 => (Some(t), false),
        _ => match default_timeout {
            Some(secs) => (Some(secs), true),
            None => (None, false),
        },
    }
}

fn text_content(text: impl Into<String>) -> Content {
    Content::Text(TextContent {
        text_signature: None,
        text: text.into(),
    })
}

/// The TypeBox schema hoocode sends for `bash` in the main session (no default timeout).
pub fn bash_parameters_schema() -> Value {
    bash_parameters_schema_with(None)
}

fn bash_parameters_schema_with(default_timeout: Option<f64>) -> Value {
    let timeout_description = match default_timeout {
        Some(secs) => format!(
            "Timeout in seconds (optional; defaults to {}s when omitted; the command is killed at the timeout; 0 means no limit)",
            js_number(secs)
        ),
        None => "Timeout in seconds (optional; 0 means no limit)".to_owned(),
    };
    json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "command": {"type": "string", "description": "Bash command to execute"},
            "timeout": {"type": "number", "description": timeout_description}
        }
    })
}

/// `new RegExp(pattern).test(command)`; an invalid pattern never matches.
fn matches(pattern: &str, command: &str) -> bool {
    regex_lite::Regex::new(pattern).is_ok_and(|re| re.is_match(command))
}

fn check_command(options: &BashToolOptions, command: &str) -> Result<(), ToolError> {
    if let Some(pattern) = options.denied_commands.iter().find(|p| matches(p, command)) {
        return Err(format!("Command blocked: matches denied pattern \"{pattern}\"").into());
    }
    if !options.allowed_commands.is_empty()
        && !options.allowed_commands.iter().any(|p| matches(p, command))
    {
        return Err("Command blocked: does not match any allowed command pattern".into());
    }
    Ok(())
}

/// The command as it will run: the command prefix on its own line, then the
/// spawn hook's changes. Shared by the foreground and background paths.
fn resolve_spawn(options: &BashToolOptions, command: &str, cwd: &Path) -> BashSpawnContext {
    let resolved = match &options.command_prefix {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}\n{command}"),
        _ => command.to_owned(),
    };
    let base = BashSpawnContext {
        command: resolved,
        cwd: cwd.to_path_buf(),
        env: get_shell_env(),
    };
    match &options.spawn_hook {
        Some(hook) => hook(base),
        None => base,
    }
}

/// `run_in_background: true`: start the job and return its id at once. The
/// timeout does not apply to a background job.
fn start_background(
    cwd: &Path,
    options: &BashToolOptions,
    jobs: &JobRegistry,
    command: &str,
) -> Result<AgentToolResult, ToolError> {
    check_command(options, command)?;
    let spawn = resolve_spawn(options, command, cwd);
    let info = jobs.spawn(&spawn.command, &spawn.cwd, spawn.env)?;
    let text = format!(
        "Started background job {id}: `{command}`.\nIts output goes to {log}. Read it with ShellOutput (id \"{id}\") and stop it with ShellKill. A notice arrives when it finishes.",
        id = info.id,
        log = info.log_path.display()
    );
    Ok(AgentToolResult {
        content: vec![text_content(text)],
        details: json!({"jobId": info.id, "logPath": info.log_path.display().to_string()}),
        terminate: false,
    })
}

/// Output plus the throttle state `emitOutputUpdate` works on.
struct Progress {
    output: OutputAccumulator,
    dirty: bool,
    last_update: Option<Instant>,
}

type SharedUpdate = Arc<Mutex<AgentToolUpdateCallback>>;

fn snapshot_details(snapshot: &OutputSnapshot) -> Value {
    let mut details = Map::new();
    if snapshot.truncation.truncated {
        details.insert(
            "truncation".into(),
            serde_json::to_value(&snapshot.truncation).expect("truncation serializes"),
        );
    }
    if let Some(path) = &snapshot.full_output_path {
        details.insert("fullOutputPath".into(), path.clone().into());
    }
    Value::Object(details)
}

/// `emitOutputUpdate`.
fn emit(progress: &mut Progress, on_update: &SharedUpdate) {
    if !progress.dirty {
        return;
    }
    progress.dirty = false;
    progress.last_update = Some(Instant::now());
    let snapshot = progress.output.snapshot(true);
    let update = AgentToolResult {
        content: vec![text_content(snapshot.content.clone())],
        details: snapshot_details(&snapshot),
        terminate: false,
    };
    let callback = on_update.lock().unwrap_or_else(|e| e.into_inner());
    callback(update);
}

/// `formatOutput`: the snapshot text plus the truncation notice.
fn format_output(
    snapshot: &OutputSnapshot,
    last_line_bytes: usize,
    max_bytes: usize,
    empty_text: &str,
) -> (String, Value) {
    let truncation = &snapshot.truncation;
    let mut text = if snapshot.content.is_empty() {
        empty_text.to_owned()
    } else {
        snapshot.content.clone()
    };
    let mut details = Value::Null;
    if truncation.truncated {
        let mut map = Map::new();
        map.insert(
            "truncation".into(),
            serde_json::to_value(truncation).expect("truncation serializes"),
        );
        if let Some(path) = &snapshot.full_output_path {
            map.insert("fullOutputPath".into(), path.clone().into());
        }
        details = Value::Object(map);
        // No temp file (it could not be created) means no "Full output" clause.
        let full_output = match snapshot.full_output_path.as_deref() {
            Some(path) => format!(". Full output: {path}]"),
            None => "]".to_owned(),
        };
        let start_line = truncation.total_lines + 1 - truncation.output_lines;
        let end_line = truncation.total_lines;
        if truncation.last_line_partial {
            text.push_str(&format!(
                "\n\n[Showing last {} of line {end_line} (line is {}){full_output}",
                format_size(truncation.output_bytes),
                format_size(last_line_bytes)
            ));
        } else if truncation.truncated_by == Some(TruncatedBy::Lines) {
            text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {}{full_output}",
                truncation.total_lines
            ));
        } else {
            text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit){full_output}",
                truncation.total_lines,
                format_size(max_bytes)
            ));
        }
    }
    (text, details)
}

fn append_status(text: &str, status: &str) -> String {
    if text.is_empty() {
        status.to_owned()
    } else {
        format!("{text}\n\n{status}")
    }
}

struct BashCall<'a> {
    cwd: &'a PathBuf,
    ops: &'a dyn BashOperations,
    options: &'a BashToolOptions,
    max_bytes: usize,
    max_lines: usize,
}

impl BashCall<'_> {
    /// `timeout` is `None` for no limit. `defaulted` marks a limit the model
    /// did not pass, which changes the timeout message.
    fn run(
        &self,
        command: &str,
        timeout: Option<f64>,
        defaulted: bool,
        signal: Option<hoocode_ai_types::AbortSignal>,
        on_update: Option<AgentToolUpdateCallback>,
    ) -> Result<AgentToolResult, ToolError> {
        check_command(self.options, command)?;

        let spawn = resolve_spawn(self.options, command, self.cwd);

        let progress = Arc::new(Mutex::new(Progress {
            output: OutputAccumulator::new(OutputAccumulatorOptions {
                max_lines: Some(self.max_lines),
                max_bytes: Some(self.max_bytes),
                temp_file_prefix: Some("hoocode-bash".into()),
                command: None,
            }),
            dirty: false,
            last_update: None,
        }));
        let on_update: Option<SharedUpdate> = on_update.map(|f| Arc::new(Mutex::new(f)));
        if let Some(callback) = &on_update {
            let callback = callback.lock().unwrap_or_else(|e| e.into_inner());
            callback(AgentToolResult {
                content: Vec::new(),
                details: Value::Null,
                terminate: false,
            });
        }

        // The throttle timer: flush pending output once 100 ms have passed
        // since the last update, even when no new data arrives. It runs as the
        // idle tick of `exec` (every 10 ms without output), so no thread is spent on it.
        let mut on_idle = on_update.clone().map(|callback| {
            let progress = progress.clone();
            move || {
                let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                let due = p
                    .last_update
                    .is_none_or(|t| t.elapsed() >= BASH_UPDATE_THROTTLE);
                if p.dirty && due {
                    emit(&mut p, &callback);
                }
            }
        });

        let mut on_data = |data: &[u8]| {
            let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
            p.output.append(data);
            if let Some(callback) = &on_update {
                p.dirty = true;
                let due = p
                    .last_update
                    .is_none_or(|t| t.elapsed() >= BASH_UPDATE_THROTTLE);
                if due {
                    emit(&mut p, callback);
                }
            }
        };
        let result = self.ops.exec(
            &spawn.command,
            &spawn.cwd,
            BashExecOptions {
                on_data: &mut on_data,
                on_idle: on_idle.as_mut().map(|f| f as &mut dyn FnMut()),
                signal,
                timeout,
                env: Some(spawn.env),
            },
        );

        // finishOutput
        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
        p.output.finish();
        if let Some(callback) = &on_update {
            emit(&mut p, callback);
        }
        let snapshot = p.output.snapshot(true);
        p.output.close_temp_file()?;
        let last_line_bytes = p.output.last_line_bytes();
        drop(p);

        match result {
            Err(error) => {
                let (text, _) = format_output(&snapshot, last_line_bytes, self.max_bytes, "");
                let message = error.to_string();
                if message == "aborted" {
                    return Err(append_status(&text, "Command aborted").into());
                }
                if let Some(secs) = message.strip_prefix("timeout:") {
                    let status = if defaulted {
                        format!(
                            "Command timed out after {secs}s (default timeout; pass a larger `timeout` for long commands)"
                        )
                    } else {
                        format!("Command timed out after {secs} seconds")
                    };
                    return Err(append_status(&text, &status).into());
                }
                Err(error)
            }
            Ok(exit_code) => {
                let (text, details) =
                    format_output(&snapshot, last_line_bytes, self.max_bytes, "(no output)");
                if let Some(code) = exit_code.filter(|c| *c != 0) {
                    let status = format!("Command exited with code {code}");
                    return Err(append_status(&text, &status).into());
                }
                Ok(AgentToolResult {
                    content: vec![text_content(text)],
                    details,
                    terminate: false,
                })
            }
        }
    }
}

/// `createBashToolDefinition`. Subagents get the default timeout; the main
/// session gets none (decided 2026-10-11).
pub fn create_bash_tool_definition(
    cwd: impl Into<PathBuf>,
    options: BashToolOptions,
) -> ToolDefinition {
    let subagent = is_subagent_context(std::env::var(SUBAGENT_DEPTH_ENV).ok().as_deref());
    let default_timeout = default_timeout_for_context(
        subagent,
        std::env::var(DEFAULT_BASH_TIMEOUT_ENV).ok().as_deref(),
    );
    build_bash_tool_definition(cwd, options, default_timeout)
}

fn build_bash_tool_definition(
    cwd: impl Into<PathBuf>,
    options: BashToolOptions,
    default_timeout: Option<f64>,
) -> ToolDefinition {
    let cwd: PathBuf = cwd.into();
    let ops: Arc<dyn BashOperations> = options.operations.clone().unwrap_or_else(|| {
        Arc::new(LocalBashOperations::new(options.shell_path.clone()).with_nice(options.nice))
    });
    let max_bytes = options.max_output_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let max_lines = options.max_output_lines.unwrap_or(DEFAULT_MAX_LINES);
    // Math.round(maxBytes / 1024)
    let kb = (max_bytes * 2 + 1024) / 2048;
    let timeout_clause = match default_timeout {
        Some(secs) => format!(
            "Subagents default to {}s: without a timeout the command is killed after {}s and its partial output is returned; pass a larger timeout for long-running commands, or 0 for no limit.",
            js_number(secs),
            js_number(secs)
        ),
        None => "A timeout of 0 means no limit.".to_owned(),
    };
    let background_clause = if options.jobs.is_some() {
        " Pass run_in_background true to start a long-running command (a server, a watcher, a long build) detached: it returns a job id at once. ShellOutput reads its output and ShellKill stops it."
    } else {
        ""
    };
    let description = format!(
        "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last {max_lines} lines or {kb}KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds. {timeout_clause}{background_clause}"
    );
    let mut parameters = bash_parameters_schema_with(default_timeout);
    if options.jobs.is_some() {
        parameters["properties"]["run_in_background"] = json!({
            "type": "boolean",
            "description": "Run the command detached and return a job id at once (optional; the timeout does not apply)"
        });
    }
    let jobs = options.jobs.clone();
    ToolDefinition {
        ordered_start: false,
        background_when: None,
        background_placeholder: None,
        name: "Shell".into(),
        label: "Shell".into(),
        description,
        prompt_snippet: Some("Run builds, tests, linters, git, and package managers".into()),
        prompt_guidelines: if options.jobs.is_some() {
            vec!["A command over about a minute (full test suite, release build, server): Shell run_in_background: true, then keep working; short checks stay in the foreground".into()]
        } else {
            Vec::new()
        },
        parameters,
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        execute: Arc::new(move |_tool_call_id, args, signal, on_update, _ctx| {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .ok_or("The \"command\" argument must be of type string")?;
            if let Some(jobs) = &jobs {
                if args.get("run_in_background").and_then(Value::as_bool) == Some(true) {
                    return start_background(&cwd, &options, jobs, command);
                }
            }
            let (timeout, defaulted) =
                resolve_timeout(args.get("timeout").and_then(Value::as_f64), default_timeout);
            BashCall {
                cwd: &cwd,
                ops: ops.as_ref(),
                options: &options,
                max_bytes,
                max_lines,
            }
            .run(command, timeout, defaulted, signal, on_update)
        }),
    }
}

/// `createBashTool`: the definition wrapped for the agent loop.
pub fn create_bash_tool(
    cwd: impl Into<PathBuf>,
    options: BashToolOptions,
    ctx_factory: Option<ToolContextFactory>,
) -> AgentTool {
    wrap_tool_definition(create_bash_tool_definition(cwd, options), ctx_factory)
}

#[cfg(test)]
mod format_output_tests {
    use super::*;
    use hoocode_code_tool_api::TruncationResult;

    fn truncated_by_lines(full_output_path: Option<&str>) -> (OutputSnapshot, usize) {
        let truncation = TruncationResult {
            content: "tail".into(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Lines),
            total_lines: 3000,
            total_bytes: 90_000,
            output_lines: 2000,
            output_bytes: 60_000,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines: 2000,
            max_bytes: 50 * 1024,
        };
        let snapshot = OutputSnapshot {
            content: "tail".into(),
            truncation,
            full_output_path: full_output_path.map(str::to_owned),
        };
        (snapshot, 0)
    }

    #[test]
    fn truncated_output_names_the_full_output_file() {
        let (snapshot, last) = truncated_by_lines(Some("/tmp/hoocode-bash-1.log"));
        let (text, _) = format_output(&snapshot, last, 50 * 1024, "");
        assert!(
            text.ends_with("of 3000. Full output: /tmp/hoocode-bash-1.log]"),
            "got: {text}"
        );
    }

    #[test]
    fn partial_last_line_names_the_full_output_file() {
        let (mut snapshot, last) = truncated_by_lines(Some("/tmp/x.log"));
        snapshot.truncation.truncated_by = Some(TruncatedBy::Bytes);
        snapshot.truncation.last_line_partial = true;
        let (text, _) = format_output(&snapshot, last, 50 * 1024, "");
        assert!(
            text.ends_with("(line is 0B). Full output: /tmp/x.log]"),
            "got: {text}"
        );
    }

    #[test]
    fn truncated_output_without_a_temp_file_has_no_undefined_path() {
        let (snapshot, last) = truncated_by_lines(None);
        let (text, _) = format_output(&snapshot, last, 50 * 1024, "");
        assert!(!text.contains("undefined"), "got: {text}");
        assert!(!text.contains("Full output"), "got: {text}");
        assert!(text.ends_with("of 3000]"), "got: {text}");
    }
}

#[cfg(test)]
mod default_timeout_tests {
    use super::*;

    fn run_with_default(default: f64, args: Value) -> Result<AgentToolResult, ToolError> {
        let cwd = std::env::current_dir().unwrap();
        let tool = build_bash_tool_definition(&cwd, BashToolOptions::default(), Some(default));
        (tool.execute)("call".into(), args, None, None, None)
    }

    #[test]
    fn parses_the_env_override_and_falls_back_on_junk() {
        assert_eq!(parse_default_timeout(None), DEFAULT_BASH_TIMEOUT_SECS);
        assert_eq!(parse_default_timeout(Some("45")), 45.0);
        assert_eq!(parse_default_timeout(Some(" 2.5 ")), 2.5);
        for junk in ["", "0", "-3", "abc", "NaN", "inf"] {
            assert_eq!(parse_default_timeout(Some(junk)), DEFAULT_BASH_TIMEOUT_SECS);
        }
    }

    #[test]
    fn subagent_schema_and_description_name_the_default() {
        let cwd = std::env::current_dir().unwrap();
        let tool = build_bash_tool_definition(&cwd, BashToolOptions::default(), Some(600.0));
        assert!(
            tool.description.contains("Subagents default to 600s"),
            "{}",
            tool.description
        );
        assert!(
            tool.description.contains("killed after 600s"),
            "{}",
            tool.description
        );
        let timeout = &tool.parameters["properties"]["timeout"]["description"];
        assert!(
            timeout.as_str().unwrap().contains("defaults to 600s"),
            "{timeout}"
        );
    }

    #[test]
    fn main_session_schema_and_description_promise_no_default() {
        let cwd = std::env::current_dir().unwrap();
        let tool = build_bash_tool_definition(&cwd, BashToolOptions::default(), None);
        assert!(
            !tool.description.contains("default"),
            "{}",
            tool.description
        );
        assert!(!tool.description.contains("killed after"));
        assert!(tool.description.contains("0 means no limit"));
        let timeout = &tool.parameters["properties"]["timeout"]["description"];
        assert!(!timeout.as_str().unwrap().contains("default"), "{timeout}");
        assert_eq!(bash_parameters_schema()["required"], json!(["command"]));
    }

    #[test]
    fn subagent_context_is_read_from_the_depth_variable() {
        assert!(!is_subagent_context(None));
        assert!(!is_subagent_context(Some("")));
        assert!(!is_subagent_context(Some("0")));
        assert!(!is_subagent_context(Some("-2")));
        assert!(!is_subagent_context(Some("nope")));
        assert!(is_subagent_context(Some("1")));
        assert!(is_subagent_context(Some(" 2 ")));
    }

    #[test]
    fn default_timeout_applies_to_subagents_only() {
        // Main session: no default at all, whatever the override says.
        assert_eq!(default_timeout_for_context(false, None), None);
        assert_eq!(default_timeout_for_context(false, Some("45")), None);
        // Subagent: the default, or the positive override.
        assert_eq!(
            default_timeout_for_context(true, None),
            Some(DEFAULT_BASH_TIMEOUT_SECS)
        );
        assert_eq!(default_timeout_for_context(true, Some("45")), Some(45.0));
        assert_eq!(
            default_timeout_for_context(true, Some("junk")),
            Some(DEFAULT_BASH_TIMEOUT_SECS)
        );
    }

    #[test]
    fn main_session_omitted_timeout_has_no_limit() {
        // With no default, an omitted timeout runs unbounded (here: finishes in 0.5s).
        let cwd = std::env::current_dir().unwrap();
        let tool = build_bash_tool_definition(&cwd, BashToolOptions::default(), None);
        let result = (tool.execute)(
            "call".into(),
            json!({"command": "sleep 0.5; echo unbounded"}),
            None,
            None,
            None,
        )
        .unwrap();
        let shown = result
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(shown.contains("unbounded"), "{shown}");
    }

    #[test]
    fn omitted_timeout_uses_the_default_and_keeps_partial_output() {
        let started = std::time::Instant::now();
        let err = run_with_default(0.5, json!({"command": "echo partial; sleep 30"})).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("partial"), "{message}");
        assert!(
            message.contains(
                "Command timed out after 0.5s (default timeout; pass a larger `timeout` for long commands)"
            ),
            "{message}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn resolves_the_requested_timeout() {
        // omitted -> default; explicit 0 -> unlimited; positive -> as given.
        assert_eq!(resolve_timeout(None, Some(600.0)), (Some(600.0), true));
        assert_eq!(resolve_timeout(Some(0.0), Some(600.0)), (None, false));
        assert_eq!(resolve_timeout(Some(-0.0), Some(600.0)), (None, false));
        assert_eq!(resolve_timeout(Some(2.5), Some(600.0)), (Some(2.5), false));
        // negative and non-finite -> default
        for bad in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(resolve_timeout(Some(bad), Some(600.0)), (Some(600.0), true));
        }
        // main session (no default): omitted and bad values are unlimited
        assert_eq!(resolve_timeout(None, None), (None, false));
        assert_eq!(resolve_timeout(Some(-1.0), None), (None, false));
        assert_eq!(resolve_timeout(Some(2.5), None), (Some(2.5), false));
    }

    #[test]
    fn negative_timeout_falls_back_to_the_default() {
        let err = run_with_default(0.5, json!({"command": "sleep 30", "timeout": -1})).unwrap_err();
        assert!(err.to_string().contains("(default timeout;"), "{err}");
    }

    #[test]
    fn explicit_zero_timeout_means_no_limit() {
        // The default (0.2s) would kill this; an explicit 0 must not.
        let cwd = std::env::current_dir().unwrap();
        let tool = build_bash_tool_definition(&cwd, BashToolOptions::default(), Some(0.2));
        let result = (tool.execute)(
            "call".into(),
            json!({"command": "sleep 0.5; echo finished", "timeout": 0}),
            None,
            None,
            None,
        )
        .unwrap();
        let shown = result
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(shown.contains("finished"), "{shown}");
    }

    #[test]
    fn explicit_timeout_wins_over_the_default() {
        let err =
            run_with_default(600.0, json!({"command": "sleep 30", "timeout": 0.5})).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("Command timed out after 0.5 seconds"),
            "{message}"
        );
        assert!(!message.contains("default timeout"), "{message}");
    }
}
