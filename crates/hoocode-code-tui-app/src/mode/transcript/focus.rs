//! U21 single view P2: the pinned view's transcript focus. `{` and `}` move
//! the focus between tool blocks, `enter` toggles the focused block,
//! `y` copies its raw output, `esc` clears the focus, and `G` goes back to
//! live. `ctrl+x u` focuses the newest block and pins the view.
//!
//! The scroll view queues the pinned keys as [`FocusIntent`]s (it owns no
//! chains); the mode drains them here, where the chains, the session and the
//! clipboard live. The per-block toggle stops are kept as overrides on the
//! shared [`TranscriptFocus`], so they survive re-renders; the tool output
//! dial wipes them when it moves every block at once.

use std::rc::Rc;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::Content;
use hoocode_code_media::clipboard::{copy_to_clipboard, SystemClipboardHost};
use hoocode_code_paths::{agent_dir, VERSION};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};

use super::super::*;
use crate::scroll_view::{capture_top, reanchor_top, take_focus_notice, FocusIntent};

/// What focusing the newest block pins: the tail, so the block is on screen.
fn pin_to_tail(mode: &mut Mode) {
    mode.tui.scroll_to_row(i64::MAX, None);
}

/// The tool blocks of the transcript in order, across every chain.
fn focus_blocks(mode: &Mode) -> Vec<ToolBlock> {
    mode.transcript
        .chains
        .iter()
        .flat_map(|chain| chain.borrow().tool_blocks().to_vec())
        .collect()
}

/// The chain holding `block`, if it is still in the transcript.
fn focus_chain(mode: &Mode, block: &ToolBlock) -> Option<Rc<RefCell<ToolChainComponent>>> {
    mode.transcript
        .chains
        .iter()
        .find(|chain| {
            chain
                .borrow()
                .tool_blocks()
                .iter()
                .any(|b| Rc::ptr_eq(b, block))
        })
        .cloned()
}

/// Scroll the focused block's chain into view (chain granularity: the block
/// rows live inside the widget). A row already comfortably in view stays
/// where it is. Nothing when the layout is not ready yet.
fn scroll_chain_into_view(mode: &mut Mode, chain: &Rc<RefCell<ToolChainComponent>>) {
    let width = mode.tui.transcript_width();
    let chat = &mode.transcript.chat;
    let index = match chat
        .borrow()
        .children()
        .iter()
        .position(|c| Rc::ptr_eq(c, &as_component(chain)))
    {
        Some(index) => index,
        None => return,
    };
    let rel = match chat.borrow().child_row_offsets(width) {
        Some(offsets) => match offsets.get(index).copied() {
            Some(rel) => rel,
            None => return,
        },
        None => return,
    };
    let target = match &mode.scroll_body {
        Some(body) => {
            let body_borrow = body.borrow();
            let offsets = match body_borrow.child_row_offsets(width) {
                Some(offsets) => offsets,
                None => return,
            };
            let chat_index = match body_borrow
                .children()
                .iter()
                .position(|c| Rc::ptr_eq(c, &as_component(chat)))
            {
                Some(chat_index) => chat_index,
                None => return,
            };
            match offsets.get(chat_index).copied() {
                Some(chat_base) => chat_base + rel,
                None => return,
            }
        }
        None => rel,
    };
    mode.tui.scroll_to_row((target as i64).max(0), None);
}

/// Focus `block`: remember it, teach the keys once, and show its chain.
fn set_focus(mode: &mut Mode, block: &ToolBlock) {
    let call_id = block.borrow().tool_call_id().to_string();
    mode.focus.borrow_mut().focus(&call_id);
    maybe_show_focus_notice(mode);
    if !mode.tui.scroll_pinned() {
        pin_to_tail(mode);
    } else if let Some(chain) = focus_chain(mode, block) {
        scroll_chain_into_view(mode, &chain);
    }
    mode.dirty.set(true);
}

/// The one-time focus-keys notice, once per version. The state file keeps its
/// other keys (such as the Alt notice): the write merges, it never replaces.
fn maybe_show_focus_notice(mode: &mut Mode) {
    if let Some(notice) = take_focus_notice(
        &agent_dir().join(hoocode_code_tui_keybindings::STATE_FILE_NAME),
        VERSION,
    ) {
        mode.show_status(&notice);
    }
}

impl Mode {
    /// Run the transcript keys the pinned view queued, in order.
    pub(crate) fn drain_focus_intents(&mut self) {
        let intents = self.scroll_view.borrow_mut().take_focus_intents();
        for intent in intents {
            match intent {
                FocusIntent::Previous => self.focus_step(false),
                FocusIntent::Next => self.focus_step(true),
                FocusIntent::Toggle => self.toggle_focused(),
                FocusIntent::Copy => self.copy_focused(),
            }
        }
    }

    /// Focus the block `intent` names; done blocks are the only ones listed.
    /// With no blocks, say so instead of moving.
    fn focus_step(&mut self, next: bool) {
        let blocks = focus_blocks(self);
        if blocks.is_empty() {
            self.show_status("No tool calls to focus yet.");
            return;
        }
        let current = self.focus.borrow().focused_call().map(str::to_string);
        let index = current.and_then(|call| {
            blocks
                .iter()
                .position(|b| b.borrow().tool_call_id() == call)
        });
        let at = match index {
            Some(i) if next => (i + 1).min(blocks.len() - 1),
            Some(i) => i.saturating_sub(1),
            None => {
                if next {
                    0
                } else {
                    blocks.len() - 1
                }
            }
        };
        let block = blocks[at].clone();
        set_focus(self, &block);
    }

    /// `ctrl+x u`: focus the newest block and pin the view.
    pub(crate) fn focus_newest_and_pin(&mut self) {
        let newest = focus_blocks(self).pop();
        match newest {
            Some(block) => set_focus(self, &block),
            None => self.show_status("No tool calls to focus yet."),
        }
    }

    /// The focused block, if its call is still in the transcript. A stale id
    /// (a retry took its chain out) clears the focus instead.
    fn focused_block(&mut self) -> Option<ToolBlock> {
        let call = self.focus.borrow().focused_call().map(str::to_string)?;
        let found = focus_blocks(self)
            .into_iter()
            .find(|b| b.borrow().tool_call_id() == call);
        if found.is_none() {
            self.focus.borrow_mut().clear();
            self.dirty.set(true);
        }
        found
    }

    /// Whether the focused block draws expanded: its toggle stop, else the
    /// dial's stop (only the full stop opens every block).
    fn focus_expanded(&self, call_id: &str) -> bool {
        self.focus
            .borrow()
            .expanded_override(call_id)
            .unwrap_or(self.tool_output_view == ToolOutputView::Full)
    }

    /// `enter` on a focused block: open it in place, or fold it back to one
    /// row. The header keeps its screen row while the body opens under it.
    fn toggle_focused(&mut self) {
        let Some(block) = self.focused_block() else {
            return;
        };
        let call_id = block.borrow().tool_call_id().to_string();
        let expanded = !self.focus_expanded(&call_id);
        if !self.tui.scroll_pinned() {
            pin_to_tail(self);
        }
        let anchor = capture_top(&self.tui, &self.transcript.chat, self.scroll_body.as_ref());
        block.borrow_mut().set_view(if expanded {
            ToolOutputView::Full
        } else {
            ToolOutputView::Outline
        });
        self.focus.borrow_mut().set_expanded(&call_id, expanded);
        reanchor_top(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            anchor,
        );
        self.dirty.set(true);
    }

    /// The session's raw result text for one tool call: the last result with
    /// the id, text segments joined, images left out. Rebuilds read the same
    /// messages the live path drew.
    fn tool_result_text(&self, call_id: &str) -> Option<String> {
        let mut out = None;
        for message in self.session.messages() {
            if let AgentMessage::ToolResult(result) = message {
                if result.tool_call_id == call_id {
                    out = Some(
                        result
                            .content
                            .iter()
                            .filter_map(|content| match content {
                                Content::Text(text) => Some(text.text.as_str()),
                                _ => None,
                            })
                            .collect::<String>(),
                    );
                }
            }
        }
        out
    }

    /// `y` on a focused block: copy the raw output off the UI thread, never
    /// the rendered rows. Names the tool, so the notice says what landed.
    fn copy_focused(&mut self) {
        let Some(block) = self.focused_block() else {
            return;
        };
        let tool_name = block.borrow().tool_name().to_string();
        let call_id = block.borrow().tool_call_id().to_string();
        let raw = self.tool_result_text(&call_id);
        let Some(raw) = raw.filter(|text| !text.trim().is_empty()) else {
            self.show_status(&format!("{tool_name}: no output to copy yet."));
            return;
        };
        let subject = format!("{tool_name} output");
        let tx = self.tx.clone();
        hoocode_runtime::spawn_thread("hoocode-ui-task", move || {
            let host = SystemClipboardHost {
                native: super::super::clipboard::native_clipboard_writer(),
            };
            let result = copy_to_clipboard(&host, &raw)
                .map(|()| raw.len())
                .map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::CopyRawDone(subject, result));
        });
    }

    /// The end of a `y` copy: name what landed, in bytes.
    pub(crate) fn finish_raw_copy(&mut self, subject: &str, result: Result<usize, String>) {
        match result {
            Ok(bytes) => self.show_status(&format!("Copied {subject} ({bytes} bytes).")),
            Err(reason) => self.show_error(&format!("Could not copy the {subject}: {reason}.")),
        }
    }

    /// Forget the focused toggle stops (the dial moved every block at once).
    pub(crate) fn clear_focus_overrides(&mut self) {
        self.focus.borrow_mut().clear_expanded();
    }
}
