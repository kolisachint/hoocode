//! `components/tool-execution.ts`: one tool call's block in the transcript.
//!
//! The block draws a status dot, the tool's call line (its `renderCall`, or
//! the radar signal row) and, depending on the view dial, its result body
//! (`renderResult`, or a peek-trimmed fallback). Renderers come from the
//! built-in table ([`crate::tools::builtin_tool_definition`]) and the tool's
//! registered definition, slot by slot, as in the pin.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use hoocode_ai_types::Content;
use hoocode_code_tui_theme::{get_language_from_path, theme};
use hoocode_tui_components::{
    BoxComponent, ColorFn, Image, ImageOptions, ImageTheme, Spacer, Text,
};
use hoocode_tui_images::{get_capabilities, is_image_line, ImageProtocol};
use hoocode_tui_render::{fresh_stamp, Component, ComponentHandle};
use hoocode_tui_util::{
    strip_vt_control_characters, truncate_to_width, visible_width, wrap_text_with_ansi,
};
use serde_json::Value;

use crate::fold::{more_lines_hint, ELLIPSIS};
use crate::tool_chain_summary::ChainEntry;
use crate::tool_output_view::{line_budget, ToolOutputView, DEFAULT_TOOL_OUTPUT_VIEW};
use crate::tool_signal::{
    chain_subject, tool_subject, ResultHolder, ResultText, ToolResult, ToolSignalComponent,
    ToolSignalInput,
};
use crate::tools::builtin_tool_definition;

/// The exit code of a failed Shell result: our tool reports it as
/// `Command exited with code N` on the last line. Timeouts and aborts name
/// no code.
fn shell_exit_code(error_text: &str) -> Option<i32> {
    error_text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .strip_prefix("Command exited with code ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Output lines of a Shell result, ignoring the `(no output)` placeholder.
fn shell_output_lines(output: &str) -> usize {
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "(no output)" {
        0
    } else {
        trimmed.lines().count()
    }
}

/// Distinct files in CodeSearch output: its `path:start-end [sources]`
/// headers. Snippet lines are indented; notices start with `[`.
fn search_file_count(output: &str) -> usize {
    let mut files = std::collections::HashSet::new();
    for line in output.lines() {
        if line.starts_with([' ', '\t']) || line.is_empty() {
            continue;
        }
        let Some((path, rest)) = line.split_once(':') else {
            continue;
        };
        if path.is_empty() || path.starts_with('[') {
            continue;
        }
        let Some((range, sources)) = rest.split_once(' ') else {
            continue;
        };
        let Some((a, b)) = range.split_once('-') else {
            continue;
        };
        if a.parse::<u64>().is_ok() && b.parse::<u64>().is_ok() && sources.starts_with('[') {
            files.insert(path);
        }
    }
    files.len()
}

/// `ToolRenderContext`: what a renderer is told about the call.
pub struct ToolRenderContext<'a> {
    pub args: &'a Value,
    pub tool_call_id: &'a str,
    /// The component this slot returned last time, for renderers that update
    /// it in place.
    pub last_component: Option<ComponentHandle>,
    /// State shared by the call and result renderers of one block.
    pub state: &'a mut serde_json::Map<String, Value>,
    /// Renderer-owned objects shared by the slots (the pin keeps components
    /// in `state`), e.g. a call component the result slot updates.
    pub objects: &'a mut HashMap<String, Rc<dyn Any>>,
    pub cwd: &'a str,
    pub execution_started: bool,
    pub args_complete: bool,
    pub is_partial: bool,
    pub expanded: bool,
    pub show_images: bool,
    pub is_error: bool,
    /// A result (final, or a partial one) is in: the result renderer shows
    /// it, so a call renderer keeps to its header.
    pub has_result: bool,
}

/// The key a result renderer files its [`BodyLayout`] under in the block's
/// renderer objects.
const BODY_LAYOUT_KEY: &str = "bodyLayout";

/// What a result renderer says about its rows beyond the rows themselves, for
/// the chain that folds them: whether the body shows the last lines of the
/// output (a fold hint above it), and how many rows at its end are not output
/// (a `Took` line, a truncation note). The chain folds the output and leaves
/// those rows whole.
#[derive(Debug, Default)]
pub struct BodyLayout {
    /// The body shows the end of the output; its fold hint reads `earlier`.
    pub tail: bool,
    /// Rows at the end of the body that are not output.
    pub footer_rows: Cell<usize>,
}

/// The block's [`BodyLayout`], made on first use: a renderer takes it from its
/// context's objects and keeps it up to date as it renders.
pub fn body_layout(objects: &mut HashMap<String, Rc<dyn Any>>, tail: bool) -> Rc<BodyLayout> {
    if let Some(layout) = objects
        .get(BODY_LAYOUT_KEY)
        .and_then(|o| o.clone().downcast::<BodyLayout>().ok())
    {
        return layout;
    }
    let layout = Rc::new(BodyLayout {
        tail,
        footer_rows: Cell::new(0),
    });
    objects.insert(BODY_LAYOUT_KEY.into(), layout.clone());
    layout
}

/// `ToolRenderResultOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRenderResultOptions {
    pub expanded: bool,
    pub is_partial: bool,
}

/// The result as a renderer sees it.
pub struct ToolResultView<'a> {
    pub content: &'a [Content],
    pub details: &'a Value,
    /// The memo for `content`: the result's own, so every rebuild shares it.
    pub text: &'a ResultText,
}

impl ToolResultView<'_> {
    /// The result's text, as [`ToolResult::text`] gives it.
    pub fn text(&self, show_images: bool) -> &str {
        self.text.get(self.content, show_images)
    }
}

pub type RenderCallFn =
    Rc<dyn Fn(&Value, &mut ToolRenderContext<'_>) -> Result<ComponentHandle, String>>;
pub type RenderResultFn = Rc<
    dyn Fn(
        &ToolResultView<'_>,
        ToolRenderResultOptions,
        &mut ToolRenderContext<'_>,
    ) -> Result<ComponentHandle, String>,
>;

/// The rendering half of a `ToolDefinition`: each slot is optional.
#[derive(Clone, Default)]
pub struct ToolRenderDefinition {
    pub render_call: Option<RenderCallFn>,
    pub render_result: Option<RenderResultFn>,
}

/// `ToolExecutionOptions`.
#[derive(Debug, Clone, Copy)]
pub struct ToolExecutionOptions {
    pub show_images: bool,
    pub image_width_cells: u32,
    pub view: ToolOutputView,
}

impl Default for ToolExecutionOptions {
    fn default() -> Self {
        Self {
            show_images: true,
            image_width_cells: 60,
            view: DEFAULT_TOOL_OUTPUT_VIEW,
        }
    }
}

fn handle<C: Component + 'static>(c: C) -> ComponentHandle {
    Rc::new(RefCell::new(c))
}

/// Renders a child and prepends a prefix (the status dot) to its first line,
/// indenting continuation lines under the content.
struct PrefixFirstLine {
    prefix: String,
    child: ComponentHandle,
    indent_width: usize,
}

impl PrefixFirstLine {
    fn new(prefix: String, child: ComponentHandle) -> Self {
        let indent_width = visible_width(&prefix);
        Self {
            prefix,
            child,
            indent_width,
        }
    }
}

impl Component for PrefixFirstLine {
    fn render(&mut self, width: u16) -> Vec<String> {
        let child_width = (width as usize).saturating_sub(self.indent_width).max(1);
        let lines = self.child.borrow_mut().render(child_width as u16);
        if lines.is_empty() {
            return vec![self.prefix.clone()];
        }
        let indent = " ".repeat(self.indent_width);
        lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format!("{}{line}", self.prefix)
                } else {
                    format!("{indent}{line}")
                }
            })
            .collect()
    }

    fn invalidate(&mut self) {
        self.child.borrow_mut().invalidate();
    }
}

/// Whether a tool reads or writes a file: its peek block is banded.
fn is_file_tool(name: &str) -> bool {
    matches!(name, "Read" | "Write" | "Edit")
}

/// Indents every non-blank line of a child: a failure's body under a radar row.
struct IndentAll {
    child: ComponentHandle,
    indent: String,
}

impl Component for IndentAll {
    fn render(&mut self, width: u16) -> Vec<String> {
        let inner = (width as usize).saturating_sub(self.indent.len()).max(1);
        self.child
            .borrow_mut()
            .render(inner as u16)
            .into_iter()
            .map(|line| {
                if crate::is_blank(&line) {
                    line
                } else {
                    format!("{}{line}", self.indent)
                }
            })
            .collect()
    }

    fn invalidate(&mut self) {
        self.child.borrow_mut().invalidate();
    }
}

/// What a frozen block draws at each stop, captured at `frozen_width`.
struct FrozenLines {
    radar: Vec<String>,
    peek: Vec<String>,
    full: Vec<String>,
    /// The block's rows split for a chain step, at peek and at full.
    peek_parts: StepParts,
    full_parts: StepParts,
}

impl FrozenLines {
    fn for_view(&self, view: ToolOutputView) -> &[String] {
        match view {
            ToolOutputView::Radar => &self.radar,
            ToolOutputView::Peek => &self.peek,
            ToolOutputView::Full => &self.full,
        }
    }

    fn parts_for(&self, view: ToolOutputView) -> &StepParts {
        match view {
            ToolOutputView::Full => &self.full_parts,
            ToolOutputView::Radar | ToolOutputView::Peek => &self.peek_parts,
        }
    }
}

/// A block's rows split for a chain step (the peek trajectory): the call
/// header on its own, so the chain can lay it out beside the step number and
/// the stats and wrap it instead of cutting it; the body under it, which the
/// chain folds at peek and keeps whole at full; and the images, which are
/// never folded.
#[derive(Debug, Clone, Default)]
pub struct StepParts {
    /// Blank rows the block draws above itself.
    pub lead: Vec<String>,
    /// The call's first line, unwrapped, dot included. `None` when the block
    /// draws nothing.
    pub header: Option<String>,
    /// Whether the header sits on the file-tool band (`toolBandBg`).
    pub header_band: bool,
    /// The rows under the header: renderer output, at the block's width.
    pub body: Vec<String>,
    /// Whether the body shows the last lines of the output (its fold hint,
    /// when it has one, sits above the rows).
    pub tail: bool,
    /// Rows closing the body that are not output (`Took 8.2s`, a truncation
    /// note): never folded or counted.
    pub footer: Vec<String>,
    /// Image rows (or none): drawn as they are, after the body.
    pub images: Vec<String>,
}

/// Wide enough that a call's first line is not wrapped: the header is laid
/// out again by the chain.
const HEADER_WIDE: u16 = 4000;

/// The visible, non-space characters of a row: what wrapping keeps.
fn glyph_count(row: &str) -> usize {
    strip_vt_control_characters(row)
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
}

/// A frozen line laid out again for a narrower terminal. Its trailing padding
/// does not count; a line that still fits keeps its padding up to `width`
/// when `pad` is set. A longer line is cut with `…`, or at the full stop
/// wrapped, keeping its indent on every row.
fn fit_frozen_line(line: &str, width: usize, wrap: bool, pad: bool) -> Vec<String> {
    if is_image_line(line) {
        return vec![line.to_string()];
    }
    let trimmed = crate::trajectory::trim_end_styled(line);
    let used = visible_width(&trimmed);
    if used <= width {
        let fill = if pad { width - used } else { 0 };
        return vec![format!("{trimmed}{}", " ".repeat(fill))];
    }
    if !wrap {
        return vec![truncate_to_width(&trimmed, width, ELLIPSIS, false)];
    }
    let plain = strip_vt_control_characters(&trimmed);
    let indent = plain
        .chars()
        .take_while(|c| *c == ' ')
        .count()
        .min(width / 2);
    let pad_str = " ".repeat(indent);
    let body = trim_leading_spaces(&trimmed, indent);
    wrap_text_with_ansi(&body, width.saturating_sub(indent).max(1))
        .into_iter()
        .map(|row| format!("{pad_str}{row}"))
        .collect()
}

/// `line` without its first `count` visible spaces (styling in front of
/// them is kept).
fn trim_leading_spaces(line: &str, count: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut left = count;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for n in chars.by_ref() {
                out.push(n);
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c == ' ' && left > 0 {
            left -= 1;
        } else {
            out.push(c);
            out.extend(chars);
            return out;
        }
    }
    out
}

pub struct ToolExecutionComponent {
    children: Vec<ComponentHandle>,
    leading_spacer: Rc<RefCell<Spacer>>,
    content_box: Rc<RefCell<BoxComponent>>,
    content_text: Rc<RefCell<Text>>,
    call_component: Option<ComponentHandle>,
    result_component: Option<ComponentHandle>,
    renderer_state: serde_json::Map<String, Value>,
    renderer_objects: HashMap<String, Rc<dyn Any>>,
    image_handles: Vec<ComponentHandle>,
    tool_name: String,
    tool_call_id: String,
    args: Value,
    view: ToolOutputView,
    signal: Option<Rc<RefCell<ToolSignalComponent>>>,
    show_images: bool,
    image_width_cells: u32,
    is_partial: bool,
    tool_definition: Option<ToolRenderDefinition>,
    builtin_definition: Option<ToolRenderDefinition>,
    cwd: String,
    execution_started: bool,
    /// The permission gate holds this call for clearance (radar's `◉` row).
    waiting_for_clearance: bool,
    args_complete: bool,
    result: Option<Rc<ResultHolder>>,
    /// The user's abort cut this call short: it is not a failure of its own
    /// (no hit), and the abort reason is printed once, by the message.
    interrupted: bool,
    hide_component: bool,
    is_latest: bool,
    frozen: bool,
    frozen_lines: Option<FrozenLines>,
    frozen_width: usize,
    frozen_truncated: Option<(usize, ToolOutputView, Vec<String>)>,
    /// The call component the header is read from (peek and full).
    header_source: Option<ComponentHandle>,
    /// Width the frozen step parts were captured at, and the width they were
    /// last fitted to.
    frozen_parts_fit: Option<(usize, ToolOutputView, StepParts)>,
    /// The call's summary, taken when it freezes: its payloads are released then.
    frozen_entry: Option<ChainEntry>,
    /// The step's clock (M4): block creation starts it, `update_result`
    /// stops it. Rebuilt history settles at once, so its span reads ~0.
    started: Instant,
    settled_at: Option<Instant>,
    /// New whenever the block's state changes: every mutator goes through
    /// `update_display`, `freeze` or `capture_frozen` (the render stamp).
    rev: u64,
}

impl ToolExecutionComponent {
    pub fn new(
        tool_name: &str,
        tool_call_id: &str,
        args: Value,
        options: ToolExecutionOptions,
        tool_definition: Option<ToolRenderDefinition>,
        cwd: &str,
    ) -> Self {
        let leading_spacer = Rc::new(RefCell::new(Spacer::new(1)));
        let mut this = Self {
            children: vec![leading_spacer.clone()],
            leading_spacer,
            content_box: Rc::new(RefCell::new(BoxComponent::new(1, 0, None))),
            content_text: Rc::new(RefCell::new(Text::new("", 1, 0))),
            call_component: None,
            result_component: None,
            renderer_state: serde_json::Map::new(),
            renderer_objects: HashMap::new(),
            image_handles: Vec::new(),
            tool_name: tool_name.to_string(),
            tool_call_id: tool_call_id.to_string(),
            args,
            view: options.view,
            signal: None,
            show_images: options.show_images,
            image_width_cells: options.image_width_cells,
            is_partial: true,
            tool_definition,
            builtin_definition: builtin_tool_definition(tool_name),
            cwd: cwd.to_string(),
            execution_started: false,
            waiting_for_clearance: false,
            args_complete: false,
            result: None,
            interrupted: false,
            hide_component: false,
            is_latest: false,
            frozen: false,
            frozen_lines: None,
            frozen_width: 0,
            frozen_truncated: None,
            header_source: None,
            frozen_parts_fit: None,
            frozen_entry: None,
            started: Instant::now(),
            settled_at: None,
            rev: fresh_stamp(),
        };
        if this.has_renderer_definition() {
            this.children.push(this.content_box.clone());
        } else {
            this.children.push(this.content_text.clone());
        }
        this.update_display();
        this
    }

    fn call_renderer(&self) -> Option<RenderCallFn> {
        let own = self
            .tool_definition
            .as_ref()
            .and_then(|d| d.render_call.clone());
        own.or_else(|| {
            self.builtin_definition
                .as_ref()
                .and_then(|d| d.render_call.clone())
        })
    }

    fn result_renderer(&self) -> Option<RenderResultFn> {
        let own = self
            .tool_definition
            .as_ref()
            .and_then(|d| d.render_result.clone());
        own.or_else(|| {
            self.builtin_definition
                .as_ref()
                .and_then(|d| d.render_result.clone())
        })
    }

    fn has_renderer_definition(&self) -> bool {
        self.builtin_definition.is_some() || self.tool_definition.is_some()
    }

    fn create_call_fallback(&self) -> ComponentHandle {
        let t = theme();
        handle(Text::new(t.fg("toolTitle", &t.bold(&self.tool_name)), 0, 0))
    }

    /// The body for a tool with no result renderer: the same peek budget as
    /// every renderer that trims itself.
    fn create_result_fallback(&self) -> Option<ComponentHandle> {
        let output = self.text_output();
        if output.is_empty() {
            return None;
        }
        let t = theme();
        let (body, placeholders) = crate::fold::split_image_placeholders(&output);
        let lines: Vec<&str> = body.lines().collect();
        let budget = line_budget(self.expanded());
        let mut text = if lines.len() <= budget {
            t.fg("toolOutput", &body)
        } else {
            format!(
                "{}\n{}",
                t.fg("toolOutput", &lines[..budget].join("\n")),
                more_lines_hint(lines.len() - budget)
            )
        };
        if body.is_empty() {
            text.clear();
        }
        for placeholder in &placeholders {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&t.fg("toolOutput", placeholder));
        }
        Some(handle(Text::new(text, 0, 0)))
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }

    /// The call's arguments as the block shows them.
    pub fn args(&self) -> &Value {
        &self.args
    }

    pub fn update_args(&mut self, args: Value) {
        self.args = args;
        self.update_display();
    }

    pub fn mark_execution_started(&mut self) {
        self.execution_started = true;
        self.update_display();
    }

    /// Whether this call is the one a clearance card holds: the same tool,
    /// no result yet, and its subject named in the card's detail (`$ ls`).
    /// The start event lands before the gate asks, so the block already
    /// reads as started here.
    pub fn is_held_by(&self, tool: &str, detail: &str) -> bool {
        if self.tool_name != tool || self.result.is_some() || self.frozen {
            return false;
        }
        let subject = self.chain_entry().subject;
        !subject.is_empty() && detail.contains(&subject)
    }

    /// Marks the call as parked for clearance (or released from it).
    pub fn set_waiting_for_clearance(&mut self, waiting: bool) {
        if self.waiting_for_clearance != waiting {
            self.waiting_for_clearance = waiting;
            self.update_display();
        }
    }

    pub fn set_args_complete(&mut self) {
        self.args_complete = true;
        self.update_display();
    }

    pub fn update_result(&mut self, result: ToolResult, is_partial: bool) {
        self.result = Some(Rc::new(ResultHolder::new(result)));
        self.is_partial = is_partial;
        if !is_partial && self.settled_at.is_none() {
            self.settled_at = Some(Instant::now());
        }
        self.update_display();
    }

    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        self.update_display();
    }

    /// Marks the call as cut short by the user's abort. Its result stays an
    /// error to the renderer, but the step is not a hit and shows no body: the
    /// message that was aborted says so once.
    pub fn set_interrupted(&mut self) {
        self.interrupted = true;
        self.update_display();
    }

    /// Whether the user's abort cut this call short.
    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }

    /// This call's contribution to its chain's summary line.
    pub fn chain_entry(&self) -> ChainEntry {
        if let Some(entry) = &self.frozen_entry {
            return entry.clone();
        }
        let output = if self.result.is_some() {
            self.text_output()
        } else {
            String::new()
        };
        ChainEntry {
            tool: self.tool_name.clone(),
            subject: chain_subject(&self.tool_name, &self.args, &self.cwd),
            is_error: self.is_error(),
            is_partial: self.result.is_none() || self.is_partial || self.is_live_background_job(),
            output_lines: if crate::is_blank(&output) {
                0
            } else {
                // `lines` drops the trailing newline: "a\nb\n" is 2 lines.
                output.lines().count()
            },
            outcome: crate::tools::subagent::task_outcome(
                &self.tool_name,
                &self.args,
                self.result.as_deref().map(ResultHolder::result),
            ),
        }
    }

    /// A background shell job returns its id at once, and the call stays
    /// running in its chain while the job does. Only a live, tracked job reads
    /// running: its row in the task store is in progress. A replayed history
    /// has no such row, so its launch reads done; a job that has ended, or
    /// was killed, reads done too.
    pub fn is_live_background_job(&self) -> bool {
        if self.tool_name != "Shell" {
            return false;
        }
        let Some(result) = self.result.as_ref().filter(|r| !r.is_error) else {
            return false;
        };
        let Some(id) = result.result().details.get("jobId") else {
            return false;
        };
        let job_id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
        hoocode_code_task_store::task_store()
            .find_keyed(&hoocode_code_tool_bash::jobs::row_key(&job_id))
            .is_some_and(|task| task.status == hoocode_code_task_store::TaskStatus::InProgress)
    }

    /// Whether the settled result is an error (M4: the step shows the HIT
    /// chip in place of its success stats). A call the user's abort cut short
    /// is no failure of its own.
    pub fn is_error(&self) -> bool {
        !self.interrupted && self.result.as_ref().is_some_and(|r| r.is_error)
    }

    /// Whether this block is an Agent call (M4: its body keeps the violet
    /// agent rail).
    pub fn is_agent(&self) -> bool {
        self.tool_name == "Agent"
    }

    /// The pool run an Agent call dispatched, the key of its live tail: the
    /// run its result names, else the one the call remembered. `None` for
    /// other tools.
    pub fn agent_run_id(&self) -> Option<String> {
        if !self.is_agent() {
            return None;
        }
        self.result
            .as_ref()
            .and_then(|result| result.details.get("poolTaskId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| hoocode_code_subagents::live::run_for_tool_call(&self.tool_call_id))
    }

    /// The own tool calls of the subagent run this Agent call dispatched, once
    /// the run has settled (see [`hoocode_code_subagents::live::settled_steps`]).
    /// Empty for other tools and while the run is going.
    pub fn settled_sub_steps(&self) -> Vec<hoocode_code_subagents::live::SettledStep> {
        self.agent_run_id()
            .map(|run| hoocode_code_subagents::live::settled_steps(&run))
            .unwrap_or_default()
    }

    /// The background job id, while its job is still live (M4: the step
    /// shows the satellite row `⌁ sat job-1 · in orbit 4s`).
    pub fn background_job_id(&self) -> Option<String> {
        if !self.is_live_background_job() {
            return None;
        }
        self.result
            .as_ref()?
            .details
            .get("jobId")?
            .as_str()
            .map(str::to_string)
    }

    /// Whether the call is parked waiting for user clearance: the permission
    /// gate set the flag and the call has no result yet (Mission Control
    /// spec r5: the `holding for clearance` row).
    pub fn is_waiting_for_clearance(&self) -> bool {
        self.waiting_for_clearance && self.result.is_none() && !self.frozen
    }

    /// The call's expected run time in seconds, from its `timeout` arg
    /// when it carries one (Mission Control spec r4: the live row's
    /// `3s of ~6s` estimate). `None` when unknown: the row reads elapsed
    /// only.
    pub fn estimated_secs(&self) -> Option<u64> {
        let timeout = self.args.get("timeout")?;
        let secs = match timeout {
            Value::Number(n) => n.as_f64()?,
            Value::String(s) => s.trim().parse::<f64>().ok()?,
            _ => return None,
        };
        (secs.is_finite() && secs > 0.0).then(|| secs.ceil() as u64)
    }

    /// An Agent call's `(subagent_type, task)`: the type, and the
    /// `description` (else the prompt's first line), for the flight log's
    /// `◈ general-purpose · Compute a product` row (spec r7). `None` for
    /// other tools or a call that names no task.
    pub fn agent_label(&self) -> Option<(String, String)> {
        if !self.is_agent() {
            return None;
        }
        let subtype = self
            .args
            .get("subagent_type")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("agent")
            .to_string();
        let summary = ["description", "prompt"]
            .iter()
            .filter_map(|key| self.args.get(*key))
            .filter_map(Value::as_str)
            .map(|s| s.lines().next().unwrap_or("").trim().to_string())
            .find(|s| !s.is_empty())?;
        Some((subtype, summary))
    }

    /// Added and removed lines of a settled Edit diff, for the `+a −b`
    /// counts on the step header (M4). `None` for other tools, errors, or a
    /// result with no diff.
    pub fn edit_diff_stats(&self) -> Option<(usize, usize)> {
        if self.tool_name != "Edit" || self.is_error() {
            return None;
        }
        let diff = self.result.as_ref()?.details.get("diff")?.as_str()?;
        Some(crate::trajectory::diff_stats(diff))
    }

    /// The step header's right side (M4): the stats, right-aligned by the
    /// chain. A failed step shows only the `HIT` chip (plus the exit code
    /// and duration when known): it replaces the success stats, never
    /// appends to them. Empty when the step carries no stats.
    pub fn peek_right(&self, palette: &hoocode_tui_ratatui::Palette) -> String {
        let t = theme();
        let dim = |s: &str| t.fg("muted", s);
        if self.interrupted {
            return dim("interrupted");
        }
        if self.is_error() {
            let mut stats = crate::trajectory::hit_chip(palette);
            if self.tool_name == "Shell" {
                if let Some(code) = shell_exit_code(&self.text_output()) {
                    stats.push_str(&format!(" {}", dim(&format!("exit {code}"))));
                }
            }
            if let Some(dur) = self.result_duration() {
                stats.push_str(&format!(" {} {}", dim("·"), dim(&dur)));
            }
            return stats;
        }
        if let Some(job) = self.background_job_id() {
            return crate::trajectory::satellite_text(&job, self.block_elapsed_secs(), palette);
        }
        match self.tool_name.as_str() {
            "Shell" => {
                let Some(_) = self.result.as_ref() else {
                    return String::new();
                };
                let mut parts = vec![dim("exit 0")];
                let lines = shell_output_lines(&self.text_output());
                if lines > 0 {
                    parts.push(dim(&crate::trajectory::lines_text(lines)));
                }
                if let Some(dur) = self.result_duration() {
                    parts.push(dim(&dur));
                }
                parts.join(&dim(" · "))
            }
            "Read" => {
                let Some(_) = self.result.as_ref() else {
                    return String::new();
                };
                let total = self
                    .result
                    .as_ref()
                    .and_then(|r| r.details.get("truncation"))
                    .and_then(|tr| tr.get("totalLines"))
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or_else(|| self.text_output().lines().count());
                let ranged = self.args.get("offset").is_some() || self.args.get("limit").is_some();
                if !ranged {
                    return dim(&crate::trajectory::lines_text(total));
                }
                let start = self
                    .args
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1) as usize;
                let shown = if self.expanded() {
                    total
                } else {
                    total.min(crate::trajectory::TRAJECTORY_BODY_LINES)
                };
                dim(&format!(
                    "lines {start}–{} of {total}",
                    start + shown.saturating_sub(1)
                ))
            }
            "CodeSearch" => {
                let Some(result) = self.result.as_ref() else {
                    return String::new();
                };
                let hits = result
                    .details
                    .get("resultCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let mut stats = if hits == 1 {
                    "1 hit".to_string()
                } else {
                    format!("{hits} hits")
                };
                let files = search_file_count(&self.text_output());
                if files > 0 {
                    stats.push_str(&format!(" · {}", crate::trajectory::files_text(files)));
                }
                dim(&stats)
            }
            "Edit" => self
                .edit_diff_stats()
                .map(|(added, removed)| crate::trajectory::diff_counts_text(added, removed))
                .unwrap_or_default(),
            "Write" => {
                let Some(_) = self.result.as_ref() else {
                    return String::new();
                };
                let mut stats = crate::trajectory::new_chip(palette);
                let lines = self
                    .args
                    .get("content")
                    .and_then(Value::as_str)
                    .map_or(0, |c| c.lines().count());
                stats.push_str(&format!("  +{lines}"));
                let path = self
                    .args
                    .get("file_path")
                    .or_else(|| self.args.get("path"))
                    .and_then(Value::as_str);
                if let Some(lang) = path.and_then(get_language_from_path) {
                    stats.push_str(&format!(" {} {}", dim("·"), dim(lang)));
                }
                stats
            }
            "Agent" => self
                .block_elapsed_secs()
                .filter(|s| *s >= 1)
                .map(|s| dim(&crate::trajectory::short_dur(s)))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The settled result's `durationMs`, as `0.4s`. `None` while running
    /// or when the loop stored no duration.
    fn result_duration(&self) -> Option<String> {
        self.result
            .as_ref()?
            .details
            .get("durationMs")?
            .as_f64()
            .map(crate::tools::bash::format_duration)
    }

    /// Seconds since the block opened (settled at `update_result`).
    /// Rebuilt history settles at once, so its span reads ~0.
    pub(crate) fn block_elapsed_secs(&self) -> Option<u64> {
        // A live background job outlives its start (which returns at once):
        // its satellite clock keeps running until the job exits.
        let end = match self.settled_at {
            Some(settled) if !self.is_live_background_job() => settled,
            _ => Instant::now(),
        };
        end.duration_since(self.started).as_secs().into()
    }

    /// The failure text, for a collapsed chain to print under its line.
    pub fn error_text(&self) -> String {
        if self.is_error() {
            self.text_output()
        } else {
            String::new()
        }
    }

    /// Whether the block draws everything uncut: the full stop.
    fn expanded(&self) -> bool {
        self.view == ToolOutputView::Full
    }

    /// A failure always shows its reason, in every view.
    fn should_show_body(&self) -> bool {
        !self.interrupted && (self.view != ToolOutputView::Radar || self.is_error())
    }

    fn should_show_signal_line(&self) -> bool {
        self.view == ToolOutputView::Radar
    }

    /// Whether this block draws its own blank row above itself (radar rows do not).
    pub fn draws_leading_gap(&self) -> bool {
        !self.should_show_signal_line()
    }

    /// Peek indents under the tool name, past the `● ` dot; radar under its row.
    fn indent_under_signal_row(&self, component: ComponentHandle) -> ComponentHandle {
        let indent = if self.should_show_signal_line() { 3 } else { 2 };
        handle(IndentAll {
            child: component,
            indent: " ".repeat(indent),
        })
    }

    /// Mark this block as the newest call in the transcript, or no longer it.
    pub fn set_latest(&mut self, is_latest: bool) {
        if self.is_latest == is_latest {
            return;
        }
        self.is_latest = is_latest;
        self.update_display();
    }

    fn signal_component(&mut self) -> ComponentHandle {
        let input = ToolSignalInput {
            tool_name: self.tool_name.clone(),
            args: self.args.clone(),
            cwd: self.cwd.clone(),
            result: self.result.clone(),
            is_partial: self.is_partial,
            show_images: self.show_images,
            is_latest: self.is_latest,
        };
        match &self.signal {
            Some(signal) => {
                signal.borrow_mut().set_input(input);
                signal.clone()
            }
            None => {
                let signal = Rc::new(RefCell::new(ToolSignalComponent::new(input)));
                self.signal = Some(signal.clone());
                signal
            }
        }
    }

    pub fn set_show_images(&mut self, show: bool) {
        self.show_images = show;
        self.update_display();
    }

    pub fn set_image_width_cells(&mut self, width: u32) {
        self.image_width_cells = width.max(1);
        self.update_display();
    }

    /// Whether a renderer asked to be re-run every second (bash's live
    /// `Elapsed`, `setInterval(invalidate, 1000)` in the pin).
    pub fn is_ticking(&self) -> bool {
        self.renderer_state
            .contains_key(crate::tools::bash::TICKING_KEY)
    }

    /// Settled, and drawn from the result alone: nothing it shows changes with
    /// the clock, a job or a subagent run.
    fn is_settled(&self) -> bool {
        let Some(result) = self.result.as_ref() else {
            return false;
        };
        let has_job = self.tool_name == "Shell" && result.result().details.get("jobId").is_some();
        !self.is_partial
            && self.settled_at.is_some()
            && !has_job
            && !self.is_ticking()
            && !self
                .renderer_state
                .contains_key(crate::tools::subagent::LIVE_KEY)
    }

    /// A finished, visible result still holding its payloads. A block whose
    /// renderer shows a live run stays unfrozen until the run settles.
    pub fn is_freezable(&self) -> bool {
        !self.frozen
            && !self.is_partial
            && !self.hide_component
            && self.result.is_some()
            && !self
                .renderer_state
                .contains_key(crate::tools::subagent::LIVE_KEY)
    }

    /// Rebuilds the display: the renderers run again (a live run's block
    /// switches between its tail and its result when the run starts or settles).
    pub fn refresh(&mut self) {
        self.update_display();
    }

    /// Freeze on the next render: capture the lines, release the payloads.
    pub fn freeze(&mut self) {
        if self.is_freezable() {
            self.rev = fresh_stamp();
            self.frozen_entry = Some(self.chain_entry());
            self.frozen = true;
        }
    }

    fn release_heavy_state(&mut self) {
        self.result = None;
        self.image_handles.clear();
        self.renderer_state.clear();
        self.renderer_objects.clear();
        self.call_component = None;
        self.result_component = None;
        self.header_source = None;
        self.children.clear();
    }

    fn add_to_shell(&mut self, component: ComponentHandle) {
        self.content_box.borrow_mut().add_child(component);
    }

    fn context(
        &mut self,
        last: Option<ComponentHandle>,
    ) -> (ToolRenderContextOwned, Option<ComponentHandle>) {
        (
            ToolRenderContextOwned {
                execution_started: self.execution_started,
                args_complete: self.args_complete,
                is_partial: self.is_partial,
                expanded: self.expanded(),
                show_images: self.show_images,
                is_error: self.result.as_ref().is_some_and(|r| r.is_error),
                has_result: self.result.is_some(),
            },
            last,
        )
    }

    /// Runs `render` with this block's render context: its flags, args, call
    /// id, cwd and renderer state.
    fn render_with_context<R>(
        &mut self,
        last: Option<ComponentHandle>,
        render: impl FnOnce(&Value, &mut ToolRenderContext<'_>) -> R,
    ) -> R {
        let (flags, last) = self.context(last);
        let args = self.args.clone();
        let id = self.tool_call_id.clone();
        let cwd = self.cwd.clone();
        let mut ctx = flags.borrow(
            &args,
            &id,
            last,
            &mut self.renderer_state,
            &mut self.renderer_objects,
            &cwd,
        );
        render(&args, &mut ctx)
    }

    fn update_display(&mut self) {
        self.rev = fresh_stamp();
        // A frozen block's lines are captured; nothing it shows changes now.
        if self.frozen {
            return;
        }
        self.rebuild_display();
    }

    #[allow(unused_assignments)]
    fn rebuild_display(&mut self) {
        let mut has_content = false;
        self.hide_component = false;
        self.header_source = None;
        self.leading_spacer
            .borrow_mut()
            .set_lines(if self.should_show_signal_line() { 0 } else { 1 });

        if self.has_renderer_definition() {
            // Peek sits 2 columns in, past the dot; radar keeps 1.
            let padding_x = if self.should_show_signal_line() { 1 } else { 2 };
            self.content_box.borrow_mut().set_padding_x(padding_x);
            // A peek file block gets a full-width band; everything else none.
            let band_bg: Option<ColorFn> =
                if !self.should_show_signal_line() && is_file_tool(&self.tool_name) {
                    Some(Box::new(|s: &str| theme().bg("toolBandBg", s)))
                } else {
                    None
                };
            self.content_box.borrow_mut().set_bg_fn(band_bg);
            self.content_box.borrow_mut().clear();

            let t = theme();
            let dot_color = if self.interrupted {
                "dim"
            } else if self.result.as_ref().is_some_and(|r| r.is_error) {
                "error"
            } else if self.is_partial {
                "warning"
            } else {
                "success"
            };
            let dot = t.fg(dot_color, "● ");

            if self.should_show_signal_line() {
                let signal = self.signal_component();
                self.add_to_shell(handle(PrefixFirstLine::new(dot.clone(), signal)));
                has_content = true;
            } else {
                match self.call_renderer() {
                    None => {
                        let fallback = self.create_call_fallback();
                        let header = handle(PrefixFirstLine::new(dot.clone(), fallback));
                        self.header_source = Some(header.clone());
                        self.add_to_shell(header);
                        has_content = true;
                    }
                    Some(render_call) => {
                        let last = self.call_component.clone();
                        let rendered =
                            self.render_with_context(last, |args, ctx| render_call(args, ctx));
                        let component = match rendered {
                            Ok(component) => {
                                self.call_component = Some(component.clone());
                                component
                            }
                            Err(_) => {
                                self.call_component = None;
                                self.create_call_fallback()
                            }
                        };
                        let header = handle(PrefixFirstLine::new(dot.clone(), component));
                        self.header_source = Some(header.clone());
                        self.add_to_shell(header);
                        has_content = true;
                    }
                }
            }

            if self.result.is_some() && self.should_show_body() {
                match self.result_renderer() {
                    None => {
                        if let Some(component) = self.create_result_fallback() {
                            let component = self.indent_under_signal_row(component);
                            self.add_to_shell(component);
                            has_content = true;
                        }
                    }
                    Some(render_result) => {
                        let result = self.result.clone().unwrap();
                        let last = self.result_component.clone();
                        // Only the full stop opens a result: renderers trim it
                        // otherwise.
                        let options = ToolRenderResultOptions {
                            expanded: self.expanded(),
                            is_partial: self.is_partial,
                        };
                        let view = ToolResultView {
                            content: &result.content,
                            details: &result.details,
                            text: &result.memo,
                        };
                        let rendered = self
                            .render_with_context(last, |_, ctx| render_result(&view, options, ctx));
                        match rendered {
                            Ok(component) => {
                                self.result_component = Some(component.clone());
                                let component = self.indent_under_signal_row(component);
                                self.add_to_shell(component);
                                has_content = true;
                            }
                            Err(_) => {
                                self.result_component = None;
                                if let Some(component) = self.create_result_fallback() {
                                    self.add_to_shell(component);
                                    has_content = true;
                                }
                            }
                        }
                    }
                }
            }
        } else {
            self.content_text.borrow_mut().set_custom_bg_fn(None);
            let text = self.format_tool_execution();
            self.content_text.borrow_mut().set_text(text);
            has_content = true;
        }

        for image in self.image_handles.drain(..) {
            self.children.retain(|c| !Rc::ptr_eq(c, &image));
        }

        if let Some(result) = &self.result {
            if !self.should_show_signal_line() {
                let caps = get_capabilities();
                let mut added = Vec::new();
                for content in &result.content {
                    let Content::Image(img) = content else {
                        continue;
                    };
                    if caps.images.is_none()
                        || !self.show_images
                        || img.data.is_empty()
                        || img.media_type.is_empty()
                    {
                        continue;
                    }
                    // Kitty draws PNG only; the pin converts other formats in
                    // the background first. Unconverted images are skipped.
                    if caps.images == Some(ImageProtocol::Kitty) && img.media_type != "image/png" {
                        continue;
                    }
                    added.push(handle(Spacer::new(1)));
                    added.push(handle(Image::new(
                        img.data.clone(),
                        img.media_type.clone(),
                        ImageTheme {
                            fallback_color: Box::new(|s: &str| theme().fg("toolOutput", s)),
                        },
                        ImageOptions {
                            max_width_cells: Some(self.image_width_cells),
                            ..Default::default()
                        },
                        None,
                    )));
                }
                for h in added {
                    self.children.push(h.clone());
                    self.image_handles.push(h);
                }
            }
        }

        if self.has_renderer_definition() && !has_content && self.image_handles.is_empty() {
            self.hide_component = true;
        }
    }

    fn text_output(&self) -> String {
        self.result
            .as_ref()
            .map_or_else(String::new, |r| r.text(self.show_images).to_string())
    }

    /// The whole block as text, for a tool with no definition at all.
    fn format_tool_execution(&self) -> String {
        let t = theme();
        let dot_color = if self.interrupted {
            "dim"
        } else if self.result.as_ref().is_some_and(|r| r.is_error) {
            "error"
        } else if self.is_partial {
            "warning"
        } else {
            "success"
        };
        if self.should_show_signal_line() {
            let subject = tool_subject(&self.args, &self.cwd);
            return format!(
                "{}{}{}",
                t.fg(dot_color, "● "),
                t.fg("toolTitle", &t.bold(&self.tool_name)),
                if subject.is_empty() {
                    String::new()
                } else {
                    format!(" {}", t.fg("toolOutput", &subject))
                }
            );
        }
        let mut text = format!(
            "{}{}",
            t.fg(dot_color, "● "),
            t.fg("toolTitle", &t.bold(&self.tool_name))
        );
        let content = crate::js_json::stringify_pretty(&self.args);
        if !content.is_empty() {
            text.push_str(&format!("\n\n{content}"));
        }
        let output = if self.should_show_body() {
            self.text_output()
        } else {
            String::new()
        };
        if !output.is_empty() {
            text.push_str(&format!("\n{output}"));
        }
        text
    }
}

/// The flag half of a render context, captured before borrowing the block.
struct ToolRenderContextOwned {
    execution_started: bool,
    args_complete: bool,
    is_partial: bool,
    expanded: bool,
    show_images: bool,
    is_error: bool,
    has_result: bool,
}

impl ToolRenderContextOwned {
    fn borrow<'a>(
        &self,
        args: &'a Value,
        tool_call_id: &'a str,
        last_component: Option<ComponentHandle>,
        state: &'a mut serde_json::Map<String, Value>,
        objects: &'a mut HashMap<String, Rc<dyn Any>>,
        cwd: &'a str,
    ) -> ToolRenderContext<'a> {
        ToolRenderContext {
            args,
            tool_call_id,
            last_component,
            state,
            objects,
            cwd,
            execution_started: self.execution_started,
            args_complete: self.args_complete,
            is_partial: self.is_partial,
            expanded: self.expanded,
            show_images: self.show_images,
            is_error: self.is_error,
            has_result: self.has_result,
        }
    }
}

impl Component for ToolExecutionComponent {
    /// A frozen block whose lines are captured draws from the capture and its
    /// view alone; a settled block draws from the components its renderers built
    /// (the renderers are all in this crate). A live block reads clocks and the
    /// job registry, an Agent block the subagent registry.
    fn render_stamp(&self) -> Option<u64> {
        if self.is_agent() {
            return None;
        }
        let stable = if self.frozen {
            self.frozen_lines.is_some()
        } else {
            self.is_settled()
        };
        stable.then_some(self.rev)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        if self.frozen {
            return self.render_frozen(width);
        }
        if self.hide_component {
            return Vec::new();
        }
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.borrow_mut().render(width));
        }
        lines
    }

    fn invalidate(&mut self) {
        // A frozen block's snapshot is authoritative.
        if self.frozen {
            return;
        }
        self.rev = fresh_stamp();
        for child in &self.children {
            child.borrow_mut().invalidate();
        }
        self.update_display();
    }
}

impl ToolExecutionComponent {
    /// A frozen block draws the lines captured for its current stop. The first
    /// render after [`freeze`](Self::freeze) captures every stop.
    fn render_frozen(&mut self, width: u16) -> Vec<String> {
        if self.frozen_lines.is_none() {
            self.capture_frozen(width);
        }
        let w = width as usize;
        let view = self.view;
        let Some(frozen) = &self.frozen_lines else {
            return Vec::new();
        };
        let lines = frozen.for_view(view);
        // A narrower terminal lays the lines out again so none overflows it:
        // cut at peek, wrapped at full.
        if w >= self.frozen_width {
            return lines.to_vec();
        }
        if let Some((cached_width, cached_view, cached)) = &self.frozen_truncated {
            if *cached_width == w && *cached_view == view {
                return cached.clone();
            }
        }
        let wrap = view == ToolOutputView::Full;
        let fitted: Vec<String> = lines
            .iter()
            .flat_map(|line| fit_frozen_line(line, w, wrap, true))
            .collect();
        self.frozen_truncated = Some((w, view, fitted.clone()));
        fitted
    }

    /// Draws the block at every stop, keeps them, and releases the payloads.
    fn capture_frozen(&mut self, width: u16) {
        self.rev = fresh_stamp();
        let current = self.view;
        let radar = self.draw_at(ToolOutputView::Radar, width);
        let (peek, peek_parts) = self.draw_with_parts(ToolOutputView::Peek, width);
        let (full, full_parts) = self.draw_with_parts(ToolOutputView::Full, width);
        self.view = current;
        self.frozen_lines = Some(FrozenLines {
            radar,
            peek,
            full,
            peek_parts,
            full_parts,
        });
        self.frozen_width = width as usize;
        self.release_heavy_state();
    }

    /// The block's lines at `view`, drawn fresh.
    fn draw_at(&mut self, view: ToolOutputView, width: u16) -> Vec<String> {
        self.view = view;
        self.rebuild_display();
        if self.hide_component {
            return Vec::new();
        }
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.borrow_mut().render(width));
        }
        lines
    }

    /// [`draw_at`](Self::draw_at), and the same drawing split for a chain step.
    fn draw_with_parts(&mut self, view: ToolOutputView, width: u16) -> (Vec<String>, StepParts) {
        let lines = self.draw_at(view, width);
        let parts = self.build_step_parts(width);
        (lines, parts)
    }

    /// The block's rows split for a chain step (see [`StepParts`]), drawn at
    /// `width` less the two columns the trajectory rail takes beyond the
    /// block's own padding, so a body row fits on the rail whole.
    pub fn step_parts(&mut self, width: u16) -> StepParts {
        if !self.frozen {
            return self.build_step_parts(width);
        }
        if self.frozen_lines.is_none() {
            self.capture_frozen(width);
        }
        let view = self.view;
        // The rail takes four columns beyond the block's own two of padding:
        // a body row, padding included, fits `width - 4`.
        let target = (width as usize).saturating_sub(4).max(1);
        let Some(frozen) = &self.frozen_lines else {
            return StepParts::default();
        };
        let parts = frozen.parts_for(view);
        let captured = self.frozen_width.saturating_sub(4).max(1);
        if target >= captured {
            return parts.clone();
        }
        if let Some((cached_width, cached_view, cached)) = &self.frozen_parts_fit {
            if *cached_width == target && *cached_view == view {
                return cached.clone();
            }
        }
        let wrap = view == ToolOutputView::Full;
        let mut fitted = parts.clone();
        let fit = |rows: &[String]| -> Vec<String> {
            rows.iter()
                .flat_map(|line| fit_frozen_line(line, target, wrap, false))
                .collect()
        };
        fitted.body = fit(&parts.body);
        fitted.footer = fit(&parts.footer);
        self.frozen_parts_fit = Some((target, view, fitted.clone()));
        fitted
    }

    fn build_step_parts(&mut self, width: u16) -> StepParts {
        if self.hide_component {
            return StepParts::default();
        }
        let main_width = width.saturating_sub(2).max(1);
        let mut lead = Vec::new();
        let mut main = Vec::new();
        let mut images = Vec::new();
        for (index, child) in self.children.iter().enumerate() {
            match index {
                0 => lead = child.borrow_mut().render(width),
                1 => main = child.borrow_mut().render(main_width),
                _ => images.extend(child.borrow_mut().render(width)),
            }
        }
        let Some(first) = main.iter().position(|l| !crate::is_blank(l)) else {
            lead.extend(main);
            return StepParts {
                lead,
                header: None,
                header_band: false,
                body: Vec::new(),
                tail: false,
                footer: Vec::new(),
                images,
            };
        };
        // The header is the call's first line, laid out unwrapped; the rows it
        // takes in `main` are found by counting the glyphs it carries.
        let header = self
            .header_source
            .as_ref()
            .and_then(|source| {
                source
                    .borrow_mut()
                    .render(HEADER_WIDE)
                    .into_iter()
                    .find(|row| !crate::is_blank(row))
            })
            .map(|row| crate::trajectory::trim_end_styled(&row))
            .unwrap_or_else(|| crate::trajectory::trim_end_styled(&main[first]));
        let target = glyph_count(&header);
        let mut taken = 0;
        let mut seen = 0;
        for row in &main[first..] {
            if seen >= target.max(1) {
                break;
            }
            seen += glyph_count(row);
            taken += 1;
        }
        let mut body = main.split_off(first + taken);
        main.truncate(first);
        lead.extend(main);
        let layout = self
            .renderer_objects
            .get(BODY_LAYOUT_KEY)
            .and_then(|o| o.clone().downcast::<BodyLayout>().ok());
        let (tail, footer_rows) = layout.map_or((false, 0), |l| (l.tail, l.footer_rows.get()));
        let footer = body.split_off(body.len() - footer_rows.min(body.len()));
        StepParts {
            lead,
            header: Some(header),
            header_band: is_file_tool(&self.tool_name) && !self.should_show_signal_line(),
            body,
            tail,
            footer,
            images,
        }
    }
}
