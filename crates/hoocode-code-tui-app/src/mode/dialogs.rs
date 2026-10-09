//! The dialogs that take over the editor slot: extension selectors, ask-options, and the multi-line
//! editor dialog.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self};

use hoocode_code_tools_optin::AskQuestion;
use hoocode_code_tui_selectors::ask_options::{AskOptionsComponent, AskOptionsOptions};
use hoocode_tui_components::EditorHost;
use hoocode_tui_render::ComponentHandle;

use crate::extension_editor::{EditorOutcome, ExtensionEditorComponent};
use crate::extension_selector::{ExtensionSelectorComponent, SelectorOutcome};

use super::*;

/// The open extension selector and the channel its answer goes back on.
pub(super) type OpenSelector = (
    Rc<RefCell<ExtensionSelectorComponent>>,
    mpsc::Sender<Option<String>>,
);

/// The open options pane and the channel its answers go back on.
pub(super) type OpenAskOptions = (
    Rc<RefCell<AskOptionsComponent>>,
    mpsc::Sender<Option<Vec<String>>>,
);

/// The open editor dialog and where its text goes.
pub(super) type OpenEditorDialog = (
    Rc<RefCell<ExtensionEditorComponent>>,
    mpsc::Sender<Option<String>>,
);

/// A pane's answers, collected by its `on_done`.
pub(super) type Outcomes = Rc<RefCell<Vec<SelectorOutcome>>>;

impl Mode {
    /// `showSelector`: swap the selector into the editor's slot.
    pub(super) fn show_selector(
        &mut self,
        title: &str,
        options: Vec<String>,
        reply: mpsc::Sender<Option<String>>,
    ) {
        if let Some((previous, previous_reply)) = self.selector.take() {
            previous.borrow_mut().dispose();
            let _ = previous_reply.send(None);
        }
        let sink = self.actions.clone();
        let selector = handle(ExtensionSelectorComponent::new(
            title,
            options,
            None,
            Box::new(move |outcome| sink.borrow_mut().push(Action::SelectorDone(outcome))),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.selector = Some((selector, reply));
        self.dirty.set(true);
    }

    /// `hideSelector` + `restoreEditor`, answering the asker.
    pub(super) fn close_selector(&mut self, outcome: SelectorOutcome) {
        let Some((selector, reply)) = self.selector.take() else {
            return;
        };
        selector.borrow_mut().dispose();
        let _ = reply.send(match outcome {
            SelectorOutcome::Selected(option) => Some(option),
            SelectorOutcome::Cancelled => None,
        });
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&self.editor));
        }
        self.tui.set_focus(Some(as_component(&self.editor)));
        self.dirty.set(true);
    }

    /// `showAskOptions`: the options pane in the editor's slot.
    pub(super) fn show_ask_options(
        &mut self,
        questions: Vec<AskQuestion>,
        reply: mpsc::Sender<Option<Vec<String>>>,
    ) {
        if questions.is_empty() {
            let _ = reply.send(None);
            return;
        }
        // A pane still up belongs to an asker that is gone: it reads as skipped.
        self.hide_ask_options(None);
        let on_submit = self.actions.clone();
        let on_cancel = self.actions.clone();
        let pane = handle(AskOptionsComponent::new(
            questions,
            Box::new(move |answers| {
                on_submit
                    .borrow_mut()
                    .push(Action::AskOptionsDone(Some(answers)))
            }),
            Box::new(move || on_cancel.borrow_mut().push(Action::AskOptionsDone(None))),
            AskOptionsOptions::default(),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&pane));
        }
        self.tui.set_focus(Some(as_component(&pane)));
        self.ask_options = Some((pane, reply));
        self.dirty.set(true);
    }

    /// `hideAskOptions`: answer the asker and put the prompt back.
    pub(super) fn hide_ask_options(&mut self, answers: Option<Vec<String>>) {
        let Some((_, reply)) = self.ask_options.take() else {
            return;
        };
        let _ = reply.send(answers);
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&self.editor));
        }
        self.tui.set_focus(Some(as_component(&self.editor)));
        self.dirty.set(true);
    }

    /// The editor's slot back to the prompt, focused.
    pub(super) fn restore_editor(&mut self) {
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&self.editor));
        }
        self.tui.set_focus(Some(as_component(&self.editor)));
        self.dirty.set(true);
    }

    /// `showEditor`: the multi-line editor dialog in the editor's slot.
    pub(super) fn show_editor_dialog(&mut self, title: &str, reply: mpsc::Sender<Option<String>>) {
        if let Some((_, previous)) = self.editor_dialog.take() {
            let _ = previous.send(None);
        }
        let rows = self.size.clone();
        let flag = self.dirty.clone();
        let sink = self.actions.clone();
        let dialog = handle(ExtensionEditorComponent::new(
            EditorHost {
                rows: Box::new(move || rows.get().1),
                request_render: Box::new(move || flag.set(true)),
            },
            title,
            None,
            Box::new(move |outcome| sink.borrow_mut().push(Action::EditorDialogDone(outcome))),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&dialog));
        }
        self.tui.set_focus(Some(as_component(&dialog)));
        self.editor_dialog = Some((dialog, reply));
        self.dirty.set(true);
    }

    /// `hideEditor`, answering the asker.
    pub(super) fn close_editor_dialog(&mut self, outcome: EditorOutcome) {
        let Some((_, reply)) = self.editor_dialog.take() else {
            return;
        };
        let _ = reply.send(match outcome {
            EditorOutcome::Submitted(text) => Some(text),
            EditorOutcome::Cancelled => None,
        });
        self.restore_editor();
    }

    /// Put `component` in the editor's slot, focused.
    pub(super) fn show_in_editor_slot(&mut self, component: ComponentHandle) {
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(component.clone());
        }
        self.tui.set_focus(Some(component));
        self.dirty.set(true);
    }
}
