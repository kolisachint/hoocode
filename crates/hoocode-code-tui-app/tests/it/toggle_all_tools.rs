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
    let plan = toggle_all_plan(&[(false, false), (false, false), (false, false)]);
    assert_eq!(
        plan,
        ToggleAll {
            expanded: true,
            changed: vec![0, 1, 2],
            skipped: 0,
        }
    );
}

#[test]
fn closes_every_block_when_any_is_open() {
    let plan = toggle_all_plan(&[(false, false), (true, false), (true, false)]);
    assert!(!plan.expanded);
    assert_eq!(plan.changed, vec![1, 2]);
    assert_eq!(plan.skipped, 0);
    assert_eq!(plan.notice(), None, "closing needs no notice");
}

#[test]
fn a_frozen_open_block_does_not_count_as_open() {
    // The only open block is frozen, so nothing live is open: open all.
    let plan = toggle_all_plan(&[(true, true), (false, false)]);
    assert!(plan.expanded);
    assert_eq!(plan.changed, vec![1]);
    assert_eq!(plan.skipped, 1);
}

#[test]
fn frozen_blocks_are_skipped_and_the_open_says_so() {
    let mut states = vec![(false, true); 30];
    states.extend(vec![(false, false); 12]);
    let plan = toggle_all_plan(&states);
    assert!(plan.expanded);
    assert_eq!(plan.changed.len(), 12);
    assert_eq!(plan.skipped, 30);
    assert!(plan.changed.iter().all(|index| *index >= 30));
    assert_eq!(
        plan.notice().as_deref(),
        Some("Opened 12 rows. 30 older rows have no output to show.")
    );
}

#[test]
fn the_notice_reads_singular_for_one_row() {
    let plan = toggle_all_plan(&[(false, true), (false, false)]);
    assert_eq!(
        plan.notice().as_deref(),
        Some("Opened 1 row. 1 older row has no output to show.")
    );
}

#[test]
fn an_open_that_skips_nothing_says_nothing() {
    let plan = toggle_all_plan(&[(false, false), (false, false)]);
    assert_eq!(plan.notice(), None);
}

#[test]
fn with_no_live_blocks_it_opens_nothing_and_still_names_the_frozen_ones() {
    let plan = toggle_all_plan(&[(false, true), (false, true)]);
    assert!(plan.changed.is_empty());
    assert_eq!(
        plan.notice().as_deref(),
        Some("Opened 0 rows. 2 older rows have no output to show.")
    );
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
        .map(|b| (b.borrow().is_expanded(), b.borrow().is_frozen()))
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
        .map(|b| (b.borrow().is_expanded(), b.borrow().is_frozen()))
        .collect();
    let again = toggle_all_plan(&states);
    assert!(!again.expanded, "an open block means close all");
    assert_eq!(again.changed, vec![0, 2]);
}
