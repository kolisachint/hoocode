//! The `<skill …>` block a `/skill:name` message carries, as the transcript
//! draws it. Collapsed, one row: the name in bold (no mark and no prefix:
//! a skill block has no state). Expanded, the body follows, whole.
//! One toggle (`expanded`); the transcript flips it in place.

use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Text;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::{Component, Fold, FoldHost};

use crate::fold::{decorate_header, line_budget, more_lines_hint, FoldState};

pub struct SkillBlockComponent {
    name: String,
    body: String,
    expanded: bool,
    text: Text,
    fold: FoldState,
}

impl SkillBlockComponent {
    /// `name` and `body` come from `parse_skill_block`.
    pub fn new(name: &str, body: &str) -> Self {
        let mut this = Self {
            name: name.to_string(),
            body: body.to_string(),
            expanded: true,
            text: Text::new("", 0, 0),
            fold: FoldState::new("skill"),
        };
        this.rebuild();
        this
    }

    /// The block's own toggle; collapsed is the bold-name row,
    /// expanded the whole body.
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rebuild();
    }

    /// Whether the block's own toggle is open.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Whether opening shows anything: the block has a body.
    fn can_open(&self) -> bool {
        !self.body.trim().is_empty()
    }

    fn rebuild(&mut self) {
        let t = theme();
        let row = t.fg("customMessageText", &t.bold(&self.name)).to_string();
        // Expanded, the whole body: the toggle opens the block.
        let content = if self.expanded {
            let body = self.body.trim();
            if body.is_empty() {
                row
            } else {
                let lines: Vec<&str> = body.split('\n').collect();
                let budget = line_budget(self.expanded);
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
        let width = usize::from(width);
        let mut rows = self.text.render(width as u16);
        decorate_header(
            &mut rows,
            &[0],
            self.can_open(),
            self.expanded,
            self.fold.focused(),
            width,
            &Palette::current(),
        );
        rows
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn fold_host(&self) -> Option<&dyn FoldHost> {
        Some(self)
    }

    fn fold_host_mut(&mut self) -> Option<&mut dyn FoldHost> {
        Some(self)
    }

    fn invalidate(&mut self) {
        self.text.invalidate();
    }
}

impl FoldHost for SkillBlockComponent {
    fn folds(&self) -> Vec<Fold> {
        vec![Fold {
            id: self.fold.id().to_string(),
            label: "Skill".to_string(),
            header: vec![0],
            fold_line: None,
            expanded: self.expanded,
            can_open: self.can_open(),
        }]
    }

    fn set_fold_expanded(&mut self, id: &str, expanded: bool) -> bool {
        if !self.fold.is(id) {
            return false;
        }
        self.set_expanded(expanded);
        true
    }

    fn set_fold_focus(&mut self, id: Option<&str>) {
        self.fold.set_focus(id);
    }

    fn fold_text(&self, id: &str) -> Option<String> {
        self.fold.is(id).then(|| self.body.clone())
    }
}
