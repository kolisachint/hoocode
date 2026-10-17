//! The dialogs that take over the editor slot: extension selectors, ask-options, and the multi-line
//! editor dialog.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self};

use hoocode_code_permissions::{ClearanceChoice, ClearanceInfo};
use hoocode_code_tools_optin::AskQuestion;
use hoocode_code_tui_selectors::ask_options::{AskOptionsComponent, AskOptionsOptions};
use hoocode_tui_components::EditorHost;
use hoocode_tui_render::ComponentHandle;

use crate::clearance_card::ClearanceCard;
use crate::end_shift::{EndShiftCard, EndShiftJobsCard, ShiftDebrief, ShiftJob};

use crate::extension_editor::{EditorOutcome, ExtensionEditorComponent};
use crate::extension_selector::{ExtensionSelectorComponent, SelectorOutcome};

use super::*;

/// The picker in the editor's slot. There is at most one: a picker shown over
/// another replaces it, and the replaced one is disposed (`set_picker`).
pub(super) enum ActivePicker {
    /// An extension selector and the callback its answer goes to.
    Selector(Rc<RefCell<ExtensionSelectorComponent>>, Answer),
    Session(Rc<RefCell<SessionSelectorComponent>>),
    /// The session tree, and the leaf it opened on.
    Tree(Rc<RefCell<TreeSelectorComponent>>, Option<String>),
    Fork(Rc<RefCell<UserMessageSelectorComponent>>),
    Model(Rc<RefCell<ModelSelectorComponent>>),
    ScopedModels(Rc<RefCell<ScopedModelsSelectorComponent>>),
    Settings(Rc<RefCell<SettingsSelectorComponent>>),
}

impl ActivePicker {
    /// The deadline the loop must wake for, if the picker has one.
    pub(super) fn deadline(&self) -> Option<Instant> {
        match self {
            ActivePicker::Selector(s, _) => s.borrow().deadline(),
            ActivePicker::Fork(s) => s.borrow().deadline(),
            ActivePicker::Tree(s, _) => s.borrow().deadline(),
            _ => None,
        }
    }

    /// The picker is gone. An extension selector's asker gets no answer.
    pub(super) fn dispose(self) {
        if let ActivePicker::Selector(selector, answer) = self {
            selector.borrow_mut().dispose();
            answer(None);
        }
    }
}

/// The open options pane and the channel its answers go back on.
pub(super) type OpenAskOptions = (
    Rc<RefCell<AskOptionsComponent>>,
    mpsc::Sender<Option<Vec<String>>>,
);

/// The open editor dialog and where its text goes.
pub(super) type OpenEditorDialog = (Rc<RefCell<ExtensionEditorComponent>>, Answer);

/// Where a selector's or editor dialog's answer goes (`None` = cancelled). It runs on the
/// UI thread, so a mode question posts its `AppEvent` here and needs no poll.
pub(super) type Answer = Box<dyn FnOnce(Option<String>)>;

/// A pane's answers, collected by its `on_done`.
pub(super) type Outcomes = Rc<RefCell<Vec<SelectorOutcome>>>;

/// The open clearance card and the channel its answer goes back on.
pub(super) type OpenClearance = (Rc<RefCell<ClearanceCard>>, mpsc::Sender<ClearanceChoice>);

/// The open end-of-shift card (its answer is an [`Action`], so no channel).
pub(super) type OpenEndShift = Rc<RefCell<EndShiftCard>>;

/// The open end-of-shift-with-satellites card.
pub(super) type OpenEndShiftJobs = Rc<RefCell<EndShiftJobsCard>>;

impl Mode {
    /// Puts `picker` in the editor's slot, focused, after disposing the picker
    /// that was there. `child` is the component the slot shows.
    pub(super) fn set_picker(&mut self, picker: ActivePicker, child: ComponentHandle) {
        if let Some(previous) = self.picker.take() {
            previous.dispose();
        }
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(child.clone());
        }
        self.tui.set_focus(Some(child));
        self.picker = Some(picker);
        self.dirty.set(true);
    }

    /// An answer that posts `event(choice)` to the loop, which handles it there.
    pub(super) fn answer_to(
        &self,
        event: impl FnOnce(Option<String>) -> AppEvent + 'static,
    ) -> Answer {
        let tx = self.tx.clone();
        Box::new(move |choice| {
            let _ = tx.send(event(choice));
        })
    }

    /// The selector pickers' own timers and loads (`poll`). True if anything changed.
    pub(super) fn poll_picker_component(&mut self) -> bool {
        match &self.picker {
            Some(ActivePicker::Selector(selector, _)) => selector.borrow_mut().poll(),
            Some(ActivePicker::Session(selector)) => selector.borrow_mut().poll(),
            _ => false,
        }
    }

    /// `showSelector`: swap the selector into the editor's slot.
    pub(super) fn show_selector(&mut self, title: &str, options: Vec<String>, answer: Answer) {
        let sink = self.actions.clone();
        let selector = handle(ExtensionSelectorComponent::new(
            title,
            options,
            None,
            Box::new(move |outcome| sink.borrow_mut().push(Action::SelectorDone(outcome))),
        ));
        self.set_picker(
            ActivePicker::Selector(selector.clone(), answer),
            as_component(&selector),
        );
    }

    /// `hideSelector` + `restoreEditor`, answering the asker.
    pub(super) fn close_selector(&mut self, outcome: SelectorOutcome) {
        if !matches!(self.picker, Some(ActivePicker::Selector(..))) {
            return;
        }
        let Some(ActivePicker::Selector(selector, answer)) = self.picker.take() else {
            return;
        };
        selector.borrow_mut().dispose();
        answer(match outcome {
            SelectorOutcome::Selected(option) => Some(option),
            SelectorOutcome::Cancelled => None,
        });
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(self.prompt_component());
        }
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
    }

    /// `showAskOptions`: the options pane in the editor's slot.
    pub(super) fn show_ask_options(
        &mut self,
        questions: Vec<AskQuestion>,
        reply: mpsc::Sender<Option<Vec<String>>>,
        esc_answers_first: bool,
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
            AskOptionsOptions { esc_answers_first },
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
            container.add_child(self.prompt_component());
        }
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
    }

    /// `showClearance`: the clearance card in the editor's slot (it replaces
    /// the comms frame). A card still up belongs to a call that is gone: it
    /// reads as denied.
    pub(super) fn show_clearance_card(
        &mut self,
        info: ClearanceInfo,
        reply: mpsc::Sender<ClearanceChoice>,
    ) {
        self.close_clearance(ClearanceChoice::Deny { reason: None });
        let on_done = self.actions.clone();
        let card = handle(ClearanceCard::new(
            info,
            Box::new(move |choice| {
                on_done.borrow_mut().push(Action::ClearanceDone(choice));
            }),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&card));
        }
        self.tui.set_focus(Some(as_component(&card)));
        self.clearance = Some((card, reply));
        self.dirty.set(true);
    }

    /// Answers the gate and puts the prompt back. `Revise` also sends the
    /// edited text as the next message, so the agent runs the revised
    /// command (the gate cannot retarget the call it already holds).
    pub(super) fn close_clearance(&mut self, choice: ClearanceChoice) {
        let Some((_, reply)) = self.clearance.take() else {
            return;
        };
        let revised = match &choice {
            ClearanceChoice::Revise(text) => Some(text.clone()),
            _ => None,
        };
        let _ = reply.send(choice);
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(self.prompt_component());
        }
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
        if let Some(text) = revised {
            self.prompt(text);
        }
    }

    /// `showEndShift`: the amber end-of-shift card with the shift's debrief.
    /// `enter` ends the shift, `esc` stays (spec §6.2).
    pub(super) fn show_end_shift(&mut self, debrief: ShiftDebrief) {
        let on_done = self.actions.clone();
        let card = handle(EndShiftCard::new(
            debrief,
            Box::new(move |choice| {
                on_done.borrow_mut().push(Action::EndShiftDone(choice));
            }),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&card));
        }
        self.tui.set_focus(Some(as_component(&card)));
        self.end_shift_card = Some(card);
        self.dirty.set(true);
    }

    /// `showEndShiftJobs`: the amber card for quitting with satellites up
    /// (spec §4 c8). `k` brings them down, `l` leaves them, `esc` stays.
    pub(super) fn show_end_shift_jobs(&mut self, jobs: Vec<ShiftJob>) {
        let on_done = self.actions.clone();
        let card = handle(EndShiftJobsCard::new(
            jobs,
            Box::new(move |choice| {
                on_done.borrow_mut().push(Action::EndShiftJobsDone(choice));
            }),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&card));
        }
        self.tui.set_focus(Some(as_component(&card)));
        self.end_shift_jobs_card = Some(card);
        self.dirty.set(true);
    }

    /// The editor's slot back to the prompt, focused.
    pub(super) fn restore_editor(&mut self) {
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(self.prompt_component());
        }
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
    }

    /// `showEditor`: the multi-line editor dialog in the editor's slot.
    pub(super) fn show_editor_dialog(&mut self, title: &str, answer: Answer) {
        if let Some((_, previous)) = self.editor_dialog.take() {
            previous(None);
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
        self.editor_dialog = Some((dialog, answer));
        self.dirty.set(true);
    }

    /// `hideEditor`, answering the asker.
    pub(super) fn close_editor_dialog(&mut self, outcome: EditorOutcome) {
        let Some((_, answer)) = self.editor_dialog.take() else {
            return;
        };
        answer(match outcome {
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
