//! A loaded extension: its validated tool list and the per-call path.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use wasmtime::Store;

use crate::error::WasmError;
use crate::guest::{self, GuestToolInfo, State, ToolError};
use crate::{AbortSignal, CallEnv, Inner};

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
        let mut store =
            guest::new_store(&self.host.engine, &self.plugin, &self.data_dir, env.clone())?;
        guest::set_deadline(
            &mut store,
            self.call_timeout,
            env.abort.clone(),
            reason.clone(),
        );

        match self.run_call(&mut store, tool, &args_json).await {
            Err(err) => Err(classify(err, &reason)),
            Ok(Err(tool_error)) => Err(map_tool_error(tool_error)),
            Ok(Ok(text)) => {
                serde_json::from_str(&text).map_err(|e| WasmError::BadResult(e.to_string()))
            }
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
        )?;
        guest::set_deadline(&mut store, self.call_timeout, None, reason.clone());
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

/// Check the guest's tool list: names of `[a-z0-9_-]`, no duplicates, and each
/// schema is JSON text for an object.
fn validate_tools(raw: Vec<GuestToolInfo>) -> Result<Vec<ToolInfo>, WasmError> {
    let mut seen = HashSet::new();
    let mut tools = Vec::with_capacity(raw.len());
    for tool in raw {
        let valid_name = !tool.name.is_empty()
            && tool
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !valid_name {
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
        ])
        .unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[1].input_schema["type"], "object");
    }

    #[test]
    fn rejects_bad_names() {
        for name in ["", "Echo", "a b", "a.b", "é"] {
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
