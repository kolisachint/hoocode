//! The plan's done/total covers every item TodoWrite last sent, across the
//! store's `reset` at each new user message. Before the fix the reset dropped
//! the done plan items while others were still open, so a plan of 8 with 3 done
//! read `0/5` until the next TodoWrite put them back (`3/8`), and flipped again
//! at the next message.

use std::sync::Arc;

use hoocode_code_task_store::{TaskStatus, TaskStore};
use hoocode_code_tool_api::ToolDefinition;
use hoocode_code_tools_optin::*;
use serde_json::{json, Value};

fn setup() -> (Arc<TaskStore>, ToolDefinition) {
    let store = Arc::new(TaskStore::new());
    let tool = create_todo_write_tool_definition(StoreRef::Owned(store.clone()));
    (store, tool)
}

fn write(tool: &ToolDefinition, todos: Value) {
    (tool.execute)("plan".into(), json!({ "todos": todos }), None, None, None)
        .expect("TodoWrite succeeds");
}

/// Eight items: the first `done` completed, the next one in progress.
fn plan(done: usize) -> Value {
    let items: Vec<Value> = (0..8)
        .map(|i| {
            let status = match i.cmp(&done) {
                std::cmp::Ordering::Less => "completed",
                std::cmp::Ordering::Equal => "in_progress",
                std::cmp::Ordering::Greater => "pending",
            };
            json!({ "content": format!("step {i}"), "status": status })
        })
        .collect();
    Value::Array(items)
}

/// The plan's (done, total), as the panel's Plan tab counts it.
fn counts(store: &TaskStore) -> (usize, usize) {
    let items: Vec<_> = store
        .list()
        .into_iter()
        .filter(|t| t.is_plan_item())
        .collect();
    let done = items
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    (done, items.len())
}

#[test]
fn a_plan_of_eight_with_three_done_counts_three_of_eight_across_turns() {
    let (store, tool) = setup();
    for done in 0..=3 {
        write(&tool, plan(done));
        assert_eq!(
            counts(&store),
            (done, 8),
            "after TodoWrite with {done} done"
        );
    }
    // The same set, rewritten as statuses change, with new user messages
    // (the app resets the store at each) in between.
    for _ in 0..3 {
        store.reset();
        assert_eq!(counts(&store), (3, 8), "a new message keeps the plan whole");
        write(&tool, plan(3));
        assert_eq!(counts(&store), (3, 8), "the same plan rewritten");
    }
    let ids: Vec<u64> = store.list().iter().map(|t| t.id).collect();
    write(&tool, plan(4));
    store.reset();
    assert_eq!(counts(&store), (4, 8));
    assert_eq!(
        store.list().iter().map(|t| t.id).collect::<Vec<_>>(),
        ids,
        "rewrites keep every row's id: no stale or duplicate items"
    );
}

#[test]
fn a_finished_plan_still_leaves_at_the_next_message() {
    let (store, tool) = setup();
    write(&tool, plan(8));
    assert_eq!(counts(&store), (8, 8));
    store.reset();
    assert_eq!(counts(&store), (0, 0), "a fully done plan goes as a whole");
}
