//! `components/tool-execution.ts`: one tool call's block in the transcript.
//!
//! The block draws a status dot, the tool's call line (its `renderCall`, or
//! the outline signal row) and, depending on the view dial, its result body
//! (`renderResult`, or a peek-trimmed fallback). Renderers come from the
//! built-in table ([`crate::tools::builtin_tool_definition`]) and the tool's
//! registered definition, slot by slot, as in the pin.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use hoocode_ai_types::Content;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::{BoxComponent, Image, ImageOptions, ImageTheme, Spacer, Text};
use hoocode_tui_images::{get_capabilities, is_image_line, ImageProtocol};
use hoocode_tui_render::{fresh_stamp, Component, ComponentHandle};
use hoocode_tui_util::{
    strip_vt_control_characters, truncate_to_width, visible_width, wrap_text_with_ansi,
};
use serde_json::Value;

use crate::fold::{line_budget, more_lines_hint, ELLIPSIS};
use crate::tool_row::Mark;
use crate::tool_signal::{chain_subject, ResultHolder, ResultText, ToolResult};
use crate::tools::subagent::TaskOutcome;

/// One tool call's contribution to its chain, and to the transcript's
/// mission flight log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainEntry {
    pub tool: String,
    /// The call's subject — a path, a command, a pattern.
    pub subject: String,
    pub is_error: bool,
    /// Still executing.
    pub is_partial: bool,
    /// Lines of text output the call returned.
    pub output_lines: usize,
    /// What a subagent call said about its task; `None` for other tools and
    /// for calls that report no outcome (a background placeholder, a roster).
    pub outcome: Option<TaskOutcome>,
}
use crate::tools::builtin_tool_definition;
use hoocode_code_agent_session::format::format_tokens;
use hoocode_tui_ratatui::core::text::Span;

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

/// The first error-like line of failed Shell output, dimmed after `exit N`:
/// the tool's own `Command exited with code N` trailer is never the reason.
/// Falls back to the first output line when no line reads like an error.
fn shell_failure_reason(output: &str) -> Option<String> {
    let mut first: Option<String> = None;
    for line in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("Command exited with code") {
            continue;
        }
        if first.is_none() {
            first = Some(line.to_string());
        }
        let lower = line.to_lowercase();
        if ["error", "fail", "fatal", "panic"]
            .iter()
            .any(|w| lower.contains(w))
        {
            return Some(line.to_string());
        }
    }
    first
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

/// How `attachToolBlock` makes one: images on or off.
#[derive(Debug, Clone, Copy)]
pub struct ToolExecutionOptions {
    pub show_images: bool,
    pub image_width_cells: u32,
}

impl Default for ToolExecutionOptions {
    fn default() -> Self {
        Self {
            show_images: true,
            image_width_cells: 60,
        }
    }
}

fn handle<C: Component + 'static>(c: C) -> ComponentHandle {
    Rc::new(RefCell::new(c))
}

/// Renders a child and prepends a prefix (the status glyph) to its first line.
/// Continuation lines hang under the content: as far in as the prefix is wide,
/// or one column further for a call line ([`PrefixFirstLine::hanging`]), so
/// with the block's one column of padding they sit at the body column, 4.
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

    /// A call line: continuation lines start at the body column.
    fn hanging(prefix: String, child: ComponentHandle) -> Self {
        let mut line = Self::new(prefix, child);
        line.indent_width += 1;
        line
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

/// A running Shell's single dim tail line while collapsed (single view):
/// its last output line, dim, truncated to one row. With no output yet,
/// the dim `○` stands in, mirroring [`crate::bash_execution`]'s pulse
/// fallback. The chain's indent wrapper puts the row at column 4.
struct RunningTailLine {
    line: String,
}

impl RunningTailLine {
    fn new(line: String) -> Self {
        Self { line }
    }
}

impl Component for RunningTailLine {
    fn render(&mut self, width: u16) -> Vec<String> {
        let t = theme();
        let content_width = (width as usize).max(1);
        let trimmed = self.line.trim();
        if trimmed.is_empty() {
            return vec![t.fg("dim", hoocode_tui_glyphs::status::TODO_STR)];
        }
        let cut = truncate_to_width(trimmed, content_width, "\u{2026}", false);
        vec![t.fg("dim", &cut)]
    }

    fn invalidate(&mut self) {}
}

/// Indents every non-blank line of a child: a failure's body under a outline row.
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

/// What a frozen block draws, captured at `frozen_width`.
#[derive(Debug, Clone, Default)]
struct FrozenLines {
    lines: Vec<String>,
    /// The block's rows split for a chain step.
    parts: StepParts,
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
    /// The call's first line, unwrapped, without its glyph: the bold tool name
    /// and the subject. `None` when the block draws nothing.
    pub header: Option<String>,
    /// The rows under the header: renderer output, at the block's width,
    /// already at body indent (column 4).
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
    show_images: bool,
    image_width_cells: u32,
    is_partial: bool,
    tool_definition: Option<ToolRenderDefinition>,
    builtin_definition: Option<ToolRenderDefinition>,
    cwd: String,
    execution_started: bool,
    /// The permission gate holds this call for clearance (outline's `◉` row).
    waiting_for_clearance: bool,
    args_complete: bool,
    result: Option<Rc<ResultHolder>>,
    /// The user's abort cut this call short: it is not a failure of its own
    /// (no hit), and the abort reason is printed once, by the message.
    interrupted: bool,
    hide_component: bool,
    /// The single-view toggle: the body opens in place on click or Enter.
    /// In memory only (a resumed session opens collapsed); survives resizes,
    /// theme changes and rewraps.
    expanded: bool,
    /// The header's unstyled state, taken at freeze while the payloads are
    /// still in. The chain builds the header from it on every render, in
    /// the live palette.
    snapshot: Option<RowSnapshot>,
    frozen: bool,
    frozen_lines: Option<FrozenLines>,
    frozen_width: usize,
    frozen_truncated: Option<(usize, Vec<String>)>,
    /// The call component the header line is read from (peek and full).
    header_source: Option<ComponentHandle>,
    /// Width the frozen step parts were captured at, and the width they were
    /// last fitted to.
    frozen_parts_fit: Option<(usize, StepParts)>,
    /// The call's summary, taken when it freezes: its payloads are released then.
    frozen_entry: Option<ChainEntry>,
    /// A failed call's first error line, kept when it freezes: the result is
    /// released then, and the outline's HIT row still reads its snippet.
    frozen_error: String,
    /// The step's clock (M4): block creation starts it, `update_result`
    /// stops it. Rebuilt history settles at once, so its span reads ~0.
    started: Instant,
    settled_at: Option<Instant>,
    /// New whenever the block's state changes: every mutator goes through
    /// `update_display`, `freeze` or `capture_frozen` (the render stamp).
    rev: u64,
}

/// The tone of one fact segment: plain text, a dim fact, a green
/// count, or a red one. Stored unstyled so a frozen row renders in the
/// current palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactTone {
    /// Plain text, no style (the space inside `+2 −2`).
    Plain,
    /// A dim fact (`30 lines`, `8.2s`).
    Dim,
    /// A green count (`+2`).
    Ok,
    /// A red count or exit (`−2`, `exit 1`).
    Err,
}

/// One styled segment of a fact (`+2`, `exit 1`, `8.2s`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSegment {
    pub text: String,
    pub tone: FactTone,
}

impl FactSegment {
    fn span(&self, palette: &hoocode_tui_ratatui::Palette) -> Span<'static> {
        match self.tone {
            FactTone::Plain => Span::raw(self.text.clone()),
            FactTone::Dim => Span::styled(self.text.clone(), palette.dim()),
            FactTone::Ok => Span::styled(self.text.clone(), palette.ok()),
            FactTone::Err => Span::styled(self.text.clone(), palette.err()),
        }
    }
}

/// One fact on the row: segments joined without a separator (`+2 −2`).
pub type FactRun = Vec<FactSegment>;

/// A frozen row's header, taken at `ToolExecutionComponent::freeze` while the payloads
/// are still in: the mark, target and facts the settled call showed.
/// The chain builds the header from it on every render, in the live
/// palette, so frozen rows survive resizes, rewraps and theme changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSnapshot {
    pub mark: Mark,
    pub tool: String,
    pub target: String,
    /// Facts the cut order never drops (`exit 1`, `+2 −2`).
    pub primary: Vec<FactRun>,
    /// Facts the cut order drops first (`8.2s`, `3 files`).
    pub secondary: Vec<FactRun>,
    pub is_error: bool,
    pub interrupted: bool,
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
            expanded: false,
            snapshot: None,
            frozen: false,
            frozen_lines: None,
            frozen_width: 0,
            frozen_truncated: None,
            header_source: None,
            frozen_parts_fit: None,
            frozen_entry: None,
            frozen_error: String::new(),
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
        // A frozen call has released its result: it reads the flag taken
        // at freeze.
        if self.frozen {
            return self.snapshot.as_ref().is_some_and(|s| s.is_error);
        }
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
    /// spec r5: the `waiting for permission` row).
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

    /// The glyph of the call's row: green `●` done, accent `◉` running, red
    /// `⊗` failed, dim `⊗` stopped. A frozen call has released its result:
    /// it reads the mark taken at freeze.
    pub fn mark(&self) -> Mark {
        if self.frozen {
            if let Some(snapshot) = &self.snapshot {
                return snapshot.mark;
            }
        }
        if self.interrupted {
            Mark::Stopped
        } else if self.result.as_ref().is_some_and(|r| r.is_error) {
            Mark::Failed
        } else if self.result.is_none() || self.is_partial {
            Mark::Running
        } else {
            Mark::Done
        }
    }

    /// The facts after the subject (`30 lines`, `+2 −2`, `exit 0 · 8.2s`):
    /// dim, a zero value left out. A failed shell call shows `exit N` in red
    /// first. Empty when the call has no fact to give yet. A frozen call
    /// reads the facts taken at freeze, styled in the live palette.
    pub fn facts(&self, palette: &hoocode_tui_ratatui::Palette) -> Vec<Span<'static>> {
        use crate::tool_row::join_facts;
        let (primary, secondary) = match &self.snapshot {
            Some(snapshot) if self.frozen => (snapshot.primary.clone(), snapshot.secondary.clone()),
            _ => self.fact_runs(),
        };
        let groups: Vec<Vec<Span<'static>>> = primary
            .into_iter()
            .chain(secondary)
            .map(|run| run.into_iter().map(|seg| seg.span(palette)).collect())
            .filter(|spans: &Vec<Span<'static>>| !spans.is_empty())
            .collect();
        join_facts(groups, palette)
    }

    /// The single-view header's structured parts: the target after the tool
    /// name, the primary facts (never dropped), the secondary facts
    /// (dropped first) and a failed Shell row's dim reason (cut, never
    /// wrapped). [`Self::fact_runs`] records the reason as the first
    /// secondary run of a failed Shell row; it is split out here so the
    /// chain's cut order can treat it apart. Read-only: fact content is
    /// unchanged, for live and frozen rows alike.
    pub fn header_parts(&self) -> (String, Vec<String>, Vec<String>, Option<String>) {
        // The Agent row names the subtype, as the call line does
        // (`Agent explore · Trace escaped quotes`): the `Agent` word comes
        // from the tool name, the rest is the label.
        let target = if self.is_agent() {
            let subtype = self
                .args
                .get("subagent_type")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("agent")
                .to_string();
            let resume = if self.args.get("resume_task_id").is_some() {
                "resume "
            } else {
                ""
            };
            match self.agent_label() {
                Some((_, summary)) => format!("{resume}{subtype} · {summary}"),
                None => format!("{resume}{subtype}"),
            }
        } else {
            chain_subject(&self.tool_name, &self.args, &self.cwd)
        };
        // Mirrors [`Self::facts`]: a frozen call has released its payloads,
        // so the facts come from the capture taken at freeze.
        let (primary_runs, secondary_runs) = match &self.snapshot {
            Some(snapshot) if self.frozen => (snapshot.primary.clone(), snapshot.secondary.clone()),
            _ => self.fact_runs(),
        };
        let flatten = |runs: Vec<FactRun>| -> Vec<String> {
            runs.into_iter()
                .map(|run| run.into_iter().map(|seg| seg.text).collect::<String>())
                .filter(|text| !text.trim().is_empty())
                .collect()
        };
        let primary = flatten(primary_runs);
        let mut secondary = flatten(secondary_runs);
        let mut reason = None;
        if self.tool_name == "Shell" && self.is_error() && !secondary.is_empty() {
            // Live, the output still in tells whether a reason was found;
            // frozen, the payloads are released, but `freeze` kept the
            // first error line, which is non-empty exactly when a reason
            // was recorded first.
            let recorded = if self.frozen {
                !self.frozen_error.trim().is_empty()
            } else {
                shell_failure_reason(&self.text_output()).is_some()
            };
            if recorded {
                reason = Some(secondary.remove(0));
            }
        }
        (target, primary, secondary, reason)
    }

    /// The row's facts unstyled: the primary facts (never dropped by the cut
    /// order) and the secondary ones (dropped first). [`Self::facts`] styles
    /// them; [`Self::row_snapshot`] keeps them for frozen rows.
    fn fact_runs(&self) -> (Vec<FactRun>, Vec<FactRun>) {
        use crate::tool_row::{lines_fact, short_dur};
        let dim_run = |text: String| -> Option<FactRun> {
            (!text.is_empty()).then(|| {
                vec![FactSegment {
                    text,
                    tone: FactTone::Dim,
                }]
            })
        };
        if self.interrupted {
            return (
                dim_run("stopped".to_string()).into_iter().collect(),
                Vec::new(),
            );
        }
        if self.is_error() {
            let mut primary = Vec::new();
            let mut secondary = Vec::new();
            if self.tool_name == "Shell" {
                if let Some(code) = shell_exit_code(&self.text_output()) {
                    primary.push(vec![FactSegment {
                        text: format!("exit {code}"),
                        tone: FactTone::Err,
                    }]);
                }
                // The reason first, then the size: the duration drops first.
                if let Some(reason) = shell_failure_reason(&self.text_output()) {
                    secondary.push(vec![FactSegment {
                        text: reason,
                        tone: FactTone::Dim,
                    }]);
                }
                if let Some(size) = dim_run(lines_fact(shell_output_lines(&self.text_output()))) {
                    secondary.push(size);
                }
            }
            if let Some(dur) = dim_run(self.result_duration().unwrap_or_default()) {
                secondary.push(dur);
            }
            return (primary, secondary);
        }
        if let Some(job) = self.background_job_id() {
            let mut text = format!("background {job}");
            if let Some(secs) = self.block_elapsed_secs().filter(|s| *s >= 1) {
                text.push_str(&format!(" · {}", short_dur(secs)));
            }
            return (dim_run(text).into_iter().collect(), Vec::new());
        }
        let settled = self.result.is_some() && !self.is_partial;
        match self.tool_name.as_str() {
            // `exit 0 · size · duration`: the duration drops first.
            "Shell" if settled => {
                let mut secondary = Vec::new();
                if let Some(size) = dim_run(lines_fact(shell_output_lines(&self.text_output()))) {
                    secondary.push(size);
                }
                if let Some(dur) = dim_run(self.result_duration().unwrap_or_default()) {
                    secondary.push(dur);
                }
                (
                    vec![vec![FactSegment {
                        text: "exit 0".to_string(),
                        tone: FactTone::Dim,
                    }]],
                    secondary,
                )
            }
            "Read" if settled => {
                let truncation = self
                    .result
                    .as_ref()
                    .and_then(|r| r.details.get("truncation"));
                let count = |key: &str| {
                    truncation
                        .and_then(|tr| tr.get(key))
                        .and_then(Value::as_u64)
                        .map(|n| n as usize)
                };
                let text_lines = shell_output_lines(&self.text_output());
                let ranged = self.args.get("offset").is_some() || self.args.get("limit").is_some();
                if !ranged {
                    return (
                        dim_run(lines_fact(count("totalLines").unwrap_or(text_lines)))
                            .into_iter()
                            .collect(),
                        Vec::new(),
                    );
                }
                let start = self
                    .args
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1) as usize;
                let read = count("outputLines").unwrap_or(text_lines);
                if read == 0 {
                    return (Vec::new(), Vec::new());
                }
                (
                    dim_run(format!("lines {start}–{}", start + read - 1))
                        .into_iter()
                        .collect(),
                    Vec::new(),
                )
            }
            "CodeSearch" if settled => {
                let hits = self
                    .result
                    .as_ref()
                    .and_then(|r| r.details.get("resultCount"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let files = search_file_count(&self.text_output());
                let primary = dim_run(match hits {
                    0 => String::new(),
                    1 => "1 hit".to_string(),
                    n => format!("{n} hits"),
                })
                .into_iter()
                .collect();
                let secondary = dim_run(match files {
                    0 => String::new(),
                    1 => "1 file".to_string(),
                    n => format!("{n} files"),
                })
                .into_iter()
                .collect();
                (primary, secondary)
            }
            "Edit" => match self.edit_diff_stats() {
                Some((added, removed)) => {
                    let mut run = Vec::new();
                    if added > 0 {
                        run.push(FactSegment {
                            text: format!("+{added}"),
                            tone: FactTone::Ok,
                        });
                    }
                    if removed > 0 {
                        if !run.is_empty() {
                            run.push(FactSegment {
                                text: " ".to_string(),
                                tone: FactTone::Plain,
                            });
                        }
                        run.push(FactSegment {
                            // U+2212, as `diff_facts` draws it.
                            text: format!("−{removed}"),
                            tone: FactTone::Err,
                        });
                    }
                    if run.is_empty() {
                        (Vec::new(), Vec::new())
                    } else {
                        (vec![run], Vec::new())
                    }
                }
                None => (Vec::new(), Vec::new()),
            },
            "Write" if settled => {
                let lines = self
                    .args
                    .get("content")
                    .and_then(Value::as_str)
                    .map_or(0, |c| c.lines().count());
                if lines == 0 {
                    (Vec::new(), Vec::new())
                } else {
                    (
                        vec![vec![FactSegment {
                            text: format!("+{lines}"),
                            tone: FactTone::Ok,
                        }]],
                        Vec::new(),
                    )
                }
            }
            "Agent" => {
                if settled {
                    // `3 calls · 1 failed`: the run's own calls, failed only
                    // when any failed. Empty while the run is unknown.
                    let mut primary = Vec::new();
                    let steps = self.settled_sub_steps();
                    if !steps.is_empty() {
                        let calls = match steps.len() {
                            1 => "1 call".to_string(),
                            n => format!("{n} calls"),
                        };
                        primary.push(vec![FactSegment {
                            text: calls,
                            tone: FactTone::Dim,
                        }]);
                        let failed = steps.iter().filter(|s| s.failure.is_some()).count();
                        if failed > 0 {
                            primary.push(vec![FactSegment {
                                text: format!("{failed} failed"),
                                tone: FactTone::Dim,
                            }]);
                        }
                    }
                    return (
                        primary,
                        dim_run(self.result_duration().unwrap_or_default())
                            .into_iter()
                            .collect(),
                    );
                }
                (
                    dim_run(
                        self.block_elapsed_secs()
                            .filter(|s| *s >= 1)
                            .map(short_dur)
                            .unwrap_or_default(),
                    )
                    .into_iter()
                    .collect(),
                    Vec::new(),
                )
            }
            // The plan the call landed, as the header shows it.
            "TodoWrite" if settled => {
                let todos: &[Value] = self
                    .args
                    .get("todos")
                    .and_then(Value::as_array)
                    .map_or(&[], Vec::as_slice);
                let done = todos
                    .iter()
                    .filter(|t| t.get("status").and_then(Value::as_str) == Some("completed"))
                    .count();
                let items = match todos.len() {
                    1 => "1 item".to_owned(),
                    n => format!("{n} items"),
                };
                (
                    vec![vec![FactSegment {
                        text: format!("{items} · {done} done"),
                        tone: FactTone::Dim,
                    }]],
                    dim_run(self.result_duration().unwrap_or_default())
                        .into_iter()
                        .collect(),
                )
            }
            // `~8.1k tokens`: the hidden size the collapsed row stands for.
            "WebFetch" | "WebSearch" if settled => {
                let tokens = self
                    .result
                    .as_ref()
                    .and_then(|r| r.details.get("tokenEstimate"))
                    .and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_f64().map(|f| f.max(0.0).round() as u64))
                    })
                    .filter(|&n| n > 0)
                    .map(|n| format!("~{} tokens", format_tokens(n)));
                (
                    dim_run(tokens.unwrap_or_default()).into_iter().collect(),
                    dim_run(self.result_duration().unwrap_or_default())
                        .into_iter()
                        .collect(),
                )
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// The header's unstyled state, for [`Self::freeze`]: the mark, target
    /// and facts the settled call shows, while the payloads are still in.
    fn row_snapshot(&self) -> RowSnapshot {
        let (primary, secondary) = self.fact_runs();
        RowSnapshot {
            mark: self.mark(),
            tool: self.tool_name.clone(),
            target: crate::tool_signal::chain_subject(&self.tool_name, &self.args, &self.cwd),
            primary,
            secondary,
            is_error: self.is_error(),
            interrupted: self.interrupted,
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
        // its clock keeps running until the job exits.
        let end = match self.settled_at {
            Some(settled) if !self.is_live_background_job() => settled,
            _ => Instant::now(),
        };
        end.duration_since(self.started).as_secs().into()
    }

    /// The failure text, for a collapsed chain to print under its line. A frozen
    /// call has released its result: it reads the first line kept at freeze.
    pub fn error_text(&self) -> String {
        if self.frozen {
            return self.frozen_error.clone();
        }
        if self.is_error() {
            self.text_output()
        } else {
            String::new()
        }
    }

    /// Whether this block's body is open in the transcript. One toggle per
    /// block, in memory only: a resumed session opens collapsed, and the
    /// flag survives resizes, theme changes and rewraps.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Open or close this block's body. Failed rows settle collapsed and
    /// never open themselves: only a toggle opens them.
    pub fn set_expanded(&mut self, expanded: bool) {
        if self.expanded == expanded {
            return;
        }
        self.expanded = expanded;
        self.rev = fresh_stamp();
        if !self.frozen {
            self.update_display();
        }
    }

    /// Flip this block's toggle.
    pub fn toggle_expanded(&mut self) {
        self.set_expanded(!self.expanded);
    }

    /// Whether the block draws everything uncut: its own toggle, open in
    /// place (single view: every block starts as one row).
    fn expanded(&self) -> bool {
        self.expanded
    }

    /// Whether the block draws its body at all: an open toggle, or a
    /// running call whose live tail the collapsed row keeps (Decision 4).
    /// A failure settles collapsed like every other call.
    fn should_show_body(&self) -> bool {
        if self.interrupted {
            return false;
        }
        if self.expanded() {
            return true;
        }
        self.result.is_none() || self.is_partial || self.is_live_background_job()
    }

    /// Whether this block draws its own blank row above itself.
    pub fn draws_leading_gap(&self) -> bool {
        true
    }

    /// A body sits at column 4: the block's one column of padding plus these
    /// three.
    fn indent_under_signal_row(&self, component: ComponentHandle) -> ComponentHandle {
        handle(IndentAll {
            child: component,
            indent: " ".repeat(3),
        })
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
    pub(crate) fn is_settled(&self) -> bool {
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
    /// Whether the block's payloads are released and its rows come from
    /// the captures at freeze.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn freeze(&mut self) {
        if self.is_freezable() {
            self.rev = fresh_stamp();
            self.snapshot = Some(self.row_snapshot());
            self.frozen_entry = Some(self.chain_entry());
            self.frozen_error = self
                .error_text()
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_string();
            self.frozen = true;
        }
    }

    /// The revision of a frozen block, which keys it for a holder that reads
    /// its rows. `None` while the block is not frozen.
    pub(crate) fn frozen_rev(&self) -> Option<u64> {
        self.frozen.then_some(self.rev)
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

    /// The glyph and the space after it, themed for the block's state.
    fn mark_prefix(&self) -> String {
        let t = theme();
        let (glyph, color) = match self.mark() {
            Mark::Done => (hoocode_tui_glyphs::status::DONE_STR, "success"),
            Mark::Running => (hoocode_tui_glyphs::status::DONE_STR, "dim"),
            Mark::Failed => (hoocode_tui_glyphs::status::FAILED_STR, "error"),
            Mark::Stopped => (hoocode_tui_glyphs::status::FAILED_STR, "dim"),
        };
        format!("{} ", t.fg(color, glyph))
    }

    /// A running Shell's tail line while collapsed: its last output line
    /// (empty when nothing has streamed yet, for the `○` fallback).
    /// Shell-only and partial: settled and open rows keep the result body
    /// (or none) as before.
    fn running_shell_tail(&self) -> Option<String> {
        if self.tool_name != "Shell" {
            return None;
        }
        if !self.is_partial {
            return None;
        }
        if self.interrupted {
            return None;
        }
        if self.expanded() {
            return None;
        }
        let result = self.result.as_ref()?;
        if result.is_error {
            return None;
        }
        let output = self.text_output();
        let last = output
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        Some(last)
    }

    #[allow(unused_assignments)]
    fn rebuild_display(&mut self) {
        let mut has_content = false;
        self.hide_component = false;
        self.header_source = None;
        self.leading_spacer.borrow_mut().set_lines(1);

        if self.has_renderer_definition() {
            // One column in, then the glyph: ` ● Name subject`. Nothing has a
            // fill but the user's band, so a tool block has no background.
            self.content_box.borrow_mut().set_padding_x(1);
            self.content_box.borrow_mut().set_bg_fn(None);
            self.content_box.borrow_mut().clear();

            let dot = self.mark_prefix();

            match self.call_renderer() {
                None => {
                    let fallback = self.create_call_fallback();
                    let header = handle(PrefixFirstLine::hanging(dot.clone(), fallback.clone()));
                    self.header_source = Some(fallback);
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
                    let header = handle(PrefixFirstLine::hanging(dot.clone(), component.clone()));
                    self.header_source = Some(component);
                    self.add_to_shell(header);
                    has_content = true;
                }
            }

            // The result renderer always runs: its state (a live run's
            // tail, a ticking clock) is read even while the body stays shut.
            // Only an open toggle attaches what it drew.
            if self.result.is_some() {
                // A running Shell's collapsed row keeps the canonical
                // `RunningTailLine` below: the result body stays shut so the
                // tail renders once, not twice (w-tail + w-wire overlap).
                let show_body = self.should_show_body() && self.running_shell_tail().is_none();
                match self.result_renderer() {
                    None => {
                        if show_body {
                            if let Some(component) = self.create_result_fallback() {
                                let component = self.indent_under_signal_row(component);
                                self.add_to_shell(component);
                                has_content = true;
                            }
                        }
                    }
                    Some(render_result) => {
                        let result = self.result.clone().unwrap();
                        let last = self.result_component.clone();
                        // Only an open toggle shows a result whole:
                        // renderers trim it otherwise.
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
                                if show_body {
                                    let component = self.indent_under_signal_row(component);
                                    self.add_to_shell(component);
                                    has_content = true;
                                }
                            }
                            Err(_) => {
                                self.result_component = None;
                                if show_body {
                                    if let Some(component) = self.create_result_fallback() {
                                        self.add_to_shell(component);
                                        has_content = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // A running Shell stays one header plus one dim tail line while
            // collapsed: the live output's last line, or the `○` stand-in
            // with nothing yet. Expanded rows keep the result body instead.
            if let Some(tail) = self.running_shell_tail() {
                let component = self.indent_under_signal_row(handle(RunningTailLine::new(tail)));
                self.add_to_shell(component);
                has_content = true;
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
            {
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
        let dot = self.mark_prefix();
        let mut text = format!("{}{}", dot, t.fg("toolTitle", &t.bold(&self.tool_name)));
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
        // A frozen block's snapshot is unstyled, so a theme change is free:
        // bump the stamp, leave the released payloads alone.
        self.rev = fresh_stamp();
        if self.frozen {
            return;
        }
        for child in &self.children {
            child.borrow_mut().invalidate();
        }
        self.update_display();
    }
}

impl ToolExecutionComponent {
    /// A frozen block draws its captured lines. The first render after
    /// [`freeze`](Self::freeze) captures them.
    fn render_frozen(&mut self, width: u16) -> Vec<String> {
        if self.frozen_lines.is_none() {
            self.capture_frozen(width);
        }
        let w = width as usize;
        let Some(frozen) = &self.frozen_lines else {
            return Vec::new();
        };
        let lines = &frozen.lines;
        // A narrower terminal lays the lines out again so none overflows
        // it: frozen rows are cut, never wrapped.
        if w >= self.frozen_width {
            return lines.to_vec();
        }
        if let Some((cached_width, cached)) = &self.frozen_truncated {
            if *cached_width == w {
                return cached.clone();
            }
        }
        let fitted: Vec<String> = lines
            .iter()
            .flat_map(|line| fit_frozen_line(line, w, false, true))
            .collect();
        self.frozen_truncated = Some((w, fitted.clone()));
        fitted
    }

    /// Draws the block, keeps the drawing, and releases the payloads.
    fn capture_frozen(&mut self, width: u16) {
        self.rev = fresh_stamp();
        // Capture the open body: a later toggle shows the whole output,
        // not the collapsed rows. The collapsed row is rebuilt from the
        // snapshot, so the capture's width is only the toggle's.
        let was_expanded = self.expanded;
        self.expanded = true;
        let (lines, parts) = self.draw_with_parts(width);
        self.expanded = was_expanded;
        self.frozen_lines = Some(FrozenLines { lines, parts });
        self.frozen_width = width as usize;
        self.release_heavy_state();
    }

    /// The block's lines, drawn fresh.
    fn draw_at(&mut self, width: u16) -> Vec<String> {
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
    fn draw_with_parts(&mut self, width: u16) -> (Vec<String>, StepParts) {
        let lines = self.draw_at(width);
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
        let target = (width as usize).max(1);
        let Some(frozen) = &self.frozen_lines else {
            return StepParts::default();
        };
        let parts = &frozen.parts;
        let captured = self.frozen_width.max(1);
        if target >= captured {
            return parts.clone();
        }
        if let Some((cached_width, cached)) = &self.frozen_parts_fit {
            if *cached_width == target {
                return cached.clone();
            }
        }
        let mut fitted = parts.clone();
        let fit = |rows: &[String]| -> Vec<String> {
            rows.iter()
                .flat_map(|line| fit_frozen_line(line, target, false, false))
                .collect()
        };
        fitted.body = fit(&parts.body);
        fitted.footer = fit(&parts.footer);
        self.frozen_parts_fit = Some((target, fitted.clone()));
        fitted
    }

    fn build_step_parts(&mut self, width: u16) -> StepParts {
        if self.hide_component {
            return StepParts::default();
        }
        let main_width = width.max(1);
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
        // The glyph of the row counts as one visible character.
        let target = glyph_count(&header) + 1;
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
            body,
            tail,
            footer,
            images,
        }
    }
}
