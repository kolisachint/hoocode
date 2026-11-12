//! The blocks of the transcript that open in place, and the pinned view's
//! focus on one of them. One path for all of it: a click on a header row or
//! a fold line, `ctrl+x u` (focus the newest block and pin the view), `{`
//! and `}` (move the focus), `enter` (toggle the focused block), `y` (copy
//! its raw text), and `ctrl+x o` (open or close them all). `esc` clears the
//! focus. `q` and `End` clear the focus and go back to live. Any other
//! typed key clears the focus and types at the prompt.
//!
//! Every path reads the same list (`chat_folds`: each chat child offers its
//! blocks through `Component::fold_host`) and writes the block's own
//! expanded flag through `FoldHost::set_fold_expanded`, so a click, `enter`
//! and `ctrl+x o` always agree. A tool chain offers its tool rows and the
//! thinking traces above them. Other chat children offer their own block.
//!
//! The scroll view queues the pinned keys as [`FocusIntent`]s (it owns no
//! blocks); the mode drains them here, where the chat, the session and the
//! clipboard live. The expanded state lives on each block, not here.

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::Content;
use hoocode_code_media::clipboard::{copy_to_clipboard, SystemClipboardHost};
use hoocode_tui_render::Fold;

use super::super::*;
use crate::scroll_view::{
    capture_top, chat_folds, find_fold, fold_at, fold_buffer_row, reanchor_top, set_fold_held,
    toggle_all_plan, FocusIntent, FoldHit, FoldRef,
};

/// What focusing the newest block pins: the tail, so the block is on screen.
fn pin_to_tail(mode: &mut Mode) {
    mode.tui.scroll_to_row(i64::MAX, None);
}

/// The blocks of the transcript that open in place, in order. A thinking
/// trace hidden by `ctrl+x b` is not one, so the walk skips it.
fn focus_folds(mode: &Mode) -> Vec<FoldRef> {
    chat_folds(&mode.transcript.chat, mode.tui.transcript_width())
}

/// Scroll the focused block's header into view, from the last layout. A
/// row already comfortably in view stays where it is. Nothing when the
/// layout is not ready yet.
fn scroll_fold_into_view(mode: &mut Mode, id: &str) {
    let row = fold_buffer_row(
        &mode.tui,
        &mode.transcript.chat,
        mode.scroll_body.as_ref(),
        id,
    );
    if let Some(row) = row {
        mode.tui.scroll_to_row(row.max(0), None);
    }
}

/// Push the transcript focus into every chat child that holds blocks, so
/// the focused header draws the `›` gutter and the selection fill. Runs
/// from `set_focus` and after every key (esc, q and End clear the shared
/// state from the scroll view). A child takes a new render stamp only when
/// its focus changes.
fn sync_fold_focus(mode: &mut Mode) {
    let focused = mode.focus.borrow().focused_call().map(str::to_string);
    let children = mode.transcript.chat.borrow().children().clone();
    for child in children {
        if let Ok(mut component) = child.try_borrow_mut() {
            if let Some(host) = component.fold_host_mut() {
                host.set_fold_focus(focused.as_deref());
            }
        }
    }
}

/// Focus the block `id`: remember it and show its header. The pinned hint
/// row (`enter open · y copy · { } move · esc clear`) teaches the keys.
fn set_focus(mode: &mut Mode, id: &str) {
    mode.focus.borrow_mut().focus(id);
    if !mode.tui.scroll_pinned() {
        pin_to_tail(mode);
    } else {
        scroll_fold_into_view(mode, id);
    }
    sync_fold_focus(mode);
    mode.dirty.set(true);
}

/// Where `{`/`}` lands from `current` among `count` blocks: the next index,
/// or which end is already focused (named instead of re-focusing).
fn step_target(count: usize, current: Option<usize>, next: bool) -> Result<usize, &'static str> {
    if count == 0 {
        return Err(NO_BLOCKS);
    }
    match current {
        Some(i) if next && i + 1 >= count => Err("Already on the newest block."),
        Some(i) if !next && i == 0 => Err("Already on the oldest block."),
        Some(i) if next => Ok((i + 1).min(count - 1)),
        Some(i) => Ok(i.saturating_sub(1)),
        None => Ok(if next { 0 } else { count - 1 }),
    }
}

/// What the focus keys say when the transcript has no block to focus.
const NO_BLOCKS: &str = "No blocks to focus yet.";

/// What a click or `enter` says on a block that cannot open, so it never
/// looks like the key did nothing.
fn cannot_open_notice(fold: &Fold) -> String {
    if fold.label == "TodoWrite" {
        return "TodoWrite: the plan is in the task panel.".to_string();
    }
    format!("{}: no output to show.", fold.label)
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
        sync_fold_focus(self);
    }

    /// Focus the next or previous block; the ends say so instead of
    /// re-focusing. With no blocks, say so instead of moving.
    fn focus_step(&mut self, next: bool) {
        let folds = focus_folds(self);
        let current = self.focus.borrow().focused_call().map(str::to_string);
        let index = current.and_then(|id| folds.iter().position(|found| found.fold.id == id));
        match step_target(folds.len(), index, next) {
            Ok(at) => {
                let id = folds[at].fold.id.clone();
                set_focus(self, &id);
            }
            Err(ends) => self.show_status(ends),
        }
    }

    /// `ctrl+x u`: focus the newest block and pin the view.
    pub(crate) fn focus_newest_and_pin(&mut self) {
        match focus_folds(self).pop() {
            Some(found) => set_focus(self, &found.fold.id),
            None => self.show_status(NO_BLOCKS),
        }
    }

    /// The focused block, if the transcript still holds it. A stale id (a
    /// retry took its chain out) clears the focus instead.
    fn focused_fold(&mut self) -> Option<FoldRef> {
        let id = self.focus.borrow().focused_call().map(str::to_string)?;
        let found = find_fold(&self.transcript.chat, self.tui.transcript_width(), &id);
        if found.is_none() {
            self.focus.borrow_mut().clear();
            self.dirty.set(true);
        }
        found
    }

    /// A click while a block is focused moves the focus onto the clicked
    /// block, so the next `enter` or `y` acts on it. With no focus the
    /// click leaves the keyboard alone: the mouse never takes it from the
    /// prompt.
    fn move_focus_on_click(&mut self, id: &str) {
        if !self.focus.borrow().is_focused() {
            return;
        }
        self.focus.borrow_mut().focus(id);
        sync_fold_focus(self);
    }

    /// A click on chat row `rel` (laid out at `width`), `context` screen
    /// rows under the transcript's first screen row. On a header row it
    /// toggles the block, on a fold line it opens it, and the header keeps
    /// its screen row. A block that cannot open says so in the status slot.
    /// Any other row does nothing.
    pub(crate) fn click_fold(&mut self, rel: usize, width: u16, context: i64) {
        let Some((found, hit)) = fold_at(&self.transcript.chat, width, rel) else {
            return;
        };
        self.move_focus_on_click(&found.fold.id);
        self.dirty.set(true);
        if !found.fold.can_open {
            self.show_status(&cannot_open_notice(&found.fold));
            return;
        }
        let expanded = match hit {
            FoldHit::Header => !found.fold.expanded,
            FoldHit::FoldLine => true,
        };
        // The header's own screen row: the clicked row, less the rows the
        // click sits under the header's first row.
        let under = found
            .header_row()
            .map_or(0, |header| rel.saturating_sub(header));
        set_fold_held(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            &found.host,
            &found.fold.id,
            expanded,
            Some(context - under as i64),
        );
    }

    /// `enter` on a focused block: open it in place, or fold it back to
    /// its header. A header in view keeps its screen row while the body
    /// opens under it; with the header off screen the top row stays put.
    fn toggle_focused(&mut self) {
        let Some(found) = self.focused_fold() else {
            return;
        };
        self.dirty.set(true);
        if !found.fold.can_open {
            self.show_status(&cannot_open_notice(&found.fold));
            return;
        }
        let screen = fold_buffer_row(
            &self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            &found.fold.id,
        )
        .and_then(|row| self.tui.transcript_screen_row(row));
        set_fold_held(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            &found.host,
            &found.fold.id,
            !found.fold.expanded,
            screen,
        );
    }

    /// `ctrl+x o`: open every block that can open, or close the open ones
    /// when any is open. A live view stays live, and a pinned one keeps its
    /// top row. Focus does not move.
    pub(crate) fn toggle_all_tool_rows(&mut self) {
        let folds = focus_folds(self);
        let states: Vec<(bool, bool)> = folds
            .iter()
            .map(|found| (found.fold.expanded, found.fold.can_open))
            .collect();
        let plan = toggle_all_plan(&states);
        if plan.changed.is_empty() {
            self.show_status("No blocks to open yet.");
            return;
        }
        let anchor = capture_top(&self.tui, &self.transcript.chat, self.scroll_body.as_ref());
        for &index in &plan.changed {
            let found = &folds[index];
            if let Some(host) = found.host.borrow_mut().fold_host_mut() {
                host.set_fold_expanded(&found.fold.id, plan.expanded);
            }
        }
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

    /// `y` on a focused block: copy its raw text off the UI thread, never
    /// the rendered rows. A tool call's output comes from the session; any
    /// other block gives its own text. Names the block, so the notice says
    /// what landed.
    fn copy_focused(&mut self) {
        let Some(found) = self.focused_fold() else {
            return;
        };
        let label = found.fold.label.clone();
        let own = found
            .host
            .borrow()
            .fold_host()
            .and_then(|host| host.fold_text(&found.fold.id));
        let (raw, subject) = match own {
            Some(text) => (Some(text), label.clone()),
            None => (
                self.tool_result_text(&found.fold.id),
                format!("{label} output"),
            ),
        };
        let Some(raw) = raw.filter(|text| !text.trim().is_empty()) else {
            self.show_status(&format!("{label}: no output to copy yet."));
            return;
        };
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
        assert_eq!(step_target(0, None, true), Err("No blocks to focus yet."));
        assert_eq!(step_target(3, None, true), Ok(0));
        assert_eq!(step_target(3, None, false), Ok(2));
        assert_eq!(step_target(3, Some(0), true), Ok(1));
        assert_eq!(step_target(3, Some(1), false), Ok(0));
        assert_eq!(
            step_target(3, Some(2), true),
            Err("Already on the newest block.")
        );
        assert_eq!(
            step_target(3, Some(0), false),
            Err("Already on the oldest block.")
        );
    }
}

/// The blocks that open in place, through the paths a person uses: a click
/// on the header row, `enter` on the focus, `ctrl+x o`, and the focus walk.
/// Every kind of block of the transcript goes through the same list.
#[cfg(test)]
mod block_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use hoocode_agent_types::{BranchSummaryMessage, CompactionSummaryMessage, CustomMessage};
    use hoocode_ai_types::{AssistantMessage, Content, ThinkingContent, UserContent};
    use hoocode_code_tui_widgets::background_notice::{
        BackgroundNotice, BackgroundNoticeComponent, NoticeVerdict,
    };
    use hoocode_code_tui_widgets::custom_message::{
        BranchSummaryMessageComponent, CompactionSummaryMessageComponent, CustomMessageComponent,
    };
    use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolChainComponent};
    use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
    use hoocode_code_tui_widgets::tool_signal::ToolResult;
    use hoocode_code_tui_widgets::tools::builtin_tool_definition;
    use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
    use hoocode_tui_render::{Component, ComponentHandle};

    use super::super::super::test_support::test_mode;
    use super::*;
    use crate::expandable_text::ExpandableText;

    fn add<C: Component + 'static>(mode: &mut Mode, component: C) {
        let handle: ComponentHandle = Rc::new(RefCell::new(component));
        mode.add_to_chat(handle);
    }

    fn thinking_message() -> AssistantMessage {
        AssistantMessage {
            content: vec![Content::Thinking(ThinkingContent {
                thinking: "Weighing the options. The first one hides a trap.".into(),
                signature: Some(String::new()),
                ..Default::default()
            })],
            ..Default::default()
        }
    }

    /// One block of every kind that opens in place besides a tool call.
    fn mode_with_every_block(display: ThinkingDisplay) -> Mode {
        let mut mode = test_mode(None);
        let markdown = mode.markdown_theme();
        add(
            &mut mode,
            AssistantMessageComponent::with_theme(
                Some(&thinking_message()),
                display,
                markdown.clone(),
                "",
            ),
        );
        add(
            &mut mode,
            SkillBlockComponent::new("write-tests", "one\ntwo\nthree"),
        );
        add(
            &mut mode,
            BackgroundNoticeComponent::with_elapsed(
                BackgroundNotice {
                    label: "explore#4".into(),
                    description: "audit the db layer".into(),
                    call_id: None,
                    verdict: NoticeVerdict::Finished,
                    partial: false,
                    summary: "Mapped the module".into(),
                    model: Some("haiku-5-5".into()),
                    effort: None,
                },
                Some("1m27s".into()),
            ),
        );
        let mut bash = BashExecutionComponent::new("cargo test", false);
        bash.append_output("one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        bash.set_complete(Some(0), false, None, None);
        add(&mut mode, bash);
        add(
            &mut mode,
            CompactionSummaryMessageComponent::new(
                CompactionSummaryMessage {
                    summary: "What the turns settled.".into(),
                    tokens_before: 12_400,
                    tokens_after: Some(3_100),
                    timestamp: 0,
                    mission_range: Some((1, 3)),
                },
                markdown.clone(),
            ),
        );
        add(
            &mut mode,
            BranchSummaryMessageComponent::new(
                BranchSummaryMessage {
                    summary: "The branch tried a parser rewrite.".into(),
                    from_id: "a".into(),
                    timestamp: 0,
                },
                markdown.clone(),
            ),
        );
        add(
            &mut mode,
            CustomMessageComponent::new(
                CustomMessage {
                    custom_type: "note".into(),
                    content: UserContent::Text("A note\nwith two lines".into()),
                    display: true,
                    details: None,
                    timestamp: 0,
                },
                markdown,
            ),
        );
        add(
            &mut mode,
            ExpandableText::new(
                String::new,
                || "[Skills]\n  /skill:alpha".to_string(),
                false,
                0,
                0,
            )
            .with_header(" Resources".to_string()),
        );
        mode
    }

    /// The blocks of the last layout, in order.
    fn folds(mode: &mut Mode) -> Vec<FoldRef> {
        let width = mode.tui.transcript_width();
        mode.transcript.chat.borrow_mut().render(width);
        chat_folds(&mode.transcript.chat, width)
    }

    fn fold_labelled(mode: &mut Mode, label: &str) -> FoldRef {
        folds(mode)
            .into_iter()
            .find(|found| found.fold.label == label)
            .unwrap_or_else(|| panic!("no {label} block"))
    }

    fn open_state(mode: &mut Mode, id: &str) -> bool {
        folds(mode)
            .into_iter()
            .find(|found| found.fold.id == id)
            .expect("the block")
            .fold
            .expanded
    }

    const KINDS: [&str; 8] = [
        "thinking",
        "Skill",
        "Background run",
        "Shell",
        "Compaction",
        "Branch summary",
        "Message",
        "Resources",
    ];

    #[test]
    fn the_transcript_lists_every_kind_of_block_in_order() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        let labels: Vec<String> = folds(&mut mode)
            .into_iter()
            .map(|found| found.fold.label)
            .collect();
        assert_eq!(labels, KINDS);
        assert!(folds(&mut mode).iter().all(|found| found.fold.can_open));
    }

    #[test]
    fn a_click_on_a_header_row_toggles_every_kind_of_block() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        let width = mode.tui.transcript_width();
        for label in KINDS {
            let found = fold_labelled(&mut mode, label);
            let id = found.fold.id.clone();
            let before = found.fold.expanded;
            let row = found.header_row().expect("a header row");
            mode.click_fold(row, width, 0);
            assert_eq!(open_state(&mut mode, &id), !before, "{label}: first click");
            let found = fold_labelled(&mut mode, label);
            mode.click_fold(found.header_row().expect("a header row"), width, 0);
            assert_eq!(open_state(&mut mode, &id), before, "{label}: second click");
        }
    }

    #[test]
    fn a_click_on_a_body_row_does_nothing() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        let width = mode.tui.transcript_width();
        let shell = fold_labelled(&mut mode, "Shell");
        // Open it, so it has body rows under its header.
        mode.click_fold(shell.header_row().expect("header"), width, 0);
        let shell = fold_labelled(&mut mode, "Shell");
        assert!(shell.fold.expanded);
        let header = shell.header_row().expect("header");
        for below in 1..=3 {
            mode.click_fold(header + below, width, 0);
            assert!(
                open_state(&mut mode, &shell.fold.id),
                "row {below} under the header"
            );
        }
        // Nor does a click past the last block.
        let rows = mode.transcript.chat.borrow_mut().render(width).len();
        mode.click_fold(rows + 5, width, 0);
        assert!(open_state(&mut mode, &shell.fold.id));
    }

    #[test]
    fn enter_on_the_focus_toggles_every_kind_of_block() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        for label in KINDS {
            let found = fold_labelled(&mut mode, label);
            let id = found.fold.id.clone();
            let before = found.fold.expanded;
            set_focus(&mut mode, &id);
            mode.toggle_focused();
            assert_eq!(open_state(&mut mode, &id), !before, "{label}");
        }
    }

    #[test]
    fn ctrl_x_o_closes_what_is_open_then_opens_every_block() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        // The skill block, the notice and the message start open, so the
        // first press closes the open ones, and the second opens all.
        mode.toggle_all_tool_rows();
        assert!(folds(&mut mode).iter().all(|found| !found.fold.expanded));
        mode.toggle_all_tool_rows();
        let all = folds(&mut mode);
        assert_eq!(all.len(), KINDS.len());
        assert!(all.iter().all(|found| found.fold.expanded));
    }

    #[test]
    fn the_focus_walk_reaches_every_block_and_hidden_thinking_is_skipped() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        mode.focus_newest_and_pin();
        let mut seen = vec![mode.focus.borrow().focused_call().map(str::to_string)];
        for _ in 1..KINDS.len() {
            mode.focus_step(false);
            seen.push(mode.focus.borrow().focused_call().map(str::to_string));
        }
        let mut expected: Vec<Option<String>> = folds(&mut mode)
            .into_iter()
            .map(|found| Some(found.fold.id))
            .collect();
        expected.reverse();
        assert_eq!(seen, expected);

        // `ctrl+x b` hides the trace: it is no block to walk or open.
        let mut hidden = mode_with_every_block(ThinkingDisplay::Label);
        let labels: Vec<String> = folds(&mut hidden)
            .into_iter()
            .map(|found| found.fold.label)
            .collect();
        assert_eq!(labels, &KINDS[1..]);
        let mut omitted = mode_with_every_block(ThinkingDisplay::Omit);
        assert!(!folds(&mut omitted)
            .iter()
            .any(|found| found.fold.label == "thinking"));
    }

    #[test]
    fn the_focused_header_takes_the_gutter_and_the_marker_shows_the_state() {
        let mut mode = mode_with_every_block(ThinkingDisplay::Full);
        let width = mode.tui.transcript_width();
        let found = fold_labelled(&mut mode, "Compaction");
        set_focus(&mut mode, &found.fold.id.clone());
        let rows = mode.transcript.chat.borrow_mut().render(width);
        let header = fold_labelled(&mut mode, "Compaction")
            .header_row()
            .expect("header");
        let plain = hoocode_tui_util::strip_vt_control_characters(&rows[header]);
        assert!(plain.starts_with('›'), "{plain:?}");
        assert!(plain.trim_end().ends_with('▸'), "{plain:?}");
        mode.toggle_focused();
        let rows = mode.transcript.chat.borrow_mut().render(width);
        let header = fold_labelled(&mut mode, "Compaction")
            .header_row()
            .expect("header");
        let plain = hoocode_tui_util::strip_vt_control_characters(&rows[header]);
        assert!(plain.trim_end().ends_with('▾'), "{plain:?}");
    }

    /// A settled Shell call whose output is empty, frozen: nothing to open.
    fn frozen_empty_chain() -> ToolChainComponent {
        let definition = builtin_tool_definition("Shell").expect("the Shell tool");
        let mut block = ToolExecutionComponent::new(
            "Shell",
            "call-1",
            serde_json::json!({"command": "true"}),
            ToolExecutionOptions::default(),
            Some(definition),
            "/work",
        );
        block.update_result(
            ToolResult {
                content: vec![Content::text("")],
                details: serde_json::Value::Null,
                is_error: false,
            },
            false,
        );
        block.freeze();
        let mut chain = ToolChainComponent::new();
        chain.add(Rc::new(RefCell::new(block)));
        chain.close(ChainState::Done);
        chain
    }

    fn notice_titles(mode: &Mode) -> Vec<String> {
        let panel = mode.notifications.borrow();
        panel
            .showing()
            .into_iter()
            .chain(panel.pending())
            .map(|n| n.title.clone())
            .collect()
    }

    #[test]
    fn a_click_on_a_frozen_row_with_no_output_says_so_and_changes_nothing() {
        let mut mode = test_mode(None);
        add(&mut mode, frozen_empty_chain());
        let width = mode.tui.transcript_width();
        let found = fold_labelled(&mut mode, "Shell");
        assert!(!found.fold.can_open, "nothing to open");
        assert!(notice_titles(&mode).is_empty());
        mode.click_fold(found.header_row().expect("header"), width, 0);
        assert_eq!(notice_titles(&mode), ["Shell: no output to show."]);
        assert!(!fold_labelled(&mut mode, "Shell").fold.expanded);
    }
}
