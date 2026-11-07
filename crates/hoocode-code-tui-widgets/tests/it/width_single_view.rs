//! Single-view width goldens (UI-21 P3, slice width): representative collapsed
//! rows at 40, 60, 80 and 120 columns, plus the cut-order contract those rows
//! follow (mock `docs/assets/mockups/tui-single-view.html` §2, tui.md
//! "Proposed: single view" cut order).
//!
//! The component goldens pin today's `Peek` rows at four widths: the target
//! never drops (it wraps), primary facts never drop, failed rows are red only
//! on `⊗`/`exit N`, and a running Shell keeps one dim tail line. The
//! single-view cut itself (secondary drops first, word-cut target, reason cut
//! at 12+ columns or dropped) lives in [`fit_header`] here as the executable
//! contract for the core `tool_row` builder slice; these goldens move to it
//! when P3 lands.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::builtin_tool_definition;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use hoocode_tui_util::visible_width;
use serde_json::{json, Value};

const CWD: &str = "/work";
const READ_TARGET: &str = "/work/crates/hoocode-code-tui-widgets/src/tool_chain.rs";
const DONE_CMD: &str = "cargo test -p hoocode-config 2>&1 | tail -40";
const FAILED_CMD: &str = "cargo test -p hoocode-parser escaped_quote";
const FAILED_REASON: &str = "error: could not compile `hoocode-parser`";
const RUN_CMD: &str = "cargo clippy -p hoocode-parser";
const WIDTHS: [u16; 4] = [40, 60, 80, 120];

fn block(
    view: ToolOutputView,
    tool: &str,
    args: Value,
    output: &str,
    details: Value,
    is_error: bool,
) -> Rc<RefCell<ToolExecutionComponent>> {
    let definition =
        builtin_tool_definition(tool).unwrap_or_else(|| panic!("missing builtin tool {tool}"));
    let mut b = ToolExecutionComponent::new(
        tool,
        "id",
        args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(definition),
        CWD,
    );
    b.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details,
            is_error,
        },
        false,
    );
    Rc::new(RefCell::new(b))
}

fn numbered(prefix: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("{prefix}{i:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read (1451 lines) plus a done Shell: the mock ladder's settled rows.
fn settled_chain(view: ToolOutputView) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    chain.add(block(
        view,
        "Read",
        json!({"file_path": READ_TARGET}),
        "ok",
        json!({"truncation": {"totalLines": 1451}}),
        false,
    ));
    chain.add(block(
        view,
        "Shell",
        json!({"command": DONE_CMD}),
        &numbered("test config::tests::case_", 40),
        json!({"durationMs": 8200}),
        false,
    ));
    chain.close(ChainState::Done);
    chain
}

/// A failed Shell: red `⊗` and `exit 1`, the reason dim, never opened.
fn failed_chain(view: ToolOutputView) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    chain.add(block(
        view,
        "Shell",
        json!({"command": FAILED_CMD}),
        "error: could not compile `hoocode-parser` (lib)\nnote: run with `cargo check` for details\nthread 'main' panicked at parser.rs:12\nCommand exited with code 1",
        json!({"durationMs": 3100}),
        true,
    ));
    chain.close(ChainState::Done);
    chain
}

/// A running Shell with one line of partial output: header plus one dim tail.
fn running_chain(view: ToolOutputView) -> ToolChainComponent {
    let definition = builtin_tool_definition("Shell").expect("builtin Shell tool");
    let mut b = ToolExecutionComponent::new(
        "Shell",
        "run-1",
        json!({"command": RUN_CMD}),
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(definition),
        CWD,
    );
    b.update_result(
        ToolResult {
            content: vec![Content::text("Compiling hoocode-parser v0.1.0")],
            details: json!({}),
            is_error: false,
        },
        true,
    );
    let mut chain = ToolChainComponent::new(view);
    chain.add(Rc::new(RefCell::new(b)));
    chain.close(ChainState::Done);
    chain
}

fn stripped_rows(chain: &mut ToolChainComponent, width: u16) -> Vec<String> {
    chain
        .render(width)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect()
}

/// Style runs of a golden's `--- styles ---` section: `(row, tokens)`.
fn style_runs(golden: &str) -> Vec<(usize, String)> {
    golden
        .split("--- styles ---\n")
        .nth(1)
        .unwrap_or("")
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let row: usize = parts.next()?.strip_prefix('r')?.parse().ok()?;
            parts.next()?; // the column span
            Some((row, parts.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

// ---- component goldens: one representative chain per width ----

#[test]
fn width_single_view_settled_40() {
    let _g = lock();
    let mut c = settled_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_settled_40", render_golden(&mut c, 40));
}

#[test]
fn width_single_view_settled_60() {
    let _g = lock();
    let mut c = settled_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_settled_60", render_golden(&mut c, 60));
}

#[test]
fn width_single_view_settled_80() {
    let _g = lock();
    let mut c = settled_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_settled_80", render_golden(&mut c, 80));
}

#[test]
fn width_single_view_settled_120() {
    let _g = lock();
    let mut c = settled_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_settled_120", render_golden(&mut c, 120));
}

#[test]
fn width_single_view_failed_40() {
    let _g = lock();
    let mut c = failed_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_failed_40", render_golden(&mut c, 40));
}

#[test]
fn width_single_view_failed_60() {
    let _g = lock();
    let mut c = failed_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_failed_60", render_golden(&mut c, 60));
}

#[test]
fn width_single_view_failed_80() {
    let _g = lock();
    let mut c = failed_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_failed_80", render_golden(&mut c, 80));
}

#[test]
fn width_single_view_failed_120() {
    let _g = lock();
    let mut c = failed_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_failed_120", render_golden(&mut c, 120));
}

#[test]
fn width_single_view_running_40() {
    let _g = lock();
    let mut c = running_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_running_40", render_golden(&mut c, 40));
}

#[test]
fn width_single_view_running_60() {
    let _g = lock();
    let mut c = running_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_running_60", render_golden(&mut c, 60));
}

#[test]
fn width_single_view_running_80() {
    let _g = lock();
    let mut c = running_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_running_80", render_golden(&mut c, 80));
}

#[test]
fn width_single_view_running_120() {
    let _g = lock();
    let mut c = running_chain(ToolOutputView::Peek);
    assert_golden!("width_single_view_running_120", render_golden(&mut c, 120));
}

// ---- behaviour the single view keeps ----

#[test]
fn rows_fit_every_width() {
    let _g = lock();
    for width in WIDTHS {
        for mut c in [
            settled_chain(ToolOutputView::Peek),
            failed_chain(ToolOutputView::Peek),
            running_chain(ToolOutputView::Peek),
        ] {
            for (i, line) in c.render(width).iter().enumerate() {
                let w = visible_width(line);
                assert!(
                    w <= width as usize,
                    "width {width} line {i} is {w}: {line:?}"
                );
            }
        }
    }
}

#[test]
fn primary_facts_and_targets_survive_narrow_widths() {
    let _g = lock();
    for width in WIDTHS {
        let out = strip(&stripped_rows(&mut settled_chain(ToolOutputView::Peek), width).join("\n"));
        assert!(out.contains("1451 lines"), "Read lines at {width}: {out}");
        assert!(out.contains("exit 0"), "Shell exit at {width}: {out}");
        assert!(
            out.contains("tool_chain.rs"),
            "Read file name at {width}: {out}"
        );
        assert!(
            out.contains("cargo test"),
            "Shell command at {width}: {out}"
        );

        let out = strip(&stripped_rows(&mut failed_chain(ToolOutputView::Peek), width).join("\n"));
        assert!(out.contains("exit 1"), "failed exit at {width}: {out}");
        assert!(
            out.contains("cargo test"),
            "failed command at {width}: {out}"
        );
    }
    let out = strip(&stripped_rows(&mut failed_chain(ToolOutputView::Peek), 80).join("\n"));
    assert!(
        out.contains("could not compile"),
        "failed reason at 80: {out}"
    );

    let out = strip(&stripped_rows(&mut running_chain(ToolOutputView::Peek), 40).join("\n"));
    assert!(out.contains("cargo clippy"), "running command at 40: {out}");
    assert!(
        out.contains("Compiling hoocode-parser"),
        "running tail at 40: {out}"
    );
}

#[test]
fn failed_red_only_on_mark_and_exit() {
    let _g = lock();
    let golden = render_golden(&mut failed_chain(ToolOutputView::Peek), 80);
    let text: Vec<&str> = golden
        .split("--- styles ---")
        .next()
        .unwrap_or("")
        .lines()
        .collect();
    let header = text
        .iter()
        .position(|r| r.contains('⊗'))
        .expect("failed header");
    let red: Vec<usize> = style_runs(&golden)
        .into_iter()
        .filter(|(_, tokens)| tokens.contains("#d58181"))
        .map(|(row, _)| row)
        .collect();
    assert!(red.len() >= 2, "mark and exit share red: {golden}");
    assert!(
        red.iter().all(|r| *r == header),
        "red only on the failed header: {golden}"
    );
}

#[test]
fn settled_rows_carry_no_red() {
    let _g = lock();
    let golden = render_golden(&mut settled_chain(ToolOutputView::Peek), 80);
    assert!(
        !golden.contains("#d58181"),
        "settled rows carry no red: {golden}"
    );
    let text: Vec<&str> = golden
        .split("--- styles ---")
        .next()
        .unwrap_or("")
        .lines()
        .collect();
    for name in ["Read", "Shell"] {
        let header = text
            .iter()
            .position(|r| r.contains(name))
            .expect("settled header");
        assert!(
            style_runs(&golden)
                .iter()
                .any(|(row, tokens)| *row == header && tokens.contains("#b5bd68")),
            "{name} mark stays green: {golden}"
        );
    }
}

#[test]
fn running_collapsed_keeps_one_dim_tail_line() {
    let _g = lock();
    for width in WIDTHS {
        let rows = stripped_rows(&mut running_chain(ToolOutputView::Peek), width);
        let header = rows
            .iter()
            .position(|r| r.contains("cargo clippy"))
            .expect("running header");
        // Today's dim `●`; P1 repaints working rows in accent `◉`.
        assert!(rows[header].starts_with(" ● "), "running mark: {rows:?}");
        let tail: Vec<&String> = rows[header + 1..]
            .iter()
            .filter(|r| !r.is_empty())
            .collect();
        assert_eq!(tail.len(), 1, "one collapsed tail line: {rows:?}");
        assert!(
            tail[0].starts_with("    ") && tail[0].contains("Compiling hoocode-parser"),
            "dim tail at column 4: {rows:?}"
        );
    }
    let golden = render_golden(&mut running_chain(ToolOutputView::Peek), 80);
    assert!(
        !golden.contains("#d58181") && !golden.contains("#b5bd68"),
        "running carries no red or green: {golden}"
    );
    let text: Vec<&str> = golden
        .split("--- styles ---")
        .next()
        .unwrap_or("")
        .lines()
        .collect();
    let tail = text
        .iter()
        .position(|r| r.contains("Compiling"))
        .expect("tail line");
    assert!(
        style_runs(&golden)
            .iter()
            .filter(|(row, _)| *row == tail)
            .all(|(_, tokens)| tokens.contains("#989898")),
        "tail line stays dim: {golden}"
    );
}

// ---- the cut order as an executable contract (mock §2) ----
//
// Steps for a header that is still too wide: (1) secondary facts drop whole,
// right to left; (2) the target shortens to its minimum, a path dropping
// whole leading directories and keeping the file name, a command cutting at a
// word; (3) a failed row's reason cuts at a word, keeping 12+ columns;
// (4) otherwise the reason drops and the target takes the space back.
// Primary facts never drop.

/// Grid cells: the fixtures are ASCII except `…`, one cell each.
fn cells(s: &str) -> usize {
    s.chars().count()
}

/// Whole leading words of `text` that fit in `avail` cells.
fn keep_words(text: &str, avail: usize) -> String {
    let mut kept = String::new();
    for w in text.split_whitespace() {
        let cand = if kept.is_empty() {
            w.to_string()
        } else {
            format!("{kept} {w}")
        };
        if cells(&cand) > avail {
            break;
        }
        kept = cand;
    }
    kept
}

/// Smallest whole-word command prefix at 12+ cells, then `…`: the target
/// minimum for a command.
fn min_command(cmd: &str) -> String {
    let mut kept = String::new();
    for w in cmd.split_whitespace() {
        kept = if kept.is_empty() {
            w.to_string()
        } else {
            format!("{kept} {w}")
        };
        if cells(&kept) >= 12 {
            break;
        }
    }
    format!("{kept}…")
}

fn middle_cut(name: &str, avail: usize) -> String {
    if cells(name) <= avail {
        return name.to_string();
    }
    if avail == 0 {
        return String::new();
    }
    if avail == 1 {
        return "…".to_string();
    }
    let tail = (avail - 1) / 2;
    let head = avail - 1 - tail;
    let chars: Vec<char> = name.chars().collect();
    let h: String = chars.iter().take(head).collect();
    let t: String = chars[chars.len() - tail..].iter().collect();
    format!("{h}…{t}")
}

fn shorten_path(path: &str, avail: usize) -> String {
    if cells(path) <= avail {
        return path.to_string();
    }
    let mut segs: Vec<&str> = path.split('/').collect();
    while segs.len() > 1 {
        segs.remove(0);
        let cand = format!("…/{}", segs.join("/"));
        if cells(&cand) <= avail {
            return cand;
        }
    }
    middle_cut(segs.last().copied().unwrap_or(path), avail)
}

fn shorten_command(cmd: &str, avail: usize) -> String {
    if cells(cmd) <= avail {
        return cmd.to_string();
    }
    let kept = keep_words(cmd, avail.saturating_sub(1));
    if cells(&kept) >= 12 {
        return format!("{kept}…");
    }
    min_command(cmd)
}

fn shorten_target(target: &str, avail: usize) -> String {
    if cells(target) <= avail {
        return target.to_string();
    }
    if target.contains('/') {
        shorten_path(target, avail)
    } else {
        shorten_command(target, avail)
    }
}

/// A failed row's reason cut at a word, or dropped when 12 columns do not
/// fit.
fn cut_reason(reason: &str, avail: usize) -> Option<String> {
    if cells(reason) <= avail {
        return Some(reason.to_string());
    }
    if avail < 12 {
        return None;
    }
    let kept = keep_words(reason, avail - 1);
    if kept.is_empty() {
        return None;
    }
    Some(format!("{kept}…"))
}

/// One single-view header row through the cut order.
fn fit_header(
    mark: char,
    tool: &str,
    target: &str,
    primary: &[&str],
    secondary: &[&str],
    reason: Option<&str>,
    width: usize,
) -> String {
    let head = format!("{mark} {tool} ");
    let prim = primary.join(" · ");
    let render = |t: &str, secs: &[&str], r: Option<&str>| {
        let mut out = format!("{head}{t}  {prim}");
        if let Some(r) = r {
            out.push_str(&format!(" · {r}"));
        }
        if !secs.is_empty() {
            out.push_str(&format!(" · {}", secs.join(" · ")));
        }
        out
    };
    // Step 1: secondary facts drop whole, right to left.
    let mut kept: Vec<&str> = secondary.to_vec();
    loop {
        if cells(&render(target, &kept, reason)) <= width {
            return render(target, &kept, reason);
        }
        if kept.pop().is_none() {
            break;
        }
    }
    // Step 2: the target shortens against the full reason, never below its
    // minimum.
    let rpart = reason.map(|r| cells(r) + 3).unwrap_or(0);
    let avail_t = width.saturating_sub(cells(&head) + 2 + cells(&prim) + rpart);
    let short = shorten_target(target, avail_t);
    // Steps 3-4: the reason cuts at a word, or drops.
    let base = format!("{head}{short}  {prim}");
    let avail_r = width.saturating_sub(cells(&base) + 3);
    match reason {
        Some(r) => match cut_reason(r, avail_r) {
            Some(cut) => format!("{base} · {cut}"),
            None => base,
        },
        None => base,
    }
}

#[test]
fn cut_order_matches_the_mock_ladder() {
    let read = "crates/hoocode-code-tui-widgets/src/tool_chain.rs";
    assert_eq!(
        fit_header('●', "Read", read, &["1451 lines"], &[], None, 40),
        "● Read …/src/tool_chain.rs  1451 lines"
    );
    assert_eq!(
        fit_header('●', "Read", read, &["1451 lines"], &[], None, 60),
        "● Read …/src/tool_chain.rs  1451 lines"
    );
    assert_eq!(
        fit_header('●', "Read", read, &["1451 lines"], &[], None, 80),
        "● Read crates/hoocode-code-tui-widgets/src/tool_chain.rs  1451 lines"
    );

    let done_primary = ["exit 0"];
    let done_secondary = ["40 lines", "8.2s"];
    assert_eq!(
        fit_header(
            '●',
            "Shell",
            DONE_CMD,
            &done_primary,
            &done_secondary,
            None,
            40
        ),
        "● Shell cargo test -p…  exit 0"
    );
    assert_eq!(
        fit_header(
            '●',
            "Shell",
            DONE_CMD,
            &done_primary,
            &done_secondary,
            None,
            80
        ),
        "● Shell cargo test -p hoocode-config 2>&1 | tail -40  exit 0 · 40 lines · 8.2s"
    );
    assert_eq!(
        fit_header(
            '●',
            "Shell",
            DONE_CMD,
            &done_primary,
            &done_secondary,
            None,
            120
        ),
        "● Shell cargo test -p hoocode-config 2>&1 | tail -40  exit 0 · 40 lines · 8.2s"
    );

    let failed_primary = ["exit 1"];
    let failed_secondary = ["3.1s"];
    assert_eq!(
        fit_header(
            '⊗',
            "Shell",
            FAILED_CMD,
            &failed_primary,
            &failed_secondary,
            Some(FAILED_REASON),
            40
        ),
        "⊗ Shell cargo test -p…  exit 1"
    );
    assert_eq!(
        fit_header(
            '⊗',
            "Shell",
            FAILED_CMD,
            &failed_primary,
            &failed_secondary,
            Some(FAILED_REASON),
            60
        ),
        "⊗ Shell cargo test -p…  exit 1 · error: could not compile…"
    );
    assert_eq!(
        fit_header(
            '⊗',
            "Shell",
            FAILED_CMD,
            &failed_primary,
            &failed_secondary,
            Some(FAILED_REASON),
            80
        ),
        "⊗ Shell cargo test -p…  exit 1 · error: could not compile `hoocode-parser`"
    );
    assert_eq!(
        fit_header('⊗', "Shell", FAILED_CMD, &failed_primary, &failed_secondary, Some(FAILED_REASON), 120),
        "⊗ Shell cargo test -p hoocode-parser escaped_quote  exit 1 · error: could not compile `hoocode-parser` · 3.1s"
    );

    assert_eq!(
        fit_header('◉', "Shell", RUN_CMD, &["4s"], &[], None, 80),
        "◉ Shell cargo clippy -p hoocode-parser  4s"
    );
}

#[test]
fn cut_order_drops_secondary_first_and_never_primary() {
    // Secondary goes before primary is touched: at 60 the done Shell keeps
    // `exit 0` with no duration or size, and still names the command.
    let row = fit_header(
        '●',
        "Shell",
        DONE_CMD,
        &["exit 0"],
        &["40 lines", "8.2s"],
        None,
        60,
    );
    assert!(row.contains("exit 0"), "{row}");
    assert!(!row.contains("8.2s") && !row.contains("40 lines"), "{row}");
    assert!(row.contains("cargo test"), "{row}");
    assert!(cells(&row) <= 60, "{row}");
    // The primary fact survives every width, including 40.
    for width in [40, 60, 80, 120] {
        let row = fit_header(
            '⊗',
            "Shell",
            FAILED_CMD,
            &["exit 1"],
            &["3.1s"],
            Some(FAILED_REASON),
            width,
        );
        assert!(row.contains("exit 1"), "{row}");
        assert!(cells(&row) <= width, "{row}");
        let row = fit_header('●', "Read", READ_TARGET, &["1451 lines"], &[], None, width);
        assert!(row.contains("1451 lines"), "{row}");
        assert!(cells(&row) <= width, "{row}");
    }
}

#[test]
fn target_minimum_keeps_the_file_name() {
    // A path shortens through whole leading directories first.
    let row = fit_header('●', "Read", READ_TARGET, &["1451 lines"], &[], None, 40);
    assert!(row.contains("tool_chain.rs"), "{row}");
    // A file name longer than the row cuts in the middle and keeps its
    // extension, never dropping the target.
    let long = "crates/hoocode-code-tui-widgets/src/a_very_long_tool_chain_renderer_name.rs";
    let row = fit_header('●', "Read", long, &["1451 lines"], &[], None, 40);
    assert!(row.contains('…'), "{row}");
    assert!(row.contains(".rs"), "{row}");
    assert!(row.contains("1451 lines"), "{row}");
    assert!(cells(&row) <= 40, "{row}");
}
