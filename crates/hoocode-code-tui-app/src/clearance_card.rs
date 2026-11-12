//! The permission ("clearance") card, a blocking pane in the prompt's slot
//! (declutter spec, Dialogs): a yellow `Allow Shell?` chip, the `$ command`
//! row, an `access:` row only when the call writes, uses the network or
//! pipes, then numbered options (`1 yes`, `2 yes, always for <program> this
//! session`, `3 no, tell it why`), the reply row and the hint row (`1–3 pick ·
//! e edit first · or type a reply`: `a` still highlights the always row). `1`/`2`/`3`
//! pick, `e` edits the command first, and every other letter types a reply
//! that `enter` sends as the denial's reason. While the reply is empty, `y`
//! answers yes and `n` no at once (the hint does not list them); `a` only moves the highlight to the always
//! row, and `enter` confirms it. With no always row, `a` types like any letter.
//!
//! `e edit first` cannot run inside the gate (the call's arguments are
//! fixed), so `enter` denies the original call and the host sends the edited
//! text as the next message instead; the agent then runs the revised command
//! (asking again when it still needs clearance).

use hoocode_code_permissions::{is_always_wrapper, shell_program, ClearanceChoice, ClearanceInfo};
use hoocode_code_tui_keybindings::key_hint_text;
use hoocode_code_tui_widgets::pane::{fit_hint_line, input_line, wrap_text, Chip, Pane, PaneStyle};
use hoocode_tui_components::Input;
use hoocode_tui_keys::{get_keybindings, matches_key};
use hoocode_tui_ratatui::{Line, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

/// What the card answers with (the gate's [`ClearanceChoice`], sent on).
pub type ClearanceAnswer = Box<dyn FnOnce(ClearanceChoice)>;

/// The clearance card in the editor's slot, focused while it waits.
pub struct ClearanceCard {
    info: ClearanceInfo,
    reply: String,
    editing: bool,
    edit: Input,
    /// The highlighted option (0 is `yes`).
    selected: usize,
    focused: bool,
    on_done: Option<ClearanceAnswer>,
}

impl ClearanceCard {
    /// The card for `info`; `on_done` answers the gate exactly once.
    pub fn new(info: ClearanceInfo, on_done: ClearanceAnswer) -> Self {
        Self {
            info,
            reply: String::new(),
            editing: false,
            edit: Input::new(),
            selected: 0,
            focused: false,
            on_done: Some(on_done),
        }
    }

    fn done(&mut self, choice: ClearanceChoice) {
        if let Some(answer) = self.on_done.take() {
            answer(choice);
        }
    }

    /// `Allow Shell?`
    fn title(&self) -> String {
        format!("Allow {}?", self.info.tool)
    }

    /// The facts after the chip: the subagent that asks, when one does.
    fn facts(&self) -> Option<String> {
        self.info
            .from
            .as_ref()
            .map(|from| format!("asked by {from}"))
    }

    /// The program the `always` option names, when it is offered at all:
    /// the Shell command's first word, else the tool. Never offered in PLAN
    /// mode, nor for slash programs (`/bin/sh`) or wrappers like sudo (the
    /// word comes from the gate's [`shell_program`], so the label and the
    /// stored rule always agree).
    fn always_offer(&self) -> Option<String> {
        if self.info.mode.trim().eq_ignore_ascii_case("plan") {
            return None;
        }
        if self.info.tool == "Shell" {
            let command = self.info.detail.trim_start_matches("$ ").trim_start();
            let program = shell_program(command)?;
            (!program.contains('/') && !is_always_wrapper(&program)).then_some(program)
        } else {
            Some(self.info.tool.clone())
        }
    }

    /// The editable command: the Shell command without its `$ `, else the detail.
    fn editable(&self) -> String {
        if self.info.tool == "Shell" {
            self.info
                .detail
                .trim_start_matches("$ ")
                .trim_start()
                .to_string()
        } else {
            self.info.detail.clone()
        }
    }

    /// What the call reaches: `writes notes.txt, network, pipes 2
    /// programs` after the dim `access:` label. None when it does none.
    fn access_text(&self) -> Option<String> {
        let reach = &self.info.reach;
        let mut parts = Vec::new();
        if reach.write.on {
            parts.push(match &reach.write.qualifier {
                Some(path) => format!("writes {path}"),
                None => "writes files".to_string(),
            });
        }
        if reach.network.on {
            parts.push("network".to_string());
        }
        if reach.pipe.on {
            parts.push(match &reach.pipe.qualifier {
                Some(count) => format!("pipes {count}"),
                None => "pipes".to_string(),
            });
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// The option labels with their scope, in key order: yes, [always],
    /// no. The scope rides on the always label's row while it fits, else
    /// on its own continuation row (mock §S4).
    fn options(&self) -> Vec<(String, Option<String>)> {
        let mut options = vec![("yes".to_string(), None)];
        if let Some(program) = self.always_offer() {
            // Shell approvals live for this session only; other tools
            // persist to the mode's auto-allow.
            let scope = if self.info.tool == "Shell" {
                "this session".to_string()
            } else {
                format!("in {}", self.info.mode.trim().to_lowercase())
            };
            options.push((format!("yes, always for {program}"), Some(scope)));
        }
        options.push(("no, tell it why".to_string(), None));
        options
    }

    /// The pane for the current state.
    fn pane(&self, width: usize) -> Pane {
        let st = PaneStyle::current();
        let mut pane = Pane::new(self.title());
        pane.chip = Chip::Warn;
        if let Some(facts) = self.facts() {
            pane.facts = Line::from(vec![Span::styled(facts, st.dim_style())]);
        }
        // The subject: `$ command` for Shell, the detail otherwise. A
        // narrow pane wraps the command to a second row aligned under it
        // (mock §S4).
        let is_shell = self.info.tool == "Shell";
        let wrap = width.saturating_sub(if is_shell { 6 } else { 4 });
        for (n, part) in wrap_text(&self.editable(), wrap).iter().enumerate() {
            let mut subject = if n == 0 {
                vec![Span::raw("  ")]
            } else if is_shell {
                vec![Span::raw("    ")]
            } else {
                vec![Span::raw("  ")]
            };
            if is_shell && n == 0 {
                subject.push(Span::styled("$ ", st.dim_style()));
            }
            subject.push(Span::styled(part.clone(), st.text_style()));
            pane.top.push(Line::from(subject));
        }
        if let Some(access) = self.access_text() {
            pane.top.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("access:".to_string(), st.dim_style()),
                Span::styled(format!(" {access}"), st.text_style()),
            ]));
        }
        if self.editing {
            pane.input = Some(input_line(self.edit.get_value(), "", self.focused));
            let run = key_hint_text("tui.input.submit");
            let cancel = key_hint_text("tui.select.cancel");
            pane.hint = fit_hint_line(
                &[(run.as_str(), "run"), (cancel.as_str(), "cancel")],
                false,
                width,
            );
        } else {
            let options = self.options();
            let count = options.len();
            let item_budget = width.saturating_sub(4);
            for (i, (label, scope)) in options.into_iter().enumerate() {
                // The row's number is the key that picks it (K-P5).
                let number = format!("{}  ", key_hint_text(PICK_IDS[i.min(2)]));
                let inline = match &scope {
                    Some(scope) => format!("{label} {scope}"),
                    None => label.clone(),
                };
                let mut spans = vec![
                    Span::styled(number.clone(), st.text_style()),
                    Span::styled(label, st.text_style()),
                ];
                if visible_width(&number) + 2 + visible_width(&inline) <= item_budget {
                    if let Some(scope) = scope {
                        spans.push(Span::styled(format!(" {scope}"), st.text_style()));
                    }
                } else if let Some(scope) = scope {
                    pane.flow.entry(i).or_default().push(Line::from(vec![
                        Span::raw("        "),
                        Span::styled(scope, st.text_style()),
                    ]));
                }
                pane.rows.push(Line::from(spans));
            }
            pane.selected = Some(self.selected.min(count - 1));
            pane.input = Some(input_line(&self.reply, "or type a reply", self.focused));
            let pick = format!(
                "{}–{}",
                key_hint_text(PICK_IDS[0]),
                key_hint_text(PICK_IDS[count.min(PICK_IDS.len()) - 1])
            );
            let edit = key_hint_text("app.permission.edit");
            // The hint names the design's items only. `y` and `n` still
            // answer while the reply is empty (decision 2026-11-05); the
            // hint does not list them.
            let mut hint = fit_hint_line(
                &[(pick.as_str(), "pick"), (edit.as_str(), "edit first")],
                false,
                width,
            );
            let tail = "or type a reply";
            if hint.width() + " · ".len() + tail.len() <= width.saturating_sub(2) {
                hint.spans.push(Span::styled(" · ", st.dim_style()));
                hint.spans.push(Span::styled(tail, st.dim_style()));
            }
            pane.hint = hint;
        }
        pane.style = Some(st);
        pane
    }

    /// Answers with the option at `index` (0 is yes, the last is no).
    fn choose(&mut self, index: usize) {
        let count = self.options().len();
        if index == 0 {
            self.done(ClearanceChoice::Once);
        } else if index + 1 >= count {
            self.done(ClearanceChoice::Deny { reason: None });
        } else {
            self.done(ClearanceChoice::Always);
        }
    }
}

/// The option keys, in order: `pick1` answers option 1, and so on.
const PICK_IDS: [&str; 3] = [
    "app.permission.pick1",
    "app.permission.pick2",
    "app.permission.pick3",
];

impl Component for ClearanceCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.pane(width as usize).render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        if self.editing {
            // An exact esc cancels the edit; sequences (arrows) edit the text.
            if data == "\x1b" {
                self.editing = false;
                self.edit.set_focused(false);
                return;
            }
            if data == "\r" || data == "\n" {
                let revised = self.edit.get_value().trim().to_string();
                if !revised.is_empty() {
                    self.done(ClearanceChoice::Revise(revised));
                }
                return;
            }
            self.edit.handle_input(data);
            return;
        }
        // Card keys fire only while no reply is typed: once text is there,
        // letters join it, Enter sends it as the denial's reason, and Esc
        // clears it first (a second Esc denies).
        if !self.reply.is_empty() {
            match data {
                "\r" | "\n" => {
                    let reply = std::mem::take(&mut self.reply);
                    self.done(ClearanceChoice::Deny {
                        reason: non_empty(reply),
                    });
                }
                "\x1b" => self.reply.clear(),
                "\u{7f}" | "\u{8}" => {
                    self.reply.pop();
                }
                _ => push_text(&mut self.reply, data),
            }
            return;
        }
        let count = self.options().len();
        let kb = get_keybindings();
        match data {
            // Esc alone is no (spec §6.2).
            "\x1b" => self.done(ClearanceChoice::Deny { reason: None }),
            "\r" | "\n" => self.choose(self.selected),
            _ if kb.matches(data, "app.permission.edit") => {
                self.editing = true;
                self.edit.set_value(self.editable());
                // Editing starts at the end of the command (ctrl+e).
                self.edit.handle_input("\x05");
                self.edit.set_focused(self.focused);
            }
            _ if kb.matches(data, "app.permission.pick1") => self.choose(0),
            _ if kb.matches(data, "app.permission.pick2") && count >= 2 => self.choose(1),
            _ if kb.matches(data, "app.permission.pick3") && count >= 3 => self.choose(2),
            // The dialog letters (K-P4), only here with no reply typed: y and n
            // answer at once. a moves the highlight to the always row; enter confirms it.
            _ if kb.matches(data, "app.dialog.yes") => self.choose(0),
            _ if kb.matches(data, "app.dialog.always") && count >= 3 => self.selected = 1,
            _ if kb.matches(data, "app.dialog.no") => self.choose(count - 1),
            "\u{7f}" | "\u{8}" => {}
            _ if matches_key(data, "up") => {
                self.selected = (self.selected + count - 1) % count;
            }
            _ if matches_key(data, "down") => self.selected = (self.selected + 1) % count,
            _ => push_text(&mut self.reply, data),
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if self.editing {
            self.edit.set_focused(focused);
        }
    }
}

/// Appends `data` to the typed reply when it is a single printable key
/// (multi-key sequences like arrows never type).
fn push_text(reply: &mut String, data: &str) {
    let mut chars = data.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if !c.is_control() {
            reply.push(c);
        }
    }
}

fn non_empty(text: String) -> Option<String> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}
