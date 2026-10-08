use std::time::{Duration, Instant};

use hoocode_agent_mcp::{
    CallOptions, ClientOptions, McpClient, McpError, McpServerConfig, ToolContent, ToolProgress,
    MAX_RESPONSE_BYTES,
};
use serde_json::json;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::support::{start_http_server, stdio_config};

const WAIT: Duration = Duration::from_secs(10);

async fn connect(config: McpServerConfig, options: ClientOptions) -> McpClient {
    McpClient::connect("test", config, options)
        .await
        .expect("connect to the test server")
}

fn text_of(output: &hoocode_agent_mcp::ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text(t) => Some(t.as_str()),
            ToolContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The pid the test server reports in its `echo` text ("<text> pid=<n>").
fn pid_of(output: &hoocode_agent_mcp::ToolOutput) -> String {
    text_of(output)
        .rsplit("pid=")
        .next()
        .unwrap_or_default()
        .to_owned()
}

async fn list_and_call(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    let tools = client.list_tools().await.expect("list tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"echo"), "echo is listed: {names:?}");
    assert!(names.contains(&"progress"), "progress is listed: {names:?}");
    assert_eq!(client.server_name(), "test");

    let output = client
        .call_tool("echo", json!({"text": "hi"}), CallOptions::default())
        .await
        .expect("call echo");
    assert!(!output.is_error);
    assert!(text_of(&output).starts_with("hi pid="), "got {output:?}");
    assert_eq!(output.content.len(), 1);
}

async fn failing_tool_is_an_error_result(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    let output = client
        .call_tool("fail", json!({}), CallOptions::default())
        .await
        .expect("a tool error is still a result");
    assert!(output.is_error);
    assert_eq!(text_of(&output), "it failed");
}

async fn progress_keeps_the_latest(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    let (tx, mut rx) = watch::channel::<Option<ToolProgress>>(None);
    let options = CallOptions {
        progress: Some(tx),
        ..CallOptions::default()
    };
    let output = client
        .call_tool("progress", json!({"steps": 4}), options)
        .await
        .expect("call progress");
    assert_eq!(text_of(&output), "done");

    // The server sends its reports before the result, so the last one is
    // already on its way; wait for it.
    let latest = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(p) = rx.borrow_and_update().clone() {
                if p.progress == 4.0 {
                    return p;
                }
            }
            rx.changed()
                .await
                .expect("sender alive until the call ends");
        }
    })
    .await
    .expect("the latest progress report arrives");
    assert_eq!(latest.total, Some(4.0));
    assert_eq!(latest.message.as_deref(), Some("step 4"));
}

async fn cancel_aborts_the_call(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let result = client
        .call_tool(
            "slow",
            json!({"ms": 60_000}),
            CallOptions {
                cancel,
                ..CallOptions::default()
            },
        )
        .await;
    assert_eq!(result, Err(McpError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel is prompt"
    );

    // The client is still usable after a cancel.
    let output = client
        .call_tool("echo", json!({"text": "again"}), CallOptions::default())
        .await
        .expect("call after cancel");
    assert!(text_of(&output).starts_with("again"));
}

async fn deadline_ends_a_slow_call(config: McpServerConfig) {
    let options = ClientOptions {
        request_timeout: Duration::from_millis(300),
        ..ClientOptions::default()
    };
    let client = connect(config, options).await;
    let started = Instant::now();
    let result = client
        .call_tool("slow", json!({"ms": 60_000}), CallOptions::default())
        .await;
    assert_eq!(result, Err(McpError::Timeout(Duration::from_millis(300))));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "deadline is prompt"
    );
}

async fn in_flight_cap_is_eight(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    client
        .call_tool("reset_peak", json!({}), CallOptions::default())
        .await
        .expect("reset");

    let mut calls = Vec::new();
    for _ in 0..12 {
        let client = client.clone();
        calls.push(tokio::spawn(async move {
            client
                .call_tool("hold", json!({"ms": 400}), CallOptions::default())
                .await
        }));
    }
    for call in calls {
        let output = call.await.expect("task").expect("every call finishes");
        assert_eq!(text_of(&output), "held");
    }

    let peak = client
        .call_tool("peak", json!({}), CallOptions::default())
        .await
        .expect("peak");
    assert_eq!(text_of(&peak), "8", "at most 8 calls ran at once");
}

async fn tools_changed_flag(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    assert!(!client.tools_changed());
    client
        .call_tool("notify_tools_changed", json!({}), CallOptions::default())
        .await
        .expect("call");
    let flagged = tokio::time::timeout(WAIT, async {
        while !client.tools_changed() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(flagged.is_ok(), "the notification sets the flag");
    assert!(client.take_tools_changed(), "take returns the flag");
    assert!(!client.tools_changed(), "take clears the flag");
}

async fn oversized_response_is_an_error_result(config: McpServerConfig) {
    let options = ClientOptions {
        max_response_bytes: 1024,
        ..ClientOptions::default()
    };
    let client = connect(config, options).await;
    let output = client
        .call_tool("big", json!({"bytes": 4096}), CallOptions::default())
        .await
        .expect("an oversized response is a result, not a transport error");
    assert!(output.is_error);
    assert!(
        text_of(&output).contains("exceeds 1024 bytes"),
        "{output:?}"
    );

    let small = client
        .call_tool("big", json!({"bytes": 100}), CallOptions::default())
        .await
        .expect("small response");
    assert!(!small.is_error);
    assert_eq!(text_of(&small).len(), 100);
}

async fn non_object_arguments_are_rejected(config: McpServerConfig) {
    let client = connect(config, ClientOptions::default()).await;
    let result = client
        .call_tool(
            "echo",
            json!(["not", "an", "object"]),
            CallOptions::default(),
        )
        .await;
    assert!(matches!(result, Err(McpError::Config(_))), "{result:?}");
}

#[tokio::test]
async fn stdio_lists_and_calls() {
    list_and_call(stdio_config()).await;
}

#[tokio::test]
async fn http_lists_and_calls() {
    let server = start_http_server().await;
    list_and_call(server.config()).await;
}

#[tokio::test]
async fn stdio_failing_tool_is_an_error_result() {
    failing_tool_is_an_error_result(stdio_config()).await;
}

#[tokio::test]
async fn http_failing_tool_is_an_error_result() {
    let server = start_http_server().await;
    failing_tool_is_an_error_result(server.config()).await;
}

#[tokio::test]
async fn stdio_progress_keeps_the_latest() {
    progress_keeps_the_latest(stdio_config()).await;
}

#[tokio::test]
async fn http_progress_keeps_the_latest() {
    let server = start_http_server().await;
    progress_keeps_the_latest(server.config()).await;
}

#[tokio::test]
async fn stdio_cancel_aborts_the_call() {
    cancel_aborts_the_call(stdio_config()).await;
}

#[tokio::test]
async fn http_cancel_aborts_the_call() {
    let server = start_http_server().await;
    cancel_aborts_the_call(server.config()).await;
}

#[tokio::test]
async fn stdio_deadline_ends_a_slow_call() {
    deadline_ends_a_slow_call(stdio_config()).await;
}

#[tokio::test]
async fn http_deadline_ends_a_slow_call() {
    let server = start_http_server().await;
    deadline_ends_a_slow_call(server.config()).await;
}

#[tokio::test]
async fn stdio_in_flight_cap_is_eight() {
    in_flight_cap_is_eight(stdio_config()).await;
}

#[tokio::test]
async fn http_in_flight_cap_is_eight() {
    let server = start_http_server().await;
    in_flight_cap_is_eight(server.config()).await;
}

#[tokio::test]
async fn stdio_tools_changed_flag() {
    tools_changed_flag(stdio_config()).await;
}

#[tokio::test]
async fn http_tools_changed_flag() {
    let server = start_http_server().await;
    tools_changed_flag(server.config()).await;
}

#[tokio::test]
async fn stdio_oversized_response_is_an_error_result() {
    oversized_response_is_an_error_result(stdio_config()).await;
}

#[tokio::test]
async fn http_oversized_response_is_an_error_result() {
    let server = start_http_server().await;
    oversized_response_is_an_error_result(server.config()).await;
}

#[tokio::test]
async fn stdio_default_cap_is_32_mib() {
    let client = connect(stdio_config(), ClientOptions::default()).await;
    let bytes = MAX_RESPONSE_BYTES + 1;
    let output = client
        .call_tool("big", json!({"bytes": bytes}), CallOptions::default())
        .await
        .expect("result");
    assert!(output.is_error, "a 32 MiB + 1 byte response is refused");
}

#[tokio::test]
async fn stdio_non_object_arguments_are_rejected() {
    non_object_arguments_are_rejected(stdio_config()).await;
}

#[tokio::test]
async fn http_non_object_arguments_are_rejected() {
    let server = start_http_server().await;
    non_object_arguments_are_rejected(server.config()).await;
}

#[tokio::test]
async fn stdio_reconnects_once_after_the_server_exits() {
    let client = connect(stdio_config(), ClientOptions::default()).await;
    let before = client
        .call_tool("echo", json!({"text": "a"}), CallOptions::default())
        .await
        .expect("first call");
    let pid_before = pid_of(&before);

    // The server exits while answering: this call fails.
    let dropped = client
        .call_tool("exit", json!({}), CallOptions::default())
        .await;
    assert!(
        matches!(dropped, Err(McpError::Disconnected(_))),
        "{dropped:?}"
    );

    // The next call starts the server again and succeeds.
    let mut after = None;
    for _ in 0..50 {
        match client
            .call_tool("echo", json!({"text": "b"}), CallOptions::default())
            .await
        {
            Ok(output) => {
                after = Some(output);
                break;
            }
            Err(McpError::Disconnected(_)) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    let after = after.expect("the reconnect succeeded");
    assert!(text_of(&after).starts_with("b pid="));
    assert_ne!(pid_of(&after), pid_before, "a new server process answers");
}

#[tokio::test]
async fn calls_after_shutdown_fail_without_restarting_the_server() {
    let client = connect(stdio_config(), ClientOptions::default()).await;
    client.shutdown().await;
    let result = client
        .call_tool("echo", json!({"text": "x"}), CallOptions::default())
        .await;
    assert!(
        matches!(result, Err(McpError::Disconnected(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_server_that_cannot_start_is_a_connect_error() {
    let config = McpServerConfig::Stdio {
        command: "/nonexistent/hoocode-mcp-server".to_owned(),
        args: Vec::new(),
        env: Default::default(),
        cwd: None,
    };
    let result = McpClient::connect("missing", config, ClientOptions::default()).await;
    assert!(matches!(result, Err(McpError::Connect(_))), "{result:?}");
}

#[tokio::test]
async fn an_invalid_header_is_a_config_error() {
    let server = start_http_server().await;
    let config = McpServerConfig::Http {
        url: server.url.clone(),
        headers: [("bad header".to_owned(), "x".to_owned())].into(),
    };
    let result = McpClient::connect("headers", config, ClientOptions::default()).await;
    assert!(matches!(result, Err(McpError::Config(_))), "{result:?}");
}
