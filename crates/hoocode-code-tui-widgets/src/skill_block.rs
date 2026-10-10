//! The `<skill …>` block a `/skill:name` message carries, as the transcript
//! draws it. Radar shows one row, `◆ skill: <name>`. Peek (the default)
//! adds the first lines of the skill's body, then how many are left.

use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Text;
use hoocode_tui_render::Component;

use crate::tool_output_view::PEEK_LINES;

pub struct SkillBlockComponent {
    name: String,
    body: String,
    expanded: bool,
    text: Text,
}

impl SkillBlockComponent {
    /// `name` and `body` come from `parse_skill_block`.
    pub fn new(name: &str, body: &str) -> Self {
        let mut this = Self {
            name: name.to_string(),
            body: body.to_string(),
            expanded: true,
            text: Text::new("", 0, 0),
        };
        this.rebuild();
        this
    }

    /// `true` at peek (the body shows), `false` at radar (the row only).
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let t = theme();
        let row = t.fg(
            "customMessageText",
            &t.bold(&format!("◆ skill: {}", self.name)),
        );
        let content = if self.expanded {
            let body = self.body.trim();
            if body.is_empty() {
                row
            } else {
                let lines: Vec<&str> = body.split('\n').collect();
                let shown = lines
                    .iter()
                    .take(PEEK_LINES)
                    .map(|l| t.fg("toolOutput", l))
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut text = format!("{row}\n{shown}");
                if lines.len() > PEEK_LINES {
                    let remaining = lines.len() - PEEK_LINES;
                    text.push_str(&t.fg("muted", &format!("\n... ({remaining} more lines)")));
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
