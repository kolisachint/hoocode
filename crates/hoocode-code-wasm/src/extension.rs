//! A loaded extension: its validated tool list and the per-call path.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_agent_types::valid_plugin_tool_name;
use serde_json::Value;
use wasmtime::Store;

use crate::error::WasmError;
use crate::guest::{self, GuestToolInfo, State, ToolError};
use crate::http::PauseClock;
use crate::{AbortSignal, CallEnv, Inner};

/// How often the wall-clock timer re-reads the paused time.
const WALL_TICK: Duration = Duration::from_millis(50);

/// One tool a plugin offers, as listed at load.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    /// Short name, without the host's `ext_<plugin>_` prefix.
    pub name: String,
    /// One sentence the model reads to decide when to call the tool.
    pub description: String,
    /// The JSON Schema object for the call arguments.
    pub input_schema: Value,
}

/// A loaded extension. Each `call` runs in a fresh store and instance.
pub struct Extension {
    /// Keeps the engine and its epoch ticker alive while the extension exists.
    pub(crate) host: Arc<Inner>,
    pub(crate) pre: guest::ExtensionPre<State>,
    pub(crate) plugin: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) call_timeout: Duration,
    pub(crate) allow_loopback: bool,
    pub(crate) tls: Arc<rustls::ClientConfig>,
    pub(crate) tools: Vec<ToolInfo>,
}

impl Extension {
    /// The plugin name this extension was loaded under.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The tools this extension offers.
    pub fn tools(&self) -> &[ToolInfo] {
        &self.tools
    }

    /// Run one tool with `args` (a JSON value matching its input schema).
    ///
    /// Three things end the call early, whichever comes first:
    /// - the guest trapping at an epoch deadline (pure CPU work that never
    ///   yields), which gives `Timeout` or `Cancelled` through `classify`;
    /// - the wall-clock timer, which also covers a guest blocked in a host
    ///   await such as a stalled response body;
    /// - the abort signal, which covers the same host awaits.
    ///
    /// Time spent waiting on the network gate is not counted by either timer.
    /// Dropping the call future drops the store, which is fine: every call has
    /// its own store.
    pub async fn call(&self, tool: &str, args: Value, env: CallEnv) -> Result<Value, WasmError> {
        if !self.tools.iter().any(|t| t.name == tool) {
            return Err(WasmError::UnknownTool(tool.to_owned()));
        }
        if env.abort.as_ref().is_some_and(AbortSignal::aborted) {
            return Err(WasmError::Cancelled);
        }
        let args_json =
            serde_json::to_string(&args).map_err(|e| WasmError::InvalidArgs(e.to_string()))?;
        let reason = Arc::new(AtomicU8::new(guest::REASON_NONE));
        let clock = Arc::new(PauseClock::default());
        let mut store = guest::new_store(
            &self.host.engine,
            &self.plugin,
            &self.data_dir,
            env.clone(),
            self.allow_loopback,
            self.tls.clone(),
            clock.clone(),
        )?;
        guest::set_deadline(
            &mut store,
            self.call_timeout,
            env.abort.clone(),
            reason.clone(),
            clock.clone(),
        );

        tokio::select! {
            result = self.run_call(&mut store, tool, &args_json) => match result {
                Err(err) => Err(classify(err, &reason)),
                Ok(Err(tool_error)) => Err(map_tool_error(tool_error)),
                Ok(Ok(text)) => {
                    serde_json::from_str(&text).map_err(|e| WasmError::BadResult(e.to_string()))
                }
            },
            () = wall_clock(self.call_timeout, &clock) => Err(WasmError::Timeout),
            () = aborted(env.abort.as_ref()) => Err(WasmError::Cancelled),
        }
    }

    async fn run_call(
        &self,
        store: &mut Store<State>,
        tool: &str,
        args_json: &str,
    ) -> wasmtime::Result<Result<String, ToolError>> {
        let bindings = self.pre.instantiate_async(&mut *store).await?;
        bindings
            .hoocode_extension_tool()
            .call_call(&mut *store, tool, args_json)
            .await
    }

    /// Ask the guest for its tools and check them. Used once, at load.
    pub(crate) async fn discover_tools(&self) -> Result<Vec<ToolInfo>, WasmError> {
        let reason = Arc::new(AtomicU8::new(guest::REASON_NONE));
        let mut store = guest::new_store(
            &self.host.engine,
            &self.plugin,
            &self.data_dir,
            CallEnv::for_discovery(),
            self.allow_loopback,
            self.tls.clone(),
            Arc::new(PauseClock::default()),
        )?;
        guest::set_deadline(
            &mut store,
            self.call_timeout,
            None,
            reason.clone(),
            Arc::new(PauseClock::default()),
        );
        let raw = async {
            let bindings = self.pre.instantiate_async(&mut store).await?;
            bindings
                .hoocode_extension_tool()
                .call_list_tools(&mut store)
                .await
        }
        .await
        .map_err(|err| classify(err, &reason))?;
        validate_tools(raw)
    }
}

/// Completes once `timeout` of call time has passed, not counting gate waits.
/// Re-checks every `WALL_TICK`, so a gate wait that is still going on is
/// subtracted too.
async fn wall_clock(timeout: Duration, clock: &PauseClock) {
    let started = Instant::now();
    loop {
        let active = started.elapsed().saturating_sub(clock.paused());
        if active >= timeout {
            return;
        }
        tokio::time::sleep((timeout - active).min(WALL_TICK)).await;
    }
}

/// Completes when `abort` is set. Without a signal it never completes.
async fn aborted(abort: Option<&AbortSignal>) {
    match abort {
        Some(signal) => signal.cancelled().await,
        None => std::future::pending().await,
    }
}

fn classify(err: wasmtime::Error, reason: &AtomicU8) -> WasmError {
    match reason.load(Ordering::Relaxed) {
        guest::REASON_TIMEOUT => WasmError::Timeout,
        guest::REASON_CANCELLED => WasmError::Cancelled,
        _ => WasmError::Trap(err.to_string()),
    }
}

fn map_tool_error(err: ToolError) -> WasmError {
    match err {
        ToolError::InvalidArgs(msg) => WasmError::InvalidArgs(msg),
        ToolError::UnknownTool(name) => WasmError::UnknownTool(name),
        ToolError::Failed(msg) => WasmError::Failed(msg),
    }
}

/// Check the guest's tool list: names that `valid_plugin_tool_name` accepts, no duplicates,
/// and each schema is JSON text for an object.
fn validate_tools(raw: Vec<GuestToolInfo>) -> Result<Vec<ToolInfo>, WasmError> {
    let mut seen = HashSet::new();
    let mut tools = Vec::with_capacity(raw.len());
    for tool in raw {
        if !valid_plugin_tool_name(&tool.name) {
            return Err(WasmError::InvalidTools(format!(
                "bad tool name {:?}",
                tool.name
            )));
        }
        if !seen.insert(tool.name.clone()) {
            return Err(WasmError::InvalidTools(format!(
                "duplicate tool name {:?}",
                tool.name
            )));
        }
        let input_schema: Value = serde_json::from_str(&tool.input_schema_json).map_err(|e| {
            WasmError::InvalidTools(format!("{}: schema is not JSON: {e}", tool.name))
        })?;
        if !input_schema.is_object() {
            return Err(WasmError::InvalidTools(format!(
                "{}: schema is not a JSON object",
                tool.name
            )));
        }
        tools.push(ToolInfo {
            name: tool.name,
            description: tool.description,
            input_schema,
        });
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, schema: &str) -> GuestToolInfo {
        GuestToolInfo {
            name: name.to_owned(),
            description: "d".to_owned(),
            input_schema_json: schema.to_owned(),
        }
    }

    #[test]
    fn accepts_plain_names_and_object_schemas() {
        let tools = validate_tools(vec![
            raw("echo", "{}"),
            raw("web-search_2", r#"{"type":"object"}"#),
            raw("WebFetch", "{}"),
        ])
        .unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[2].name, "WebFetch");
        assert_eq!(tools[1].input_schema["type"], "object");
    }

    #[test]
    fn rejects_bad_names() {
        let long = "a".repeat(65);
        for name in ["", "1st", "_x", "a b", "a.b", "é", long.as_str()] {
            let err = validate_tools(vec![raw(name, "{}")]).unwrap_err();
            assert!(
                matches!(err, WasmError::InvalidTools(_)),
                "{name:?}: {err:?}"
            );
        }
    }

    #[test]
    fn rejects_duplicates_and_non_object_schemas() {
        let dup = validate_tools(vec![raw("a", "{}"), raw("a", "{}")]).unwrap_err();
        assert!(
            matches!(dup, WasmError::InvalidTools(ref m) if m.contains("duplicate")),
            "{dup:?}"
        );
        let not_object = validate_tools(vec![raw("a", "[1]")]).unwrap_err();
        assert!(
            matches!(not_object, WasmError::InvalidTools(_)),
            "{not_object:?}"
        );
        let not_json = validate_tools(vec![raw("a", "{")]).unwrap_err();
        assert!(
            matches!(not_json, WasmError::InvalidTools(_)),
            "{not_json:?}"
        );
    }
}
