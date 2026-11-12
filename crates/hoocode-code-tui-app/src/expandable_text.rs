//! `ExpandableText` (`resource-display.ts`): text with a collapsed and an
//! expanded form, both computed on demand.

use hoocode_code_tui_widgets::fold::{decorate_header, FoldState};
use hoocode_tui_components::Text;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::{Component, Fold, FoldHost};

type TextFn = Box<dyn Fn() -> String>;

/// Something the expand key can open and close.
pub trait Expandable {
    fn set_expanded(&mut self, expanded: bool);
}

pub struct ExpandableText {
    text: Text,
    collapsed: TextFn,
    expanded: TextFn,
    is_expanded: bool,
    /// The row a header block shows while closed, and above the text while
    /// open: its click target and its open or closed indicator. A block
    /// with none draws only its text.
    header: Option<String>,
    fold: FoldState,
}

impl ExpandableText {
    pub fn new(
        collapsed: impl Fn() -> String + 'static,
        expanded: impl Fn() -> String + 'static,
        initially_expanded: bool,
        padding_x: usize,
        padding_y: usize,
    ) -> Self {
        let initial = if initially_expanded {
            expanded()
        } else {
            collapsed()
        };
        Self {
            text: Text::new(initial, padding_x, padding_y),
            collapsed: Box::new(collapsed),
            expanded: Box::new(expanded),
            is_expanded: initially_expanded,
            header: None,
            fold: FoldState::new("details"),
        }
    }

    /// Gives the block its header row, themed text that starts at column 1.
    pub fn with_header(mut self, header: String) -> Self {
        self.header = Some(header);
        self
    }

    /// Re-evaluate the current state's text (what it reads may have changed,
    /// e.g. the working directory after `/cd`).
    pub fn refresh(&mut self) {
        let text = if self.is_expanded {
            (self.expanded)()
        } else {
            (self.collapsed)()
        };
        self.text.set_text(text);
    }
}

impl Expandable for ExpandableText {
    fn set_expanded(&mut self, expanded: bool) {
        self.is_expanded = expanded;
        self.refresh();
    }
}

impl Component for ExpandableText {
    fn render(&mut self, width: u16) -> Vec<String> {
        let Some(header) = self.header.clone() else {
            return self.text.render(width);
        };
        let mut rows = Text::new(header, 0, 0).render(width);
        rows.truncate(1);
        if self.is_expanded {
            rows.extend(self.text.render(width));
        }
        decorate_header(
            &mut rows,
            &[0],
            true,
            self.is_expanded,
            self.fold.focused(),
            usize::from(width),
            &Palette::current(),
        );
        rows
    }

    fn fold_host(&self) -> Option<&dyn FoldHost> {
        self.header.is_some().then_some(self as &dyn FoldHost)
    }

    fn fold_host_mut(&mut self) -> Option<&mut dyn FoldHost> {
        if self.header.is_some() {
            Some(self)
        } else {
            None
        }
    }

    fn invalidate(&mut self) {
        self.text.invalidate();
    }
}

impl FoldHost for ExpandableText {
    fn folds(&self) -> Vec<Fold> {
        vec![Fold {
            id: self.fold.id().to_string(),
            label: "Resources".to_string(),
            header: vec![0],
            fold_line: None,
            expanded: self.is_expanded,
            can_open: true,
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
        self.fold.is(id).then(|| (self.expanded)())
    }
}
