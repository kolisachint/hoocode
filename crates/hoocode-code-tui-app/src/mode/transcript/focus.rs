//! U21 single view P2: the pinned view's transcript focus. `ctrl+x u` focuses
//! the newest block and pins the view, and `ctrl+x o` opens or closes all
//! tool blocks (`toggle_all_tool_rows`). With a block focused over an empty
//! prompt, `{` and `}` move the focus between tool blocks, `enter` toggles
//! the focused block, `y` copies its raw output, and `esc` clears the focus.
//! `q` and `End` clear the focus and go back to live. Any other typed key
//! clears the focus and types at the prompt.
//!
//! The scroll view queues the pinned keys as [`FocusIntent`]s (it owns no
//! chains); the mode drains them here, where the chains, the session and the
//! clipboard live. The expanded state lives on each block, not here.

use std::rc::Rc;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::Content;
use hoocode_code_media::clipboard::{copy_to_clipboard, SystemClipboardHost};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};

use super::super::*;
use crate::scroll_view::{capture_top, reanchor_top, toggle_all_plan, FocusIntent};

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

/// Scroll the focused block's header into view: the block's header row
/// (from the last render) over the chain's base. Falls back to the chain's
/// base when the header row is not placed yet. A row already comfortably in
/// view stays where it is. Nothing when the layout is not ready yet.
fn scroll_chain_into_view(
    mode: &mut Mode,
    chain: &Rc<RefCell<ToolChainComponent>>,
    block: &ToolBlock,
) {
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
    // The block's header row when the last render placed it; the chain's
    // base otherwise (never a silent no-op).
    let header = chain
        .borrow()
        .tool_blocks()
        .iter()
        .position(|b| Rc::ptr_eq(b, block))
        .and_then(|index| chain.borrow().header_start_row(index))
        .unwrap_or(0);
    mode.tui
        .scroll_to_row(((target + header) as i64).max(0), None);
}

/// Push the transcript focus into every chain, so the focused header draws
/// the `›` gutter and the paneSel fill. Runs from `set_focus` and after
/// every key (esc, q and End clear the shared state from the scroll view).
fn sync_chain_focus(mode: &mut Mode) {
    let focused = mode.focus.borrow().focused_call().map(str::to_string);
    for chain in &mode.transcript.chains {
        chain.borrow_mut().set_focused_call(focused.as_deref());
    }
}

/// Focus `block`: remember it and show its header. The pinned hint row
/// (`enter open · y copy · { } move · esc clear`) teaches the keys.
fn set_focus(mode: &mut Mode, block: &ToolBlock) {
    let call_id = block.borrow().tool_call_id().to_string();
    mode.focus.borrow_mut().focus(&call_id);
    if !mode.tui.scroll_pinned() {
        pin_to_tail(mode);
    } else if let Some(chain) = focus_chain(mode, block) {
        scroll_chain_into_view(mode, &chain, block);
    }
    sync_chain_focus(mode);
    mode.dirty.set(true);
}

/// Where `{`/`}` lands from `current` among `count` blocks: the next index,
/// or which end is already focused (named instead of re-focusing).
fn step_target(count: usize, current: Option<usize>, next: bool) -> Result<usize, &'static str> {
    if count == 0 {
        return Err("No tool calls to focus yet.");
    }
    match current {
        Some(i) if next && i + 1 >= count => Err("Already on the newest tool call."),
        Some(i) if !next && i == 0 => Err("Already on the oldest tool call."),
        Some(i) if next => Ok((i + 1).min(count - 1)),
        Some(i) => Ok(i.saturating_sub(1)),
        None => Ok(if next { 0 } else { count - 1 }),
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
        sync_chain_focus(self);
    }

    /// Focus the next or previous block; the ends say so instead of
    /// re-focusing. With no blocks, say so instead of moving.
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
        let at = match step_target(blocks.len(), index, next) {
            Ok(at) => at,
            Err(ends) => {
                self.show_status(ends);
                return;
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

    /// One primitive for both toggle paths (click + `enter`): set the block's
    /// own expanded flag. Both paths read the flag, so a click and `enter`
    /// always agree.
    pub(crate) fn set_block_expanded(&mut self, block: &ToolBlock, expanded: bool) {
        block.borrow_mut().set_expanded(expanded);
    }

    /// A header click while a block is focused moves the focus onto the
    /// clicked block, so the next `enter` or `y` acts on it. With no focus
    /// the click leaves the keyboard alone: the mouse never takes it from the
    /// prompt.
    pub(crate) fn move_focus_on_click(&mut self, block: &ToolBlock) {
        if !self.focus.borrow().is_focused() {
            return;
        }
        let call_id = block.borrow().tool_call_id().to_string();
        self.focus.borrow_mut().focus(&call_id);
        sync_chain_focus(self);
    }

    /// `enter` on a focused block: open it in place, or fold it back to one
    /// row. The header keeps its screen row while the body opens under it.
    fn toggle_focused(&mut self) {
        let Some(block) = self.focused_block() else {
            return;
        };
        let expanded = !block.borrow().is_expanded();
        if !self.tui.scroll_pinned() {
            pin_to_tail(self);
        }
        let anchor = capture_top(&self.tui, &self.transcript.chat, self.scroll_body.as_ref());
        self.set_block_expanded(&block, expanded);
        reanchor_top(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            anchor,
        );
        self.dirty.set(true);
    }

    /// `ctrl+x o`: open every tool block, or close them all when one is open.
    /// Frozen blocks are skipped (their output is gone), and an open that
    /// skips some says so in the status slot. A live view stays live, and a
    /// pinned one keeps its top row. Focus does not move.
    pub(crate) fn toggle_all_tool_rows(&mut self) {
        let blocks = focus_blocks(self);
        if blocks.is_empty() {
            self.show_status("No tool calls to open yet.");
            return;
        }
        let states: Vec<(bool, bool)> = blocks
            .iter()
            .map(|block| {
                let block = block.borrow();
                (block.is_expanded(), block.is_frozen())
            })
            .collect();
        let plan = toggle_all_plan(&states);
        let anchor = capture_top(&self.tui, &self.transcript.chat, self.scroll_body.as_ref());
        for &index in &plan.changed {
            self.set_block_expanded(&blocks[index], plan.expanded);
        }
        reanchor_top(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            anchor,
        );
        self.dirty.set(true);
        if let Some(notice) = plan.notice() {
            self.show_status(&notice);
        }
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
}

#[cfg(test)]
mod tests {
    use super::step_target;

    #[test]
    fn stepping_past_either_end_names_it() {
        assert_eq!(
            step_target(0, None, true),
            Err("No tool calls to focus yet.")
        );
        assert_eq!(step_target(3, None, true), Ok(0));
        assert_eq!(step_target(3, None, false), Ok(2));
        assert_eq!(step_target(3, Some(0), true), Ok(1));
        assert_eq!(step_target(3, Some(1), false), Ok(0));
        assert_eq!(
            step_target(3, Some(2), true),
            Err("Already on the newest tool call.")
        );
        assert_eq!(
            step_target(3, Some(0), false),
            Err("Already on the oldest tool call.")
        );
    }
}
