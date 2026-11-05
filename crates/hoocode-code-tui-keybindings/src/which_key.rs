//! The which-key popup: the leader keys and what each one does. It shows
//! once the leader has waited [`WHICH_KEY_DELAY`], so a quick chord never
//! flashes it. The rows come from the keymap, so a user's changes show.

use std::time::Duration;

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;

use crate::config::DEFAULT_LEADER;
use crate::hints::format_key_text;
use crate::keybindings::LeaderEntry;

/// How long a pending leader waits before the popup shows.
pub const WHICH_KEY_DELAY: Duration = Duration::from_millis(400);

/// Gap between two columns of the popup.
const COLUMN_GAP: usize = 3;

/// `text` cut to `max` characters, the last one `…` when it was cut.
fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The popup rows at `width` columns: the entries in as many columns as fit,
/// read down each column first. Descriptions are cut to fit one column.
/// Empty when there are no entries.
pub fn which_key_rows(entries: &[LeaderEntry], width: usize) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }
    let keys: Vec<String> = entries
        .iter()
        .map(|e| format_key_text(&e.key, false))
        .collect();
    let key_w = keys.iter().map(|k| k.chars().count()).max().unwrap_or(0);
    // A cell is the key, two spaces, the description. A row is one leading
    // space, `columns` cells and `columns - 1` gaps.
    let desc_budget = width.saturating_sub(1 + key_w + 2).max(1);
    let descriptions: Vec<String> = entries
        .iter()
        .map(|e| fit(&e.description, desc_budget))
        .collect();
    let desc_w = descriptions
        .iter()
        .map(|d| d.chars().count())
        .max()
        .unwrap_or(0);
    let cell_w = key_w + 2 + desc_w;
    let columns = ((width.max(1) + COLUMN_GAP - 1) / (cell_w + COLUMN_GAP))
        .max(1)
        .min(entries.len());
    let rows = entries.len().div_ceil(columns);
    let t = theme();
    (0..rows)
        .map(|row| {
            let mut line = String::from(" ");
            for col in 0..columns {
                // Column-major: the cell at (col, row) is entry `col * rows + row`.
                let index = col * rows + row;
                let Some(key) = keys.get(index) else {
                    break;
                };
                let description = &descriptions[index];
                // The last cell of a row has no gap after it.
                let last_in_row = col + 1 == columns || (col + 1) * rows + row >= entries.len();
                let pad_key = " ".repeat(key_w - key.chars().count());
                line.push_str(&t.fg("accent", key));
                line.push_str(&pad_key);
                line.push_str("  ");
                if last_in_row {
                    line.push_str(&t.fg("muted", description));
                } else {
                    let pad = " ".repeat(desc_w - description.chars().count());
                    line.push_str(&t.fg("muted", &format!("{description}{pad}")));
                    line.push_str(&" ".repeat(COLUMN_GAP));
                }
            }
            line
        })
        .collect()
}

/// The popup as a component, for a fixed list of entries.
pub struct WhichKeyPopup {
    entries: Vec<LeaderEntry>,
}

impl WhichKeyPopup {
    pub fn new(entries: Vec<LeaderEntry>) -> Self {
        Self { entries }
    }
}

impl Component for WhichKeyPopup {
    fn render(&mut self, width: u16) -> Vec<String> {
        which_key_rows(&self.entries, usize::from(width))
    }
}

/// The leader's name as the footer shows it: `Ctrl+X` (from the configured
/// leader, not a literal). Falls back to the default leader if it is empty.
pub fn leader_label(leader: &str) -> String {
    let leader = if leader.is_empty() {
        DEFAULT_LEADER
    } else {
        leader
    };
    format_key_text(leader, true)
}
