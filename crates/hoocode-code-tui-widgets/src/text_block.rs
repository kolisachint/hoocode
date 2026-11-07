//! Transcript text blocks (spec §4 S22/S23): a bold heading with optional
//! dim facts, then rows, no blank rows inside. One helper serves `/mcp`,
//! `/session`, `/perf`, `/changelog` and the rest.
//!
//! Two row shapes: `Fact` rows with dim labels padded to the block and ink
//! values (`Name        amber-harbor`), and `Status` glyph rows where only
//! the glyph carries colour (`● docs  4 tools`). `auth needed` reads
//! warning yellow because it waits on you; working `◉` is accent everywhere.

use hoocode_code_tui_theme::theme;

/// The glyph heading a [`BlockRow::Status`] row: done, working or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusGlyph {
    /// `●` in success green.
    Done,
    /// `◉` in accent (never yellow: yellow is for blocking only).
    Working,
    /// `⊗` in error red.
    Failed,
}

/// A text-block row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockRow {
    /// `Name        amber-harbor`: the dim label pads to the block's
    /// labels, the value reads ink.
    Fact { label: String, value: String },
    /// `● docs  4 tools`: the glyph in its colour, the name in ink, then
    /// `text` cells in their colour, each two spaces apart.
    Status {
        glyph: StatusGlyph,
        name: String,
        cells: Vec<StatusCell>,
    },
    /// A pre-styled line (a detail under a status row).
    Raw(String),
}

/// One trailing cell of a [`BlockRow::Status`] row: `text` in the `color`
/// theme token (`dim`, `warning`, `ink`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusCell {
    pub text: String,
    pub color: &'static str,
}

/// A transcript text block: the bold `heading` with dim `facts`, then rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBlock {
    heading: String,
    facts: String,
    rows: Vec<BlockRow>,
}

impl TextBlock {
    /// A block titled `heading`, with dim `facts` after it (empty draws none).
    pub fn new(heading: &str, facts: &str) -> Self {
        Self {
            heading: heading.to_string(),
            facts: facts.to_string(),
            rows: Vec::new(),
        }
    }

    /// A `label value` fact row (S23).
    pub fn fact(&mut self, label: &str, value: &str) {
        self.rows.push(BlockRow::Fact {
            label: label.to_string(),
            value: value.to_string(),
        });
    }

    /// A glyph row (S22).
    pub fn status(&mut self, glyph: StatusGlyph, name: &str, cells: Vec<StatusCell>) {
        self.rows.push(BlockRow::Status {
            glyph,
            name: name.to_string(),
            cells,
        });
    }

    /// A pre-styled line.
    pub fn raw(&mut self, line: String) {
        self.rows.push(BlockRow::Raw(line));
    }

    /// The block as transcript text: the bold heading, then one row per
    /// line, no blank rows inside. Rows sit one cell in (the transcript
    /// adds its own pad outside).
    pub fn render(self) -> String {
        let t = theme();
        let mut out = if self.facts.is_empty() {
            t.bold(&self.heading)
        } else {
            format!("{}  {}", t.bold(&self.heading), t.fg("dim", &self.facts))
        };
        let label_w = self
            .rows
            .iter()
            .filter_map(|row| match row {
                BlockRow::Fact { label, .. } => Some(label.chars().count()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let name_w = self
            .rows
            .iter()
            .filter_map(|row| match row {
                BlockRow::Status { name, .. } => Some(name.chars().count()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        for row in self.rows {
            let line = match row {
                BlockRow::Fact { label, value } => {
                    let fill = label_w.saturating_sub(label.chars().count()) + 2;
                    format!(
                        " {}{}",
                        t.fg("dim", &format!("{label}{}", " ".repeat(fill))),
                        t.fg("ink", &value)
                    )
                }
                BlockRow::Status { glyph, name, cells } => {
                    let (mark, color) = match glyph {
                        StatusGlyph::Done => ("●", "success"),
                        StatusGlyph::Working => ("◉", "accent"),
                        StatusGlyph::Failed => ("⊗", "error"),
                    };
                    let fill = name_w.saturating_sub(name.chars().count()) + 2;
                    let mut line = format!(
                        " {} {}{}",
                        t.fg(color, mark),
                        t.fg("ink", &name),
                        " ".repeat(fill)
                    );
                    for (n, cell) in cells.iter().enumerate() {
                        if n > 0 {
                            line.push_str("  ");
                        }
                        line.push_str(&t.fg(cell.color, &cell.text));
                    }
                    line
                }
                BlockRow::Raw(line) => line,
            };
            out.push('\n');
            out.push_str(&line);
        }
        out
    }
}

/// `text` in the `color` theme token (a [`StatusCell`] without the struct).
pub fn cell(text: &str, color: &'static str) -> StatusCell {
    StatusCell {
        text: text.to_string(),
        color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        hoocode_tui_util::strip_vt_control_characters(text)
    }

    #[test]
    fn facts_pad_labels_and_read_ink() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        hoocode_code_tui_theme::chalk::set_enabled(true);
        let mut block = TextBlock::new("Session info", "");
        block.fact("Name", "amber-harbor");
        block.fact("Messages", "4 user · 4 assistant");
        assert_eq!(
            plain(&block.render()),
            "Session info\n Name      amber-harbor\n Messages  4 user · 4 assistant"
        );
    }

    #[test]
    fn status_rows_color_only_the_glyph() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        hoocode_code_tui_theme::chalk::set_enabled(true);
        let mut block = TextBlock::new("MCP servers", "~/.agents/mcp.json");
        block.status(StatusGlyph::Done, "docs", vec![cell("4 tools", "dim")]);
        block.status(
            StatusGlyph::Working,
            "github",
            vec![cell("auth needed", "warning")],
        );
        let rendered = block.render();
        assert_eq!(
            plain(&rendered),
            "MCP servers  ~/.agents/mcp.json\n ● docs    4 tools\n ◉ github  auth needed"
        );
        // Only the glyph carries a state colour: the names read ink, the
        // states read their own token (`auth needed` waits on you, so it
        // reads warning yellow).
        let t = theme();
        let expected = format!(
            "{}  {}\n {} {}    {}\n {} {}  {}",
            t.bold("MCP servers"),
            t.fg("dim", "~/.agents/mcp.json"),
            t.fg("success", "●"),
            t.fg("ink", "docs"),
            t.fg("dim", "4 tools"),
            t.fg("accent", "◉"),
            t.fg("ink", "github"),
            t.fg("warning", "auth needed"),
        );
        assert_eq!(rendered, expected);
    }
}
