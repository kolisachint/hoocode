//! `components/custom-message.ts`, `components/branch-summary-message.ts`
//! and `components/compaction-summary-message.ts`: the message kinds drawn on
//! the custom-message fill.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_agent_types::{BranchSummaryMessage, CompactionSummaryMessage, CustomMessage};
use hoocode_ai_types::{Content, UserContent};
use hoocode_code_agent_session::format::group_digits;
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::{apply_block_fill, message_label, theme, BlockFill};
use hoocode_tui_components::{
    BoxComponent, DefaultTextStyle, Markdown, MarkdownTheme, Spacer, Text,
};
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{Palette, RatatuiComponent};
use hoocode_tui_render::{Component, Container};
use hoocode_tui_util::{truncate_at_word, truncate_to_width, visible_width};

fn custom_text_markdown(text: &str, markdown_theme: MarkdownTheme) -> Markdown {
    Markdown::new(
        text,
        0,
        0,
        markdown_theme,
        Some(DefaultTextStyle {
            color: Some(Box::new(|s: &str| theme().fg("customMessageText", s))),
            ..Default::default()
        }),
    )
}

/// The hint a folded message ends with: Ctrl+O opens it at peek.
fn peek_hint() -> String {
    theme().fg("dim", &peek_hint_text())
}

/// The folded hint as plain text, for rows drawn with ratatui.
fn peek_hint_text() -> String {
    format!(" · {} for peek", key_text("app.tools.expand"))
}

/// The folded (outline) line: label, plain summary and peek hint on one row.
/// The summary gives way first: it is cut to the room the label and hint
/// leave. If that room is under 10 columns the hint is dropped. The row is
/// never wrapped onto a second row.
struct OneRow {
    label: String,
    summary: String,
    hint: String,
}

impl Component for OneRow {
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let label_width = visible_width(&self.label);
        let mut hint = self.hint.as_str();
        let mut room = width.saturating_sub(label_width + 1 + visible_width(hint));
        if room < 10 {
            hint = "";
            room = width.saturating_sub(label_width + 1);
        }
        let summary = truncate_to_width(&self.summary, room, "…", false);
        let line = format!(
            "{} {}{}",
            self.label,
            theme().fg("customMessageText", &summary),
            hint
        );
        vec![truncate_to_width(&line, width, "…", false)]
    }

    fn invalidate(&mut self) {}
}

/// `CustomMessageComponent`: a displayed custom message, as its label over
/// its markdown. Outline folds it to its label and first line; peek (the
/// default) shows all of it. (Extension message renderers arrive with 12.3.)
pub struct CustomMessageComponent {
    message: CustomMessage,
    markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    expanded: bool,
    container: Container,
}

impl CustomMessageComponent {
    pub fn new(message: CustomMessage, markdown_theme: Rc<dyn Fn() -> MarkdownTheme>) -> Self {
        let mut this = Self {
            message,
            markdown_theme,
            expanded: true,
            container: Container::new(),
        };
        this.rebuild();
        this
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rebuild();
    }

    /// Whether the block's own toggle (UI-21 P4) is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    fn rebuild(&mut self) {
        let mut sheet = BoxComponent::new(1, 1, None);
        apply_block_fill(&mut sheet, BlockFill::CustomMessageBg);
        let text = match &self.message.content {
            UserContent::Text(text) => text.clone(),
            UserContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|c| match c {
                    Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        if self.expanded {
            sheet.add_child(Rc::new(RefCell::new(Text::new(
                message_label(&self.message.custom_type),
                0,
                0,
            ))));
            sheet.add_child(Rc::new(RefCell::new(Spacer::new(1))));
            sheet.add_child(Rc::new(RefCell::new(custom_text_markdown(
                &text,
                (self.markdown_theme)(),
            ))));
        } else {
            let first = text.lines().map(str::trim).find(|l| !l.is_empty());
            sheet.add_child(Rc::new(RefCell::new(OneRow {
                label: message_label(&self.message.custom_type),
                summary: first.unwrap_or("").to_string(),
                hint: peek_hint(),
            })));
        }
        self.container.clear();
        self.container
            .add_child(Rc::new(RefCell::new(Spacer::new(1))));
        self.container.add_child(Rc::new(RefCell::new(sheet)));
    }
}

impl Component for CustomMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.rebuild();
    }
}

/// A summary message's sheet: its label, then either the folded line (the
/// caller's text in the folded style, with the peek hint) or the markdown body.
fn summary_sheet(
    label: String,
    folded: &str,
    body: &str,
    expanded: bool,
    markdown_theme: &Rc<dyn Fn() -> MarkdownTheme>,
) -> BoxComponent {
    let mut sheet = BoxComponent::new(1, 1, None);
    apply_block_fill(&mut sheet, BlockFill::CustomMessageBg);
    sheet.add_child(Rc::new(RefCell::new(Text::new(label, 0, 0))));
    sheet.add_child(Rc::new(RefCell::new(Spacer::new(1))));
    if expanded {
        sheet.add_child(Rc::new(RefCell::new(custom_text_markdown(
            body,
            (markdown_theme)(),
        ))));
    } else {
        sheet.add_child(Rc::new(RefCell::new(Text::new(
            format!("{}{}", theme().fg("customMessageText", folded), peek_hint()),
            0,
            0,
        ))));
    }
    sheet
}

/// `BranchSummaryMessageComponent`: the summary of an abandoned branch,
/// folded to one line until expanded.
pub struct BranchSummaryMessageComponent {
    message: BranchSummaryMessage,
    markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    expanded: bool,
    sheet: BoxComponent,
}

impl BranchSummaryMessageComponent {
    pub fn new(
        message: BranchSummaryMessage,
        markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    ) -> Self {
        let mut this = Self {
            message,
            markdown_theme,
            expanded: false,
            sheet: BoxComponent::new(1, 1, None),
        };
        this.update_display();
        this
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.update_display();
    }

    /// Whether the block's own toggle (UI-21 P4) is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    fn update_display(&mut self) {
        let body = format!("**Branch Summary**\n\n{}", self.message.summary);
        self.sheet = summary_sheet(
            message_label("branch"),
            "Branch summary",
            &body,
            self.expanded,
            &self.markdown_theme,
        );
    }
}

impl Component for BranchSummaryMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.sheet.render(width)
    }

    fn invalidate(&mut self) {
        self.update_display();
    }
}

/// The compaction row's parts (UI-21 P4 cut order): the label, the primary
/// fact (never dropped: `saved N`, `grew N`, or `from N tokens` when the
/// range is unknown), and the token range (secondary: dropped first when
/// the row is too wide). `earlier turns` covers summaries from before
/// compaction knew the transcript's turns.
pub(crate) fn jettison_parts(
    message: &CompactionSummaryMessage,
) -> (String, String, Option<String>) {
    let range = match message.mission_range {
        Some((first, last)) if first == last => format!("turn {first}"),
        Some((first, last)) => format!("turns {first}–{last}"),
        None => "earlier turns".to_string(),
    };
    let label = format!("Compacted {range}");
    match (message.tokens_before, message.tokens_after) {
        (before, Some(after)) if before > 0 => {
            let change = if after >= before {
                format!("grew {}", group_digits(after - before))
            } else {
                format!("saved {}", group_digits(before - after))
            };
            let range = format!("{} → {} tokens", group_digits(before), group_digits(after));
            (label, change, Some(range))
        }
        (before, _) => (label, format!("from {} tokens", group_digits(before)), None),
    }
}

/// The compaction row's text: the label, the primary fact, then the token
/// range.
pub(crate) fn jettison_text(message: &CompactionSummaryMessage) -> String {
    let (label, primary, secondary) = jettison_parts(message);
    match secondary {
        Some(range) => format!("{label} · {primary} · {range}"),
        None => format!("{label} · {primary}"),
    }
}

/// The folded compaction row as ratatui spans, exactly `width` cells: the
/// row in dim with the peek hint. The hint shows only when whole; past that
/// the row is cut short, and blanks pad to the full width.
fn jettison_spans(text: &str, hint: &str, width: usize, pal: &Palette) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let push_vio = |text: String, spans: &mut Vec<Span<'static>>| {
        spans.push(Span::styled(text, pal.dim()));
    };
    let (text_width, hint_width) = (visible_width(text), visible_width(hint));
    if text_width + hint_width <= width {
        push_vio(text.to_string(), &mut spans);
        spans.push(Span::styled(hint.to_string(), pal.dim()));
    } else if text_width < width {
        // The hint would not fit whole: the row keeps the line.
        push_vio(text.to_string(), &mut spans);
    } else {
        push_vio(truncate_to_width(text, width, "…", false), &mut spans);
        return spans;
    }
    let used: usize = spans
        .iter()
        .map(|s| visible_width(s.content.as_ref()))
        .sum();
    if used < width {
        push_vio(" ".repeat(width - used), &mut spans);
    }
    spans
}

/// The folded compaction row with the UI-21 cut order: the full
/// `label · primary · secondary` row when it fits (with the hint when that
/// fits too), else the row without its secondary token range, else the
/// label cut short with the primary `saved N` fact kept while it fits on
/// its own. Past that the row is cut short as before.
fn jettison_cut_row(message: &CompactionSummaryMessage, hint: String, width: usize) -> Vec<String> {
    let (label, primary, secondary) = jettison_parts(message);
    let short = format!("{label} · {primary}");
    if let Some(range) = &secondary {
        let full = format!("{short} · {range}");
        if visible_width(&full) <= width {
            return jettison_row(full, hint, width);
        }
    }
    if visible_width(&short) <= width {
        return jettison_row(short, hint, width);
    }
    let keep = format!(" · {primary}");
    let keep_width = visible_width(&keep);
    if keep_width < width {
        // Cut at a word; `truncate_at_word` leaves plain text plain (its
        // reset only closes a style the input opened), so no escape lands
        // in the ratatui spans.
        let cut = truncate_at_word(&label, width - keep_width);
        return jettison_row(format!("{cut}{keep}"), hint, width);
    }
    jettison_row(short, hint, width)
}

/// The folded compaction row: one ratatui line, `width` cells wide.
fn jettison_row(text: String, hint: String, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let render_width = width.min(u16::MAX as usize) as u16;
    // One palette per render pass, not per draw: the closure may draw again
    // after an invalidate.
    let pal = Palette::current();
    let mut row = RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let spans = jettison_spans(&text, &hint, usize::from(area.width), &pal);
        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    });
    row.render(render_width)
}

/// `CompactionSummaryMessageComponent`: what a compaction kept, folded to
/// the compaction row until expanded.
pub struct CompactionSummaryMessageComponent {
    message: CompactionSummaryMessage,
    markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    expanded: bool,
    sheet: BoxComponent,
}

impl CompactionSummaryMessageComponent {
    pub fn new(
        message: CompactionSummaryMessage,
        markdown_theme: Rc<dyn Fn() -> MarkdownTheme>,
    ) -> Self {
        let mut this = Self {
            message,
            markdown_theme,
            expanded: false,
            sheet: BoxComponent::new(1, 1, None),
        };
        this.update_display();
        this
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.update_display();
    }

    /// Whether the block's own toggle (UI-21 P4) is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    fn update_display(&mut self) {
        // The sheet is the expanded view; folded, the message is the
        // compaction row drawn with ratatui.
        let summary_text = jettison_text(&self.message);
        let body = format!("**{summary_text}**\n\n{}", self.message.summary);
        self.sheet = summary_sheet(
            message_label("compaction"),
            &summary_text,
            &body,
            self.expanded,
            &self.markdown_theme,
        );
    }
}

impl Component for CompactionSummaryMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.expanded {
            return self.sheet.render(width);
        }
        jettison_cut_row(&self.message, peek_hint_text(), width as usize)
    }

    fn invalidate(&mut self) {
        self.update_display();
    }
}
