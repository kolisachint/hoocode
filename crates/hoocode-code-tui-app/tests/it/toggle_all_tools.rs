//! UI-24 `ctrl+x o`: open every tool block, or close them all when one is
//! open. The decision is `toggle_all_plan`; the blocks take the result through
//! their own expanded flag, the one Enter and a click also flip.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_code_tui_app::scroll_view::{toggle_all_plan, ToggleAll};
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use serde_json::{json, Value};

use crate::support::lock;

/// A settled Read block with a text result, so it can freeze.
fn settled_block(id: usize) -> ToolBlock {
    let mut block = ToolExecutionComponent::new(
        "Read",
        &format!("call-{id}"),
        json!({"path": "src/main.rs"}),
        ToolExecutionOptions::default(),
        None,
        "/work",
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text("line one\nline two\n")],
            details: Value::Null,
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(block))
}

#[test]
fn opens_every_block_when_none_is_open() {
    let plan = toggle_all_plan(&[(false, true), (false, true), (false, true)]);
    assert_eq!(
        plan,
        ToggleAll {
            expanded: true,
            changed: vec![0, 1, 2],
        }
    );
}

#[test]
fn closes_every_block_when_any_is_open() {
    let plan = toggle_all_plan(&[(false, true), (true, true), (true, true)]);
    assert!(!plan.expanded);
    assert_eq!(plan.changed, vec![1, 2]);
}

#[test]
fn a_block_that_cannot_open_is_left_alone_and_never_counts_as_open() {
    // The only open block cannot open (a stopped call), so nothing that
    // can open is open: open all of those, and leave it be.
    let plan = toggle_all_plan(&[(true, false), (false, true)]);
    assert!(plan.expanded);
    assert_eq!(plan.changed, vec![1]);
}

#[test]
fn older_rows_open_like_any_other() {
    // A frozen row keeps its output, so it opens and closes with the rest.
    let mut states = vec![(false, true); 30];
    states.extend(vec![(false, true); 12]);
    let plan = toggle_all_plan(&states);
    assert!(plan.expanded);
    assert_eq!(plan.changed.len(), 42);
}

#[test]
fn with_nothing_that_can_open_it_changes_nothing() {
    let plan = toggle_all_plan(&[(false, false), (false, false)]);
    assert!(plan.changed.is_empty());
}

#[test]
fn the_plan_drives_the_blocks_flag_that_enter_reads() {
    // Open all, then Enter on one block closes just that block: the flag
    // the plan sets is the one Enter flips (`toggle_focused` reads
    // `is_expanded` and writes it back).
    let _theme = lock(Some("dark"));
    let blocks: Vec<ToolBlock> = (0..3).map(settled_block).collect();
    let states: Vec<(bool, bool)> = blocks
        .iter()
        .map(|b| (b.borrow().is_expanded(), !b.borrow().is_frozen()))
        .collect();
    let plan = toggle_all_plan(&states);
    for &index in &plan.changed {
        blocks[index].borrow_mut().set_expanded(plan.expanded);
    }
    assert!(blocks.iter().all(|b| b.borrow().is_expanded()));

    let enter_on_middle = !blocks[1].borrow().is_expanded();
    blocks[1].borrow_mut().set_expanded(enter_on_middle);
    assert!(!blocks[1].borrow().is_expanded());
    assert!(blocks[0].borrow().is_expanded());
    assert!(blocks[2].borrow().is_expanded());

    // The next press sees open blocks, so it closes the open ones only.
    let states: Vec<(bool, bool)> = blocks
        .iter()
        .map(|b| (b.borrow().is_expanded(), !b.borrow().is_frozen()))
        .collect();
    let again = toggle_all_plan(&states);
    assert!(!again.expanded, "an open block means close all");
    assert_eq!(again.changed, vec![0, 2]);
}
