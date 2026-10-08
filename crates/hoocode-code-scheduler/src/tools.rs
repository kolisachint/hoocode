//! The CronCreate, CronList and CronDelete tool definitions
//! (`extensions/core/loop.ts`, names, descriptions and schemas verbatim).
//!
//! They have no prompt snippet, so they are not in the system prompt's tool
//! list, as in hoocode-ts.

use std::path::Path;
use std::sync::Arc;

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::{Content, TextContent};
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use serde_json::{json, Value};

use crate::cron;
use crate::store::TaskStore;

/// `CronCreate`.
pub const CRON_CREATE_TOOL_NAME: &str = "CronCreate";
/// `CronList`.
pub const CRON_LIST_TOOL_NAME: &str = "CronList";
/// `CronDelete`.
pub const CRON_DELETE_TOOL_NAME: &str = "CronDelete";

fn text_result(text: String) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text,
        })],
        details: Value::Null,
        terminate: false,
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string").into())
}

/// `CronCreate`, `CronList` and `CronDelete` for the store of `cwd`.
pub fn create_cron_tool_definitions(cwd: &Path) -> Vec<ToolDefinition> {
    let store = Arc::new(TaskStore::for_cwd(cwd));
    vec![
        create_cron_create_tool_definition(store.clone()),
        create_cron_list_tool_definition(store.clone()),
        create_cron_delete_tool_definition(store),
    ]
}

/// `createCronCreateToolDefinition`.
pub fn create_cron_create_tool_definition(store: Arc<TaskStore>) -> ToolDefinition {
    ToolDefinition {
        name: CRON_CREATE_TOOL_NAME.into(),
        label: "Schedule Task".into(),
        description: "Schedule a prompt to be re-submitted on a cron schedule (5-field, local time: minute hour day-of-month month day-of-week). recurring=false fires once then deletes.".into(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        parameters: json!({
            "type": "object",
            "properties": {
                "cron": {"type": "string", "description": "5-field cron expression in local time"},
                "prompt": {"type": "string", "description": "Prompt to enqueue at each fire time"},
                "recurring": {"type": "boolean", "description": "Fire repeatedly (default true) or once"}
            },
            "required": ["cron", "prompt"]
        }),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        ordered_start: false,
        execute: Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let cron_expr = string_arg(&args, "cron")?;
            let prompt = string_arg(&args, "prompt")?;
            let recurring = args.get("recurring").and_then(Value::as_bool).unwrap_or(true);
            if !cron::is_five_field(cron_expr) {
                return Ok(text_result(format!(
                    "Invalid cron \"{cron_expr}\" (need 5 fields)."
                )));
            }
            let task = store
                .create(cron_expr, prompt, recurring)
                .map_err(|e| format!("Failed to save the scheduled task: {e}"))?;
            Ok(text_result(format!(
                "Scheduled {}: \"{}\" ({})",
                task.id,
                task.cron,
                if task.recurring { "recurring" } else { "once" }
            )))
        }),
    }
}

/// `createCronListToolDefinition`.
pub fn create_cron_list_tool_definition(store: Arc<TaskStore>) -> ToolDefinition {
    ToolDefinition {
        name: CRON_LIST_TOOL_NAME.into(),
        label: "List Scheduled Tasks".into(),
        description: "List all scheduled tasks (id, cron, recurring, prompt).".into(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        parameters: json!({"type": "object", "properties": {}}),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        ordered_start: false,
        execute: Arc::new(move |_id, _args, _signal, _on_update, _ctx| {
            let tasks = store.list();
            if tasks.is_empty() {
                return Ok(text_result("No scheduled tasks.".into()));
            }
            let lines: Vec<String> = tasks
                .iter()
                .map(|t| {
                    format!(
                        "{}  {}  {}  {}",
                        t.id,
                        t.cron,
                        if t.recurring { "recurring" } else { "once" },
                        serde_json::to_string(&t.prompt).unwrap_or_default()
                    )
                })
                .collect();
            Ok(text_result(lines.join("\n")))
        }),
    }
}

/// `createCronDeleteToolDefinition`.
pub fn create_cron_delete_tool_definition(store: Arc<TaskStore>) -> ToolDefinition {
    ToolDefinition {
        name: CRON_DELETE_TOOL_NAME.into(),
        label: "Delete Scheduled Task".into(),
        description: "Delete a scheduled task by id.".into(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Task id from CronCreate/CronList"}
            },
            "required": ["id"]
        }),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        ordered_start: false,
        execute: Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let id = string_arg(&args, "id")?;
            let removed = store
                .delete(id)
                .map_err(|e| format!("Failed to update the scheduled tasks: {e}"))?;
            Ok(text_result(if removed {
                format!("Deleted {id}.")
            } else {
                format!("No task {id}.")
            }))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(def: &ToolDefinition, args: Value) -> Result<String, String> {
        match (def.execute)("call".into(), args, None, None, None) {
            Ok(result) => Ok(match &result.content[0] {
                Content::Text(t) => t.text.clone(),
                other => panic!("unexpected content {other:?}"),
            }),
            Err(error) => Err(error.to_string()),
        }
    }

    fn three(dir: &Path) -> [ToolDefinition; 3] {
        create_cron_tool_definitions(dir)
            .try_into()
            .unwrap_or_else(|_| panic!("three tools"))
    }

    #[test]
    fn definitions_have_the_hoocode_ts_names_labels_and_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let [create, list, delete] = three(dir.path());
        assert_eq!(create.name, "CronCreate");
        assert_eq!(create.label, "Schedule Task");
        assert_eq!(list.name, "CronList");
        assert_eq!(list.label, "List Scheduled Tasks");
        assert_eq!(delete.name, "CronDelete");
        assert_eq!(delete.label, "Delete Scheduled Task");
        for tool in [&create, &list, &delete] {
            assert!(
                tool.prompt_snippet.is_none(),
                "{} has no snippet in TS",
                tool.name
            );
        }
        assert_eq!(create.parameters["required"], json!(["cron", "prompt"]));
        assert_eq!(
            create.parameters["properties"]["recurring"]["description"],
            "Fire repeatedly (default true) or once"
        );
        assert_eq!(list.parameters, json!({"type": "object", "properties": {}}));
        assert_eq!(delete.parameters["required"], json!(["id"]));
    }

    #[test]
    fn create_list_delete_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let [create, list, delete] = three(dir.path());
        assert_eq!(run(&list, json!({})).unwrap(), "No scheduled tasks.");

        let created = run(
            &create,
            json!({"cron": "*/5 * * * *", "prompt": "check the build"}),
        )
        .unwrap();
        let id = created
            .strip_prefix("Scheduled ")
            .and_then(|rest| rest.split(':').next())
            .unwrap()
            .to_owned();
        assert_eq!(
            created,
            format!("Scheduled {id}: \"*/5 * * * *\" (recurring)")
        );

        let once = run(
            &create,
            json!({"cron": "0 9 * * *", "prompt": "standup", "recurring": false}),
        )
        .unwrap();
        assert!(once.ends_with("(once)"), "{once}");

        let listed = run(&list, json!({})).unwrap();
        let lines: Vec<&str> = listed.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            format!("{id}  */5 * * * *  recurring  \"check the build\"")
        );

        assert_eq!(
            run(&delete, json!({"id": id})).unwrap(),
            format!("Deleted {id}.")
        );
        assert_eq!(
            run(&delete, json!({"id": "nope"})).unwrap(),
            "No task nope."
        );
        assert_eq!(run(&list, json!({})).unwrap().lines().count(), 1);
    }

    #[test]
    fn create_rejects_cron_without_five_fields() {
        let dir = tempfile::tempdir().unwrap();
        let [create, _, _] = three(dir.path());
        assert_eq!(
            run(&create, json!({"cron": "*/5 * * *", "prompt": "x"})).unwrap(),
            "Invalid cron \"*/5 * * *\" (need 5 fields)."
        );
        assert!(TaskStore::for_cwd(dir.path()).list().is_empty());
    }

    #[test]
    fn missing_arguments_are_tool_errors() {
        let dir = tempfile::tempdir().unwrap();
        let [create, _, delete] = three(dir.path());
        assert_eq!(
            run(&create, json!({"cron": "* * * * *"})).unwrap_err(),
            "prompt must be a string"
        );
        assert_eq!(run(&delete, json!({})).unwrap_err(), "id must be a string");
    }
}
