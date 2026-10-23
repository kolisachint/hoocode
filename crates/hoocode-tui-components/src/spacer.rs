//! Spacer component that renders empty lines, ported from `components/spacer.ts`.

use hoocode_tui_render::{fresh_stamp, Component};

pub struct Spacer {
    lines: usize,
    rev: u64,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self {
            lines,
            rev: fresh_stamp(),
        }
    }

    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines;
        self.rev = fresh_stamp();
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Component for Spacer {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec![String::new(); self.lines]
    }

    fn render_stamp(&self) -> Option<u64> {
        Some(self.rev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_configured_number_of_empty_lines() {
        let mut spacer = Spacer::new(3);
        let lines = spacer.render(10);
        assert_eq!(lines, vec!["".to_string(); 3]);
    }

    #[test]
    fn default_is_one_line() {
        let mut spacer = Spacer::default();
        assert_eq!(spacer.render(10).len(), 1);
    }

    #[test]
    fn set_lines_changes_output() {
        let mut spacer = Spacer::new(1);
        spacer.set_lines(5);
        assert_eq!(spacer.render(10).len(), 5);
    }
}
