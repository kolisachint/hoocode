//! The prompt (declutter spec v2): one row on the `surface` colour, ` ›` in
//! teal, then the input. No frame, no placeholder, no title. It grows with
//! the input: wrapped lines keep the marker's indent. In bash mode the marker
//! is `!` in the bash-mode colour.
//!
//! Messages queued while the agent works are dim rows directly above it:
//! ` ○ text · sends after this step` (steering) and ` ○ text · sends after
//! this turn` (follow-up). The completion menu (slash commands, files,
//! arguments) is drawn above the prompt, pmenu style: pane colours, as wide as
//! its longest row, its text under the token being completed. In monochrome it
//! gets light `┌─┐│└┘` borders instead of colour.

use std::cell::Cell;
use std::rc::Rc;

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::{Component, ComponentHandle};
use hoocode_tui_util::{apply_background_to_line, truncate_to_width, visible_width};

/// Columns before the input: a blank and the marker. The editor's own left
/// padding (one column by default) follows.
pub const GUTTER: usize = 2;

/// Reads the queued `(steering, follow-up)` messages.
pub type ReadQueue = Rc<dyn Fn() -> (Vec<String>, Vec<String>)>;
/// Reads how many of the editor's last rows were text (the rest is the
/// autocomplete list).
pub type TextRows = Rc<dyn Fn() -> usize>;

/// Reads the completion menu for a maximum width: its rows, their width and
/// the token's column in the text (`None` when no menu shows).
pub type ReadMenu = Rc<dyn Fn(usize) -> Option<(Vec<String>, usize, usize)>>;

/// The prompt row around the editor. The editor keeps its key handling and
/// focus, reached through this component.
pub struct PromptRow {
    editor: ComponentHandle,
    text_rows: TextRows,
    read_menu: ReadMenu,
    read_queue: ReadQueue,
    bash: Rc<Cell<bool>>,
}

impl PromptRow {
    /// `editor` renders its text only (no border); `text_rows` says how many
    /// of its rows were text; `bash` is the bash-mode flag the mode keeps.
    pub fn new(
        editor: ComponentHandle,
        text_rows: TextRows,
        read_menu: ReadMenu,
        read_queue: ReadQueue,
        bash: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            editor,
            text_rows,
            read_menu,
            read_queue,
            bash,
        }
    }
}

/// `line` on the surface colour, padded to `width`. Text on the surface also
/// sets its foreground, so a light theme keeps its contrast.
pub fn on_surface(line: &str, width: usize) -> String {
    apply_background_to_line(line, width, |text: &str| {
        let t = theme();
        t.bg("surface", &t.fg("text", text))
    })
}

/// The queued messages as dim rows, each cut to `width`.
pub fn queue_rows(steering: &[String], follow_up: &[String], width: usize) -> Vec<String> {
    let t = theme();
    let mut rows = Vec::new();
    for (texts, tail) in [
        (steering, " · sends after this step"),
        (follow_up, " · sends after this turn"),
    ] {
        for text in texts {
            let text = text.replace(['\n', '\r', '\t'], " ");
            let room = width.saturating_sub(3 + visible_width(tail)).max(1);
            let text = truncate_to_width(&text, room, "…", false);
            rows.push(t.fg("dim", &format!(" ○ {text}{tail}")));
        }
    }
    rows
}

/// The menu's rows placed at screen column `left`; in monochrome inside light
/// borders.
pub fn menu_block(rows: &[String], menu_width: usize, left: usize, mono: bool) -> Vec<String> {
    let indent = " ".repeat(left);
    if !mono {
        return rows.iter().map(|row| format!("{indent}{row}")).collect();
    }
    let t = theme();
    let rule = "─".repeat(menu_width);
    let side = t.fg("dim", "│");
    let mut out = vec![format!("{indent}{}", t.fg("dim", &format!("┌{rule}┐")))];
    for row in rows {
        let pad = " ".repeat(menu_width.saturating_sub(visible_width(row)));
        out.push(format!("{indent}{side}{row}{pad}{side}"));
    }
    out.push(format!("{indent}{}", t.fg("dim", &format!("└{rule}┘"))));
    out
}

impl Component for PromptRow {
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        if width == 0 {
            return Vec::new();
        }
        let (steering, follow_up) = (self.read_queue)();
        let mut out = queue_rows(&steering, &follow_up, width);
        let inner = width.saturating_sub(GUTTER).max(1);
        let mut lines = self.editor.borrow_mut().render(inner as u16);
        let text_rows = (self.text_rows)().min(lines.len()).max(1);
        let mono = !hoocode_code_tui_theme::chalk::enabled();
        let border = if mono { 2 } else { 0 };
        if let Some((rows, menu_width, anchor)) = (self.read_menu)(width.saturating_sub(border)) {
            // The menu's text sits under the token: the marker's three columns
            // before it, the prompt's three columns before the text.
            let left = anchor.min(width.saturating_sub(menu_width + border));
            out.extend(menu_block(&rows, menu_width, left, mono));
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        let t = theme();
        let marker = if self.bash.get() {
            format!(" {}", t.bash_mode_border("!"))
        } else {
            format!(" {}", t.fg("accent", "›"))
        };
        for (i, line) in lines.into_iter().enumerate() {
            if i < text_rows {
                let lead = if i == 0 { marker.as_str() } else { "  " };
                out.push(on_surface(&format!("{lead}{line}"), width));
            } else {
                out.push(format!("{}{line}", " ".repeat(GUTTER)));
            }
        }
        out
    }

    fn handle_input(&mut self, data: &str) {
        self.editor.borrow_mut().handle_input(data);
    }

    fn invalidate(&mut self) {
        self.editor.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        self.editor.borrow().is_focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.borrow_mut().set_focused(focused);
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn a_colour_menu_is_its_rows_indented_to_the_token() {
        let rows = vec!["one".to_string(), "two".to_string()];
        assert_eq!(menu_block(&rows, 3, 4, false), ["    one", "    two"]);
    }

    #[test]
    fn a_monochrome_menu_gets_light_borders() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rows = vec!["one".to_string(), "t".to_string()];
        let plain: Vec<String> = menu_block(&rows, 3, 1, true)
            .iter()
            .map(|r| hoocode_tui_util::strip_vt_control_characters(r))
            .collect();
        assert_eq!(plain, [" ┌───┐", " │one│", " │t  │", " └───┘"]);
    }
}
