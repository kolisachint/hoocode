//! The Mission Control clearance card (M6, spec §4 c1, r5): the warn frame
//! that replaces the comms frame while a gated call waits. The title names
//! the held step when the host knows it; the right label names the mode and
//! the tool; the REACH line reads the call's real data ([`reach_for`]); the
//! keys row keeps each key's behaviour (`y` once, `a` always, `n` abort,
//! `e` edit first). Typed text is the abort's reason (`enter` sends it).
//!
//! `e edit first` cannot run inside the gate (the call's arguments are
//! fixed), so `enter` denies the original call and the host sends the edited
//! text as the next message instead; the agent then runs the revised command
//! (asking again when it still needs clearance).

use hoocode_code_permissions::{
    is_always_wrapper, shell_program, ClearanceChoice, ClearanceInfo, ReachCell,
};
use hoocode_code_tui_keybindings::raw_key_hint;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::card_frame::render_card;
use hoocode_tui_components::Input;
use hoocode_tui_render::Component;

/// What the card answers with (the gate's [`ClearanceChoice`], sent on).
pub type ClearanceAnswer = Box<dyn FnOnce(ClearanceChoice)>;

/// The clearance card in the editor's slot, focused while it waits.
pub struct ClearanceCard {
    info: ClearanceInfo,
    reply: String,
    editing: bool,
    edit: Input,
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
            focused: false,
            on_done: Some(on_done),
        }
    }

    fn done(&mut self, choice: ClearanceChoice) {
        if let Some(answer) = self.on_done.take() {
            answer(choice);
        }
    }

    /// `◉ CLEARANCE NEEDED · step NN` (the step only when the host knows it).
    fn title(&self) -> String {
        match self.info.step {
            Some(step) => format!("◉ CLEARANCE NEEDED · step {step:02}"),
            None => "◉ CLEARANCE NEEDED".to_string(),
        }
    }

    /// `BUILD asks for Shell`.
    fn right(&self) -> String {
        format!(
            "{} asks for {}",
            self.info.mode.to_uppercase(),
            self.info.tool
        )
    }

    /// The `a` key's program, when `a` is offered at all: the Shell
    /// command's first word (never a wrapper like sudo), else the tool.
    /// The word comes from the gate's [`shell_program`], so the label and
    /// the stored rule always agree.
    fn always_offer(&self) -> Option<String> {
        if self.info.tool == "Shell" {
            let command = self.info.detail.trim_start_matches("$ ").trim_start();
            let program = shell_program(command)?;
            (!is_always_wrapper(&program)).then_some(program)
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

    fn reach_row(&self) -> String {
        let t = theme();
        let cells = [
            reach_cell("read", &self.info.reach.read),
            reach_cell("write", &self.info.reach.write),
            reach_cell("network", &self.info.reach.network),
            reach_cell("pipe", &self.info.reach.pipe),
        ];
        format!("{}   {}", t.fg("dim", "REACH"), cells.join("   "))
    }

    fn keys_row(&self) -> String {
        let t = theme();
        let sep = t.fg("muted", " · ");
        let mut hints = vec![raw_key_hint("y", "go once")];
        // Wrappers (sudo, …) are never approvable: no `a` hint, and the
        // key answers once.
        if let Some(program) = self.always_offer() {
            hints.push(raw_key_hint(
                "a",
                &format!("always {} in {}", program, self.info.mode.to_uppercase()),
            ));
        }
        hints.push(raw_key_hint("n", "abort + say why"));
        hints.push(raw_key_hint("e", "edit first"));
        hints.join(&sep)
    }

    fn body(&self) -> Vec<String> {
        let t = theme();
        let mut rows = Vec::new();
        if self.editing {
            let caret = if self.focused {
                t.fg("warning", "▏")
            } else {
                String::new()
            };
            rows.push(format!(
                "{} {}{caret}",
                t.fg("warning", "✎"),
                t.fg("text", self.edit.get_value()),
            ));
        } else {
            rows.push(t.fg("text", &self.info.detail));
            if !self.reply.is_empty() {
                let caret = if self.focused {
                    t.fg("warning", "▏")
                } else {
                    String::new()
                };
                rows.push(format!(
                    "{} {}{caret}",
                    t.fg("warning", "▸"),
                    t.fg("text", &self.reply),
                ));
            }
        }
        rows.push(self.reach_row());
        rows.push(self.keys_row());
        rows
    }

    fn foot(&self) -> String {
        if self.editing {
            "enter run · esc cancel".to_string()
        } else {
            "or type a reply".to_string()
        }
    }
}

/// One REACH cell: `■` warn when on, `□` faint when off, then the name and
/// the qualifier (a real path, a program count).
fn reach_cell(name: &str, cell: &ReachCell) -> String {
    let t = theme();
    let mark = if cell.on {
        t.fg("warning", "■")
    } else {
        t.fg("faint", "□")
    };
    let mut out = format!("{mark} {}", t.fg("text", name));
    if let Some(qualifier) = &cell.qualifier {
        out += &t.fg("dim", &format!(" {qualifier}"));
    }
    out
}

impl Component for ClearanceCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        render_card(
            &self.title(),
            &self.right(),
            &self.body(),
            &self.foot(),
            width as usize,
        )
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
        // Card keys fire only while no reason is typed: once text is
        // there, letters join it, Enter sends it as the abort reason, and
        // Esc clears it first (a second Esc denies).
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
        match data {
            "y" => self.done(ClearanceChoice::Once),
            // Without an `a` offer (a wrapper), `a` answers once.
            "a" => match self.always_offer() {
                Some(_) => self.done(ClearanceChoice::Always),
                None => self.done(ClearanceChoice::Once),
            },
            "n" => self.done(ClearanceChoice::Deny { reason: None }),
            // Esc alone is no (spec §6.2).
            "\x1b" => self.done(ClearanceChoice::Deny { reason: None }),
            "\r" | "\n" => self.done(ClearanceChoice::Once),
            "e" => {
                self.editing = true;
                self.edit.set_value(self.editable());
                // Editing starts at the end of the command (ctrl+e).
                self.edit.handle_input("\x05");
                self.edit.set_focused(self.focused);
            }
            "\u{7f}" | "\u{8}" => {}
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

/// Appends `data` to the typed abort reason when it is a single printable
/// key (multi-key sequences like arrows never type).
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
