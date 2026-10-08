//! A small MCP server for the client tests. The same code runs over stdio
//! (`examples/mcp_test_server.rs`) and in-process over Streamable HTTP
//! (`support/mod.rs`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ElicitRequestParams,
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Map, Value};

/// Tool calls running right now (for the in-flight cap test).
pub static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// The most tool calls that ran at once since the last `reset_peak`.
pub static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The test server. `allow_exit` lets the `exit` tool end the process; only
/// the stdio binary sets it, because the HTTP server shares the test process.
#[derive(Clone)]
pub struct TestServer {
    pub allow_exit: bool,
}

fn schema(value: Value) -> Arc<Map<String, Value>> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => unreachable!("schemas are objects"),
    }
}

fn tool(name: &'static str, description: &'static str) -> Tool {
    Tool::new(name, description, schema(json!({"type": "object"})))
}

fn arg_u64(args: Option<&Map<String, Value>>, key: &str, default: u64) -> u64 {
    args.and_then(|a| a.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(default)
}

fn arg_str<'a>(args: Option<&'a Map<String, Value>>, key: &str) -> &'a str {
    args.and_then(|a| a.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            tool("echo", "Returns its text argument and the server pid."),
            tool("slow", "Sleeps for ms milliseconds."),
            tool(
                "hold",
                "Sleeps for ms milliseconds and counts as in flight.",
            ),
            tool("peak", "Returns the most calls that ran at once."),
            tool("reset_peak", "Resets the in-flight peak."),
            tool(
                "progress",
                "Sends `steps` progress notifications, then returns.",
            ),
            tool("big", "Returns a text of `bytes` bytes."),
            tool("fail", "Returns a tool error (isError)."),
            tool("exit", "Ends the server process (stdio only)."),
            tool(
                "elicit",
                "Asks the client for input (elicitation/create) and reports the answer. `mode` is form (default) or url.",
            ),
            tool(
                "notify_tools_changed",
                "Sends notifications/tools/list_changed.",
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.as_ref();
        let result = match request.name.as_ref() {
            "echo" => {
                let text = arg_str(args, "text");
                CallToolResult::success(vec![ContentBlock::text(format!(
                    "{text} pid={}",
                    std::process::id()
                ))])
            }
            "slow" => {
                tokio::time::sleep(Duration::from_millis(arg_u64(args, "ms", 0))).await;
                CallToolResult::success(vec![ContentBlock::text("slept")])
            }
            "hold" => {
                let now = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
                PEAK.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(arg_u64(args, "ms", 0))).await;
                IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
                CallToolResult::success(vec![ContentBlock::text("held")])
            }
            "peak" => CallToolResult::success(vec![ContentBlock::text(
                PEAK.load(Ordering::SeqCst).to_string(),
            )]),
            "reset_peak" => {
                PEAK.store(0, Ordering::SeqCst);
                CallToolResult::success(vec![ContentBlock::text("reset")])
            }
            "progress" => {
                let steps = arg_u64(args, "steps", 3);
                let token = context.meta.get_progress_token();
                for step in 1..=steps {
                    if let Some(token) = token.clone() {
                        let mut note = ProgressNotificationParam::new(token, step as f64);
                        note.total = Some(steps as f64);
                        note.message = Some(format!("step {step}"));
                        let _ = context.peer.notify_progress(note).await;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                CallToolResult::success(vec![ContentBlock::text("done")])
            }
            "big" => {
                let bytes = arg_u64(args, "bytes", 0) as usize;
                CallToolResult::success(vec![ContentBlock::text("x".repeat(bytes))])
            }
            "fail" => CallToolResult::error(vec![ContentBlock::text("it failed")]),
            "exit" if self.allow_exit => std::process::exit(3),
            "elicit" => {
                let params = if arg_str(args, "mode") == "url" {
                    json!({
                        "mode": "url",
                        "message": "Consent to the request",
                        "url": "https://example.test/consent",
                        "elicitationId": "consent-1",
                    })
                } else {
                    json!({
                        "mode": "form",
                        "message": "Pick a colour",
                        "requestedSchema": {
                            "type": "object",
                            "properties": {
                                "colour": {"type": "string", "enum": ["red", "blue"]}
                            },
                            "required": ["colour"],
                        },
                    })
                };
                let params: ElicitRequestParams =
                    serde_json::from_value(params).expect("elicitation params");
                match context.peer.create_elicitation(params).await {
                    Ok(answer) => CallToolResult::success(vec![ContentBlock::text(format!(
                        "action={:?} content={}",
                        answer.action,
                        answer.content.map(|c| c.to_string()).unwrap_or_default()
                    ))]),
                    Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                        "elicitation failed: {error}"
                    ))]),
                }
            }
            "notify_tools_changed" => {
                let _ = context.peer.notify_tool_list_changed().await;
                CallToolResult::success(vec![ContentBlock::text("notified")])
            }
            other => {
                CallToolResult::error(vec![ContentBlock::text(format!("unknown tool {other}"))])
            }
        };
        Ok(result.into())
    }
}
