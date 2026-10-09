//! The session tree (`/tree`): the selector, branch summaries and navigation.

use std::sync::mpsc::{self};
use std::time::Instant;

use hoocode_code_agent_session::{NavigateTreeOptions, NavigateTreeResult};
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_selectors::tree_selector::{TreeEvent, TreeSelectorComponent};
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::{Loader, Spacer};

use super::*;

/// A question waiting on a dialog: the tree entry it is about, and where
/// the answer arrives.
pub(super) type PendingTreeAnswer = (String, mpsc::Receiver<Option<String>>);

/// A summarizing tree navigation in flight: the target and its result.
pub(super) type TreeNavigation = (String, mpsc::Receiver<Result<NavigateTreeResult, String>>);

impl Mode {
    /// `showTreeSelector`: the session tree in the editor's slot.
    pub(super) fn show_tree_selector(&mut self, initial_selected_id: Option<String>) {
        let (tree, leaf) = {
            let manager = self.session.session_manager();
            (manager.tree(), manager.leaf_id().map(str::to_string))
        };
        let filter = self.session.settings().tree_filter_mode();
        if tree.is_empty() {
            self.show_status("No entries in session");
            return;
        }
        let selector = handle(TreeSelectorComponent::new(
            &tree,
            leaf.as_deref(),
            self.size.get().1 as usize,
            initial_selected_id.as_deref(),
            Some(filter),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.tree_selector = Some((selector, leaf));
        self.dirty.set(true);
    }

    pub(super) fn poll_tree_selector(&mut self) {
        let Some((selector, leaf)) = &self.tree_selector else {
            return;
        };
        let events = selector.borrow_mut().poll(Instant::now());
        let leaf = leaf.clone();
        for event in events {
            match event {
                TreeEvent::LabelChange(id, label) => {
                    let _ = self
                        .session
                        .session_manager()
                        .append_label_change(id, label);
                    self.dirty.set(true);
                }
                TreeEvent::Cancel => {
                    self.tree_selector = None;
                    self.restore_editor();
                    return;
                }
                TreeEvent::Select(id) => {
                    self.tree_selector = None;
                    self.restore_editor();
                    if leaf.as_deref() == Some(id.as_str()) {
                        // Selecting the current leaf is a no-op.
                        self.show_status("Already at this point");
                    } else if self.session.settings().branch_summary_skip_prompt() {
                        self.navigate_tree(id, false, None);
                    } else {
                        self.ask_tree_summary(id);
                    }
                    return;
                }
            }
        }
    }

    /// "Summarize branch?" for a tree selection.
    fn ask_tree_summary(&mut self, entry_id: String) {
        let (reply, answer) = mpsc::channel();
        self.show_selector(
            "Summarize branch?",
            vec![
                "No summary".into(),
                "Summarize".into(),
                "Summarize with custom prompt".into(),
            ],
            reply,
        );
        self.pending_tree_summary = Some((entry_id, answer));
    }

    pub(super) fn poll_tree_summary(&mut self) {
        let Some((_, answer)) = &self.pending_tree_summary else {
            return;
        };
        let choice = match answer.try_recv() {
            Ok(choice) => choice,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        let Some((entry_id, _)) = self.pending_tree_summary.take() else {
            return;
        };
        match choice.as_deref() {
            // Escape: back to the tree, on the same entry.
            None => self.show_tree_selector(Some(entry_id)),
            Some("Summarize with custom prompt") => {
                let (reply, answer) = mpsc::channel();
                self.show_editor_dialog("Custom summarization instructions", reply);
                self.pending_tree_instructions = Some((entry_id, answer));
            }
            Some(choice) => self.navigate_tree(entry_id, choice != "No summary", None),
        }
    }

    pub(super) fn poll_tree_instructions(&mut self) {
        let Some((_, answer)) = &self.pending_tree_instructions else {
            return;
        };
        let text = match answer.try_recv() {
            Ok(text) => text,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        let Some((entry_id, _)) = self.pending_tree_instructions.take() else {
            return;
        };
        match text {
            // Cancelled: back to the summary question.
            None => self.ask_tree_summary(entry_id),
            Some(text) => self.navigate_tree(entry_id, true, Some(text)),
        }
    }

    /// `session.navigateTree`: at once without a summary; with one, off the
    /// input loop behind a loader, so escape can cancel it.
    fn navigate_tree(&mut self, entry_id: String, summarize: bool, instructions: Option<String>) {
        let options = NavigateTreeOptions {
            summarize,
            custom_instructions: instructions,
            ..Default::default()
        };
        let session = self.session.clone();
        let target = entry_id.clone();
        if !summarize {
            if !self.session_op_free() {
                return;
            }
            self.start_session_op("Navigating the tree");
            let tx = self.tx.clone();
            self.runtime.spawn(async move {
                let result = session
                    .navigate_tree(&target, options)
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppEvent::SessionOp(Box::new(SessionOpDone {
                    runtime: None,
                    outcome: SessionOutcome::Tree {
                        entry_id,
                        result: Box::new(result),
                    },
                })));
            });
            return;
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.stop_working_loader();
        let mut loader = Loader::new(
            Box::new(|s: &str| theme().fg("accent", s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            format!(
                "Summarizing branch... ({} to cancel)",
                key_text("app.interrupt")
            ),
            None,
        );
        loader.start();
        let loader = handle(loader);
        self.status.borrow_mut().add_child(as_component(&loader));
        self.loader = Some(loader);
        let (done, result) = mpsc::channel();
        let wake = self.tx.clone();
        self.runtime.spawn(async move {
            let outcome = session
                .navigate_tree(&target, options)
                .await
                .map_err(|e| e.to_string());
            let _ = done.send(outcome);
            let _ = wake.send(AppEvent::Rerender);
        });
        self.tree_navigation = Some((entry_id, result));
        self.dirty.set(true);
    }

    pub(super) fn poll_tree_navigation(&mut self) {
        let Some((_, result)) = &self.tree_navigation else {
            return;
        };
        let outcome = match result.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Branch summarization failed".into()),
        };
        let Some((entry_id, _)) = self.tree_navigation.take() else {
            return;
        };
        self.stop_working_loader();
        self.finish_tree_navigation(entry_id, outcome);
    }

    /// After `navigateTree`: redraw the transcript for the new position.
    pub(super) fn finish_tree_navigation(
        &mut self,
        entry_id: String,
        outcome: Result<NavigateTreeResult, String>,
    ) {
        match outcome {
            Ok(result) if result.aborted => {
                self.show_status("Branch summarization cancelled");
                self.show_tree_selector(Some(entry_id));
            }
            Ok(result) if result.cancelled => self.show_status("Navigation cancelled"),
            Ok(result) => {
                self.reset_transcript_view();
                self.render_initial_messages();
                if let Some(text) = result.editor_text {
                    let mut editor = self.editor.borrow_mut();
                    if editor.editor.get_text().trim().is_empty() {
                        editor.editor.set_text(&text);
                    }
                }
                self.show_status("Navigated to selected point");
            }
            Err(error) => self.show_error(&error),
        }
        self.dirty.set(true);
    }
}
