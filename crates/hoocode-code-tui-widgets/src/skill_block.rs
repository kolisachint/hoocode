//! The `<skill …>` block a `/skill:name` message carries, as the transcript
//! draws it. Collapsed, one row: `skill <name>` with the name in bold (no
//! mark: a skill block has no state). Expanded, the body follows, whole.
//! One toggle (`expanded`); the transcript flips it in place.

use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Text;
use hoocode_tui_render::Component;

use crate::fold::more_lines_hint;
use crate::tool_output_view::line_budget;

pub struct SkillBlockComponent {
    name: String,
    body: String,
    expanded: bool,
    full: bool,
    text: Text,
}

impl SkillBlockComponent {
    /// `name` and `body` come from `parse_skill_block`.
    pub fn new(name: &str, body: &str) -> Self {
        let mut this = Self {
            name: name.to_string(),
            body: body.to_string(),
            expanded: true,
            full: false,
            text: Text::new("", 0, 0),
        };
        this.rebuild();
        this
    }

    /// `true` at peek (the body shows), `false` at outline (the row only).
    /// Single view (UI-21 P4): the block's own toggle; collapsed is the
    /// `skill <name>` row, expanded the whole body.
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rebuild();
    }

    /// Whether the block's own toggle is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// `true` at the full stop: the whole body shows, nothing folded.
    pub fn set_full(&mut self, full: bool) {
        self.full = full;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let t = theme();
        let row = format!("skill {}", t.fg("customMessageText", &t.bold(&self.name)));
        // Expanded, the whole body: the toggle opens the block, and the
        // dial's full stop with it.
        let content = if self.expanded {
            let body = self.body.trim();
            if body.is_empty() {
                row
            } else {
                let lines: Vec<&str> = body.split('\n').collect();
                let budget = line_budget(self.full || self.expanded);
                let shown = lines
                    .iter()
                    .take(budget)
                    .map(|l| t.fg("toolOutput", l))
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut text = format!("{row}\n{shown}");
                if lines.len() > budget {
                    text.push_str(&format!("\n{}", more_lines_hint(lines.len() - budget)));
                }
                text
            }
        } else {
            row
        };
        self.text.set_text(content);
    }
}

impl Component for SkillBlockComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.text.render(width)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn invalidate(&mut self) {
        self.text.invalidate();
    }
}
