//! The per-mode tool policy and the approval prompt, as a [`PermissionGate`].

mod safe_shell;
pub use safe_shell::is_safe_shell_command;

use hoocode_agent_types::{
    AgentToolCall, PermissionDecision, PermissionGate, ToolOrigin, ToolSelector,
};
use hoocode_code_modes::config::{self, HooConfig};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

/// `GATED_TOOLS`: prompted in interactive sessions, for the built-in tool (or category) of
/// that name only (`ToolOrigin::BuiltIn`). `ShellKill` stops a background job, so it asks like
/// `Shell`; `ShellOutput` only reads and is not gated. `WebFetch` is the network category: a
/// plugin's network request is asked as it (`ext_tools.rs`, once per origin). No built-in tool
/// has that name, and a plugin tool named `WebFetch` is not gated by name. `SessionSpawn` starts
/// a detached session that runs unattended, so it always asks unless `auto_allow` names it.
pub const GATED_TOOLS: [&str; 6] = [
    "Shell",
    "Write",
    "Edit",
    "WebFetch",
    "ShellKill",
    "SessionSpawn",
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

/// Whether a call is to an MCP tool, or to a plugin lifecycle tool (`PLUGIN_TOOLS`, built in).
/// Decided by origin: a model-facing name says nothing about where a tool comes from. A plugin
/// WASM tool is not one of these, so it is never gated at call level. A tier C native tool is
/// gated by its own permission hint instead (see [`native_hint_for_origin`]).
fn is_mcp_or_plugin_tool(tool_name: &str, origin: &ToolOrigin) -> bool {
    match origin {
        ToolOrigin::Mcp { .. } => true,
        ToolOrigin::BuiltIn => PLUGIN_TOOLS.contains(&tool_name),
        ToolOrigin::Plugin { .. } => false,
    }
}

/// Whether a call is to a built-in tool or category that the interactive mode prompts for.
fn is_gated_builtin(tool_name: &str, origin: &ToolOrigin) -> bool {
    *origin == ToolOrigin::BuiltIn && GATED_TOOLS.contains(&tool_name)
}

/// Whether a call to `tool_name` from `origin` is one an approval-asking client would be asked
/// about: the interactive gated tools, plus the MCP and plugin tools that rpc gates, plus tier C
/// native tools with an `act` or `dangerous` hint. Print and json run these without asking (a
/// `dangerous` native tool still blocks there); the CLI names the ones that ran.
pub fn is_approval_tool(tool_name: &str, origin: &ToolOrigin) -> bool {
    if is_gated_builtin(tool_name, origin) || is_mcp_or_plugin_tool(tool_name, origin) {
        return true;
    }
    matches!(
        native_hint_for_origin(origin),
        Some(NativeToolHint::Act | NativeToolHint::Dangerous)
    )
}

/// The permission hint of one tier C native tool (spec D2: `read`, `act`, `dangerous`).
/// Missing means [`Act`]. The gate maps it; parsing lives in `hoocode-code-plugins`.
///
/// [`Act`]: NativeToolHint::Act
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeToolHint {
    Read,
    Act,
    Dangerous,
}

/// The session's tier C native hints, keyed by (plugin, tool): origin-keyed, never name-keyed.
/// Set from the loaded plugin set whenever it changes; read by [`evaluate`].
static NATIVE_HINTS: std::sync::LazyLock<Mutex<HashMap<(String, String), NativeToolHint>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Replace the session's tier C native hints (one entry per declared native tool).
/// Called from the plugin source whenever the plugin set changes.
pub fn set_native_hints(hints: HashMap<(String, String), NativeToolHint>) {
    *NATIVE_HINTS.lock().unwrap_or_else(|e| e.into_inner()) = hints;
}

/// The tier C hint for one plugin tool, or `None` when it declares no native tool of that name.
/// Origin-keyed: the same bare name in another plugin is a different tool.
pub fn native_hint_for(plugin: &str, tool: &str) -> Option<NativeToolHint> {
    NATIVE_HINTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(plugin.to_owned(), tool.to_owned()))
        .copied()
}

/// The tier C hint for `origin`, or `None` for anything but a native plugin tool
/// (built-ins, MCP tools and WASM plugin tools have no hint).
pub fn native_hint_for_origin(origin: &ToolOrigin) -> Option<NativeToolHint> {
    match origin {
        ToolOrigin::Plugin { plugin, tool } => native_hint_for(plugin, tool),
        _ => None,
    }
}

/// The mode lists whose entries are tool selectors (`auto_allow`, `denied_tools`,
/// `enabled_tools`). A malformed entry is refused when hoocode starts (`rule_errors`) and, for
/// the restrictive lists, blocks the calls of that mode (`evaluate`).
pub const RULE_LISTS: [&str; 3] = ["auto_allow", "denied_tools", "enabled_tools"];

/// The selectors of mode `mode`'s rule list `key`, and every problem with its entries: a value
/// that is not a list, an entry that is not a string, or an entry that is not a selector. An
/// absent or null list has neither.
fn read_selectors(config: &HooConfig, mode: &str, key: &str) -> (Vec<ToolSelector>, Vec<String>) {
    let value = config::mode_config(config, mode)
        .and_then(|m| m.get(key))
        .filter(|v| !v.is_null());
    let Some(value) = value else {
        return (Vec::new(), Vec::new());
    };
    let Some(entries) = value.as_array() else {
        return (Vec::new(), vec![format!("{key} is not a list")]);
    };
    let mut selectors = Vec::new();
    let mut problems = Vec::new();
    for entry in entries {
        let parsed = match entry.as_str() {
            Some(text) => ToolSelector::parse(text).map_err(|e| format!("{key}: {e}")),
            None => Err(format!("{key} entry {entry} is not a string")),
        };
        match parsed {
            Ok(selector) => selectors.push(selector),
            Err(problem) => problems.push(problem),
        }
    }
    (selectors, problems)
}

/// The selectors of mode `mode`'s rule list `key`. `Err` names the first problem: an unreadable
/// list is never read as empty.
fn selectors(config: &HooConfig, mode: &str, key: &str) -> Result<Vec<ToolSelector>, String> {
    match read_selectors(config, mode, key) {
        (selectors, problems) if problems.is_empty() => Ok(selectors),
        (_, problems) => Err(problems[0].clone()),
    }
}

/// Whether one of `selectors` takes the tool `name` from `origin`.
fn takes(selectors: &[ToolSelector], name: &str, origin: &ToolOrigin) -> bool {
    selectors.iter().any(|s| s.matches(name, origin))
}

/// Every malformed rule of `config`, one line each (`mode "build" denied_tools: ...`). Empty when
/// every rule list reads. hoocode refuses to start while this is not empty.
pub fn rule_errors(config: &HooConfig) -> Vec<String> {
    let Some(modes) = config.get("modes").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    for mode in modes.keys() {
        for key in RULE_LISTS {
            for problem in read_selectors(config, mode, key).1 {
                errors.push(format!("mode \"{mode}\" {problem}"));
            }
        }
    }
    errors
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

/// One word of a Shell command as the parser sees it (quotes and escapes
/// removed), plus whether the shell may expand it into words that start
/// with `-`: an unquoted glob or brace (`*`, `?`, `[`, `{`) before any `/`
/// (`*` can match a file named `--pre=x`; `src/*` cannot).
#[derive(Debug, Default)]
struct Word {
    text: String,
    /// An unquoted `*?[{` came before any `/`.
    expands_to_flag: bool,
    /// The word holds a `/` so far.
    slash: bool,
}

impl Word {
    fn push(&mut self, c: char, quoted: bool) {
        if c == '/' {
            self.slash = true;
        }
        if !quoted && !self.slash && matches!(c, '*' | '?' | '[' | '{') {
            self.expands_to_flag = true;
        }
        self.text.push(c);
    }
}

/// Ends the current word, if any.
fn push_word(words: &mut Vec<Word>, current: &mut Word) {
    if !current.text.is_empty() {
        words.push(std::mem::take(current));
    }
}

/// The pipe segments of a Shell command: words per segment, plus the write
/// redirect targets. Quotes and backslash escapes hold words together;
/// `||`, `&&`, `;`, `<` and `(` end a segment without joining a pipe.
fn split_segments(command: &str) -> (Vec<Vec<String>>, Vec<String>, bool) {
    let (segments, redirects, piped) = split_words(command);
    let segments = segments
        .into_iter()
        .map(|words| words.into_iter().map(|w| w.text).collect())
        .collect();
    (segments, redirects, piped)
}

/// [`split_segments`] with each word's [`Word`] marks.
fn split_words(command: &str) -> (Vec<Vec<Word>>, Vec<String>, bool) {
    let mut segments: Vec<Vec<Word>> = vec![Vec::new()];
    let mut redirects = Vec::new();
    let mut current = Word::default();
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
            current.push(c, true);
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
                current.push(c, true);
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
                    current.push(c, false);
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
                current.push(c, false);
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
fn take_redirect(redirects: &mut Vec<String>, pending: &mut bool, current: &mut Word) {
    if *pending && !current.text.is_empty() {
        redirects.push(std::mem::take(current).text);
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
    /// A detached run (SA-12): nobody is here to answer now. Every rule is as
    /// `Headless { fail_closed: true }`, except that a gated call that needs
    /// approval parks the run ([`Verdict::Park`]) instead of being blocked.
    Detached,
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
#[derive(Debug, Clone)]
pub struct RelayRequest {
    /// The subagent the call comes from (`explore`), when it did not
    /// start in the asking process. `None` for the process's own calls.
    pub via: Option<String>,
    pub tool: String,
    /// Where the tool comes from; the asking process gates on it, not on the name.
    pub origin: ToolOrigin,
    pub input: Value,
    /// Set once nobody waits for the answer any more (the subagent ended or
    /// was cancelled). The asker then stops waiting and the call stays denied.
    pub cancelled: Arc<AtomicBool>,
}

/// A shared tier C helper, as a child's EnsureHelper answer carries it:
/// the socket path plus the helper's resolved executable (missing when an
/// old parent answered without it).
pub type EnsureHelperAnswer = Result<(String, Option<String>), String>;

/// Where a gate that cannot prompt sends a call that needs approval: a
/// subagent child sends it up to its parent, and the parent's own gate (its
/// config rules, its session grants, its card) decides.
pub trait ClearanceRelay: Send + Sync {
    /// Blocks until the call is decided. `None` lets it run, `Some(reason)`
    /// blocks it. Any failure to ask (no UI, a closed pipe) is a denial.
    fn ask(&self, request: &RelayRequest) -> Option<String>;
    /// Asks the parent for a shared tier C helper (PL-18 slice 4b): the
    /// parent ensures its own connection, registers the asking agent, and
    /// answers with the socket path plus the helper's executable (missing
    /// when an old parent answers). Only a child's relay (a pipe to a
    /// parent that runs the pool) can answer; every other relay reports
    /// that there is no parent channel.
    fn ask_ensure(&self, _plugin: &str) -> EnsureHelperAnswer {
        Err("Shared tier C tools need the parent session's helper: no parent channel in this process.".to_string())
    }
    /// Whether [`ask_ensure`](Self::ask_ensure) can reach a parent: true only
    /// for a child's relay. A headless fail-closed child of an rpc root and a
    /// warm worker have no parent channel, so their relays always fail closed;
    /// callers check this instead of treating any set relay as a channel.
    fn can_ensure(&self) -> bool {
        false
    }
}

/// The relay of a headless json or print run that has subagents. Nobody can
/// prompt there, so a subagent's call that needs approval is decided by the
/// run's own gate: the same rules as its own calls, and the same headless
/// default (`fail_closed` denies with the gate's reason, otherwise it runs).
/// A call the run's rules would deny is therefore denied for the subagent too.
pub struct HeadlessClearanceRelay {
    gate: HooPermissionGate,
}

impl HeadlessClearanceRelay {
    /// The relay for a run in `cwd`, failing closed when `fail_closed` is set.
    pub fn new(cwd: PathBuf, fail_closed: bool) -> Self {
        Self::for_channel(cwd, ApprovalChannel::Headless { fail_closed })
    }

    /// The relay for a detached run in `cwd`: a subagent's call that needs approval is denied.
    pub fn detached(cwd: PathBuf) -> Self {
        Self::for_channel(cwd, ApprovalChannel::Detached)
    }

    fn for_channel(cwd: PathBuf, channel: ApprovalChannel) -> Self {
        Self {
            gate: HooPermissionGate::new(cwd, channel, None),
        }
    }
}

impl ClearanceRelay for HeadlessClearanceRelay {
    fn ask(&self, request: &RelayRequest) -> Option<String> {
        // A subagent's call cannot park the run: it has no id, and no one would answer.
        self.gate.decide(
            ParkSource::Subagent,
            &request.tool,
            &request.origin,
            &request.input,
        )
    }
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
    // `.hoocode/../src/x` would match `.hoocode/*` but write outside it.
    if Path::new(file_path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
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

/// `file_path` (relative to `cwd`) lexically, with `.`/`..` folded.
fn lexical_abs(cwd: &Path, file_path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in cwd.join(file_path).components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Whether a symlink inside the project sends a write to `file_path`
/// somewhere other than where its path says (`.hoocode/link/x` with
/// `link -> ../src`), so a path rule cannot be trusted.
fn resolves_elsewhere(cwd: &Path, file_path: &str) -> bool {
    let target = lexical_abs(cwd, file_path);
    let Ok(rel) = target.strip_prefix(cwd) else {
        // Outside the project: the path rules already judged the text.
        return false;
    };
    let Ok(real_cwd) = cwd.canonicalize() else {
        return false;
    };
    // The deepest part of the path that exists, resolved, plus the rest.
    let mut existing = target.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if std::fs::symlink_metadata(&existing).is_ok() {
            break;
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return false,
        }
    }
    let Ok(mut resolved) = existing.canonicalize() else {
        // A dangling symlink: it would create a file at its target.
        return true;
    };
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    resolved != real_cwd.join(rel)
}

/// Whether `file_path` is one of hoocode's config files directly under the
/// project's `.hoocode/` (`hoo-config.json`, `settings.json`, ...).
fn is_project_config_file(cwd: &Path, file_path: &str) -> bool {
    if file_path.is_empty() {
        return false;
    }
    let target = lexical_abs(cwd, file_path);
    let config_dir = cwd.join(hoocode_code_paths::CONFIG_DIR_NAME);
    target.parent() == Some(config_dir.as_path())
        && target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
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
    if !redirects_are_harmless(command, &redirects) {
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

/// Whether a command's redirects write no file: each target is `/dev/null`
/// or a descriptor dup (`&1`, `&2`, `&-`; `>&name` writes the file `name`),
/// and there is no clobbering `>|` (which the segment split reads as a pipe).
pub(crate) fn redirects_are_harmless(command: &str, redirects: &[String]) -> bool {
    if command.contains(">|") {
        return false;
    }
    redirects.iter().all(|target| {
        target == "/dev/null"
            || target.strip_prefix('&').is_some_and(|fd| {
                fd == "-" || (!fd.is_empty() && fd.chars().all(|c| c.is_ascii_digit()))
            })
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
    describe_tool_inner(tool_name, input)
}

/// [`describe_tool`], naming the plugin and tool for a tier C native call so the card says
/// which plugin's tool is asking (D2).
fn describe_tool_with_origin(tool_name: &str, origin: &ToolOrigin, input: &Value) -> String {
    if let ToolOrigin::Plugin { plugin, tool: own } = origin {
        if native_hint_for(plugin, own).is_some() {
            return format!("plugin {plugin} tool {own}");
        }
    }
    describe_tool_inner(tool_name, input)
}

/// The approval detail of a `SessionSpawn` call (`SessionSpawn` in the clearance card): the
/// brief, then what runs unattended in this mode, then the checkout it works in. `unattended`
/// is the mode's `auto_allow` entries; empty reads as "nothing".
pub fn describe_session_spawn(input: &Value, cwd: &Path, unattended: &[String]) -> String {
    let brief = input
        .get("brief")
        .and_then(Value::as_str)
        .unwrap_or("(unknown)");
    let runs = if unattended.is_empty() {
        "nothing".to_string()
    } else {
        unattended.join(", ")
    };
    format!(
        "{}\nRuns unattended: {runs}; any other call that needs approval waits until you open the session.\nIt works in this checkout ({}); if you keep editing the same files, the two sessions can collide.",
        brief.trim(),
        cwd.display()
    )
}

/// The `auto_allow` entries of `mode` as written in the config: the tools that run unattended.
fn auto_allow_entries(config: &HooConfig, mode: &str) -> Vec<String> {
    config::mode_config(config, mode)
        .and_then(|m| m.get("auto_allow"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The clearance detail for a call that needs approval. `SessionSpawn` names the mode's
/// unattended tools and the checkout, which the input alone does not say.
fn gate_detail(
    tool_name: &str,
    origin: &ToolOrigin,
    input: &Value,
    cwd: &Path,
    config: &HooConfig,
    mode: &str,
) -> String {
    if tool_name == "SessionSpawn" && *origin == ToolOrigin::BuiltIn {
        return describe_session_spawn(input, cwd, &auto_allow_entries(config, mode));
    }
    describe_tool_with_origin(tool_name, origin, input)
}

fn describe_tool_inner(tool_name: &str, input: &Value) -> String {
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("(unknown)")
            .to_string()
    };
    match tool_name {
        // Without the mode and checkout, only the brief; the gate adds the rest.
        "SessionSpawn" => field("brief"),
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
    /// The call needs approval in a detached run: the run parks on it (see
    /// [`ParkRequest`]) and the call does not run.
    Park,
}

/// A gated call that a detached run cannot have answered: the run parks on it.
#[derive(Debug, Clone)]
pub struct ParkRequest {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
    /// Why the call parks, for the user who opens the session.
    pub reason: String,
}

/// What a detached run does with a parked call (see [`HooPermissionGate::set_park_handler`]).
pub type ParkHandler = Arc<dyn Fn(ParkRequest) + Send + Sync>;

/// The reason a subagent's call gets in a detached run: its clearance cannot wait for a user.
pub const DETACHED_SUBAGENT_PARK_REASON: &str = "A detached session cannot ask the user for a subagent's call. Add the tool to auto_allow, or run it in the parent.";

/// Where a call that needs approval came from. Only the agent loop's per-call check may park a
/// detached run: it has a call id the session can record and resume on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParkSource<'a> {
    /// The agent loop's per-call check, with the call's id.
    Loop(&'a str),
    /// A subagent's clearance relay: it has no id and no one to answer.
    Subagent,
    /// A request a running tool makes for itself (a plugin's network request).
    InTool,
}

/// The hoocode `tool_call` handler up to the prompt, over a merged config. `origin` says where
/// the tool comes from; the built-in rules (Shell's command patterns, the write paths) apply to
/// the built-in tool of that name only.
pub fn evaluate(
    config: &HooConfig,
    cwd: &Path,
    tool_name: &str,
    origin: &ToolOrigin,
    input: &Value,
    channel: ApprovalChannel,
) -> Verdict {
    let mode = config::active_mode(config).unwrap_or_else(|| "build".into());
    let builtin = *origin == ToolOrigin::BuiltIn;
    let shell = builtin && tool_name == "Shell";
    let writes = builtin && (tool_name == "Write" || tool_name == "Edit");
    let list = |key: &str| config::mode_list(config, &mode, key);

    // An unreadable deny or allow-list blocks: a malformed deny would otherwise deny nothing.
    let denied = match selectors(config, &mode, "denied_tools") {
        Err(problem) => {
            return Verdict::Block(format!(
                "Tool \"{tool_name}\" is blocked: mode \"{mode}\" has a malformed denied_tools rule ({problem}). Fix it in hoo-config.json."
            ))
        }
        Ok(denied) => {
            if takes(&denied, tool_name, origin) {
                return Verdict::Block(format!(
                    "Tool \"{tool_name}\" is denied in mode \"{mode}\"."
                ));
            }
            denied
        }
    };
    match selectors(config, &mode, "enabled_tools") {
        Err(problem) => {
            return Verdict::Block(format!(
                "Tool \"{tool_name}\" is blocked: mode \"{mode}\" has a malformed enabled_tools rule ({problem}). Fix it in hoo-config.json."
            ))
        }
        Ok(enabled) if !enabled.is_empty() && !takes(&enabled, tool_name, origin) => {
            return Verdict::Block(format!(
                "Tool \"{tool_name}\" is not enabled in mode \"{mode}\" (enabled: {}).",
                list("enabled_tools").unwrap_or_default().join(", ")
            ))
        }
        Ok(_) => {}
    }
    if shell {
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

    // The tier C hint for a plugin tool, looked up by plugin+tool (origin-keyed, never
    // name-keyed): `read` runs like `Read`, `act` asks like an edit/write built-in under the
    // current mode, `dangerous` always asks unless `auto_allow` names the tool.
    let native_hint = native_hint_for_origin(origin);
    // A native `act` tool writes the way Write/Edit do: where the mode's
    // denied_tools denies Write or Edit, the `act` tool is denied the same
    // way (ask/debug modes deny both by default). Elsewhere it behaves as
    // now: asked, like an edit/write built-in.
    if native_hint == Some(NativeToolHint::Act)
        && (takes(&denied, "Write", &ToolOrigin::BuiltIn)
            || takes(&denied, "Edit", &ToolOrigin::BuiltIn))
    {
        return Verdict::Block(format!(
            "Tool \"{tool_name}\" is denied in mode \"{mode}\" (it acts, and the mode denies Write/Edit)."
        ));
    }
    let dangerous = native_hint == Some(NativeToolHint::Dangerous);
    let fail_closed = match channel {
        ApprovalChannel::Ui => false,
        // print/json: nobody is asked, and the CLI prints one notice per run. A `dangerous`
        // native tool still blocks there unless `auto_allow` names it, so it is treated as
        // fail-closed even in print/json.
        ApprovalChannel::Headless { fail_closed: false } if !dangerous => return Verdict::Allow,
        ApprovalChannel::Headless { fail_closed } => fail_closed || dangerous,
        ApprovalChannel::Detached => true,
    };
    // MCP and plugin lifecycle tools are gated only where nobody can answer (rpc). A UI still
    // runs them without a prompt, as before. A plugin WASM tool is never gated here: its network
    // requests are asked as `WebFetch` by the plugin host. A native `read` tool is ungated like
    // `Read`; `act` and `dangerous` are gated.
    let gated = is_gated_builtin(tool_name, origin)
        || (fail_closed && is_mcp_or_plugin_tool(tool_name, origin))
        || matches!(
            native_hint,
            Some(NativeToolHint::Act | NativeToolHint::Dangerous)
        );
    if !gated {
        return Verdict::Allow;
    }
    if writes {
        if let Some(allowed) = list("allowed_write_paths") {
            let file_path = mutation_path(input).unwrap_or_default();
            if !matches_allowed_path(&file_path, &allowed, cwd)
                || resolves_elsewhere(cwd, &file_path)
            {
                return Verdict::Block(format!(
                    "Mode \"{mode}\" only allows writes to: {}. Attempted to {tool_name}: {file_path}. Switch to \"/mode build\" to modify source files.",
                    allowed.join(", ")
                ));
            }
        }
        // hoocode's own project config (`.hoocode/hoo-config.json`,
        // `.hoocode/settings.json`, ...) can switch modes or run commands:
        // a write to it always asks, even with `skip_permissions` or
        // `auto_allow`.
        if is_project_config_file(cwd, &mutation_path(input).unwrap_or_default()) {
            if fail_closed {
                return Verdict::Block(format!(
                    "Tool \"{tool_name}\" may not change hoocode's project config without approval, and no client can answer in this session."
                ));
            }
            return Verdict::Prompt;
        }
    }
    // `skip_permissions` (config or `--skip-permissions`): nothing is
    // asked. The mode rules above still hold — except a `dangerous`
    // native tool, which always asks (or is blocked where nobody can
    // answer) unless `auto_allow` names it.
    if config::skip_permissions(config) && native_hint != Some(NativeToolHint::Dangerous) {
        return Verdict::Allow;
    }
    // `safe_shell`: a read-only command runs in every mode without asking.
    if shell
        && config::safe_shell(config)
        && is_safe_shell_command(input.get("command").and_then(Value::as_str).unwrap_or(""))
    {
        return Verdict::Allow;
    }
    // An unreadable allow-list allows nothing; the start refusal says why.
    if let Ok(allow) = selectors(config, &mode, "auto_allow") {
        if takes(&allow, tool_name, origin) {
            return Verdict::Allow;
        }
    }
    // A program the session always-approved (`a` on the clearance card)
    // runs without asking, when the whole command stays inside the
    // approval. Never consulted in PLAN mode. Anything else falls
    // through to a prompt.
    if shell
        && !mode.trim().eq_ignore_ascii_case("plan")
        && always_allows(
            input.get("command").and_then(Value::as_str).unwrap_or(""),
            &session_programs(cwd, &mode),
        )
    {
        return Verdict::Allow;
    }
    if fail_closed {
        if channel == ApprovalChannel::Detached {
            return Verdict::Park;
        }
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
    /// What a detached run does with a call it parks on. Set once, after the
    /// session exists (the gate is built before it); unset means such a call is blocked.
    park: OnceLock<ParkHandler>,
}

impl HooPermissionGate {
    pub fn new(cwd: PathBuf, channel: ApprovalChannel, ui: Option<Arc<dyn PermissionUi>>) -> Self {
        Self {
            cwd,
            channel,
            ui,
            relay: None,
            park: OnceLock::new(),
        }
    }

    /// Sets what a detached run does when a call parks: called with the call,
    /// after which the call stays blocked. Only the first handler is kept.
    pub fn set_park_handler(&self, handler: ParkHandler) {
        let _ = self.park.set(handler);
    }

    /// A gate that hands every call needing approval to `relay`. Use it with
    /// [`ApprovalChannel::Ui`], so such calls reach the prompt step.
    pub fn with_relay(mut self, relay: Arc<dyn ClearanceRelay>) -> Self {
        self.relay = Some(relay);
        self
    }

    /// The handler for a call to `tool_name` from `origin`: `None` lets it run, `Some(reason)`
    /// blocks it.
    pub fn check_call(
        &self,
        tool_name: &str,
        origin: &ToolOrigin,
        input: &Value,
    ) -> Option<String> {
        // No call id here: a call that parks in a detached run is reported with an empty id.
        self.decide(ParkSource::Loop(""), tool_name, origin, input)
    }

    /// [`check_call`](Self::check_call) for one call. `source` says where the call came from;
    /// only [`ParkSource::Loop`] may park a detached run.
    fn decide(
        &self,
        source: ParkSource<'_>,
        tool_name: &str,
        origin: &ToolOrigin,
        input: &Value,
    ) -> Option<String> {
        let merged = config::read_merged_config(&self.cwd);
        match evaluate(&merged, &self.cwd, tool_name, origin, input, self.channel) {
            Verdict::Allow => None,
            Verdict::Block(reason) => Some(reason),
            Verdict::Park => {
                let call_id = match source {
                    ParkSource::Loop(call_id) => call_id,
                    ParkSource::Subagent => return Some(DETACHED_SUBAGENT_PARK_REASON.into()),
                    ParkSource::InTool => {
                        let mode = config::active_mode(&merged).unwrap_or_else(|| "build".into());
                        let detail =
                            gate_detail(tool_name, origin, input, &self.cwd, &merged, &mode);
                        return Some(format!(
                            "A detached session cannot ask for {detail}. Add it to auto_allow, or run this in an interactive session."
                        ));
                    }
                };
                let mode = config::active_mode(&merged).unwrap_or_else(|| "build".into());
                let reason = format!(
                    "Tool \"{tool_name}\" needs approval in mode \"{mode}\"; this detached run stops until the user answers."
                );
                let Some(handler) = self.park.get() else {
                    return Some(format!(
                        "Tool \"{tool_name}\" needs approval, and this run cannot wait for an answer. Add it to auto_allow for mode \"{mode}\" in hoo-config.json to allow it."
                    ));
                };
                handler(ParkRequest {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    input: input.clone(),
                    reason: reason.clone(),
                });
                Some(reason)
            }
            Verdict::Prompt => {
                if let Some(relay) = self.relay.as_ref() {
                    return relay.ask(&RelayRequest {
                        via: None,
                        tool: tool_name.to_string(),
                        origin: origin.clone(),
                        input: input.clone(),
                        cancelled: Arc::new(AtomicBool::new(false)),
                    });
                }
                let Some(ui) = self.ui.as_ref() else {
                    let mode = config::active_mode(&merged).unwrap_or_else(|| "build".into());
                    return Some(format!(
                        "Denied by permission gate: {tool_name} needs approval in mode \"{mode}\", but no approval prompt is available in this session. Ask the user to allow it, or use a read-only alternative."
                    ));
                };
                let mode = config::active_mode(&merged).unwrap_or_else(|| "build".into());
                let info = ClearanceInfo {
                    tool: tool_name.to_string(),
                    detail: gate_detail(tool_name, origin, input, &self.cwd, &merged, &mode),
                    mode,
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
                        let shell = *origin == ToolOrigin::BuiltIn && tool_name == "Shell";
                        if shell && !mode.trim().eq_ignore_ascii_case("plan") {
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
                        } else if shell {
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
        self.decide_call(ParkSource::Loop(&tool_call.id), tool_call)
    }

    fn request_in_tool(&self, tool_call: &AgentToolCall) -> PermissionDecision {
        self.decide_call(ParkSource::InTool, tool_call)
    }
}

impl HooPermissionGate {
    fn decide_call(&self, source: ParkSource<'_>, tool_call: &AgentToolCall) -> PermissionDecision {
        match self.decide(
            source,
            &tool_call.name,
            &tool_call.origin,
            &tool_call.arguments,
        ) {
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
        session_allow_program(dir.path(), "build", "make");
        let config = build_config();
        let allow = |command: &str| {
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell(command),
                ApprovalChannel::Ui,
            )
        };
        assert_eq!(allow("make y"), Verdict::Allow);
        assert_eq!(allow("make y | sort"), Verdict::Prompt);
        assert_eq!(allow("make y; rm"), Verdict::Prompt);
        assert_eq!(allow("rm -rf target"), Verdict::Prompt);
        // Another cwd shares nothing.
        let elsewhere = fresh_cwd();
        assert_eq!(
            evaluate(
                &config,
                elsewhere.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell("make y"),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
    }

    #[test]
    fn plan_mode_always_prompts() {
        let dir = fresh_cwd();
        session_allow_program(dir.path(), "plan", "make");
        let config = json!({"active_mode": "plan", "modes": {"plan": {}}})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell("make y"),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
    }

    #[test]
    fn detached_parks_a_gated_call_that_needs_approval() {
        let dir = fresh_cwd();
        let config = build_config();
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell("touch parked_probe.txt"),
                ApprovalChannel::Detached,
            ),
            Verdict::Park
        );
    }

    #[test]
    fn detached_runs_an_auto_allowed_call() {
        let dir = fresh_cwd();
        let config = json!({"active_mode": "build", "modes": {"build": {"auto_allow": ["Shell"]}}})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell("touch parked_probe.txt"),
                ApprovalChannel::Detached,
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn detached_blocks_a_denied_call_instead_of_parking_it() {
        let dir = fresh_cwd();
        let config =
            json!({"active_mode": "build", "modes": {"build": {"denied_tools": ["Shell"]}}})
                .as_object()
                .unwrap()
                .clone();
        assert!(matches!(
            evaluate(
                &config,
                dir.path(),
                "Shell",
                &ToolOrigin::BuiltIn,
                &shell("touch parked_probe.txt"),
                ApprovalChannel::Detached,
            ),
            Verdict::Block(_)
        ));
    }

    #[test]
    fn detached_runs_a_read_only_call() {
        let dir = fresh_cwd();
        let config = build_config();
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Read",
                &ToolOrigin::BuiltIn,
                &json!({"path": "a.txt"}),
                ApprovalChannel::Detached,
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn detached_in_tool_request_is_denied_and_never_parks() {
        let dir = fresh_cwd();
        let gate =
            HooPermissionGate::new(dir.path().to_path_buf(), ApprovalChannel::Detached, None);
        let parks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = parks.clone();
        gate.set_park_handler(Arc::new(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        // A plugin's network request, made from inside a running tool.
        let in_tool = AgentToolCall {
            id: "ext-net-1".into(),
            name: "WebFetch".into(),
            arguments: json!({"url": "https://example.com/a"}),
            origin: ToolOrigin::BuiltIn,
        };
        match gate.request_in_tool(&in_tool) {
            PermissionDecision::Deny { reason } => assert_eq!(
                reason,
                "A detached session cannot ask for WebFetch https://example.com/a. Add it to auto_allow, or run this in an interactive session."
            ),
            other => panic!("expected a denial, got {other:?}"),
        }
        assert_eq!(parks.load(std::sync::atomic::Ordering::SeqCst), 0);
        // The agent loop's per-call check parks on the call's id.
        let looped = AgentToolCall {
            id: "call-7".into(),
            name: "Shell".into(),
            arguments: shell("touch parked_probe.txt"),
            origin: ToolOrigin::BuiltIn,
        };
        assert!(matches!(
            gate.request(&looped),
            PermissionDecision::Deny { .. }
        ));
        assert_eq!(parks.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Serializes the tier C hint tests: they share the process-wide hint map.
    static NATIVE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn native_guard(
        hints: Vec<((&str, &str), NativeToolHint)>,
    ) -> std::sync::MutexGuard<'static, ()> {
        let guard = NATIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_native_hints(
            hints
                .into_iter()
                .map(|((plugin, tool), hint)| ((plugin.to_owned(), tool.to_owned()), hint))
                .collect(),
        );
        guard
    }

    fn plugin_origin(plugin: &str, tool: &str) -> ToolOrigin {
        ToolOrigin::Plugin {
            plugin: plugin.to_owned(),
            tool: tool.to_owned(),
        }
    }

    #[test]
    fn native_read_runs_without_a_prompt_like_read() {
        let _guard = native_guard(vec![(("nread", "Lookup"), NativeToolHint::Read)]);
        let dir = fresh_cwd();
        let config = build_config();
        let origin = plugin_origin("nread", "Lookup");
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Lookup",
                &origin,
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Allow
        );
        // Even where rpc gates MCP tools, a `read` native tool still runs.
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Lookup",
                &origin,
                &json!({}),
                ApprovalChannel::Headless { fail_closed: true },
            ),
            Verdict::Allow
        );
        assert!(!is_approval_tool("Lookup", &origin));
    }

    #[test]
    fn native_act_asks_like_an_edit_write_builtin() {
        let _guard = native_guard(vec![(("nact", "Apply"), NativeToolHint::Act)]);
        let dir = fresh_cwd();
        let config = build_config();
        let origin = plugin_origin("nact", "Apply");
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
        // print/json (fail-open): runs without asking, like Edit.
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Headless { fail_closed: false },
            ),
            Verdict::Allow
        );
        // rpc (fail-closed): blocked unless `auto_allow` names it.
        assert!(matches!(
            evaluate(
                &config,
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Headless { fail_closed: true },
            ),
            Verdict::Block(_)
        ));
        assert!(is_approval_tool("Apply", &origin));
    }

    #[test]
    fn native_act_auto_allow_by_selector_or_name() {
        let _guard = native_guard(vec![(("nact", "Apply"), NativeToolHint::Act)]);
        let dir = fresh_cwd();
        let origin = plugin_origin("nact", "Apply");
        for allow in ["plugin:nact/Apply", "Apply"] {
            let config = json!({
                "modes": {"build": {"auto_allow": [allow]}}
            })
            .as_object()
            .unwrap()
            .clone();
            assert_eq!(
                evaluate(
                    &config,
                    dir.path(),
                    "Apply",
                    &origin,
                    &json!({}),
                    ApprovalChannel::Headless { fail_closed: true },
                ),
                Verdict::Allow,
                "auto_allow {allow:?} should allow the act tool"
            );
        }
    }

    #[test]
    fn native_dangerous_always_asks_unless_auto_allow_names_it() {
        let _guard = native_guard(vec![(("ndgr", "Wipe"), NativeToolHint::Dangerous)]);
        let dir = fresh_cwd();
        let config = build_config();
        let origin = plugin_origin("ndgr", "Wipe");
        // Interactive: asked.
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Wipe",
                &origin,
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
        // Headless fail-open (print/json): still blocked, since nobody can answer.
        assert!(matches!(
            evaluate(
                &config,
                dir.path(),
                "Wipe",
                &origin,
                &json!({}),
                ApprovalChannel::Headless { fail_closed: false },
            ),
            Verdict::Block(_)
        ));
        assert!(is_approval_tool("Wipe", &origin));
        // `auto_allow` naming the tool (selector or bare name) allows it everywhere.
        for allow in ["plugin:ndgr/Wipe", "Wipe"] {
            let config = json!({
                "modes": {"build": {"auto_allow": [allow]}}
            })
            .as_object()
            .unwrap()
            .clone();
            for channel in [
                ApprovalChannel::Ui,
                ApprovalChannel::Headless { fail_closed: false },
                ApprovalChannel::Headless { fail_closed: true },
            ] {
                assert_eq!(
                    evaluate(&config, dir.path(), "Wipe", &origin, &json!({}), channel),
                    Verdict::Allow,
                    "auto_allow {allow:?} should allow the dangerous tool"
                );
            }
        }
    }

    #[test]
    fn native_dangerous_ignores_skip_permissions_unless_auto_allow() {
        let _guard = native_guard(vec![
            (("ndgr", "Wipe"), NativeToolHint::Dangerous),
            (("nact", "Apply"), NativeToolHint::Act),
        ]);
        let dir = fresh_cwd();
        let origin = plugin_origin("ndgr", "Wipe");
        let skipped = json!({
            "active_mode": "build",
            "modes": {"build": {}},
            "skip_permissions": true,
        })
        .as_object()
        .unwrap()
        .clone();
        // Interactive: asked, not allowed.
        assert_eq!(
            evaluate(
                &skipped,
                dir.path(),
                "Wipe",
                &origin,
                &json!({}),
                ApprovalChannel::Ui
            ),
            Verdict::Prompt
        );
        // Headless fail-open (print/json): blocked, not allowed.
        assert!(matches!(
            evaluate(
                &skipped,
                dir.path(),
                "Wipe",
                &origin,
                &json!({}),
                ApprovalChannel::Headless { fail_closed: false },
            ),
            Verdict::Block(_)
        ));
        // A non-dangerous tool still passes via `skip_permissions`.
        assert_eq!(
            evaluate(
                &skipped,
                dir.path(),
                "Apply",
                &plugin_origin("nact", "Apply"),
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Allow
        );
        // `auto_allow` naming the dangerous tool allows it again, everywhere.
        let allowed = json!({
            "active_mode": "build",
            "modes": {"build": {"auto_allow": ["Wipe"]}},
            "skip_permissions": true,
        })
        .as_object()
        .unwrap()
        .clone();
        for channel in [
            ApprovalChannel::Ui,
            ApprovalChannel::Headless { fail_closed: false },
            ApprovalChannel::Headless { fail_closed: true },
        ] {
            assert_eq!(
                evaluate(&allowed, dir.path(), "Wipe", &origin, &json!({}), channel),
                Verdict::Allow,
                "auto_allow should allow the dangerous tool under skip_permissions"
            );
        }
    }

    #[test]
    fn native_act_follows_write_edit_denials() {
        let _guard = native_guard(vec![(("nact", "Apply"), NativeToolHint::Act)]);
        let dir = fresh_cwd();
        let origin = plugin_origin("nact", "Apply");
        // A mode denying Write denies the act tool the same way.
        let denied_write = json!({
            "active_mode": "ask",
            "modes": {"ask": {"denied_tools": ["Write"]}},
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(matches!(
            evaluate(
                &denied_write,
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Ui
            ),
            Verdict::Block(_)
        ));
        // Same for Edit.
        let denied_edit = json!({
            "active_mode": "ask",
            "modes": {"ask": {"denied_tools": ["Edit"]}},
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(matches!(
            evaluate(
                &denied_edit,
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Ui
            ),
            Verdict::Block(_)
        ));
        // A mode denying neither still asks, like an edit/write built-in.
        assert_eq!(
            evaluate(
                &build_config(),
                dir.path(),
                "Apply",
                &origin,
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
    }

    #[test]
    fn native_hints_are_origin_keyed_never_name_keyed() {
        let _guard = native_guard(vec![
            (("alpha", "Tool"), NativeToolHint::Read),
            (("beta", "Tool"), NativeToolHint::Dangerous),
        ]);
        let dir = fresh_cwd();
        let config = build_config();
        // Same bare name, different plugins: different hints.
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Tool",
                &plugin_origin("alpha", "Tool"),
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Allow
        );
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Tool",
                &plugin_origin("beta", "Tool"),
                &json!({}),
                ApprovalChannel::Ui,
            ),
            Verdict::Prompt
        );
        // A WASM plugin tool with no hint is never gated, even sharing the name.
        assert_eq!(
            evaluate(
                &config,
                dir.path(),
                "Tool",
                &plugin_origin("gamma", "Tool"),
                &json!({}),
                ApprovalChannel::Headless { fail_closed: true },
            ),
            Verdict::Allow
        );
    }

    /// A relay that cannot answer EnsureHelper says so: the headless
    /// fail-closed relay of an rpc root's child has no parent channel, and
    /// the trait default refuses too, so only an explicit opt-in (the
    /// subagents child relay) counts as a channel.
    #[test]
    fn relays_without_a_parent_channel_cannot_ensure() {
        let headless = HeadlessClearanceRelay::new(std::path::PathBuf::from("/tmp"), true);
        assert!(
            !ClearanceRelay::can_ensure(&headless),
            "a headless fail-closed child has no parent channel"
        );
        struct NoChannel;
        impl ClearanceRelay for NoChannel {
            fn ask(&self, _request: &RelayRequest) -> Option<String> {
                None
            }
        }
        assert!(
            !ClearanceRelay::can_ensure(&NoChannel),
            "the default relay cannot ensure"
        );
    }

    #[test]
    fn session_spawn_detail_names_the_brief_unattended_tools_and_checkout() {
        let input = json!({"brief": "## Goal\nship it\n"});
        let detail = describe_session_spawn(
            &input,
            Path::new("/work/repo"),
            &["Shell".to_string(), "Read".to_string()],
        );
        assert_eq!(
            detail,
            "## Goal\nship it\nRuns unattended: Shell, Read; any other call that needs approval waits until you open the session.\nIt works in this checkout (/work/repo); if you keep editing the same files, the two sessions can collide."
        );
        let none = describe_session_spawn(&input, Path::new("/w"), &[]);
        assert!(none.contains("Runs unattended: nothing;"), "{none}");
    }

    #[test]
    fn session_spawn_is_a_gated_tool() {
        assert!(is_approval_tool("SessionSpawn", &ToolOrigin::BuiltIn));
        assert!(!is_approval_tool(
            "SessionSpawn",
            &plugin_origin("p", "SessionSpawn")
        ));
    }

    #[test]
    fn native_prompt_card_names_the_plugin_and_tool() {
        let _guard = native_guard(vec![(("browser", "Open"), NativeToolHint::Act)]);
        let origin = plugin_origin("browser", "Open");
        let detail = describe_tool_with_origin("Open", &origin, &json!({}));
        assert!(
            detail.contains("browser") && detail.contains("Open"),
            "the card must name the plugin and tool: {detail:?}"
        );
        // A built-in keeps its argument subject.
        assert_eq!(
            describe_tool_with_origin("Edit", &ToolOrigin::BuiltIn, &json!({"path": "a.rs"})),
            "Edit a.rs"
        );
    }
}
