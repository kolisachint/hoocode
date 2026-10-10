//! `components/tool-execution.ts`: one tool call's block in the transcript.
//!
//! The block draws a status dot, the tool's call line (its `renderCall`, or
//! the radar signal row) and, depending on the view dial, its result body
//! (`renderResult`, or a peek-trimmed fallback). Renderers come from the
//! built-in table ([`crate::tools::builtin_tool_definition`]) and the tool's
//! registered definition, slot by slot, as in the pin.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::{BoxComponent, Image, ImageOptions, ImageTheme, Spacer, Text};
use hoocode_tui_images::{get_capabilities, is_image_line, ImageProtocol};
use hoocode_tui_render::{Component, ComponentHandle};
use hoocode_tui_util::{truncate_to_width, visible_width};
use serde_json::Value;

use crate::render_utils::get_text_output;
use crate::tool_chain_summary::ChainEntry;
use crate::tool_output_view::{ToolOutputView, DEFAULT_TOOL_OUTPUT_VIEW, PEEK_LINES};
use crate::tool_signal::{tool_subject, ToolResult, ToolSignalComponent, ToolSignalInput};
use crate::tools::builtin_tool_definition;

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
}

impl FrozenLines {
    fn for_view(&self, view: ToolOutputView) -> &[String] {
        match view {
            ToolOutputView::Radar => &self.radar,
            ToolOutputView::Peek => &self.peek,
        }
    }
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
    args_complete: bool,
    result: Option<ToolResult>,
    hide_component: bool,
    is_latest: bool,
    frozen: bool,
    frozen_lines: Option<FrozenLines>,
    frozen_width: usize,
    frozen_truncated: Option<(usize, ToolOutputView, Vec<String>)>,
    /// The call's summary, taken when it freezes: its payloads are released then.
    frozen_entry: Option<ChainEntry>,
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
            args_complete: false,
            result: None,
            hide_component: false,
            is_latest: false,
            frozen: false,
            frozen_lines: None,
            frozen_width: 0,
            frozen_truncated: None,
            frozen_entry: None,
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
        let lines: Vec<&str> = output.split('\n').collect();
        if lines.len() <= PEEK_LINES {
            return Some(handle(Text::new(t.fg("toolOutput", &output), 0, 0)));
        }
        let shown = lines[..PEEK_LINES].join("\n");
        let remaining = lines.len() - PEEK_LINES;
        Some(handle(Text::new(
            format!(
                "{}{}",
                t.fg("toolOutput", &shown),
                t.fg("muted", &format!("\n... ({remaining} more lines)"))
            ),
            0,
            0,
        )))
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }

    pub fn update_args(&mut self, args: Value) {
        self.args = args;
        self.update_display();
    }

    pub fn mark_execution_started(&mut self) {
        self.execution_started = true;
        self.update_display();
    }

    pub fn set_args_complete(&mut self) {
        self.args_complete = true;
        self.update_display();
    }

    pub fn update_result(&mut self, result: ToolResult, is_partial: bool) {
        self.result = Some(result);
        self.is_partial = is_partial;
        self.update_display();
    }

    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        self.update_display();
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
            subject: tool_subject(&self.args, &self.cwd),
            is_error: self.result.as_ref().is_some_and(|r| r.is_error),
            is_partial: self.result.is_none() || self.is_partial,
            output_lines: if crate::is_blank(&output) {
                0
            } else {
                output.split('\n').count()
            },
            outcome: crate::tools::subagent::task_outcome(
                &self.tool_name,
                &self.args,
                self.result.as_ref(),
            ),
        }
    }

    /// The failure text, for a collapsed chain to print under its line.
    pub fn error_text(&self) -> String {
        if self.result.as_ref().is_some_and(|r| r.is_error) {
            self.text_output()
        } else {
            String::new()
        }
    }

    /// A failure always shows its reason, in every view.
    fn should_show_body(&self) -> bool {
        self.view != ToolOutputView::Radar || self.result.as_ref().is_some_and(|r| r.is_error)
    }

    fn should_show_signal_line(&self) -> bool {
        self.view == ToolOutputView::Radar
    }

    /// Whether this block draws its own blank row above itself (radar rows do not).
    pub fn draws_leading_gap(&self) -> bool {
        !self.should_show_signal_line()
    }

    fn indent_under_signal_row(&self, component: ComponentHandle) -> ComponentHandle {
        if self.should_show_signal_line() {
            handle(IndentAll {
                child: component,
                indent: " ".repeat(3),
            })
        } else {
            component
        }
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

    /// A finished, visible result still holding its payloads.
    pub fn is_freezable(&self) -> bool {
        !self.frozen && !self.is_partial && !self.hide_component && self.result.is_some()
    }

    /// Freeze on the next render: capture the lines, release the payloads.
    pub fn freeze(&mut self) {
        if self.is_freezable() {
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
                expanded: false,
                show_images: self.show_images,
                is_error: self.result.as_ref().is_some_and(|r| r.is_error),
            },
            last,
        )
    }

    fn update_display(&mut self) {
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
        self.leading_spacer
            .borrow_mut()
            .set_lines(if self.should_show_signal_line() { 0 } else { 1 });

        if self.has_renderer_definition() {
            self.content_box.borrow_mut().set_bg_fn(None);
            self.content_box.borrow_mut().clear();

            let t = theme();
            let dot_color = if self.result.as_ref().is_some_and(|r| r.is_error) {
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
                        self.add_to_shell(handle(PrefixFirstLine::new(dot.clone(), fallback)));
                        has_content = true;
                    }
                    Some(render_call) => {
                        let last = self.call_component.clone();
                        let (flags, last) = self.context(last);
                        let args = self.args.clone();
                        let cwd = self.cwd.clone();
                        let id = self.tool_call_id.clone();
                        let mut ctx = flags.borrow(
                            &args,
                            &id,
                            last,
                            &mut self.renderer_state,
                            &mut self.renderer_objects,
                            &cwd,
                        );
                        let rendered = render_call(&args, &mut ctx);
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
                        self.add_to_shell(handle(PrefixFirstLine::new(dot.clone(), component)));
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
                        let (flags, last) = self.context(last);
                        let args = self.args.clone();
                        let cwd = self.cwd.clone();
                        let id = self.tool_call_id.clone();
                        // The dial has no stop that opens a result: renderers trim it.
                        let options = ToolRenderResultOptions {
                            expanded: false,
                            is_partial: self.is_partial,
                        };
                        let view = ToolResultView {
                            content: &result.content,
                            details: &result.details,
                        };
                        let mut ctx = flags.borrow(
                            &args,
                            &id,
                            last,
                            &mut self.renderer_state,
                            &mut self.renderer_objects,
                            &cwd,
                        );
                        match render_result(&view, options, &mut ctx) {
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
        get_text_output(
            self.result.as_ref().map(|r| r.content.as_slice()),
            self.show_images,
        )
    }

    /// The whole block as text, for a tool with no definition at all.
    fn format_tool_execution(&self) -> String {
        let t = theme();
        let dot_color = if self.result.as_ref().is_some_and(|r| r.is_error) {
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
        }
    }
}

impl Component for ToolExecutionComponent {
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
        for child in &self.children {
            child.borrow_mut().invalidate();
        }
        self.update_display();
    }
}

impl ToolExecutionComponent {
    /// A frozen block draws the lines captured for its current stop. The first
    /// render after [`freeze`](Self::freeze) captures both stops.
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
        // A narrower terminal re-truncates so no line overflows it.
        if w >= self.frozen_width {
            return lines.to_vec();
        }
        if let Some((cached_width, cached_view, cached)) = &self.frozen_truncated {
            if *cached_width == w && *cached_view == view {
                return cached.clone();
            }
        }
        let truncated: Vec<String> = lines
            .iter()
            .map(|line| {
                if is_image_line(line) || visible_width(line) <= w {
                    line.clone()
                } else {
                    truncate_to_width(line, w, "...", false)
                }
            })
            .collect();
        self.frozen_truncated = Some((w, view, truncated.clone()));
        truncated
    }

    /// Draws the block at both stops, keeps both, and releases the payloads.
    fn capture_frozen(&mut self, width: u16) {
        let current = self.view;
        let radar = self.draw_at(ToolOutputView::Radar, width);
        let peek = self.draw_at(ToolOutputView::Peek, width);
        self.view = current;
        self.frozen_lines = Some(FrozenLines { radar, peek });
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
}
