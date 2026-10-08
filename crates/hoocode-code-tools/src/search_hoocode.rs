//! The `DocSearch` tool (`extensions/core/self-knowledge.ts` in hoocode-ts).
//!
//! It searches what this session can do: loaded skills and subagents, and
//! installed plugins once they load. Docs are not indexed here yet (the Rust
//! build ships none), so the description's doc wording is the TS text kept
//! verbatim for the side-by-side check. See docs/design/semantic-search.md.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoocode_agent_types::{AgentToolResult, ToolExecutionMode};
use hoocode_ai_types::AbortSignal;
use hoocode_code_capabilities::{load_index, CapabilityIndex};
use hoocode_code_tool_api::ToolDefinition;
use serde_json::{json, Value};

use crate::text_result;

/// The tool's name, as hoocode-ts registers it.
pub const DOC_SEARCH_TOOL_NAME: &str = "DocSearch";

const DEFAULT_LIMIT: f64 = 8.0;
const MAX_LIMIT: f64 = 25.0;

/// The description hoocode-ts sends (`self-knowledge.ts`), verbatim.
pub const SEARCH_HOOCODE_DESCRIPTION: &str = "Search hoocode's own documentation and the capabilities loaded in this session (skills, slash commands, subagents, installed plugins). Use it when the user asks what hoocode can do, how one of its features works, or how to configure or extend it. Doc results come back as a file path and line number — read that range for the answer rather than replying from the excerpt alone. MCP tools are not covered here; find those with ResolveMcpTools.";

/// The one-line summary for the system prompt's tool list (`promptSnippet`).
pub const SEARCH_HOOCODE_PROMPT_SNIPPET: &str =
    "Search hoocode's own docs and this session's capabilities by describing what you need.";

/// The guideline added to the system prompt (`promptGuidelines`).
pub const SEARCH_HOOCODE_PROMPT_GUIDELINE: &str = "For questions about hoocode itself — its features, configuration, or how to extend it — use DocSearch and read the section it points at instead of answering from memory.";

const QUERY_DESCRIPTION: &str = "What you want to know about hoocode, in your own words — 'how do I write an extension', 'where are sessions stored', 'can it run subagents'.";
const LIMIT_DESCRIPTION: &str = "Maximum results. Default 8.";
const LEXICAL_ONLY_NOTE: &str = "(Lexical match only — try naming the feature if this missed.)";

/// The TypeBox schema hoocode-ts sends: `query` (required), `limit` (optional),
/// no extra keys.
pub fn search_hoocode_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": QUERY_DESCRIPTION},
            "limit": {"type": "number", "description": LIMIT_DESCRIPTION}
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

/// Supplies the index at call time. Without one, the tool loads skills and
/// subagents from disk for its cwd on each call.
pub type IndexProvider = Arc<dyn Fn() -> CapabilityIndex + Send + Sync>;

/// Options for [`create_search_hoocode_tool_definition`].
#[derive(Clone, Default)]
pub struct SearchHooCodeOptions {
    pub index: Option<IndexProvider>,
}

/// One result line, as hoocode-ts prints it: `- [kind] name — description`.
fn describe(kind: &str, name: &str, description: &str) -> String {
    let summary = if description.is_empty() {
        String::new()
    } else {
        format!(" — {description}")
    };
    format!("- [{kind}] {name}{summary}")
}

/// `createSearchHooCodeTool`'s definition.
pub fn create_search_hoocode_tool_definition(
    cwd: impl Into<PathBuf>,
    options: SearchHooCodeOptions,
) -> ToolDefinition {
    let cwd: PathBuf = cwd.into();
    ToolDefinition {
        name: DOC_SEARCH_TOOL_NAME.into(),
        label: DOC_SEARCH_TOOL_NAME.into(),
        description: SEARCH_HOOCODE_DESCRIPTION.into(),
        prompt_snippet: Some(SEARCH_HOOCODE_PROMPT_SNIPPET.into()),
        prompt_guidelines: vec![SEARCH_HOOCODE_PROMPT_GUIDELINE.into()],
        parameters: search_hoocode_parameters_schema(),
        prepare_arguments: None,
        execution_mode: Some(ToolExecutionMode::Parallel),
        background: false,
        background_when: None,
        ordered_start: false,
        execute: Arc::new(move |_id, args: Value, signal, _on_update, _ctx| {
            run_search(&cwd, &options, args, signal)
        }),
    }
}

fn run_search(
    cwd: &Path,
    options: &SearchHooCodeOptions,
    args: Value,
    _signal: Option<AbortSignal>,
) -> Result<AgentToolResult, hoocode_code_tool_api::ToolError> {
    let query = match args.get("query") {
        Some(Value::String(q)) => q.trim().to_string(),
        _ => return Err("The \"query\" argument must be of type string".into()),
    };
    if query.is_empty() {
        return Ok(text_result("Provide a query."));
    }
    // Math.min(MAX, Math.max(1, Math.trunc(limit ?? DEFAULT)))
    let requested = args
        .get("limit")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_LIMIT);
    let limit = requested.trunc().clamp(1.0, MAX_LIMIT) as usize;

    let index = match &options.index {
        Some(provider) => provider(),
        None => load_index(&cwd.to_string_lossy()),
    };
    let hits = index.search(&query, limit);
    if hits.is_empty() {
        return Ok(text_result(format!(
            "Nothing matched \"{query}\". The docs are listed in the system prompt under \"About hoocode itself\" — read the most likely file directly."
        )));
    }
    let body: Vec<String> = hits
        .iter()
        .map(|e| describe(e.kind.as_str(), &e.name, &e.description))
        .collect();
    // Only the lexical leg runs in the Rust build, so the note always applies.
    Ok(text_result(format!(
        "Results for \"{query}\":\n{}\n{LEXICAL_ONLY_NOTE}",
        body.join("\n")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_ai_types::Content;
    use hoocode_code_capabilities::{CapabilityEntry, CapabilityKind};

    fn entry(kind: CapabilityKind, name: &str, description: &str) -> CapabilityEntry {
        CapabilityEntry {
            kind,
            name: name.into(),
            description: description.into(),
            source: "test".into(),
        }
    }

    fn tool_with(entries: Vec<CapabilityEntry>) -> ToolDefinition {
        let index = CapabilityIndex::new(entries);
        let provider: IndexProvider = Arc::new(move || index.clone());
        create_search_hoocode_tool_definition(
            std::env::temp_dir(),
            SearchHooCodeOptions {
                index: Some(provider),
            },
        )
    }

    fn call(tool: &ToolDefinition, args: Value) -> Result<String, String> {
        match (tool.execute)("call-1".into(), args, None, None, None) {
            Ok(result) => match result.content.first() {
                Some(Content::Text(t)) => Ok(t.text.clone()),
                other => panic!("expected text content, got {other:?}"),
            },
            Err(e) => Err(e.to_string()),
        }
    }

    #[test]
    fn name_description_and_prompt_text_match_hoocode_ts() {
        let tool = create_search_hoocode_tool_definition(".", SearchHooCodeOptions::default());
        assert_eq!(tool.name, "DocSearch");
        assert_eq!(tool.label, "DocSearch");
        assert_eq!(tool.description, SEARCH_HOOCODE_DESCRIPTION);
        assert!(tool.description.starts_with("Search hoocode's own documentation and the capabilities loaded in this session (skills, slash commands, subagents, installed plugins)."));
        assert!(tool
            .description
            .ends_with("MCP tools are not covered here; find those with ResolveMcpTools."));
        assert_eq!(
            tool.prompt_snippet.as_deref(),
            Some("Search hoocode's own docs and this session's capabilities by describing what you need.")
        );
        assert_eq!(tool.prompt_guidelines, [SEARCH_HOOCODE_PROMPT_GUIDELINE]);
        assert_eq!(tool.execution_mode, Some(ToolExecutionMode::Parallel));
    }

    #[test]
    fn schema_matches_the_typebox_schema_in_hoocode_ts() {
        let schema = search_hoocode_parameters_schema();
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What you want to know about hoocode, in your own words — 'how do I write an extension', 'where are sessions stored', 'can it run subagents'."
                    },
                    "limit": {"type": "number", "description": "Maximum results. Default 8."}
                },
                "required": ["query"],
                "additionalProperties": false
            })
        );
        let tool = create_search_hoocode_tool_definition(".", SearchHooCodeOptions::default());
        assert_eq!(tool.parameters, schema);
    }

    #[test]
    fn results_are_listed_with_kind_name_and_description() {
        let tool = tool_with(vec![
            entry(CapabilityKind::Skill, "pdf", "Read PDF files."),
            entry(
                CapabilityKind::Subagent,
                "explore",
                "Find code in the repo.",
            ),
        ]);
        let text = call(&tool, json!({"query": "pdf"})).unwrap();
        assert_eq!(
            text,
            "Results for \"pdf\":\n- [skill] pdf — Read PDF files.\n(Lexical match only — try naming the feature if this missed.)"
        );
    }

    #[test]
    fn a_result_without_a_description_has_no_dash() {
        let tool = tool_with(vec![entry(CapabilityKind::Skill, "pdf", "")]);
        let text = call(&tool, json!({"query": "pdf"})).unwrap();
        assert!(
            text.starts_with("Results for \"pdf\":\n- [skill] pdf\n"),
            "{text}"
        );
    }

    #[test]
    fn the_query_is_trimmed_before_it_is_echoed() {
        let tool = tool_with(vec![entry(CapabilityKind::Skill, "pdf", "")]);
        let text = call(&tool, json!({"query": "  pdf  "})).unwrap();
        assert!(text.starts_with("Results for \"pdf\":"), "{text}");
    }

    #[test]
    fn an_empty_query_asks_for_one() {
        let tool = tool_with(vec![entry(CapabilityKind::Skill, "pdf", "")]);
        assert_eq!(
            call(&tool, json!({"query": "   "})).unwrap(),
            "Provide a query."
        );
    }

    #[test]
    fn a_missing_query_is_an_error() {
        let tool = tool_with(vec![]);
        assert!(call(&tool, json!({})).is_err());
    }

    #[test]
    fn no_match_says_so() {
        let tool = tool_with(vec![entry(CapabilityKind::Skill, "pdf", "PDF files")]);
        let text = call(&tool, json!({"query": "kubernetes"})).unwrap();
        assert!(
            text.starts_with("Nothing matched \"kubernetes\"."),
            "{text}"
        );
    }

    #[test]
    fn limit_defaults_to_eight_and_is_clamped() {
        let entries: Vec<CapabilityEntry> = (0..30)
            .map(|i| entry(CapabilityKind::Skill, &format!("review-{i:02}"), "review"))
            .collect();
        let tool = tool_with(entries);

        let count = |text: &str| text.lines().filter(|l| l.starts_with("- [")).count();
        assert_eq!(count(&call(&tool, json!({"query": "review"})).unwrap()), 8);
        assert_eq!(
            count(&call(&tool, json!({"query": "review", "limit": 3})).unwrap()),
            3
        );
        assert_eq!(
            count(&call(&tool, json!({"query": "review", "limit": 0})).unwrap()),
            1
        );
        assert_eq!(
            count(&call(&tool, json!({"query": "review", "limit": 2.9})).unwrap()),
            2
        );
        assert_eq!(
            count(&call(&tool, json!({"query": "review", "limit": 500})).unwrap()),
            25
        );
    }

    #[test]
    fn the_default_bundle_leaves_doc_search_to_the_cli() {
        let dir = std::env::temp_dir();
        let defs = crate::default_tool_definitions(
            dir,
            crate::permissions::PermissionPolicy::default(),
            hoocode_code_tools_fs::ReadToolOptions::default(),
            hoocode_code_tool_bash::BashToolOptions::default(),
        );
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Read"));
        assert!(!names.contains(&"DocSearch"));
    }
}
