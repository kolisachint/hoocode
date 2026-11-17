//! Test fixture: an extension component for hoocode-code-wasm's tests.
//! Built by scripts/dev/build_wasm_fixtures.sh; not shipped.

wit_bindgen::generate!({
    path: "../../../../../wit",
    world: "extension",
});

use exports::hoocode::extension::tool::Guest;
use hoocode::extension::host::{self, LogLevel};
use hoocode::extension::types::{ToolError, ToolInfo};
use serde_json::{Value, json};
use wasip2::http::outgoing_handler;
use wasip2::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
use wasip2::io::streams::StreamError;

/// The most body text `fetch` returns.
const FETCH_BODY_CAP: usize = 64 << 10;

struct Echo;

export!(Echo);

fn parse_args(args_json: &str) -> Result<Value, ToolError> {
    serde_json::from_str(args_json).map_err(|e| ToolError::InvalidArgs(e.to_string()))
}

fn tool(name: &str, description: &str, schema: Value) -> ToolInfo {
    ToolInfo {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema_json: schema.to_string(),
    }
}

impl Guest for Echo {
    fn list_tools() -> Vec<ToolInfo> {
        let any = json!({"type": "object"});
        let url = json!({
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"]
        });
        let key = json!({
            "type": "object",
            "properties": {"key": {"type": "string"}},
            "required": ["key"]
        });
        vec![
            tool("echo", "Returns the arguments unchanged.", any.clone()),
            tool("config", "Returns one of the plugin's settings.", key),
            tool("spin", "Loops forever (timeout test).", any.clone()),
            tool("alloc", "Allocates 100 MiB (memory limit test).", any.clone()),
            tool("fail", "Always fails with a message.", any.clone()),
            tool("write-data", "Writes and reads back /data/x.txt.", any),
            tool("fetch", "Does one GET of a URL over wasi:http.", url),
        ]
    }

    fn call(name: String, args_json: String) -> Result<String, ToolError> {
        match name.as_str() {
            "echo" => {
                parse_args(&args_json)?;
                host::log(LogLevel::Info, "echo called");
                Ok(args_json)
            }
            "config" => {
                let args = parse_args(&args_json)?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| ToolError::InvalidArgs("missing key".to_owned()))?;
                Ok(json!({ "value": host::config_get(key) }).to_string())
            }
            "spin" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "alloc" => {
                let big = vec![1u8; 100 << 20];
                std::hint::black_box(&big);
                Ok(json!({ "bytes": big.len() }).to_string())
            }
            "fail" => Err(ToolError::Failed("the fixture fails on purpose".to_owned())),
            "write-data" => {
                std::fs::write("/data/x.txt", "hello from the fixture")
                    .map_err(|e| ToolError::Failed(e.to_string()))?;
                let text = std::fs::read_to_string("/data/x.txt")
                    .map_err(|e| ToolError::Failed(e.to_string()))?;
                Ok(json!({ "read": text }).to_string())
            }
            "fetch" => {
                let args = parse_args(&args_json)?;
                let url = args["url"]
                    .as_str()
                    .ok_or_else(|| ToolError::InvalidArgs("missing url".to_owned()))?;
                fetch(url).map(|v| v.to_string()).map_err(ToolError::Failed)
            }
            _ => Err(ToolError::UnknownTool(name)),
        }
    }
}

/// One GET over `wasi:http`. Returns `{status, body}`; the body is cut at
/// `FETCH_BODY_CAP` bytes.
fn fetch(url: &str) -> Result<Value, String> {
    // Other schemes go to the host as-is, so the host's own check refuses them.
    let (scheme, rest) = match url.split_once("://") {
        Some(("http", rest)) => (Scheme::Http, rest),
        Some(("https", rest)) => (Scheme::Https, rest),
        Some((other, rest)) => (Scheme::Other(other.to_owned()), rest),
        None => return Err("url has no scheme".to_owned()),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    let request = OutgoingRequest::new(Fields::new());
    request
        .set_method(&Method::Get)
        .and_then(|()| request.set_scheme(Some(&scheme)))
        .and_then(|()| request.set_authority(Some(authority)))
        .and_then(|()| request.set_path_with_query(Some(path)))
        .map_err(|()| "cannot build the request".to_owned())?;
    let body = request.body().map_err(|()| "no request body".to_owned())?;
    OutgoingBody::finish(body, None).map_err(|e| format!("{e:?}"))?;

    let future = outgoing_handler::handle(request, None).map_err(|e| format!("{e:?}"))?;
    future.subscribe().block();
    let response = future
        .get()
        .ok_or("no response")?
        .map_err(|()| "response already taken".to_owned())?
        .map_err(|e| format!("{e:?}"))?;
    let status = response.status();

    let incoming = response.consume().map_err(|()| "no response body".to_owned())?;
    let stream = incoming.stream().map_err(|()| "no body stream".to_owned())?;
    let mut body = Vec::new();
    while body.len() < FETCH_BODY_CAP {
        match stream.blocking_read(16 << 10) {
            Ok(chunk) => body.extend_from_slice(&chunk),
            Err(StreamError::Closed) => break,
            Err(StreamError::LastOperationFailed(e)) => return Err(e.to_debug_string()),
        }
    }
    body.truncate(FETCH_BODY_CAP);
    drop(stream);
    drop(incoming);
    Ok(json!({ "status": status, "body": String::from_utf8_lossy(&body) }))
}
