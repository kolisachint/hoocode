//! The prompt (declutter spec v2): one row on the `surface` colour, ` ›` in
//! teal, then the input. No frame, no placeholder, no title. It grows with
//! the input: wrapped lines keep the marker's indent. In bash mode the marker
//! is `!` in the bash-mode colour.
//!
//! Messages queued while the agent works are dim rows directly above it:
//! ` ○ text · sends after this step` (steering) and ` ○ text · sends after
//! this turn` (follow-up). The autocomplete list the editor draws under its
//! text stays as it is (no surface).

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

/// The prompt row around the editor. The editor keeps its key handling and
/// focus, reached through this component.
pub struct PromptRow {
    editor: ComponentHandle,
    text_rows: TextRows,
    read_queue: ReadQueue,
    bash: Rc<Cell<bool>>,
}

impl PromptRow {
    /// `editor` renders its text only (no border); `text_rows` says how many
    /// of its rows were text; `bash` is the bash-mode flag the mode keeps.
    pub fn new(
        editor: ComponentHandle,
        text_rows: TextRows,
        read_queue: ReadQueue,
        bash: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            editor,
            text_rows,
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
