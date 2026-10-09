//! Golden snapshots for renderer and component tests.
//!
//! [`render_golden`] draws a component into a `vt100` screen and returns a
//! text form: the screen rows (trailing spaces trimmed, trailing empty rows
//! dropped), then a `--- styles ---` line, then one line per run of
//! non-default cell style:
//!
//! ```text
//! red plain bold
//! --- styles ---
//! r0 c0-2 fg=1
//! r0 c10-13 bold underline
//! ```
//!
//! [`assert_golden!`] compares that text with `tests/golden/<crate>/<name>.txt`
//! at the workspace root. `UPDATE_GOLDENS=1` writes the file instead.
//!
//! Enable with the `golden` feature. Other crates add it as a dev-dependency:
//! `hoocode-tui-render = { workspace = true, features = ["golden"] }`.

use crate::Component;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Renders `component` at `width` columns and returns its golden text form.
pub fn render_golden(component: &mut dyn Component, width: u16) -> String {
    let lines = component.render(width);
    let cols = width.max(1);
    let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX).max(1);
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(lines.join("\r\n").as_bytes());
    let screen = parser.screen();

    let mut text: Vec<String> = screen
        .rows(0, cols)
        .map(|row| row.trim_end().to_string())
        .collect();
    while text.last().is_some_and(String::is_empty) {
        text.pop();
    }

    let mut out = String::new();
    for row in &text {
        out.push_str(row);
        out.push('\n');
    }
    out.push_str("--- styles ---\n");
    for row in 0..rows {
        let styles: Vec<String> = (0..cols)
            .map(|col| screen.cell(row, col).map(cell_style).unwrap_or_default())
            .collect();
        for run in style_runs(row, &styles) {
            out.push_str(&run);
            out.push('\n');
        }
    }
    out
}

/// The style of one cell as space-separated tokens; empty for default style.
fn cell_style(cell: &vt100::Cell) -> String {
    let mut tokens = Vec::new();
    if let Some(fg) = color_token(cell.fgcolor()) {
        tokens.push(format!("fg={fg}"));
    }
    if let Some(bg) = color_token(cell.bgcolor()) {
        tokens.push(format!("bg={bg}"));
    }
    for (on, name) in [
        (cell.bold(), "bold"),
        (cell.italic(), "italic"),
        (cell.underline(), "underline"),
        (cell.inverse(), "inverse"),
    ] {
        if on {
            tokens.push(name.to_string());
        }
    }
    tokens.join(" ")
}

fn color_token(color: vt100::Color) -> Option<String> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(n) => Some(n.to_string()),
        vt100::Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
    }
}

/// Groups equal adjacent non-empty styles of one row into `r<row> c<a>-<b> <style>` lines.
fn style_runs(row: u16, styles: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut col = 0;
    while col < styles.len() {
        if styles[col].is_empty() {
            col += 1;
            continue;
        }
        let start = col;
        while col + 1 < styles.len() && styles[col + 1] == styles[start] {
            col += 1;
        }
        let span = if start == col {
            format!("c{start}")
        } else {
            format!("c{start}-{col}")
        };
        out.push(format!("r{row} {span} {}", styles[start]));
        col += 1;
    }
    out
}

/// The golden file for `name` of the crate at `manifest_dir`: `<workspace>/tests/golden/<krate>/<name>.txt`.
///
/// The workspace root is taken as two levels above a crate's manifest directory (`crates/<name>`).
pub fn golden_file(manifest_dir: &str, krate: &str, name: &str) -> PathBuf {
    Path::new(manifest_dir)
        .join("../..")
        .join("tests/golden")
        .join(krate)
        .join(format!("{name}.txt"))
}

/// Whether `UPDATE_GOLDENS=1` asks for golden files to be rewritten.
pub fn update_requested() -> bool {
    std::env::var("UPDATE_GOLDENS").is_ok_and(|v| v == "1")
}

/// Writes `text` to `path` when `update` is set; otherwise compares it with the file.
///
/// Panics when the file is missing or differs, with a line diff in the message.
pub fn check_golden(path: &Path, text: &str, update: bool) {
    if update {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .unwrap_or_else(|err| panic!("cannot create {}: {err}", dir.display()));
        }
        fs::write(path, text)
            .unwrap_or_else(|err| panic!("cannot write golden {}: {err}", path.display()));
        return;
    }
    let expected = match fs::read_to_string(path) {
        Ok(expected) => expected,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => panic!(
            "golden file {} is missing; create it with `UPDATE_GOLDENS=1 cargo test`",
            path.display()
        ),
        Err(err) => panic!("cannot read golden {}: {err}", path.display()),
    };
    if expected != text {
        panic!(
            "golden mismatch for {} (run with `UPDATE_GOLDENS=1 cargo test` to accept)\n{}",
            path.display(),
            line_diff(&expected, text)
        );
    }
}

/// A plain line diff: `  ` for common lines, `- ` for expected only, `+ ` for actual only.
fn line_diff(expected: &str, actual: &str) -> String {
    let a: Vec<&str> = expected.split('\n').collect();
    let b: Vec<&str> = actual.split('\n').collect();
    let (n, m) = (a.len(), b.len());
    // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = String::from("--- expected\n+++ actual\n");
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            let _ = writeln!(out, "  {}", a[i]);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            let _ = writeln!(out, "+ {}", b[j]);
            j += 1;
        } else {
            let _ = writeln!(out, "- {}", a[i]);
            i += 1;
        }
    }
    out
}

/// Asserts that `$text` matches `tests/golden/<crate>/<$name>.txt` at the workspace root.
///
/// `<crate>` is the calling crate's package name. With `UPDATE_GOLDENS=1` the
/// file is written instead. Needs the `golden` feature of `hoocode-tui-render`.
#[macro_export]
macro_rules! assert_golden {
    ($name:expr, $text:expr $(,)?) => {
        $crate::golden::check_golden(
            &$crate::golden::golden_file(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"), $name),
            &$text,
            $crate::golden::update_requested(),
        )
    };
}
