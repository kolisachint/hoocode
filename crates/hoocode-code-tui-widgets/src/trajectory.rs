//! Mission Control trajectory (peek) helpers (M4, spec §4 b1–b7, a2, a3).
//!
//! The peek view draws every mission as a trajectory: a `▲ M1` mission
//! header, numbered steps on a `┊` rail (an agent's sub-steps keep their
//! violet `┃` rail), and one `● LANDED` line closing the mission.
//!
//! Colours go through the legacy theme tokens; the Mission Control palette
//! (`hoocode-tui-ratatui`) maps its chrome tokens onto these (`you` and
//! `vio` onto `agent2`, `gold` onto `warning`), so both stay in step.

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::{strip_vt_control_characters, truncate_to_width, visible_width};

/// How many output lines of a step the rail shows before the fold.
pub const TRAJECTORY_BODY_LINES: usize = 3;

/// The trajectory rail: every step row and body line sits on it.
pub const RAIL: &str = "┊";

/// An agent's sub-steps keep their own rail, in violet.
pub const AGENT_RAIL: &str = "┃";

/// The slot thinking takes: it is not a tool call, so it takes no step number.
pub const THINK_SLOT: &str = "··";

/// A step number, 1-based and gold: `01`.
pub fn step_number(index: usize) -> String {
    let t = theme();
    t.fg("warning", &t.bold(&format!("{index:02}")))
}

/// The rail prefix a body line sits on: `   ┊  ` (or the agent `┃`).
fn rail_prefix(agent: bool) -> String {
    let t = theme();
    let rail = if agent { AGENT_RAIL } else { RAIL };
    let token = if agent { "agent2" } else { "muted" };
    format!("   {}  ", t.fg(token, rail))
}

/// The container padding (two cells) past leading styling: block lines are
/// already width-padded and styled, so the indent hides among the ANSI
/// openers and padding spaces trail at the end. Returns the styling prefix
/// and the line without its padding, so the rail keeps the line's colours.
/// Step headers split the same way: the number goes first, then the styling.
// The slices below cut at the ASCII `m` ending an SGR opener, which is
// always a char boundary.
#[allow(clippy::string_slice)]
/// Trailing padding of a width-padded block line, ignoring the styling:
/// spaces go, and the SGR resets around them are re-attached after, so the
/// line still resets its colours. Byte-safe: the scan only walks ASCII
/// (`m`, digits, `;`, `\x1b[`).
fn trim_end_styled(line: &str) -> String {
    use hoocode_tui_util::text_slice as ts;
    let mut s = line.trim_end_matches(' ');
    let mut tail = String::new();
    loop {
        let bytes = s.as_bytes();
        if !bytes.ends_with(b"m") {
            break;
        }
        let mut i = bytes.len() - 1;
        while i > 0 && (bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b';') {
            i -= 1;
        }
        if !(i >= 2 && bytes[i - 1] == b'[' && bytes[i - 2] == 0x1b) {
            break;
        }
        let esc = i - 2;
        tail = format!("{}{tail}", ts::suffix_from(s, esc));
        s = ts::range(s, 0, esc);
        s = s.trim_end_matches(' ');
    }
    format!("{s}{tail}")
}

// The slices below cut at ASCII boundaries only: the `m` ending an SGR
// opener, and one ASCII space of container padding.
#[allow(clippy::string_slice)]
pub fn split_padding(line: &str) -> (String, String) {
    let trimmed = trim_end_styled(line);
    let mut prefix = String::new();
    let mut rest = trimmed.as_str();
    let mut spaces = 0;
    loop {
        if let Some(after) = rest.strip_prefix("\x1b[") {
            if let Some(end) = after.find('m') {
                let len = 2 + end + 1;
                prefix.push_str(&rest[..len]);
                rest = &rest[len..];
                continue;
            }
            break;
        }
        if spaces < 2 && rest.starts_with(' ') {
            rest = &rest[1..];
            spaces += 1;
            continue;
        }
        break;
    }
    (prefix, rest.to_string())
}

/// `line` on the rail, cut to `width` columns. Container padding (two
/// cells) is dropped first, so relative indent inside the line survives; the
/// line keeps its own styling, only the rail is added. A blank line keeps
/// the rail bare, so the rail reads unbroken.
pub fn rail_line(line: &str, width: usize, agent: bool) -> String {
    let t = theme();
    if strip_vt_control_characters(line).trim().is_empty() {
        return truncate_to_width(&format!("   {}", t.fg("muted", RAIL)), width, "", false);
    }
    let (prefix, content) = split_padding(line);
    truncate_to_width(
        &format!("{}{}{}", rail_prefix(agent), prefix, content),
        width,
        "…",
        false,
    )
}

/// Whether a rendered line is a thinking-slot header (`·· …`): the prose
/// rail skips those, they carry their own slot.
pub fn is_think_header(line: &str) -> bool {
    strip_vt_control_characters(line)
        .trim_start()
        .starts_with(THINK_SLOT)
}

/// The fold under a step with more output: `   ┊  +N more`.
pub fn fold_line(hidden: usize, width: usize, agent: bool) -> String {
    let t = theme();
    let noun = if hidden == 1 { "line" } else { "lines" };
    truncate_to_width(
        &format!(
            "{}{}",
            rail_prefix(agent),
            t.fg("muted", &format!("+{hidden} more {noun}"))
        ),
        width,
        "…",
        false,
    )
}

/// The chip replacing a failed step's success stats: ` HIT`, in red.
pub fn hit_chip() -> String {
    let t = theme();
    format!(" {}", t.fg("error", &t.bold("HIT")))
}

/// The chip on a step that wrote a new file: ` NEW`, in green.
pub fn new_chip() -> String {
    let t = theme();
    format!(" {}", t.fg("success", &t.bold("NEW")))
}

/// A background job's row, in place of its stats: `⌁ sat job-1 · in orbit`.
pub fn satellite_text(job: &str) -> String {
    let t = theme();
    format!(
        "{} sat {} {} in orbit",
        t.fg("accent", "⌁"),
        t.fg("accent", job),
        t.fg("dim", "·"),
    )
}

/// A short duration: `4s`, `1m12s`.
pub fn short_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// A Think row (spec §4 a2): the `··` slot, header only, no gist (design
/// Q4). Running reads `·· ◐ Think  ✻ Ns` with the time right-aligned; done
/// reads `·· ● Think  ✻ thought Ns · ctrl+t`. Unknown time (a resumed
/// session never timed the trace) is omitted.
pub struct ThinkHeader {
    running: bool,
    secs: Option<u64>,
}

impl ThinkHeader {
    pub fn new(running: bool, secs: Option<u64>) -> Self {
        Self { running, secs }
    }
}

impl Component for ThinkHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        let t = theme();
        let width = width as usize;
        if width == 0 {
            return Vec::new();
        }
        let slot = t.fg("muted", THINK_SLOT);
        if self.running {
            let left = format!(
                "{slot} {} {}",
                t.fg("warning", "◐"),
                t.fg("toolTitle", "Think")
            );
            let right = self
                .secs
                .map(|s| t.fg("thinkingText", &format!("✻ {}", short_dur(s))))
                .unwrap_or_default();
            if right.is_empty() {
                return vec![truncate_to_width(&left, width, "…", false)];
            }
            let pad = width.saturating_sub(visible_width(&left) + visible_width(&right) + 2);
            return vec![truncate_to_width(
                &format!("{left}  {}{right}", " ".repeat(pad)),
                width,
                "…",
                false,
            )];
        }
        let thought = match self.secs {
            Some(s) => format!("✻ thought {}", short_dur(s)),
            None => "✻ thought".to_string(),
        };
        vec![truncate_to_width(
            &format!(
                "{slot} {} {}  {} {} {}",
                t.fg("success", "●"),
                t.fg("toolTitle", "Think"),
                t.fg("thinkingText", &thought),
                t.fg("dim", "·"),
                t.fg("text", "ctrl+t"),
            ),
            width,
            "…",
            false,
        )]
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The `⊗` a failed step carries in place of its status dot (spec §4 b2).
/// Only the leading dot is swapped; its error styling stays.
pub fn failure_dot(content: &str) -> String {
    content.replacen('●', "⊗", 1)
}

/// `N lines`.
pub fn lines_text(total: usize) -> String {
    if total == 1 {
        "1 line".to_string()
    } else {
        format!("{total} lines")
    }
}

/// `N files`.
pub fn files_text(total: usize) -> String {
    if total == 1 {
        "1 file".to_string()
    } else {
        format!("{total} files")
    }
}

/// A step header row at exactly the row width: ` NN {left}` flush left,
/// `right` (the stats) flush right. The label is cut with `…` first and the
/// stats stay whole; the stats are cut only when they fit nowhere.
pub fn step_header(
    number: &str,
    left_prefix: &str,
    left: &str,
    right: &str,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let stem = format!(" {number} {left_prefix}{left}");
    if right.is_empty() {
        return truncate_to_width(&stem, width, "…", false);
    }
    let gap = 2;
    let pad = width.saturating_sub(visible_width(&stem) + visible_width(right));
    if pad >= gap {
        return format!("{stem}{}{right}", " ".repeat(pad));
    }
    let budget = width.saturating_sub(visible_width(right) + gap);
    if budget == 0 {
        return truncate_to_width(right, width, "…", false);
    }
    let cut = truncate_to_width(&stem, budget, "…", false);
    let pad = width.saturating_sub(visible_width(&cut) + visible_width(right));
    format!("{cut}{}{right}", " ".repeat(pad))
}

/// `1 step`, `N steps`.
pub fn steps_text(steps: usize) -> String {
    if steps == 1 {
        "1 step".to_string()
    } else {
        format!("{steps} steps")
    }
}

/// `1 hit`, `N hits`.
pub fn hits_text(hits: usize) -> String {
    if hits == 1 {
        "1 hit".to_string()
    } else {
        format!("{hits} hits")
    }
}

/// The landing line ending every mission: steps, hits, time, tokens, cost.
pub fn landing_line(steps: usize, hits: usize, secs: &str, tokens: &str, cost: &str) -> String {
    let t = theme();
    let sep = t.fg("dim", " · ");
    [
        format!(
            "{} {}",
            t.fg("success", "●"),
            t.fg("success", &t.bold("LANDED"))
        ),
        t.fg("muted", &steps_text(steps)),
        t.fg("muted", &hits_text(hits)),
        t.fg("muted", secs),
        t.fg("muted", tokens),
        t.fg("muted", cost),
    ]
    .join(&sep)
}

/// Added and removed lines in a unified diff: `+` and `-` first-column
/// lines, skipping the `+++` / `---` file headers.
pub fn diff_stats(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// `+1 −1`: the changed-line counts on an Edit step's header.
pub fn diff_counts_text(added: usize, removed: usize) -> String {
    format!("+{added} \u{2212}{removed}")
}

/// The mission header: `▲ M1  prompt`, the `▲` and `M1` in the `you` colour.
/// A skill chip (`✦ name`) sits right-aligned. The prompt is cut with `…`
/// when it does not fit beside the chip.
pub struct MissionHeader {
    seq: usize,
    prompt: String,
    skill: Option<String>,
}

impl MissionHeader {
    pub fn new(seq: usize, prompt: &str, skill: Option<&str>) -> Self {
        // One line: the prompt's first line, flattened.
        let prompt = prompt
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            seq,
            prompt,
            skill: skill.map(str::to_string),
        }
    }
}

impl Component for MissionHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        let t = theme();
        let width = width as usize;
        if width == 0 {
            return Vec::new();
        }
        let mark = format!(
            "{} {}",
            t.fg("agent2", "▲"),
            t.fg("agent2", &t.bold(&format!("M{}", self.seq))),
        );
        let chip = self
            .skill
            .as_deref()
            .map(|name| format!("{} {}", t.fg("accent", "✦"), t.fg("accent", name)))
            .unwrap_or_default();
        let chip_width = visible_width(&chip);
        let prompt_budget = width
            .saturating_sub(visible_width(&mark) + 2 + chip_width + usize::from(!chip.is_empty()));
        let prompt = truncate_to_width(&self.prompt, prompt_budget, "…", false);
        let mut line = format!("{mark}  {prompt}");
        if !chip.is_empty() {
            let pad = width
                .saturating_sub(visible_width(&line) + chip_width)
                .max(1);
            line.push_str(&" ".repeat(pad));
            line.push_str(&chip);
        }
        vec![truncate_to_width(&line, width, "…", false)]
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_numbers_are_gold_and_two_wide() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        assert!(step_number(1).contains("01"));
        assert!(step_number(12).contains("12"));
    }

    #[test]
    fn diff_stats_skip_file_headers() {
        assert_eq!(diff_stats("--- a\n+++ b\n-old\n+new\n ctx\n"), (1, 1));
        assert_eq!(diff_stats(""), (0, 0));
    }

    #[test]
    fn landing_line_counts_read_right() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let line = landing_line(2, 1, "0.5s", "0.8k tokens", "$0.002");
        let bare = strip_vt_control_characters(&line);
        assert!(bare.contains("LANDED"), "{bare}");
        assert!(bare.contains("2 steps"), "{bare}");
        assert!(bare.contains("1 hit"), "{bare}");
        assert!(bare.contains("0.5s"), "{bare}");
    }

    #[test]
    fn step_header_right_aligns_stats_and_cuts_the_label_first() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        // Fits: stats flush right, label whole.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 40);
        let bare = strip_vt_control_characters(&line);
        assert_eq!(
            bare,
            format!(" 01 ● Shell  npm test{}exit 0", " ".repeat(13))
        );
        assert_eq!(visible_width(&line), 40);
        // Short: the label is cut with `…`, the stats stay whole.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 24);
        let bare = strip_vt_control_characters(&line);
        assert!(bare.ends_with("exit 0"), "{bare}");
        assert!(bare.contains('…'), "{bare}");
        assert_eq!(visible_width(&line), 24);
        // No room for stats either: the stats are cut last.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 5);
        assert_eq!(visible_width(&line), 5);
    }

    #[test]
    fn failure_dot_swaps_only_the_status_dot() {
        assert_eq!(failure_dot("● Shell  ok"), "⊗ Shell  ok");
    }

    #[test]
    fn short_durations_read_like_the_spec() {
        assert_eq!(short_dur(4), "4s");
        assert_eq!(short_dur(72), "1m12s");
    }

    #[test]
    fn think_header_carries_the_time_when_known() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut running = ThinkHeader::new(true, Some(4));
        let bare = strip_vt_control_characters(&running.render(40).join("\n"));
        assert!(bare.contains("◐ Think"), "{bare}");
        assert!(bare.ends_with("✻ 4s"), "{bare}");
        let mut done = ThinkHeader::new(false, Some(72));
        let bare = strip_vt_control_characters(&done.render(40).join("\n"));
        assert!(bare.contains("● Think"), "{bare}");
        assert!(bare.contains("✻ thought 1m12s"), "{bare}");
        assert!(bare.contains("ctrl+t"), "{bare}");
        let mut unknown = ThinkHeader::new(false, None);
        let bare = strip_vt_control_characters(&unknown.render(40).join("\n"));
        assert!(bare.contains("✻ thought · ctrl+t"), "{bare}");
    }

    #[test]
    fn mission_header_names_the_mission_and_chips_the_skill() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut header = MissionHeader::new(1, "check wrap.rs", Some("review"));
        let line = header.render(100).join("\n");
        let bare = strip_vt_control_characters(&line);
        assert!(bare.contains("M1"), "{bare}");
        assert!(bare.contains("check wrap.rs"), "{bare}");
        assert!(bare.contains("review"), "{bare}");
        assert_eq!(visible_width(&line), 100);
    }
}
