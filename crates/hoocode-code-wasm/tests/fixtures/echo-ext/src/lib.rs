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
            _ => Err(ToolError::UnknownTool(name)),
        }
    }
}
