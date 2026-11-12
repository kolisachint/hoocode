//! The open or closed indicator every block that opens in place ends its
//! header in (tui.md "Blocks that toggle"): `▸` closed, `▾` open, dim, after
//! the last fact, and none on a block that cannot open.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::fold::{decorate_header, focus_row, stamp_indicator};
use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::builtin_tool_definition;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::{Component, FoldHost};
use hoocode_tui_util::visible_width;
use serde_json::{json, Value};

fn chain_of(tool: &str, args: Value, output: &str) -> ToolChainComponent {
    let mut block = ToolExecutionComponent::new(
        tool,
        "call-1",
        args,
        ToolExecutionOptions::default(),
        builtin_tool_definition(tool),
        "/work",
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details: Value::Null,
            is_error: false,
        },
        false,
    );
    let mut chain = ToolChainComponent::new();
    chain.add(Rc::new(RefCell::new(block)));
    chain.close(ChainState::Done);
    chain
}

fn header(chain: &mut ToolChainComponent, width: u16) -> String {
    chain.render(width);
    let fold = chain.folds().into_iter().next().expect("one block");
    let rows = chain.render(width);
    strip(&rows[fold.header[0]]).trim_end().to_string()
}

#[test]
fn a_row_with_output_shows_closed_then_open() {
    let _g = lock();
    let mut chain = chain_of(
        "Shell",
        json!({"command": "ls"}),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight",
    );
    let closed = header(&mut chain, 80);
    assert!(closed.ends_with(" ▸"), "{closed:?}");
    assert!(
        closed.starts_with(" ● Shell"),
        "mark at column 1: {closed:?}"
    );
    assert!(chain.folds()[0].can_open);
    assert!(chain.set_fold_expanded("call-1", true));
    let open = header(&mut chain, 80);
    assert!(open.ends_with(" ▾"), "{open:?}");
    // The mark and the name keep their columns.
    let column = |row: &str| row.split("Shell").next().map(|head| head.chars().count());
    assert_eq!(column(&closed), column(&open));
    assert_eq!(column(&closed), Some(3));
}

#[test]
fn a_row_with_nothing_to_open_shows_no_indicator() {
    let _g = lock();
    let mut empty = chain_of("Shell", json!({"command": "true"}), "");
    let row = header(&mut empty, 80);
    assert!(!row.contains('▸') && !row.contains('▾'), "{row:?}");
    assert!(!empty.folds()[0].can_open);

    let mut plan = chain_of(
        "TodoWrite",
        json!({"todos": [{"status": "pending", "content": "a"}]}),
        "[]",
    );
    let row = header(&mut plan, 80);
    assert!(!row.contains('▸') && !row.contains('▾'), "{row:?}");
    assert!(!plan.folds()[0].can_open);
}

#[test]
fn the_indicator_survives_a_narrow_row() {
    let _g = lock();
    let mut chain = chain_of(
        "Shell",
        json!({"command": "cargo test -p hoocode-parser escaped_quote"}),
        "a\nb\nc\nd\ne\nf\ng\nh",
    );
    for width in [40, 30, 24] {
        let row = header(&mut chain, width);
        assert!(row.ends_with(" ▸"), "{width}: {row:?}");
        assert!(
            visible_width(&row) <= usize::from(width),
            "{width}: {row:?}"
        );
    }
}

#[test]
fn stamp_indicator_goes_after_the_text_and_keeps_the_rest_of_the_row() {
    let _g = lock();
    let palette = Palette::current();
    // A row padded to the width keeps its width; the indicator sits right
    // after the text.
    let row = stamp_indicator("Compacted turns 1–3         ", false, 28, &palette);
    assert_eq!(strip(&row).trim_end(), "Compacted turns 1–3 ▸");
    // A sheet's right edge stays where it was.
    let row = stamp_indicator("  Branch summary      ▏", true, 24, &palette);
    assert_eq!(strip(&row), "  Branch summary ▾    ▏");
    // A row too full for it is cut by what it needs, the cut shown.
    let row = stamp_indicator("0123456789", false, 10, &palette);
    assert_eq!(strip(&row), "0123456… ▸");
    assert_eq!(visible_width(&row), 10);
}

#[test]
fn the_focused_header_takes_the_gutter_without_moving_its_text() {
    let _g = lock();
    let palette = Palette::current();
    // A blank column 0 takes the marker.
    let row = focus_row(" ● Shell ls ▸", 20, &palette);
    assert_eq!(strip(&row), "›● Shell ls ▸       ");
    // Text that starts at column 0 moves one cell; the blanks at the end
    // make room, so nothing is cut.
    let row = focus_row("Compacted turns 1–3 ▸   ", 24, &palette);
    assert_eq!(strip(&row), "›Compacted turns 1–3 ▸  ");

    let mut rows = vec![" ● Shell ls".to_string()];
    decorate_header(&mut rows, &[0], true, false, true, 20, &palette);
    assert_eq!(strip(&rows[0]).trim_end(), "›● Shell ls ▸");
}
