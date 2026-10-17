//! The TodoWrite block: one header row. The plan itself shows in the task
//! panel, so the transcript keeps no list body; the result text (the list the
//! model reads back) stays out of the block unless the call failed.

use std::rc::Rc;

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Container;
use serde_json::Value;

use super::text;
use crate::tool_execution::ToolRenderDefinition;

/// `3 items · 1 done`, counted from the call's `todos` array.
fn stats(args: &Value) -> String {
    let todos: &[Value] = args
        .get("todos")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let done = todos
        .iter()
        .filter(|t| t.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    let items = match todos.len() {
        1 => "1 item".to_owned(),
        n => format!("{n} items"),
    };
    format!("{items} · {done} done")
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, _ctx| {
            let t = theme();
            Ok(text(format!(
                "{}  {}",
                t.fg("toolTitle", &t.bold("TodoWrite")),
                t.fg("muted", &stats(args))
            )))
        })),
        render_result: Some(Rc::new(|result, _options, ctx| {
            if ctx.is_error {
                let output = result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        hoocode_ai_types::Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !output.is_empty() {
                    return Ok(text(theme().fg("error", &output)));
                }
            }
            Ok(Rc::new(std::cell::RefCell::new(Container::new())))
        })),
    }
}
