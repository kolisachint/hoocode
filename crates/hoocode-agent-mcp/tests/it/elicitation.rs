//! Elicitation round trips: the test server asks for input during a tool call, a scripted
//! handler answers, and the tool reports what the server received back.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoocode_agent_mcp::{
    CallOptions, ClientOptions, ElicitationAnswer, ElicitationHandler, ElicitationRequest,
    McpClient, McpServerConfig, ToolContent, ToolOutput,
};
use serde_json::json;

use crate::support::{start_http_server, stdio_config};

/// Answers from a script (declining once it runs out) and records what it was asked.
#[derive(Default)]
struct Scripted {
    answers: Mutex<VecDeque<ElicitationAnswer>>,
    seen: Mutex<Vec<(String, ElicitationRequest)>>,
}

impl Scripted {
    fn with(answers: Vec<ElicitationAnswer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            seen: Mutex::default(),
        })
    }

    fn asked(&self) -> Vec<(String, ElicitationRequest)> {
        self.seen.lock().unwrap().clone()
    }
}

impl ElicitationHandler for Scripted {
    fn elicit<'a>(
        &'a self,
        server: &'a str,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push((server.to_owned(), request));
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(ElicitationAnswer::Decline)
        })
    }
}

fn options() -> ClientOptions {
    ClientOptions {
        request_timeout: Duration::from_secs(10),
        ..Default::default()
    }
}

fn text_of(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|block| match block {
            ToolContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Calls the test server's `elicit` tool in `mode` and returns the tool's text.
async fn ask(client: &McpClient, mode: &str) -> String {
    let output = client
        .call_tool("elicit", json!({ "mode": mode }), CallOptions::default())
        .await
        .expect("the call completes");
    assert!(!output.is_error, "the tool ran: {}", text_of(&output));
    text_of(&output)
}

async fn connect_with(config: McpServerConfig, handler: Arc<Scripted>) -> McpClient {
    McpClient::connect_with_elicitation("test", config, options(), handler)
        .await
        .expect("connect to the test server")
}

async fn form_accept(config: McpServerConfig) {
    let handler = Scripted::with(vec![ElicitationAnswer::Accept(Some(
        json!({"colour": "blue"}),
    ))]);
    let client = connect_with(config, handler.clone()).await;

    assert_eq!(
        ask(&client, "form").await,
        r#"action=Accept content={"colour":"blue"}"#
    );

    let asked = handler.asked();
    assert_eq!(asked.len(), 1, "one question: {asked:?}");
    let (server, request) = &asked[0];
    assert_eq!(server, "test");
    let ElicitationRequest::Form { message, schema } = request else {
        panic!("expected a form question, got {request:?}");
    };
    assert_eq!(message, "Pick a colour");
    assert_eq!(
        schema["properties"]["colour"]["enum"],
        json!(["red", "blue"])
    );
    client.shutdown().await;
}

async fn form_decline_and_cancel(config: McpServerConfig) {
    let handler = Scripted::with(vec![ElicitationAnswer::Decline, ElicitationAnswer::Cancel]);
    let client = connect_with(config, handler.clone()).await;

    assert_eq!(ask(&client, "form").await, "action=Decline content=");
    assert_eq!(ask(&client, "form").await, "action=Cancel content=");
    // The session is still usable after the answers.
    let echo = client
        .call_tool("echo", json!({"text": "after"}), CallOptions::default())
        .await
        .expect("echo after elicitation");
    assert!(text_of(&echo).starts_with("after pid="));
    client.shutdown().await;
}

async fn url_mode_carries_no_content(config: McpServerConfig) {
    let handler = Scripted::with(vec![ElicitationAnswer::Accept(None)]);
    let client = connect_with(config, handler.clone()).await;

    assert_eq!(ask(&client, "url").await, "action=Accept content=");

    let asked = handler.asked();
    let ElicitationRequest::Url {
        message,
        url,
        elicitation_id,
    } = &asked[0].1
    else {
        panic!("expected a URL question, got {asked:?}");
    };
    assert_eq!(message, "Consent to the request");
    assert_eq!(url, "https://example.test/consent");
    assert_eq!(elicitation_id, "consent-1");
    client.shutdown().await;
}

/// Without a handler (print, rpc, app-server) every request is declined.
async fn without_a_handler_the_request_is_declined(config: McpServerConfig) {
    let client = McpClient::connect("test", config, options())
        .await
        .expect("connect to the test server");
    assert_eq!(ask(&client, "form").await, "action=Decline content=");
    client.shutdown().await;
}

#[tokio::test]
async fn http_form_accept_round_trip() {
    let server = start_http_server().await;
    form_accept(server.config()).await;
}

#[tokio::test]
async fn stdio_form_accept_round_trip() {
    form_accept(stdio_config()).await;
}

#[tokio::test]
async fn http_form_decline_and_cancel() {
    let server = start_http_server().await;
    form_decline_and_cancel(server.config()).await;
}

#[tokio::test]
async fn stdio_form_decline_and_cancel() {
    form_decline_and_cancel(stdio_config()).await;
}

#[tokio::test]
async fn http_url_mode_carries_no_content() {
    let server = start_http_server().await;
    url_mode_carries_no_content(server.config()).await;
}

#[tokio::test]
async fn stdio_url_mode_carries_no_content() {
    url_mode_carries_no_content(stdio_config()).await;
}

#[tokio::test]
async fn http_without_a_handler_declines() {
    let server = start_http_server().await;
    without_a_handler_the_request_is_declined(server.config()).await;
}

#[tokio::test]
async fn stdio_without_a_handler_declines() {
    without_a_handler_the_request_is_declined(stdio_config()).await;
}
