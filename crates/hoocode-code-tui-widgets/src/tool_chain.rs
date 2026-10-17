//! `components/tool-chain.ts`: a run of consecutive tool calls, one line in
//! a collapsed radar view and a plain pass-through everywhere else.
//!
//! A chain closes when the agent next speaks or the turn settles; closing
//! while it is still the bottom of the screen keeps the rewrite to one line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

use crate::tool_chain_summary::{
    chain_phrase, chain_segments, chain_stats, ChainState, SegmentTone,
};
use crate::tool_execution::ToolExecutionComponent;
use crate::tool_output_view::ToolOutputView;

pub type ToolBlock = Rc<RefCell<ToolExecutionComponent>>;

/// Whether an assistant message's content ends the open chain: visible text
/// does. Thinking never does, in either view.
pub fn speaking_ends_chain(content: &[Content]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, Content::Text(t) if !t.text.trim().is_empty()))
}

/// The output lines a failed call shows under its chain line.
const FAILURE_TAIL_LINES: usize = 2;

pub struct ToolChainComponent {
    blocks: Vec<ToolBlock>,
    view: ToolOutputView,
    state: ChainState,
    /// The newest run in the transcript; radar marks it.
    latest: bool,
}

impl ToolChainComponent {
    pub fn new(view: ToolOutputView) -> Self {
        Self {
            blocks: Vec::new(),
            view,
            state: ChainState::Running,
            latest: false,
        }
    }

    pub fn add(&mut self, block: ToolBlock) {
        self.blocks.push(block);
    }

    /// Takes a block out of the chain (a failed attempt's calls).
    pub fn remove(&mut self, block: &ToolBlock) {
        self.blocks.retain(|b| !Rc::ptr_eq(b, block));
    }

    /// True until the agent speaks or the turn settles.
    pub fn is_open(&self) -> bool {
        self.state == ChainState::Running
    }

    /// Whether a summary line stands in for this chain's calls right now.
    pub fn is_summarised(&self) -> bool {
        self.is_collapsed()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn tool_blocks(&self) -> &[ToolBlock] {
        &self.blocks
    }

    /// Settle the chain. An interrupted chain reads `⊘ interrupted`.
    pub fn close(&mut self, outcome: ChainState) {
        self.state = outcome;
    }

    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        for block in &self.blocks {
            block.borrow_mut().set_view(view);
        }
    }

    pub fn set_latest(&mut self, latest: bool) {
        self.latest = latest;
    }

    /// Radar summarises every chain, a lone call included, so one call and a
    /// run of calls read the same (`Ran npm run check   1 call · 412 lines`).
    fn is_collapsed(&self) -> bool {
        self.view == ToolOutputView::Radar && !self.blocks.is_empty()
    }

    /// The blank row that holds a radar run off whatever came before it.
    fn needs_lead_in(&self) -> bool {
        if self.view != ToolOutputView::Radar {
            return false;
        }
        if self.is_collapsed() {
            return true;
        }
        self.blocks
            .first()
            .is_some_and(|b| !b.borrow().draws_leading_gap())
    }

    fn rows(&mut self, width: u16) -> Vec<String> {
        if !self.is_collapsed() {
            let mut out = Vec::new();
            for block in &self.blocks {
                out.extend(block.borrow_mut().render(width));
            }
            return out;
        }
        let width = width as usize;
        let entries: Vec<_> = self
            .blocks
            .iter()
            .map(|b| b.borrow().chain_entry())
            .collect();
        // What the line reads as: interrupted when the turn was cut short;
        // running while any call is still going; otherwise done, even while
        // the chain stays open for the model's next call.
        let shown = if self.state == ChainState::Interrupted {
            ChainState::Interrupted
        } else if entries.iter().any(|e| e.is_partial) {
            ChainState::Running
        } else {
            ChainState::Done
        };
        let running = shown == ChainState::Running;
        let failed = entries.iter().any(|e| e.is_error);
        let glyph = match shown {
            ChainState::Running => "◐",
            ChainState::Interrupted if !failed => "⊘",
            ChainState::Interrupted | ChainState::Done => "●",
        };
        let glyph_tone = if failed {
            "error"
        } else {
            match shown {
                ChainState::Running => "warning",
                ChainState::Interrupted => "muted",
                ChainState::Done => "success",
            }
        };

        let t = theme();
        let (left_plain, left_styled) = if shown == ChainState::Done {
            let phrase = chain_phrase(&entries);
            (phrase.clone(), t.fg("toolTitle", &phrase))
        } else {
            let segments = chain_segments(&entries);
            let sep = " › ";
            let ellipsis = if running { "…" } else { "" };
            let plain = format!(
                "{}{ellipsis}",
                segments
                    .iter()
                    .map(|s| s.label.as_str())
                    .collect::<Vec<_>>()
                    .join(sep)
            );
            let styled = segments
                .iter()
                .map(|s| {
                    let tone = match s.tone {
                        SegmentTone::Error => "error",
                        SegmentTone::Running => "warning",
                        SegmentTone::Ok => "toolTitle",
                    };
                    t.fg(tone, &s.label)
                })
                .collect::<Vec<_>>()
                .join(&t.fg("dim", sep))
                + &(if running {
                    t.fg("dim", "…")
                } else {
                    String::new()
                });
            (plain, styled)
        };

        let stats = chain_stats(&entries, shown);
        let stroke = self.latest && t.has_bg("activeToolBg");
        let mark = |text: &str| {
            if stroke {
                t.bg("activeToolBg", text)
            } else {
                text.to_string()
            }
        };
        let prefix = format!("{glyph} ");
        let budget = width.saturating_sub(1 + visible_width(&prefix));
        let stats_width = visible_width(&stats);
        // The stats always stay, flush right. The label gives way first.
        let line = if visible_width(&left_plain) + 2 + stats_width <= budget {
            let pad = " ".repeat(budget - visible_width(&left_plain) - stats_width);
            format!(
                " {}{}{pad}{}",
                t.fg(glyph_tone, &prefix),
                mark(&left_styled),
                t.fg("muted", &stats)
            )
        } else if budget >= stats_width + 3 {
            let label = truncate_to_width(&left_styled, budget - stats_width - 2, "…", false);
            let pad = " ".repeat(budget - visible_width(&label) - stats_width);
            format!(
                " {}{}{pad}{}",
                t.fg(glyph_tone, &prefix),
                mark(&label),
                t.fg("muted", &stats)
            )
        } else {
            // Too narrow for a label at all: the stats are cut as a last resort.
            format!(
                " {}{}",
                t.fg(glyph_tone, &prefix),
                t.fg("muted", &truncate_to_width(&stats, budget, "…", false))
            )
        };
        let mut out = vec![line];
        out.extend(self.failure_lines(width));
        out
    }

    /// Each failed call's header and the last lines of its reason, under the
    /// chain line. A longer reason folds to its tail, with the way to see it all.
    fn failure_lines(&self, width: usize) -> Vec<String> {
        let t = theme();
        let mut lines = Vec::new();
        for block in &self.blocks {
            let block = block.borrow();
            let entry = block.chain_entry();
            if !entry.is_error {
                continue;
            }
            let header = format!(
                "    {} {}  {}",
                t.fg("error", "✗"),
                t.fg("toolTitle", &entry.tool),
                t.fg("toolOutput", &entry.subject)
            );
            lines.push(truncate_to_width(&header, width, "…", false));
            let text = block.error_text();
            let reason: Vec<&str> = text
                .split('\n')
                .filter(|raw| !crate::is_blank(raw))
                .collect();
            let hidden = reason.len().saturating_sub(FAILURE_TAIL_LINES);
            for raw in &reason[hidden..] {
                lines.push(truncate_to_width(
                    &format!("      {}", t.fg("toolOutput", raw)),
                    width,
                    "…",
                    false,
                ));
            }
            if hidden > 0 {
                let noun = if hidden == 1 { "line" } else { "lines" };
                lines.push(truncate_to_width(
                    &format!(
                        "      {}",
                        t.fg("muted", &format!("… {hidden} more {noun} · ctrl+o"))
                    ),
                    width,
                    "…",
                    false,
                ));
            }
        }
        lines
    }
}

impl Component for ToolChainComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        let rows = self.rows(width);
        if rows.is_empty() || !self.needs_lead_in() {
            return rows;
        }
        let mut out = Vec::with_capacity(rows.len() + 1);
        out.push(String::new());
        out.extend(rows);
        out
    }

    fn invalidate(&mut self) {
        for block in &self.blocks {
            block.borrow_mut().invalidate();
        }
    }
}
