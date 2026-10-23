//! `components/assistant-message.ts`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Instant;

use hoocode_ai_types::{AssistantMessage, Content, StopReason, ABORTED_MESSAGE};
use hoocode_code_tui_theme::{get_markdown_theme, theme};
use hoocode_tui_components::markdown::js_trim;
use hoocode_tui_components::{DefaultTextStyle, Markdown, MarkdownTheme, Spacer, Text};
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::{
    fresh_stamp, mix_stamps, render_epoch, Component, ComponentHandle, Container,
};
use hoocode_tui_util::js_regex::is_js_space;

use crate::radar::{self, HIT_INDENT, STEP_INDENT};
use crate::tool_chain::ToolChainComponent;
use crate::tool_output_view::ToolOutputView;
use crate::trajectory;
use crate::wrap_zone;

/// How a thinking block renders.
///
/// `Omit` exists for radar: the flight log shows the message's last prose
/// line plus its `✻ thought Ns` line, so a stack of `Thinking...` lines
/// would be pure noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingDisplay {
    #[default]
    Full,
    Label,
    Omit,
}

/// Why a message ended badly, in the words every layer prints: the abort
/// line (`Operation aborted`, or the reason it carries) or `Error: …`. `None`
/// for a message that ended any other way. Peek and radar read this one text,
/// whether or not the message has tool calls.
pub fn end_reason(message: &AssistantMessage) -> Option<String> {
    match message.stop_reason {
        StopReason::Aborted => Some(match message.error_message.as_deref() {
            Some(m) if !m.is_empty() && m != ABORTED_MESSAGE => m.to_string(),
            _ => "Operation aborted".to_string(),
        }),
        StopReason::Error => {
            let error = message
                .error_message
                .as_deref()
                .filter(|m| !m.is_empty())
                .unwrap_or("Unknown error");
            Some(format!("Error: {error}"))
        }
        _ => None,
    }
}

/// The radar prose of a message (M5): all of its text, as plain lines. Block
/// markers (`#`, `>`, bullets, numbers) and inline marks (`**`, backticks,
/// link targets) drop, blank lines and fences go, and each remaining line
/// stands whole: radar wraps it, never cuts it. Empty when the message has no
/// text.
fn radar_prose(message: &AssistantMessage) -> Vec<String> {
    let mut out = Vec::new();
    for content in &message.content {
        let Content::Text(t) = content else {
            continue;
        };
        out.extend(plain_lines(&t.text));
    }
    out
}

/// `text` as plain lines (see [`radar_prose`]). Lines inside a code fence are
/// kept as they are, trimmed.
fn plain_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if is_fence(line) {
            fenced = !fenced;
            continue;
        }
        let line = line.trim();
        let plain = if fenced {
            line.to_string()
        } else {
            plain_inline(strip_block_marker(line).trim())
        };
        let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        if !plain.is_empty() {
            out.push(plain);
        }
    }
    out
}

/// Drops a line's block marker: quote `>`, ATX heading `#`, bullet `-`,
/// `*` or `+`, or ordered `1.` / `1)`. A rule line (`---`, `***`, `___`)
/// reads as empty. Only the marker goes; the rest of the line is kept.
fn strip_block_marker(line: &str) -> &str {
    let mark = line.chars().find(|c| *c != ' ');
    let marks = line.chars().filter(|c| *c != ' ').count();
    if matches!(mark, Some('-' | '*' | '_'))
        && marks >= 3
        && line.chars().all(|c| c == ' ' || Some(c) == mark)
    {
        return "";
    }
    let mut rest = line;
    loop {
        let before = rest;
        if let Some(after) = rest.strip_prefix('>') {
            rest = after.trim_start();
        }
        let body = rest.trim_start_matches('#');
        if body.len() < rest.len() && (body.is_empty() || body.starts_with(' ')) {
            rest = body.trim_start();
        }
        if let Some(after) = rest
            .strip_prefix("- ")
            .or_else(|| rest.strip_prefix("* "))
            .or_else(|| rest.strip_prefix("+ "))
        {
            rest = after.trim_start();
        }
        let number = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        let digits = rest.len() - number.len();
        if (1..=2).contains(&digits) {
            if let Some(after) = number
                .strip_prefix(". ")
                .or_else(|| number.strip_prefix(") "))
            {
                rest = after.trim_start();
            }
        }
        if rest == before {
            return rest;
        }
    }
}

/// Plain text for inline Markdown: emphasis marks (`*`, and `_` at a word
/// edge) and code backticks drop, and a link `[text](url)` or image
/// `![alt](url)` reads as its text. `snake_case` keeps its underscores.
fn plain_inline(text: &str) -> String {
    let unlinked = unlink(text);
    let chars: Vec<char> = unlinked.chars().collect();
    let mut out = String::with_capacity(unlinked.len());
    for (i, &c) in chars.iter().enumerate() {
        let is_mark = match c {
            '`' | '*' => true,
            '_' => {
                let after_word = i
                    .checked_sub(1)
                    .and_then(|p| chars.get(p))
                    .is_some_and(|p| p.is_alphanumeric());
                let before_word = chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
                !(after_word && before_word)
            }
            _ => false,
        };
        if !is_mark {
            out.push(c);
        }
    }
    out
}

/// `text` with each `[label](target)` and `![label](target)` replaced by its
/// label. A bracket with no closing `](...)` stays as written.
fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, from_open)) = rest.split_once('[') {
        let Some((label, after_label)) = from_open.split_once("](") else {
            break;
        };
        let Some((_target, tail)) = after_label.split_once(')') else {
            break;
        };
        if label.contains('[') {
            out.push_str(before);
            out.push('[');
            rest = from_open;
            continue;
        }
        out.push_str(before.strip_suffix('!').unwrap_or(before));
        out.push_str(label);
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// A thinking trace's gist (Mission Control spec a2/r11): the first
/// non-blank line of the message's first thinking block,
/// whitespace-collapsed. The Think header and the radar think row both
/// read it; the row itself truncates it to fit.
fn think_gist(message: &AssistantMessage) -> Option<String> {
    message
        .content
        .iter()
        .find_map(|content| match content {
            Content::Thinking(t) => t
                .thinking
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ")),
            _ => None,
        })
        .filter(|gist| !gist.is_empty())
}

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

/// A content child's lines already on the rail, for the stamp, width and
/// render epoch they were drawn at.
struct RailedChild {
    stamp: u64,
    width: u16,
    epoch: u64,
    lines: Rc<[String]>,
}

struct CachedMarkdown {
    md: Rc<RefCell<Markdown>>,
    /// What goes in the container: the markdown, or for a later segment the
    /// markdown behind its leading gap.
    child: ComponentHandle,
    text: String,
}

/// A later segment of a streaming block: its markdown after the blank row
/// the full text has between blocks, drawn the way the markdown draws a blank
/// row (the width in spaces). A chunk that draws nothing yet (a lone `>`; the
/// markdown then answers one empty row) is only that blank row, as in the
/// full render.
struct GappedSegment {
    md: Rc<RefCell<Markdown>>,
}

impl Component for GappedSegment {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.render_shared(width).to_vec()
    }

    fn render_shared(&mut self, width: u16) -> Rc<[String]> {
        let body = self.md.borrow_mut().render_shared(width);
        let gap = " ".repeat(usize::from(width));
        // A drawn row always holds the margins, so an empty one is the
        // markdown's answer for "nothing".
        if body.iter().all(String::is_empty) {
            return vec![gap].into();
        }
        std::iter::once(gap)
            .chain(body.iter().cloned())
            .collect::<Vec<_>>()
            .into()
    }

    fn render_stamp(&self) -> Option<u64> {
        self.md.borrow().render_stamp()
    }

    fn invalidate(&mut self) {
        self.md.borrow_mut().invalidate();
    }
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
    /// The full stop: the Think row wraps its gist instead of cutting it.
    full: bool,
    /// Radar (M5): the chain whose rows sit right above this message's
    /// prose line. The prose reads that chain's last row when it renders,
    /// so a row that turns into a hit later still moves the prose to 12.
    radar_above: Option<Weak<RefCell<ToolChainComponent>>>,
    /// Radar (M5): the mission (header number) this message belongs to; `None`
    /// before the first mission. The transcript groups prose by it.
    radar_mission: Option<usize>,
    /// Radar (M5): whether this message's prose line shows. The transcript
    /// sets it: only the last prose message of an ended mission shows one.
    radar_prose_shown: bool,
    /// Shared with the caller's streaming message, so a delta does not copy it.
    last_message: Option<Rc<AssistantMessage>>,
    has_tool_calls: bool,
    /// Markdown children reused across streaming updates, keyed by content
    /// index and kind, so finished blocks keep their render caches.
    markdown_cache: HashMap<BlockKey, CachedMarkdown>,
    has_segmented_blocks: bool,
    /// The one-row gap every block boundary uses, shared so its stamp is
    /// stable across updates.
    gap: ComponentHandle,
    /// Peek: each content child's rail lines, by the child's address. Only
    /// children with a render stamp are kept.
    rail_cache: HashMap<usize, RailedChild>,
    /// Whether the last `update_content` was a streaming update, so a
    /// `refresh` (e.g. a thinking toggle mid-stream) keeps the same form.
    streaming: bool,
    /// The Think row's clock (M4): when the trace started streaming and
    /// when it settled. Unknown for resumed sessions, which never timed
    /// the trace; the header then omits the time.
    think_start: Option<Instant>,
    think_end: Option<Instant>,
    /// Width-diagnosis log (`HOOCODE_WIDTH_DEBUG`): resolved once at
    /// construction, so per-frame rendering does no environment I/O.
    width_log: Option<PathBuf>,
    /// The last `(width, source chars, longest source line)` logged, so a
    /// steady frame does not append a line per render.
    last_width_log: Option<(u16, usize, usize)>,
    /// Changes whenever something that shows in the output does: every
    /// `&mut self` method that sets the message, the view or the radar links
    /// takes a new one (the render stamp).
    rev: u64,
}

/// The width-diagnosis log file: `HOOCODE_WIDTH_DEBUG` set to a path.
/// Unset or empty disables the logging below entirely.
fn width_log_path() -> Option<PathBuf> {
    std::env::var("HOOCODE_WIDTH_DEBUG")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
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
            full: false,
            radar_above: None,
            radar_mission: None,
            radar_prose_shown: false,
            last_message: None,
            has_tool_calls: false,
            markdown_cache: HashMap::new(),
            has_segmented_blocks: false,
            gap: handle(Spacer::new(1)),
            rail_cache: HashMap::new(),
            streaming: false,
            think_start: None,
            think_end: None,
            width_log: width_log_path(),
            last_width_log: None,
            rev: fresh_stamp(),
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
        gap_above: bool,
    ) -> ComponentHandle {
        if let Some(entry) = self.markdown_cache.get_mut(&key) {
            if entry.text != text {
                entry.md.borrow_mut().set_text(text);
                entry.text = text.to_string();
            }
            return entry.child.clone();
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
        let child: ComponentHandle = if gap_above {
            handle(GappedSegment { md: md.clone() })
        } else {
            md.clone()
        };
        self.markdown_cache.insert(
            key,
            CachedMarkdown {
                md,
                child: child.clone(),
                text: text.to_string(),
            },
        );
        child
    }

    /// Add one markdown block; a block still streaming is segmented at stable
    /// boundaries so only the tail chunk re-parses per update.
    fn add_markdown_block(&mut self, key: BlockKey, text: &str, streaming: bool, thinking: bool) {
        if streaming {
            let chunks = segment_streaming_markdown(text);
            if chunks.len() > 1 {
                for (index, chunk) in chunks.iter().enumerate() {
                    let segment = BlockKey::Segment {
                        of: Box::new(key.clone()),
                        index,
                    };
                    let child = self.reuse_markdown(segment.clone(), chunk, thinking, index > 0);
                    self.content.add_child(child);
                }
                self.has_segmented_blocks = true;
                return;
            }
        }
        let child = self.reuse_markdown(key, text, thinking, false);
        self.content.add_child(child);
    }

    pub fn set_thinking_display(&mut self, display: ThinkingDisplay) {
        self.rev = fresh_stamp();
        self.thinking_display = display;
        self.refresh();
    }

    /// The view dial's stop: peek and full put prose on the `┊` rail (M4);
    /// full also wraps the Think row's gist where the others cut it.
    pub fn set_view(&mut self, view: ToolOutputView) {
        self.rev = fresh_stamp();
        self.rail = view != ToolOutputView::Radar;
        let full = view == ToolOutputView::Full;
        if self.full != full {
            self.full = full;
            self.refresh();
        }
    }

    /// Radar (M5): links the prose line to the chain above it (`None` when
    /// no mission chain precedes it).
    pub fn set_radar_above(&mut self, chain: Option<Weak<RefCell<ToolChainComponent>>>) {
        self.rev = fresh_stamp();
        self.radar_above = chain;
    }

    /// Radar (M5): the mission this message belongs to (`None` before the
    /// first mission).
    pub fn radar_mission(&self) -> Option<usize> {
        self.radar_mission
    }

    /// Radar (M5): tags the message with its mission.
    pub fn set_radar_mission(&mut self, mission: Option<usize>) {
        self.rev = fresh_stamp();
        self.radar_mission = mission;
    }

    /// Radar (M5): whether the message has prose a radar line can show.
    pub fn has_radar_prose(&self) -> bool {
        // Cheap: the transcript asks for every message on every refresh.
        self.last_message.as_deref().is_some_and(|m| {
            end_reason(m).is_some()
                || m.content.iter().any(|content| {
                    matches!(content, Content::Text(t) if t.text.chars().any(char::is_alphanumeric))
                })
        })
    }

    /// Radar (M5): shows or hides the message's prose line. Thinking's
    /// `✻ thought` line is not affected.
    pub fn set_radar_prose_shown(&mut self, shown: bool) {
        self.rev = fresh_stamp();
        self.radar_prose_shown = shown;
    }

    /// The prose line's indent: `HIT_INDENT` (12) when the chain above ends
    /// in a hit row, otherwise `STEP_INDENT` (8) under the mission row.
    fn radar_indent(&self) -> usize {
        let after_hit = self
            .radar_above
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|chain| chain.try_borrow().is_ok_and(|chain| chain.ends_in_hit()));
        if after_hit {
            HIT_INDENT
        } else {
            STEP_INDENT
        }
    }

    /// The Think row's seconds: elapsed since the trace started while it
    /// runs, the settled span once done, `None` when never timed.
    ///
    /// The radar flight log reads the same clock for its `✻ thought Ns`
    /// line (M5), with the same gist.
    pub(crate) fn radar_think_lines(&self, width: usize, palette: &Palette) -> Vec<String> {
        let Some(secs) = self.think_secs(self.streaming) else {
            return Vec::new();
        };
        let gist = self
            .last_message
            .as_ref()
            .and_then(|message| think_gist(message));
        crate::radar::think_rows(secs, gist.as_deref(), palette, width)
    }

    fn think_secs(&self, streaming: bool) -> Option<u64> {
        let start = self.think_start?;
        if streaming {
            return Some(start.elapsed().as_secs());
        }
        self.think_end
            .map(|end| end.duration_since(start).as_secs())
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
        self.rev = fresh_stamp();
        self.last_message = Some(message.clone());
        self.streaming = streaming;
        // The Think row's clock: the first streaming update carrying
        // thinking starts it; the first settled update stops it.
        let has_thinking = message
            .content
            .iter()
            .any(|c| matches!(c, Content::Thinking(t) if !is_blank(&t.thinking)));
        if has_thinking {
            if streaming && self.think_start.is_none() {
                self.think_start = Some(Instant::now());
            }
            if !streaming && self.think_start.is_some() && self.think_end.is_none() {
                self.think_end = Some(Instant::now());
            }
        }

        // A final render replaces streaming segments with the single form.
        if !streaming && self.has_segmented_blocks {
            self.markdown_cache
                .retain(|key, _| !matches!(key, BlockKey::Segment { .. }));
            self.has_segmented_blocks = false;
        }

        self.content.clear();

        let has_visible_content = message.content.iter().any(|c| self.is_visible_block(c));
        if has_visible_content {
            self.content.add_child(self.gap.clone());
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
                    // The Think row: the `··` slot with the trace's gist
                    // (Mission Control spec a2). Full display adds the
                    // trace itself under it, on the rail.
                    let visible_after = message.content[i + 1..]
                        .iter()
                        .any(|c| self.is_visible_block(c));
                    let mut header =
                        trajectory::ThinkHeader::new(streaming, self.think_secs(streaming));
                    header.set_gist(think_gist(&message));
                    header.set_wrap(self.full);
                    self.content.add_child(handle(header));
                    if self.thinking_display == ThinkingDisplay::Full {
                        let text = format!("✻ {}", js_trim(&t.thinking));
                        self.add_markdown_block(BlockKey::Thinking(i), &text, streaming, true);
                    }
                    if visible_after {
                        self.content.add_child(self.gap.clone());
                    }
                }
                _ => {}
            }
        }

        // The abort/error line: shown whether or not the message has tool
        // calls, so the turn's end is never lost behind its blocks.
        self.has_tool_calls = message
            .content
            .iter()
            .any(|c| matches!(c, Content::ToolCall(_)));
        if let Some(reason) = end_reason(&message) {
            self.content.add_child(self.gap.clone());
            self.content
                .add_child(handle(Text::new(theme().fg("error", &reason), 1, 0)));
        }
    }
}

impl AssistantMessageComponent {
    /// The radar flight log (M5): the message's whole text as plain wrapped
    /// rows, when the transcript shows it (`set_radar_prose_shown`), the abort
    /// or error reason after it, then its `✻ thought Ns` rows. A message with
    /// none of these renders nothing.
    fn render_radar(&mut self, width: u16, palette: &Palette) -> Vec<String> {
        let width = width as usize;
        let mut out = Vec::new();
        if self.radar_prose_shown {
            if let Some(message) = self.last_message.as_deref() {
                let indent = self.radar_indent();
                out.extend(radar::prose_rows(
                    &radar_prose(message),
                    indent,
                    palette.ink(),
                    width,
                ));
                if let Some(reason) = end_reason(message) {
                    let lines: Vec<String> = reason.lines().map(str::to_string).collect();
                    out.extend(radar::prose_rows(&lines, indent, palette.err(), width));
                }
            }
        }
        out.extend(self.radar_think_lines(width, palette));
        if out.is_empty() {
            return out;
        }
        wrap_zone(out)
    }
}

impl AssistantMessageComponent {
    /// Width diagnosis (`HOOCODE_WIDTH_DEBUG`): one appended line per
    /// distinct `(width, source text)` shape: the width this render was
    /// given, the longest source-text line it was given, and the longest
    /// content run it drew. A narrow `out_longest` at a wide `width` with
    /// a narrow `src_longest` means the text arrived pre-wrapped upstream
    /// (hard breaks); a narrow `width` means size detection handed down a
    /// narrow viewport. Never fails rendering: I/O errors are dropped.
    fn log_width(&mut self, width: u16, lines: &[String]) {
        let Some(path) = self.width_log.clone() else {
            return;
        };
        let mut src_chars = 0usize;
        let mut src_longest = 0usize;
        if let Some(message) = self.last_message.as_ref() {
            for block in &message.content {
                let text = match block {
                    Content::Text(t) => t.text.as_str(),
                    Content::Thinking(t) => t.thinking.as_str(),
                    _ => continue,
                };
                src_chars += text.len();
                for line in text.split('\n') {
                    src_longest = src_longest.max(hoocode_tui_util::visible_width(line));
                }
            }
        }
        let key = (width, src_chars, src_longest);
        if self.last_width_log == Some(key) {
            return;
        }
        self.last_width_log = Some(key);
        let mut out_longest = 0usize;
        for line in lines {
            let plain = hoocode_tui_util::strip_vt_control_characters(line);
            out_longest = out_longest.max(hoocode_tui_util::visible_width(plain.trim_end()));
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "assistant-message render width={width} rail={} src_chars={src_chars} \
                 src_longest={src_longest} out_lines={} out_longest={out_longest}",
                u8::from(self.rail),
                lines.len(),
            );
        }
    }
}

impl Component for AssistantMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        let lines = self.render_inner(width);
        self.log_width(width, &lines);
        lines
    }

    /// A streaming message reads a live clock and changes with every delta:
    /// no stamp. A settled one is drawn from its own state and, for the
    /// radar and peek prose, from whether the chain above it still exists and
    /// ends in a hit.
    fn render_stamp(&self) -> Option<u64> {
        if self.streaming {
            return None;
        }
        let above = match self.radar_above.as_ref().and_then(Weak::upgrade) {
            None => 0,
            Some(chain) => 1 + u64::from(chain.try_borrow().ok()?.ends_in_hit()),
        };
        Some(mix_stamps([self.rev, above]))
    }

    fn invalidate(&mut self) {
        self.rev = fresh_stamp();
        // Cached blocks may be detached right now; drop their caches too.
        for entry in self.markdown_cache.values() {
            entry.md.borrow_mut().invalidate();
        }
        self.content.invalidate();
        self.rail_cache.clear();
        self.refresh();
    }
}

impl AssistantMessageComponent {
    /// The peek form of the content: every line on the `┊` rail. A child with
    /// a stamp keeps its railed lines between frames, so a streaming message
    /// puts only its changed tail on the rail again.
    fn render_railed(
        &mut self,
        inner: u16,
        width: usize,
        after_step: bool,
        palette: &Palette,
    ) -> Vec<String> {
        let rail = |lines: &[String], first: bool| -> Vec<String> {
            lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    let opening_blank = first && index == 0 && !after_step && is_blank(line);
                    if opening_blank || trajectory::is_think_header(line) {
                        line.clone()
                    } else {
                        trajectory::rail_line(line, width, false, palette)
                    }
                })
                .collect()
        };
        let epoch = render_epoch();
        let mut kept = HashMap::new();
        let mut out = Vec::new();
        for (index, child) in self.content.children.iter().enumerate() {
            let stamp = child.borrow().render_stamp();
            // The first child holds the opening row, which has its own rule.
            let Some(stamp) = stamp.filter(|_| index > 0) else {
                let lines = child.borrow_mut().render(inner);
                out.extend(rail(&lines, index == 0));
                continue;
            };
            let key = Rc::as_ptr(child) as *const () as usize;
            let hit = self
                .rail_cache
                .remove(&key)
                .or_else(|| kept.remove(&key))
                .filter(|e| e.stamp == stamp && e.width == inner && e.epoch == epoch);
            let entry = match hit {
                Some(entry) => {
                    // Builds with debug assertions draw every hit again, as the
                    // container does, so a stale entry fails the test that
                    // touches it.
                    if cfg!(debug_assertions) {
                        let fresh = rail(&child.borrow_mut().render(inner), false);
                        assert!(fresh[..] == entry.lines[..], "peek rail cache is stale");
                    }
                    entry
                }
                None => {
                    let lines = child.borrow_mut().render(inner);
                    RailedChild {
                        stamp,
                        width: inner,
                        epoch,
                        lines: rail(&lines, false).into(),
                    }
                }
            };
            out.extend(entry.lines.iter().cloned());
            kept.insert(key, entry);
        }
        self.rail_cache = kept;
        out
    }

    /// The render body, without the width-diagnosis logging.
    fn render_inner(&mut self, width: u16) -> Vec<String> {
        // One palette per render pass, passed down to every row.
        let palette = Palette::current();
        if self.thinking_display == ThinkingDisplay::Omit {
            return self.render_radar(width, &palette);
        }
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
        // The blank row that opens a message is a plain blank, as it is
        // anywhere else on the screen, unless a step of the mission sits right
        // above it: then the rail runs on through it.
        let after_step = self
            .radar_above
            .as_ref()
            .is_some_and(|chain| chain.upgrade().is_some());
        let width = width as usize;
        let inner = width.saturating_sub(6).max(1) as u16;
        let lines = self.render_railed(inner, width, after_step, &palette);
        if lines.is_empty() {
            return lines;
        }
        wrap_zone(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_streaming_text() -> String {
        // Long, with blank lines so it segments.
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

    fn prose_message(text: String) -> AssistantMessage {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        AssistantMessage {
            content: vec![Content::text(text)],
            ..Default::default()
        }
    }

    /// Visible width of each rendered line, and of its content before the
    /// renderer's trailing padding. Zone markers and styling are stripped.
    fn line_extents(lines: &[String]) -> Vec<(usize, usize)> {
        lines
            .iter()
            .map(|line| {
                let plain = hoocode_tui_util::strip_vt_control_characters(line);
                (
                    hoocode_tui_util::visible_width(&plain),
                    hoocode_tui_util::visible_width(plain.trim_end()),
                )
            })
            .collect()
    }

    /// The render path hands the full body width to Markdown, which wraps
    /// at width-2 and pads back to the full width: unwrapped prose on a
    /// wide terminal must run far past column 100, not wrap there.
    #[test]
    fn wide_unwrapped_prose_wraps_at_full_width() {
        let text = (0..300)
            .map(|n| format!("word{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let message = prose_message(text);
        let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
        component.update_content(&message, false);
        let lines = component.render(200);
        let extents = line_extents(&lines);
        let content: Vec<(usize, usize)> = extents
            .into_iter()
            .filter(|(_, content)| *content > 0)
            .collect();
        assert!(!content.is_empty(), "prose renders content lines");
        for (width, _) in &content {
            assert_eq!(*width, 200, "content lines pad to the full width");
        }
        let widest = content.iter().map(|(_, c)| *c).max().unwrap_or(0);
        assert!(
            widest > 150,
            "prose wraps near the full width, not ~100 (widest {widest})"
        );
    }

    /// The reported symptom — text ending near column 100 with padding to
    /// the edge on a wide terminal — reproduces if and only if the text
    /// arrives pre-wrapped with hard breaks: the renderer cannot unwrap
    /// them, it only pads each short line out to the full width.
    #[test]
    fn prewrapped_prose_keeps_narrow_breaks_with_full_width_padding() {
        let line = "a ".repeat(50);
        assert_eq!(line.len(), 100);
        let text = [line.as_str(); 5].join("\n");
        let message = prose_message(text);
        let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
        component.update_content(&message, false);
        let lines = component.render(200);
        let extents = line_extents(&lines);
        let content: Vec<(usize, usize)> = extents
            .into_iter()
            .filter(|(_, content)| *content > 0)
            .collect();
        assert!(!content.is_empty(), "prose renders content lines");
        for (width, _) in &content {
            assert_eq!(*width, 200, "short lines pad all the way to the edge");
        }
        let widest = content.iter().map(|(_, c)| *c).max().unwrap_or(0);
        assert!(
            widest <= 101,
            "hard breaks survive rendering (widest {widest})"
        );
    }

    #[test]
    fn radar_prose_reads_plain_text() {
        let text = "# Title\n\nIt is **bold**, `code` and [a link](https://x.y/z).\n- one _em_ and snake_case\n- two 2. three";
        assert_eq!(
            plain_lines(text),
            vec![
                "Title",
                "It is bold, code and a link.",
                "one em and snake_case",
                "two 2. three"
            ]
        );
        assert_eq!(plain_lines("1) first\n2) second"), vec!["first", "second"]);
        assert_eq!(
            plain_lines("> quoted\n> text\n\n---\n"),
            vec!["quoted", "text"]
        );
        assert_eq!(plain_lines("see [open"), vec!["see [open"]);
        assert_eq!(
            plain_lines("![alt text](img.png) done"),
            vec!["alt text done"]
        );
        assert_eq!(
            plain_lines("#tag and 2024. year"),
            vec!["#tag and 2024. year"]
        );
        assert!(plain_lines("   \n\n").is_empty());
        // A fence's lines stay as written, the fence marks go.
        assert_eq!(
            plain_lines("run:\n```sh\nls *.rs\n```\ndone"),
            vec!["run:", "ls *.rs", "done"]
        );
    }
}
