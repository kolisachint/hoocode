//! `extensions/core/permission-gate.ts`: the per-mode tool policy and the
//! approval prompt, as a [`PermissionGate`].

use hoocode_agent_types::{AgentToolCall, PermissionDecision, PermissionGate};
use hoocode_code_modes::config::{self, HooConfig};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// `GATED_TOOLS`: prompted in interactive sessions. `ShellKill` stops a background
/// job, so it asks like `Shell`; `ShellOutput` only reads and is not gated.
pub const GATED_TOOLS: [&str; 6] = [
    "Shell",
    "Write",
    "Edit",
    "WebFetch",
    "WebSearch",
    "ShellKill",
];

/// The plugin lifecycle tools. They are not in [`GATED_TOOLS`]; in fail-closed
/// (rpc) mode they are gated like one, so they need `auto_allow`.
pub const PLUGIN_TOOLS: [&str; 5] = [
    "SearchPlugins",
    "ListPlugins",
    "InstallPlugin",
    "UninstallPlugin",
    "ProposePlugin",
];

/// MCP tools are named `mcp_<server>_<tool>` (`hoocode-agent-mcp`'s naming).
fn is_mcp_or_plugin_tool(tool_name: &str) -> bool {
    tool_name.starts_with("mcp_") || PLUGIN_TOOLS.contains(&tool_name)
}

/// Whether a call to `tool_name` is one an approval-asking client would be
/// asked about: the interactive gated tools, plus the MCP and plugin tools that
/// rpc gates. Print and json run these without asking; the CLI names the ones
/// that ran.
pub fn is_approval_tool(tool_name: &str) -> bool {
    GATED_TOOLS.contains(&tool_name) || is_mcp_or_plugin_tool(tool_name)
}

/// The three prompt choices, in order.
pub const CHOICES: [&str; 3] = [
    "Yes (once)",
    "No (block)",
    "Always (add to auto-allow for this mode)",
];

/// Internal (not in `--help`): `run_rpc_mode` sets this to `1` in its own
/// environment, so every child it spawns (json subagents, print runs) fails
/// closed too. Warm workers opt out of their own gate with
/// [`WARM_WORKER_ENV`]; they still inherit this variable for their children.
pub const FAIL_CLOSED_ENV: &str = "HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED";

/// Internal (not in `--help`): set only by the warm subagent pool on the
/// `--mode rpc` workers it drives. Such a worker's own gate fails open, because
/// no client on its stdio can answer a prompt. Never set it anywhere else.
pub const WARM_WORKER_ENV: &str = "HOOCODE_INTERNAL_WARM_WORKER";

/// One REACH cell on the clearance card: on (`■`) or off (`□`), with an
/// optional qualifier from the call's real data (a path, a program count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachCell {
    pub on: bool,
    pub qualifier: Option<String>,
}

impl ReachCell {
    fn off() -> Self {
        Self {
            on: false,
            qualifier: None,
        }
    }

    fn on(qualifier: Option<String>) -> Self {
        Self {
            on: true,
            qualifier,
        }
    }
}

/// The clearance card's REACH line (Mission Control spec §4 c1, §8.11): what
/// the call can reach. Qualifiers use the real path, never a renamed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub read: ReachCell,
    pub write: ReachCell,
    pub network: ReachCell,
    pub pipe: ReachCell,
}

/// Commands that almost always mean the network (a hint, not enforcement).
const NETWORK_PROGRAMS: [&str; 11] = [
    "curl", "wget", "ssh", "scp", "sftp", "ping", "nc", "netcat", "ncat", "telnet", "ftp",
];

/// The REACH cells for a gated call, from the call's real data where the
/// gate has it: the Shell command's paths, pipe and redirect, the mutation
/// path of Edit/Write, the network of WebFetch/WebSearch.
pub fn reach_for(tool_name: &str, input: &Value) -> Reach {
    match tool_name {
        "Edit" => {
            let path = mutation_path(input);
            Reach {
                read: ReachCell::on(path.clone()),
                write: ReachCell::on(path),
                network: ReachCell::off(),
                pipe: ReachCell::off(),
            }
        }
        "Write" => Reach {
            read: ReachCell::off(),
            write: ReachCell::on(mutation_path(input)),
            network: ReachCell::off(),
            pipe: ReachCell::off(),
        },
        "WebFetch" | "WebSearch" => Reach {
            read: ReachCell::off(),
            write: ReachCell::off(),
            network: ReachCell::on(None),
            pipe: ReachCell::off(),
        },
        "Shell" => shell_reach(input.get("command").and_then(Value::as_str).unwrap_or("")),
        _ => Reach {
            read: ReachCell::off(),
            write: ReachCell::off(),
            network: ReachCell::off(),
            pipe: ReachCell::off(),
        },
    }
}

/// One word of a Shell command, with whether it opens a quoted run.
fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

/// The pipe segments of a Shell command: words per segment, plus the write
/// redirect targets. Quotes and backslash escapes hold words together;
/// `||`, `&&`, `;`, `<` and `(` end a segment without joining a pipe.
fn split_segments(command: &str) -> (Vec<Vec<String>>, Vec<String>, bool) {
    let mut segments: Vec<Vec<String>> = vec![Vec::new()];
    let mut redirects = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();
    // A `>`/`>>` waits for its target word; `piped` tracks single-`|` joins.
    // `prev` is the last char outside quotes, spaces included: an `&`
    // continues a word only directly after `>` (`2>&1`), so `2> &1`
    // splits and fails closed.
    let mut redirect_pending = false;
    let mut piped = false;
    let mut prev: Option<char> = None;
    while let Some(c) = chars.next() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '|' => {
                take_redirect(&mut redirects, &mut redirect_pending, &mut current);
                push_word(segments.last_mut().expect("segment"), &mut current);
                if chars.peek() == Some(&'|') {
                    chars.next();
                } else {
                    piped = true;
                }
                segments.push(Vec::new());
                redirect_pending = false;
                prev = Some('|');
            }
            '&' | ';' | '<' | '(' | ')' => {
                if c == '&' && prev == Some('>') {
                    // A descriptor dup (`2>&1`): the word continues.
                    current.push(c);
                    prev = Some(c);
                    continue;
                }
                if c == '&' && chars.peek() == Some(&'&') {
                    chars.next();
                }
                take_redirect(&mut redirects, &mut redirect_pending, &mut current);
                push_word(segments.last_mut().expect("segment"), &mut current);
                segments.push(Vec::new());
                redirect_pending = false;
                prev = Some(c);
            }
            '>' => {
                if chars.peek() == Some(&'>') {
                    chars.next();
                }
                take_redirect(&mut redirects, &mut redirect_pending, &mut current);
                push_word(segments.last_mut().expect("segment"), &mut current);
                redirect_pending = true;
                prev = Some('>');
            }
            c if c.is_whitespace() => {
                take_redirect(&mut redirects, &mut redirect_pending, &mut current);
                push_word(segments.last_mut().expect("segment"), &mut current);
                prev = Some(' ');
            }
            _ => {
                current.push(c);
                prev = Some(c);
            }
        }
    }
    take_redirect(&mut redirects, &mut redirect_pending, &mut current);
    push_word(segments.last_mut().expect("segment"), &mut current);
    segments.retain(|words| !words.is_empty());
    (segments, redirects, piped)
}

/// Completes a pending redirect target (`>` then a word).
fn take_redirect(redirects: &mut Vec<String>, pending: &mut bool, current: &mut String) {
    if *pending && !current.is_empty() {
        redirects.push(std::mem::take(current));
        *pending = false;
    }
}

/// Whether a word names a file: it holds a `/` or a `.` (flags, assignments
/// and the segment's own program are filtered before this runs).
fn is_path_word(word: &str) -> bool {
    word.contains('/') || word.contains('.')
}

/// The REACH cells of a Shell command: the first path-like argument reads,
/// a `>`/`>>` target writes, a pipe joins N programs, a network program or
/// URL reaches the network.
fn shell_reach(command: &str) -> Reach {
    let (segments, redirects, piped) = split_segments(command);
    let mut read: Option<String> = None;
    let mut network = false;
    for words in &segments {
        for (i, word) in words.iter().enumerate() {
            let lower = word.to_ascii_lowercase();
            if lower.contains("http://") || lower.contains("https://") {
                network = true;
            }
            // The segment's program: `curl` et al. reach the network, and
            // the program itself is never the read path.
            if i == 0 {
                if NETWORK_PROGRAMS.contains(&lower.as_str()) {
                    network = true;
                }
                continue;
            }
            if word.starts_with('-') || word.contains('=') {
                continue;
            }
            if read.is_none() && is_path_word(word) {
                read = Some(word.clone());
            }
        }
    }
    let programs = segments.len();
    Reach {
        read: ReachCell {
            on: read.is_some(),
            qualifier: read,
        },
        write: ReachCell {
            on: !redirects.is_empty(),
            qualifier: redirects.into_iter().next(),
        },
        network: ReachCell {
            on: network,
            qualifier: None,
        },
        pipe: ReachCell {
            on: piped,
            qualifier: piped.then(|| format!("{programs} programs")),
        },
    }
}

/// What the clearance card shows for one gated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearanceInfo {
    /// The tool asking (`Shell`, `Edit`, …).
    pub tool: String,
    /// The call's subject (`describe_tool`: `$ <command>`, `Edit <path>`, …).
    pub detail: String,
    /// The active mode id (`build`, `plan`, …); the card uppercases it.
    pub mode: String,
    /// The held step number, when the host knows the trajectory (`step NN`).
    /// The gate never knows it, so it passes `None`.
    pub step: Option<usize>,
    /// What the call can reach, from the call's real data.
    pub reach: Reach,
    /// The subagent the call comes from, when the gate that asks is not the
    /// one that holds the call (`explore`). `None` for the host's own calls.
    pub from: Option<String>,
}

/// The clearance card's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearanceChoice {
    /// Run this call once.
    Once,
    /// Add the tool to the mode's auto-allow, then run.
    Always,
    /// Block the call; `reason` is the typed abort reason, if any.
    Deny { reason: Option<String> },
    /// Do not run this call; the host sends the revised text as the next
    /// message instead (`e edit first`).
    Revise(String),
}

/// Who can answer for a gated call that needs approval (a `PermissionGate`'s
/// channel). Decides what [`evaluate`] returns for such a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChannel {
    /// A UI or client answers prompts (interactive mode, the app-server).
    Ui,
    /// Nobody can answer. A gated call that needs approval is denied when
    /// `fail_closed` (rpc mode), and runs otherwise (print, json).
    Headless { fail_closed: bool },
}

/// The UI the gate prompts through (`ctx.ui.select` / `ctx.ui.notify`).
pub trait PermissionUi: Send + Sync {
    /// Show `title` with `options`; `None` = cancelled.
    fn select(&self, title: &str, options: &[&str]) -> Option<String>;
    /// An info notification.
    fn notify(&self, message: &str);
    /// The clearance card for a gated call. The default asks the legacy
    /// three-choice prompt and maps it back, so hosts without a card keep
    /// working (`Revise` has no legacy form and reads as a denial).
    fn select_clearance(&self, info: &ClearanceInfo) -> ClearanceChoice {
        let title = format!("Allow: {}", info.detail);
        match self.select(&title, &CHOICES) {
            Some(c) if c.starts_with("Always") => ClearanceChoice::Always,
            Some(c) if c.starts_with("No") => ClearanceChoice::Deny { reason: None },
            Some(_) => ClearanceChoice::Once,
            None => ClearanceChoice::Deny { reason: None },
        }
    }
}

/// A gated call that a subagent child could not ask about itself and hands to
/// whoever started it. See [`ClearanceRelay`].
#[derive(Debug, Clone, Default)]
pub struct RelayRequest {
    /// The subagent the call comes from (`explore`), when it did not
    /// start in the asking process. `None` for the process's own calls.
    pub via: Option<String>,
    pub tool: String,
    pub input: Value,
    /// Set once nobody waits for the answer any more (the subagent ended or
    /// was cancelled). The asker then stops waiting and the call stays denied.
    pub cancelled: Arc<AtomicBool>,
}

/// Where a gate that cannot prompt sends a call that needs approval: a
/// subagent child sends it up to its parent, and the parent's own gate (its
/// config rules, its session grants, its card) decides.
pub trait ClearanceRelay: Send + Sync {
    /// Blocks until the call is decided. `None` lets it run, `Some(reason)`
    /// blocks it. Any failure to ask (no UI, a closed pipe) is a denial.
    fn ask(&self, request: &RelayRequest) -> Option<String>;
}

/// `matchesAllowedPath`: exact or `*`-glob match of the path (and, for an
/// absolute path, its cwd-relative form), all with `/` separators.
pub fn matches_allowed_path(file_path: &str, allowed: &[String], cwd: &Path) -> bool {
    if allowed.is_empty() {
        return true;
    }
    if file_path.is_empty() {
        return false;
    }
    let to_posix = |s: &str| s.replace('\\', "/");
    let mut candidates = vec![to_posix(file_path)];
    let path = Path::new(file_path);
    if path.is_absolute() {
        let rel = relative(cwd, path);
        if !rel.is_empty() && !candidates.contains(&rel) {
            candidates.push(to_posix(&rel));
        }
    }
    for pattern in allowed {
        let pattern = to_posix(pattern);
        if candidates.contains(&pattern) {
            return true;
        }
        if !pattern.contains('*') {
            continue;
        }
        let source = format!(
            "^{}$",
            pattern
                .split('*')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join(".*")
        );
        if let Ok(re) = regex::Regex::new(&source) {
            if candidates.iter().any(|c| re.is_match(c)) {
                return true;
            }
        }
    }
    false
}

/// Node's `path.relative(from, to)` for absolute paths (lexical).
fn relative(from: &Path, to: &Path) -> String {
    let norm = |p: &Path| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                std::path::Component::Normal(s) => out.push(s.to_string_lossy().into_owned()),
                std::path::Component::ParentDir => {
                    out.pop();
                }
                _ => {}
            }
        }
        out
    };
    let (a, b) = (norm(from), norm(to));
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".into(); a.len() - common];
    parts.extend(b[common..].iter().cloned());
    parts.join("/")
}

/// `matchesBashPattern`: an invalid pattern never matches.
fn matches_bash_pattern(pattern: &str, command: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(command))
}

/// A Shell command's first word (the `a always …` program), with one layer
/// of surrounding quotes stripped. Shared with the clearance card's label.
pub fn shell_program(command: &str) -> Option<String> {
    let word = command.split_whitespace().next()?;
    let word = word
        .strip_prefix('\'')
        .or_else(|| word.strip_prefix('"'))
        .unwrap_or(word);
    let word = word
        .strip_suffix('\'')
        .or_else(|| word.strip_suffix('"'))
        .unwrap_or(word);
    (!word.is_empty()).then(|| word.to_string())
}

/// Programs an always-rule may never cover: prefix runners that hand
/// control to another program, and every bash/zsh builtin and keyword
/// (they mutate shell state or run what their arguments name). Compared
/// by basename, case-insensitive (`./sudo` and `SUDO` count). For these,
/// `a` only runs once.
pub const ALWAYS_WRAPPERS: [&str; 83] = [
    "sudo", "doas", "su", "env", "sh", "bash", "zsh", "dash", "fish", "xargs", "nohup", "nice",
    "timeout", "watch", "find", "trap", "export", "declare", "typeset", "local", "readonly", "set",
    "unset", "shopt", "coproc", "function", "alias", "unalias", "cd", "pushd", "popd", "kill",
    "wait", "jobs", "fg", "bg", "disown", "suspend", "exec", "eval", "source", ".", "command",
    "builtin", "enable", "hash", "ulimit", "umask", "read", "mapfile", "let", "getopts", "fc",
    "history", "logout", "exit", "return", "break", "continue", "shift", "times", "type", "[[",
    "if", "then", "elif", "else", "fi", "for", "while", "until", "case", "esac", "select", "time",
    "do", "done", "in", "{", "}", "(", ")", "!",
];

/// Whether `program` hands control elsewhere, so no always-rule may cover
/// it: the basename, case-insensitive.
pub fn is_always_wrapper(program: &str) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    ALWAYS_WRAPPERS.contains(&base.to_ascii_lowercase().as_str())
}

/// Whether `command` hides code the first-word check cannot see: command
/// or process substitution, expansions, here-docs, or a line break. The
/// scan skips single-quoted runs (nothing expands there); double quotes
/// still expand, so they stay checked.
fn has_code_expansion(command: &str) -> bool {
    if command.contains('\n') || command.contains('\r') {
        return true;
    }
    let mut single = false;
    let mut escaped = false;
    let mut prev: Option<char> = None;
    for c in command.chars() {
        if single {
            if c == '\'' {
                single = false;
            }
            continue;
        }
        if escaped {
            escaped = false;
            prev = None;
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                prev = None;
            }
            '\'' => {
                single = true;
                prev = None;
            }
            '`' => return true,
            '(' if matches!(prev, Some('$') | Some('<') | Some('>')) => return true,
            '{' if prev == Some('$') => return true,
            '<' if prev == Some('<') => return true,
            _ => prev = matches!(c, '$' | '<' | '>').then_some(c),
        }
    }
    false
}

/// Whether `word` assigns (`NAME=value`): allowing what follows would allow
/// whatever the assignment redefines, so it never auto-allows.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

/// Whether an always-rule for `approved` programs covers `command`: every
/// pipe segment's program (first word) is approved, with no wrapper, no
/// slash in the program, no leading assignment, no code expansion, no file
/// redirect (descriptor dups like `2>&1` and `/dev/null` are fine), and no
/// command over 32 KiB. Anything else falls through to a prompt — never a
/// block, never a silent allow.
pub fn always_allows(command: &str, approved: &[String]) -> bool {
    if command.len() > 32 * 1024 {
        return false;
    }
    if has_code_expansion(command) {
        return false;
    }
    let (segments, redirects, _) = split_segments(command);
    if segments.is_empty() {
        return false;
    }
    if redirects
        .iter()
        .any(|target| target != "/dev/null" && !target.starts_with('&'))
    {
        return false;
    }
    segments.iter().all(|words| {
        let program = &words[0];
        !program.contains('/')
            && !is_always_wrapper(program)
            && !is_assignment(program)
            && approved.iter().any(|name| name == program)
    })
}

/// `mutationPath`: `path`, else `file_path` (non-empty strings).
pub fn mutation_path(input: &Value) -> Option<String> {
    ["path", "file_path"].iter().find_map(|k| {
        input
            .get(*k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// `describeTool`: the prompt's subject.
pub fn describe_tool(tool_name: &str, input: &Value) -> String {
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("(unknown)")
            .to_string()
    };
    match tool_name {
        "Shell" => {
            // `command.replace(/\s+/g, " ").slice(0, 100)` (UTF-16 units).
            let command = input.get("command").and_then(Value::as_str).unwrap_or("");
            let mut collapsed = String::new();
            let mut in_space = false;
            for c in command.chars() {
                if c.is_whitespace() || c == '\u{feff}' {
                    if !in_space {
                        collapsed.push(' ');
                    }
                    in_space = true;
                } else {
                    collapsed.push(c);
                    in_space = false;
                }
            }
            let units: Vec<u16> = collapsed.encode_utf16().take(100).collect();
            format!("$ {}", String::from_utf16_lossy(&units))
        }
        "Edit" | "Write" => format!(
            "{tool_name} {}",
            mutation_path(input).unwrap_or_else(|| "(unknown)".into())
        ),
        "WebFetch" => format!("WebFetch {}", field("url")),
        "WebSearch" => format!("WebSearch \"{}\"", field("query")),
        _ => tool_name.to_string(),
    }
}

/// The gate's verdict before any prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Block(String),
    Allow,
    /// Ask the user.
    Prompt,
}

/// The hoocode `tool_call` handler up to the prompt, over a merged config.
pub fn evaluate(
    config: &HooConfig,
    cwd: &Path,
    tool_name: &str,
    input: &Value,
    channel: ApprovalChannel,
) -> Verdict {
    let mode = config::active_mode(config).unwrap_or_else(|| "build".into());
    let list = |key: &str| config::mode_list(config, &mode, key);

    if list("denied_tools").is_some_and(|d| d.iter().any(|t| t == tool_name)) {
        return Verdict::Block(format!(
            "Tool \"{tool_name}\" is denied in mode \"{mode}\"."
        ));
    }
    if let Some(enabled) = list("enabled_tools").filter(|e| !e.is_empty()) {
        if !enabled.iter().any(|t| t == tool_name) {
            return Verdict::Block(format!(
                "Tool \"{tool_name}\" is not enabled in mode \"{mode}\" (enabled: {}).",
                enabled.join(", ")
            ));
        }
    }
    if tool_name == "Shell" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        for pattern in list("denied_bash_commands").unwrap_or_default() {
            if matches_bash_pattern(&pattern, command) {
                return Verdict::Block(format!(
                    "Bash command matches a denied pattern in mode \"{mode}\": {pattern}"
                ));
            }
        }
        if let Some(allowed) = list("allowed_bash_commands").filter(|a| !a.is_empty()) {
            if !allowed.iter().any(|p| matches_bash_pattern(p, command)) {
                return Verdict::Block(format!(
                    "Bash command is not permitted in mode \"{mode}\". Allowed patterns: {}",
                    allowed.join(", ")
                ));
            }
        }
    }

    let fail_closed = match channel {
        ApprovalChannel::Ui => false,
        // print/json: nobody is asked, and the CLI prints one notice per run.
        ApprovalChannel::Headless { fail_closed: false } => return Verdict::Allow,
        ApprovalChannel::Headless { fail_closed: true } => true,
    };
    // MCP and plugin tools are gated only where nobody can answer (rpc). A UI
    // still runs them without a prompt, as before.
    let gated =
        GATED_TOOLS.contains(&tool_name) || (fail_closed && is_mcp_or_plugin_tool(tool_name));
    if !gated {
        return Verdict::Allow;
    }
    if tool_name == "Write" || tool_name == "Edit" {
        if let Some(allowed) = list("allowed_write_paths") {
            let file_path = mutation_path(input).unwrap_or_default();
            if !matches_allowed_path(&file_path, &allowed, cwd) {
                return Verdict::Block(format!(
                    "Mode \"{mode}\" only allows writes to: {}. Attempted to {tool_name}: {file_path}. Switch to \"/mode build\" to modify source files.",
                    allowed.join(", ")
                ));
            }
        }
    }
    if list("auto_allow").is_some_and(|a| a.iter().any(|t| t == tool_name)) {
        return Verdict::Allow;
    }
    // A program the session always-approved (`a` on the clearance card)
    // runs without asking, when the whole command stays inside the
    // approval. Never consulted in PLAN mode. Anything else falls
    // through to a prompt.
    if tool_name == "Shell"
        && !mode.trim().eq_ignore_ascii_case("plan")
        && always_allows(
            input.get("command").and_then(Value::as_str).unwrap_or(""),
            &session_programs(cwd, &mode),
        )
    {
        return Verdict::Allow;
    }
    if fail_closed {
        return Verdict::Block(format!(
            "Tool \"{tool_name}\" needs approval, but no client can answer in this session (rpc mode denies it). Add it to auto_allow for mode \"{mode}\" in hoo-config.json to allow it."
        ));
    }
    Verdict::Prompt
}

/// Appends `value` to the mode's string list `key`, without duplicates.
/// "Always" goes to the global config only.
fn insert_mode_list(latest: &mut HooConfig, mode: &str, key: &str, value: String) {
    let modes = latest
        .entry("modes")
        .or_insert_with(|| Value::Object(Default::default()));
    if !modes.is_object() {
        *modes = Value::Object(Default::default());
    }
    let entry = modes.as_object_mut().map(|m| {
        m.entry(mode.to_string())
            .or_insert_with(|| Value::Object(Default::default()))
    });
    if let Some(Value::Object(mode_cfg)) = entry {
        let mut allow: Vec<Value> = mode_cfg
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !allow.iter().any(|v| v.as_str() == Some(&value)) {
            allow.push(Value::String(value));
        }
        mode_cfg.insert(key.into(), Value::Array(allow));
    }
}

/// Session-only Shell always-approvals, keyed by (cwd, mode): in memory,
/// cleared on process exit, never read from or written to config. A
/// subagent of the same session shares them only for the same cwd.
/// Session approvals by (cwd, mode).
type SessionApprovals = HashMap<(PathBuf, String), HashSet<String>>;

static SESSION_ALWAYS: std::sync::LazyLock<Mutex<SessionApprovals>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn session_store() -> std::sync::MutexGuard<'static, SessionApprovals> {
    SESSION_ALWAYS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Remembers `program` for the session: later commands it covers run
/// without asking (same cwd and mode only). Crate-visible only for the
/// gate's own tests; the UI reaches it through `ClearanceChoice::Always`.
pub(crate) fn session_allow_program(cwd: &Path, mode: &str, program: &str) {
    session_store()
        .entry((cwd.to_path_buf(), mode.to_string()))
        .or_default()
        .insert(program.to_string());
}

/// The session's always-approved programs for (cwd, mode).
fn session_programs(cwd: &Path, mode: &str) -> Vec<String> {
    session_store()
        .get(&(cwd.to_path_buf(), mode.to_string()))
        .map(|programs| programs.iter().cloned().collect())
        .unwrap_or_default()
}

/// hoocode's permission gate for one working directory.
pub struct HooPermissionGate {
    cwd: PathBuf,
    channel: ApprovalChannel,
    /// The prompt for [`ApprovalChannel::Ui`]. A `Ui` gate without one denies
    /// what would have been prompted (never a silent allow).
    ui: Option<Arc<dyn PermissionUi>>,
    /// Where a call that needs approval goes when this process cannot prompt
    /// (a subagent child). Takes precedence over `ui`.
    relay: Option<Arc<dyn ClearanceRelay>>,
}

impl HooPermissionGate {
    pub fn new(cwd: PathBuf, channel: ApprovalChannel, ui: Option<Arc<dyn PermissionUi>>) -> Self {
        Self {
            cwd,
            channel,
            ui,
            relay: None,
        }
    }

    /// A gate that hands every call needing approval to `relay`. Use it with
    /// [`ApprovalChannel::Ui`], so such calls reach the prompt step.
    pub fn with_relay(mut self, relay: Arc<dyn ClearanceRelay>) -> Self {
        self.relay = Some(relay);
        self
    }

    /// The handler: `None` lets the call run, `Some(reason)` blocks it.
    pub fn check(&self, tool_name: &str, input: &Value) -> Option<String> {
        let merged = config::read_merged_config(&self.cwd);
        match evaluate(&merged, &self.cwd, tool_name, input, self.channel) {
            Verdict::Allow => None,
            Verdict::Block(reason) => Some(reason),
            Verdict::Prompt => {
                if let Some(relay) = self.relay.as_ref() {
                    return relay.ask(&RelayRequest {
                        tool: tool_name.to_string(),
                        input: input.clone(),
                        ..Default::default()
                    });
                }
                let Some(ui) = self.ui.as_ref() else {
                    return Some("Denied by permission gate".into());
                };
                let info = ClearanceInfo {
                    tool: tool_name.to_string(),
                    detail: describe_tool(tool_name, input),
                    mode: config::active_mode(&merged).unwrap_or_else(|| "build".into()),
                    step: None,
                    reach: reach_for(tool_name, input),
                    from: None,
                };
                match ui.select_clearance(&info) {
                    ClearanceChoice::Once => None,
                    ClearanceChoice::Deny { reason } => Some(match reason {
                        Some(reason) if !reason.trim().is_empty() => {
                            format!("Denied by permission gate: {}", reason.trim())
                        }
                        _ => "Denied by permission gate".into(),
                    }),
                    // The revised text runs as a later message; this call stays denied.
                    ClearanceChoice::Revise(_) => Some("Denied by permission gate".into()),
                    // "Always" on Shell is session-only: remembered in memory
                    // for this cwd and mode (the merged mode, same value the
                    // check consults), never written to config. Slash
                    // programs, wrappers and PLAN mode only run once.
                    // Other tools keep tool-name scope in `auto_allow`.
                    ClearanceChoice::Always => {
                        // "Always" goes to the global config only.
                        let mut latest = config::read_config();
                        let mode = config::active_mode(&merged).unwrap_or_else(|| "build".into());
                        if tool_name == "Shell" && !mode.trim().eq_ignore_ascii_case("plan") {
                            let command =
                                input.get("command").and_then(Value::as_str).unwrap_or("");
                            match shell_program(command).filter(|p| {
                                !p.contains('/') && !is_always_wrapper(p) && !is_assignment(p)
                            }) {
                                Some(program) => {
                                    session_allow_program(&self.cwd, &mode, &program);
                                    ui.notify(&format!(
                                        "\"{program}\" always allowed in this session"
                                    ));
                                    None
                                }
                                None => {
                                    ui.notify("\"always\" isn't offered here; ran once");
                                    None
                                }
                            }
                        } else if tool_name == "Shell" {
                            // PLAN mode: `a` is not offered, so this only
                            // runs it once.
                            ui.notify("\"always\" isn't offered here; ran once");
                            None
                        } else {
                            insert_mode_list(&mut latest, &mode, "auto_allow", tool_name.into());
                            let _ = config::write_config(&latest);
                            ui.notify(&format!(
                                "\"{tool_name}\" added to auto-allow for mode \"{mode}\""
                            ));
                            None
                        }
                    }
                }
            }
        }
    }
}

impl PermissionGate for HooPermissionGate {
    fn request(&self, tool_call: &AgentToolCall) -> PermissionDecision {
        match self.check(&tool_call.name, &tool_call.arguments) {
            None => PermissionDecision::Grant,
            Some(reason) => PermissionDecision::Deny { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell(command: &str) -> Value {
        json!({"command": command})
    }

    fn fresh_cwd() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn build_config() -> HooConfig {
        json!({"active_mode": "build", "modes": {"build": {}}})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn gate_defers_to_the_session_rule() {
        let dir = fresh_cwd();
        session_allow_program(dir.path(), "build", "printf");
        let config = build_config();
        let allow = |command: &str| {
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &shell(command),
                ApprovalChannel::Ui,
            )
        };
        assert_eq!(allow("printf y"), Verdict::Allow);
        assert_eq!(allow("printf y | sort"), Verdict::Prompt);
        assert_eq!(allow("printf y; rm"), Verdict::Prompt);
        assert_eq!(allow("rm -rf target"), Verdict::Prompt);
        // Another cwd shares nothing.
        let elsewhere = fresh_cwd();
        assert_eq!(
            evaluate(
                &config,
                elsewhere.path(),
                "Shell",
                &shell("printf y"),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
    }

    #[test]
    fn plan_mode_always_prompts() {
        let dir = fresh_cwd();
        session_allow_program(dir.path(), "plan", "printf");
        let config = json!({"active_mode": "plan", "modes": {"plan": {}}})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &shell("printf y"),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
    }
}
