//! `components/assistant-message.ts`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hoocode_ai_types::{AssistantMessage, Content, StopReason, ABORTED_MESSAGE};
use hoocode_code_tui_theme::{get_markdown_theme, theme};
use hoocode_tui_components::markdown::js_trim;
use hoocode_tui_components::{DefaultTextStyle, Markdown, MarkdownTheme, Spacer, Text};
use hoocode_tui_render::{Component, ComponentHandle, Container};
use hoocode_tui_util::js_regex::is_js_space;

use crate::trajectory;
use crate::wrap_zone;

/// How a thinking block renders.
///
/// `Omit` exists for radar: under a chain summary, a message whose only
/// content is thinking plus tool calls has nothing left on screen once its
/// calls collapse, so a stack of `Thinking...` lines would be pure noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingDisplay {
    #[default]
    Full,
    Label,
    Omit,
}

/// Streaming messages below this size (UTF-16 units) render as a single
/// Markdown; above it the text is segmented at stable block boundaries so each
/// update re-parses only the growing tail.
const SEGMENT_MIN_CHARS: usize = 2048;

/// Up to three leading spaces, then the rest. `None` for four or more: an
/// indented code line, which no block pattern here matches.
fn after_indent(line: &str) -> Option<&str> {
    let spaces = line.bytes().take(4).take_while(|&b| b == b' ').count();
    (spaces <= 3).then(|| hoocode_tui_util::text_slice::suffix_from(line, spaces))
}

/// `/^ {0,3}(?:[-*+] |\d{1,9}[.)] )/`
fn is_list_item(line: &str) -> bool {
    let Some(rest) = after_indent(line) else {
        return false;
    };
    if matches!(rest.as_bytes(), [b'-' | b'*' | b'+', b' ', ..]) {
        return true;
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (1..=9).contains(&digits)
        && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
        && rest.as_bytes().get(digits + 1) == Some(&b' ')
}

/// `/^ {0,3}(?:```|~~~)/`
fn is_fence(line: &str) -> bool {
    after_indent(line).is_some_and(|rest| rest.starts_with("```") || rest.starts_with("~~~"))
}

/// `/^ {0,3}\[[^\]]+\]: /m`
fn has_link_def(text: &str) -> bool {
    let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1));
    for start in starts {
        let Some(rest) = after_indent(hoocode_tui_util::text_slice::suffix_from(text, start))
        else {
            continue;
        };
        let Some(label) = rest.strip_prefix('[') else {
            continue;
        };
        if let Some(close) = label.find(']') {
            if close > 0
                && hoocode_tui_util::text_slice::suffix_from(label, close + 1).starts_with(": ")
            {
                return true;
            }
        }
    }
    false
}

/// `/^ {0,3}(?:=+|-+)\s*$/`
fn is_setext_underline(line: &str) -> bool {
    let Some(rest) = after_indent(line) else {
        return false;
    };
    let Some(first) = rest.chars().next().filter(|c| *c == '=' || *c == '-') else {
        return false;
    };
    let run = rest.chars().take_while(|&c| c == first).count();
    hoocode_tui_util::text_slice::suffix_from(rest, run)
        .chars()
        .all(is_js_space)
}

fn is_blank(line: &str) -> bool {
    js_trim(line).is_empty()
}

/// Whether a blank-line gap between `prev` and `next` (both non-blank) is a
/// safe place to cut the markdown into independently-parseable chunks.
fn is_safe_boundary(prev: &str, next: &str) -> bool {
    // Indented continuation binds to the block above the gap.
    if next.chars().next().is_some_and(is_js_space) {
        return false;
    }
    // A blank line between two list items is a loose list, not two lists.
    if is_list_item(prev) && is_list_item(next) {
        return false;
    }
    let prev_start = prev.trim_start_matches(is_js_space);
    if next.starts_with('|') || prev_start.starts_with('|') {
        return false;
    }
    if next.starts_with('<') || prev_start.starts_with('<') {
        return false;
    }
    // A bare ===/--- after the gap could lex differently without its
    // preceding text; keep it attached.
    !is_setext_underline(next)
}

/// Split markdown into chunks at blank-line boundaries where each chunk lexes
/// on its own to the same blocks the whole text would. Prefix-stable:
/// appending text never changes earlier boundaries, so while streaming every
/// chunk but the last is byte-identical across updates and keeps its render
/// cache.
pub fn segment_streaming_markdown(text: &str) -> Vec<String> {
    // Reference definitions resolve across the whole document.
    if has_link_def(text) {
        return vec![text.to_string()];
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut fence_open = false;
    let mut prev_nonblank = "";
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_fence(line) {
            fence_open = !fence_open;
        }
        if is_blank(line) && !fence_open {
            let mut j = i + 1;
            while j < lines.len() && is_blank(lines[j]) {
                j += 1;
            }
            // Only cut when the gap has content on both sides.
            if j < lines.len() && chunk_start < i && is_safe_boundary(prev_nonblank, lines[j]) {
                chunks.push(lines[chunk_start..i].join("\n"));
                chunk_start = j;
            }
            i = j;
            continue;
        }
        if !is_blank(line) {
            prev_nonblank = line;
        }
        i += 1;
    }
    let last = lines[chunk_start.min(lines.len())..].join("\n");
    if !is_blank(&last) {
        chunks.push(last);
    }
    if chunks.is_empty() {
        vec![text.to_string()]
    } else {
        chunks
    }
}

/// Which Markdown child a block is, across streaming updates: a content block
/// (text or thinking, by its index), or one chunk of such a block once it is
/// segmented.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BlockKey {
    Text(usize),
    Thinking(usize),
    Segment { of: Box<BlockKey>, index: usize },
}

struct CachedMarkdown {
    md: Rc<RefCell<Markdown>>,
    text: String,
}

/// A complete (or streaming) assistant message: its text and thinking blocks,
/// and the abort/error line when it has no tool calls.
pub struct AssistantMessageComponent {
    content: Container,
    thinking_display: ThinkingDisplay,
    markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    /// Mission Control peek (M4): prose lines sit on the `┊` rail. Think
    /// headers (`·· …`) keep their own slot and skip the rail.
    rail: bool,
    /// Shared with the caller's streaming message, so a delta does not copy it.
    last_message: Option<Rc<AssistantMessage>>,
    has_tool_calls: bool,
    /// Markdown children reused across streaming updates, keyed by content
    /// index and kind, so finished blocks keep their render caches.
    markdown_cache: HashMap<BlockKey, CachedMarkdown>,
    has_segmented_blocks: bool,
    /// Whether the last `update_content` was a streaming update, so a
    /// `refresh` (e.g. a thinking toggle mid-stream) keeps the same form.
    streaming: bool,
}

fn handle<C: Component + 'static>(c: C) -> ComponentHandle {
    Rc::new(RefCell::new(c))
}

impl AssistantMessageComponent {
    pub fn new(message: Option<&AssistantMessage>, thinking_display: ThinkingDisplay) -> Self {
        Self::with_theme(
            message,
            thinking_display,
            Rc::new(get_markdown_theme),
            "Thinking...",
        )
    }

    pub fn with_theme(
        message: Option<&AssistantMessage>,
        thinking_display: ThinkingDisplay,
        markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
        // The collapsed-thinking label predates the M4 Think row, which
        // carries its own header; kept for signature stability.
        _hidden_thinking_label: &str,
    ) -> Self {
        let mut this = Self {
            content: Container::new(),
            thinking_display,
            markdown_theme,
            rail: false,
            last_message: None,
            has_tool_calls: false,
            markdown_cache: HashMap::new(),
            has_segmented_blocks: false,
            streaming: false,
        };
        if let Some(message) = message {
            this.update_content(message, false);
        }
        this
    }

    fn reuse_markdown(
        &mut self,
        key: BlockKey,
        text: &str,
        thinking: bool,
    ) -> Rc<RefCell<Markdown>> {
        if let Some(entry) = self.markdown_cache.get_mut(&key) {
            if entry.text != text {
                entry.md.borrow_mut().set_text(text);
                entry.text = text.to_string();
            }
            return entry.md.clone();
        }
        let style = thinking.then(|| DefaultTextStyle {
            color: Some(Box::new(|s: &str| theme().fg("thinkingText", s))),
            italic: true,
            ..Default::default()
        });
        let md = Rc::new(RefCell::new(Markdown::new(
            text,
            1,
            0,
            (self.markdown_theme)(),
            style,
        )));
        self.markdown_cache.insert(
            key,
            CachedMarkdown {
                md: md.clone(),
                text: text.to_string(),
            },
        );
        md
    }

    /// Add one markdown block; large blocks still streaming are segmented so
    /// only the tail chunk re-parses per update.
    fn add_markdown_block(&mut self, key: BlockKey, text: &str, streaming: bool, thinking: bool) {
        if streaming && text.encode_utf16().count() >= SEGMENT_MIN_CHARS {
            let chunks = segment_streaming_markdown(text);
            if chunks.len() > 1 {
                for (index, chunk) in chunks.iter().enumerate() {
                    if index > 0 {
                        self.content.add_child(handle(Spacer::new(1)));
                    }
                    let segment = BlockKey::Segment {
                        of: Box::new(key.clone()),
                        index,
                    };
                    let md = self.reuse_markdown(segment, chunk, thinking);
                    self.content.add_child(md);
                }
                self.has_segmented_blocks = true;
                return;
            }
        }
        let md = self.reuse_markdown(key, text, thinking);
        self.content.add_child(md);
    }

    pub fn set_thinking_display(&mut self, display: ThinkingDisplay) {
        self.thinking_display = display;
        self.refresh();
    }

    /// Mission Control peek (M4): prose lines sit on the `┊` rail.
    pub fn set_rail(&mut self, rail: bool) {
        self.rail = rail;
    }

    fn refresh(&mut self) {
        if let Some(message) = self.last_message.clone() {
            let streaming = self.streaming;
            self.update_shared(message, streaming);
        }
    }

    /// Whether a content block puts anything on screen under the current settings.
    fn is_visible_block(&self, content: &Content) -> bool {
        match content {
            Content::Text(t) => !is_blank(&t.text),
            Content::Thinking(t) => {
                self.thinking_display != ThinkingDisplay::Omit && !is_blank(&t.thinking)
            }
            _ => false,
        }
    }

    pub fn update_content(&mut self, message: &AssistantMessage, streaming: bool) {
        self.update_shared(Rc::new(message.clone()), streaming);
    }

    /// [`Self::update_content`] for a message the caller already holds in an
    /// `Rc`: each streaming delta then shares it instead of copying it.
    pub fn update_shared(&mut self, message: Rc<AssistantMessage>, streaming: bool) {
        self.last_message = Some(message.clone());
        self.streaming = streaming;

        // A final render replaces streaming segments with the single form.
        if !streaming && self.has_segmented_blocks {
            self.markdown_cache
                .retain(|key, _| !matches!(key, BlockKey::Segment { .. }));
            self.has_segmented_blocks = false;
        }

        self.content.clear();

        let has_visible_content = message.content.iter().any(|c| self.is_visible_block(c));
        if has_visible_content {
            self.content.add_child(handle(Spacer::new(1)));
        }

        for (i, content) in message.content.iter().enumerate() {
            match content {
                Content::Text(t) if !is_blank(&t.text) => {
                    self.add_markdown_block(BlockKey::Text(i), js_trim(&t.text), streaming, false);
                }
                Content::Thinking(t) if !is_blank(&t.thinking) => {
                    if self.thinking_display == ThinkingDisplay::Omit {
                        continue;
                    }
                    // The M4 Think row: the `··` slot, header only. Full
                    // display adds the trace itself under it, on the rail.
                    let visible_after = message.content[i + 1..]
                        .iter()
                        .any(|c| self.is_visible_block(c));
                    self.content.add_child(handle(Text::new(
                        trajectory::think_header(streaming),
                        0,
                        0,
                    )));
                    if self.thinking_display == ThinkingDisplay::Full {
                        let text = format!("✻ {}", js_trim(&t.thinking));
                        self.add_markdown_block(BlockKey::Thinking(i), &text, streaming, true);
                    }
                    if visible_after {
                        self.content.add_child(handle(Spacer::new(1)));
                    }
                }
                _ => {}
            }
        }

        // The abort/error line, unless tool blocks will show it.
        self.has_tool_calls = message
            .content
            .iter()
            .any(|c| matches!(c, Content::ToolCall(_)));
        if !self.has_tool_calls {
            let t = theme();
            match message.stop_reason {
                StopReason::Aborted => {
                    let abort_message = match message.error_message.as_deref() {
                        Some(m) if !m.is_empty() && m != ABORTED_MESSAGE => m.to_string(),
                        _ => "Operation aborted".to_string(),
                    };
                    self.content.add_child(handle(Spacer::new(1)));
                    self.content
                        .add_child(handle(Text::new(t.fg("error", &abort_message), 1, 0)));
                }
                StopReason::Error => {
                    let error = message
                        .error_message
                        .as_deref()
                        .filter(|m| !m.is_empty())
                        .unwrap_or("Unknown error");
                    self.content.add_child(handle(Spacer::new(1)));
                    self.content.add_child(handle(Text::new(
                        t.fg("error", &format!("Error: {error}")),
                        1,
                        0,
                    )));
                }
                _ => {}
            }
        }
    }
}

impl Component for AssistantMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.has_tool_calls {
            return self.content.render(width);
        }
        if !self.rail {
            let lines = self.content.render(width);
            if lines.is_empty() {
                return lines;
            }
            return wrap_zone(lines);
        }
        // Mission Control peek: prose sits on the `┊` rail, so the inner
        // content wraps six cells narrower and every line takes the rail.
        // Think headers keep their own `··` slot.
        let width = width as usize;
        let inner = width.saturating_sub(6).max(1) as u16;
        let lines: Vec<String> = self
            .content
            .render(inner)
            .into_iter()
            .map(|line| {
                if trajectory::is_think_header(&line) {
                    line
                } else {
                    trajectory::rail_line(&line, width, false)
                }
            })
            .collect();
        if lines.is_empty() {
            return lines;
        }
        wrap_zone(lines)
    }

    fn invalidate(&mut self) {
        // Cached blocks may be detached right now; drop their caches too.
        for entry in self.markdown_cache.values() {
            entry.md.borrow_mut().invalidate();
        }
        self.content.invalidate();
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_streaming_text() -> String {
        // Well above SEGMENT_MIN_CHARS, with blank lines so it segments.
        (0..60)
            .map(|n| format!("Paragraph {n} with enough words to make the text long."))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn streaming_message() -> AssistantMessage {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        AssistantMessage {
            content: vec![Content::Text(hoocode_ai_types::TextContent {
                text: long_streaming_text(),
                text_signature: None,
            })],
            ..Default::default()
        }
    }

    #[test]
    fn thinking_toggle_mid_stream_keeps_segmented_blocks() {
        let message = streaming_message();
        let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
        component.update_content(&message, true);
        assert!(component.has_segmented_blocks, "a long stream segments");

        component.set_thinking_display(ThinkingDisplay::Label);

        assert!(
            component.has_segmented_blocks,
            "a display change mid-stream must not drop the streaming form"
        );
        assert!(component
            .markdown_cache
            .keys()
            .any(|k| matches!(k, BlockKey::Segment { .. })));
    }

    #[test]
    fn indented_four_or_more_spaces_is_not_a_block_prefix() {
        assert_eq!(after_indent("   - item"), Some("- item"));
        assert_eq!(after_indent("    - item"), None);
        assert!(!is_list_item("    - item"));
        assert!(!is_fence("    ```"));
        assert!(!is_setext_underline("    ==="));
    }

    #[test]
    fn shared_update_keeps_the_callers_message_without_a_copy() {
        let message = Rc::new(streaming_message());
        let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
        component.update_shared(message.clone(), true);
        let held = component.last_message.clone().expect("the message is held");
        assert!(
            Rc::ptr_eq(&held, &message),
            "the held message is the caller's"
        );
        component.set_thinking_display(ThinkingDisplay::Label);
        assert!(
            component.has_segmented_blocks,
            "a refresh reuses the held message"
        );
    }

    #[test]
    fn aborted_without_a_reason_shows_operation_aborted() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rendered = |error: Option<&str>| {
            let message = AssistantMessage {
                stop_reason: StopReason::Aborted,
                error_message: error.map(String::from),
                ..Default::default()
            };
            let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
            component.update_content(&message, false);
            component.render(80).join("\n")
        };
        assert!(rendered(Some(ABORTED_MESSAGE)).contains("Operation aborted"));
        assert!(rendered(None).contains("Operation aborted"));
        assert!(rendered(Some("Aborted after 2 retry attempts"))
            .contains("Aborted after 2 retry attempts"));
    }

    #[test]
    fn final_render_still_replaces_segments() {
        let message = streaming_message();
        let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
        component.update_content(&message, true);
        component.update_content(&message, false);
        assert!(!component.has_segmented_blocks);
        assert!(!component
            .markdown_cache
            .keys()
            .any(|k| matches!(k, BlockKey::Segment { .. })));
    }
}
