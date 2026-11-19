//! Port of hoocode `packages/coding-agent/test/agent-frontmatter.test.ts` (v0.5.89).

use hoocode_code_resources::{
    normalize_model, normalize_tools, parse_agent_definition, AgentSource, CLAUDE_TOOL_ALIASES,
    HOOCODE_TOOL_NAMES, MODEL_INHERIT,
};
use serde_json::{json, Value};

fn tools(value: Value) -> (Vec<String>, Vec<String>) {
    let (tools, diagnostics) = normalize_tools(&value, None);
    (tools, diagnostics.into_iter().map(|d| d.message).collect())
}

// normalizeTools (D7 Claude Code shim)

#[test]
fn maps_claude_tool_names_case_insensitive() {
    let (t, d) = tools(json!("Read, Grep, Bash"));
    assert_eq!(t, ["Read", "CodeSearch", "Shell"]);
    assert!(d.is_empty());
}

#[test]
fn keeps_qualified_selectors_as_written() {
    let (t, d) = tools(json!(
        "Read, plugin:webtools/WebFetch, plugin:webtools/WebFetch, Grep"
    ));
    assert_eq!(t, ["Read", "plugin:webtools/WebFetch", "CodeSearch"]);
    assert!(d.is_empty());
}

#[test]
fn accepts_a_yaml_list_but_emits_a_format_warning() {
    let (value, diagnostics) = normalize_tools(&json!(["Read", "CodeSearch"]), None);
    assert_eq!(value, ["Read", "CodeSearch"]);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].kind,
        hoocode_code_resources::DiagnosticType::Warning
    );
    assert!(diagnostics[0].message.contains("comma-separated string"));
}

#[test]
fn keeps_unknown_names_as_selectors_without_a_diagnostic() {
    // Matching is checked at spawn (a warning there), not here.
    let (t, d) = tools(json!("Read, NotebookEdit, Task, MultiEdit"));
    assert_eq!(t, ["Read", "NotebookEdit", "Task", "MultiEdit"]);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn maps_claude_webfetch_websearch_to_the_webtools_plugin_tools() {
    let (t, d) = tools(json!("WebFetch, WebSearch"));
    assert_eq!(t, ["WebFetch", "WebSearch"]);
    assert!(d.is_empty());
}

#[test]
fn dedupes_resolved_tools() {
    assert_eq!(tools(json!("Grep, Glob, find")).0, ["CodeSearch"]);
}

#[test]
fn keeps_the_pre_rename_search_name_as_a_selector() {
    let (t, d) = tools(json!("read, search"));
    assert_eq!(t, ["Read", "search"]);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn keeps_ls_as_a_selector_with_no_alias() {
    let (t, d) = tools(json!("Read, LS"));
    assert_eq!(t, ["Read", "LS"]);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn alias_map_only_targets_known_hoocode_tools() {
    // Web aliases point at the webtools plugin's tools, which are not built-in.
    for (_, target) in CLAUDE_TOOL_ALIASES {
        assert!(
            HOOCODE_TOOL_NAMES.contains(target) || *target == "WebFetch" || *target == "WebSearch",
            "{target}"
        );
    }
}

// normalizeModel

#[test]
fn preserves_the_inherit_sentinel() {
    assert_eq!(
        normalize_model(Some(&json!("inherit"))).as_deref(),
        Some(MODEL_INHERIT)
    );
}

#[test]
fn passes_through_aliases_and_trims() {
    assert_eq!(
        normalize_model(Some(&json!("  sonnet "))).as_deref(),
        Some("sonnet")
    );
}

#[test]
fn returns_undefined_for_empty_or_missing() {
    assert_eq!(normalize_model(Some(&json!(""))), None);
    assert_eq!(normalize_model(None), None);
}

// parseAgentDefinition

fn parse(
    raw: &str,
    source: AgentSource,
) -> (Option<hoocode_code_resources::AgentDefinition>, Vec<String>) {
    let (agent, diagnostics) = parse_agent_definition(raw, source, None, None);
    (agent, diagnostics.into_iter().map(|d| d.message).collect())
}

#[test]
fn parses_a_claude_code_style_agent_natively() {
    let raw = "---\nname: explorer\ndescription: Use this agent to explore the codebase read-only.\ntools: Read, Grep, Glob, Bash\nmodel: sonnet\n---\nYou are a read-only explorer.";
    let (agent, d) = parse(raw, AgentSource::ClaudeProject);
    assert!(d.is_empty());
    let agent = agent.unwrap();
    assert_eq!(agent.name, "explorer");
    assert_eq!(agent.tools.unwrap(), ["Read", "CodeSearch", "Shell"]);
    assert_eq!(agent.model.as_deref(), Some("sonnet"));
    assert_eq!(agent.prompt, "You are a read-only explorer.");
    assert_eq!(agent.source, AgentSource::ClaudeProject);
}

#[test]
fn omitted_tools_means_inherit_all() {
    let raw =
        "---\nname: agent-a\ndescription: An agent that inherits all parent tools.\n---\nbody";
    assert_eq!(parse(raw, AgentSource::Project).0.unwrap().tools, None);
}

#[test]
fn falls_back_to_fallback_name_when_name_is_omitted() {
    let raw = "---\ndescription: Description long enough to be valid.\n---\nbody";
    let (agent, _) = parse_agent_definition(raw, AgentSource::Builtin, None, Some("explore"));
    assert_eq!(agent.unwrap().name, "explore");
}

#[test]
fn returns_null_when_description_is_missing() {
    let (agent, d) = parse("---\nname: no-desc\n---\nbody", AgentSource::Project);
    assert!(agent.is_none());
    assert!(d.iter().any(|m| m.contains("description is required")));
}

#[test]
fn returns_null_for_an_invalid_name() {
    let raw = "---\nname: Bad_Name\ndescription: Description long enough to be valid.\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert!(agent.is_none());
    assert!(d.iter().any(|m| m.contains("invalid characters")));
}

#[test]
fn captures_a_disallowed_tools_denylist() {
    let raw = "---\nname: limited\ndescription: An agent with a denied tool.\ntools: Read, CodeSearch, Shell\ndisallowedTools: Shell\n---\nbody";
    let agent = parse(raw, AgentSource::Project).0.unwrap();
    assert!(agent.tools.unwrap().contains(&"Shell".to_string()));
    assert_eq!(agent.disallowed_tools.unwrap(), ["Shell"]);
}

#[test]
fn captures_the_max_turns_extension() {
    let raw = "---\nname: capped\ndescription: An agent with a turn cap.\nmaxTurns: 12\n---\nbody";
    assert_eq!(
        parse(raw, AgentSource::Project).0.unwrap().max_turns,
        Some(12)
    );
}

#[test]
fn captures_the_token_budget_extension() {
    let raw = "---\nname: budgeted\ndescription: An agent with a token budget.\ntokenBudget: 90000\n---\nbody";
    assert_eq!(
        parse(raw, AgentSource::Project).0.unwrap().token_budget,
        Some(90_000)
    );
    let absent = "---\nname: plain\ndescription: No budget.\n---\nbody";
    assert_eq!(
        parse(absent, AgentSource::Project).0.unwrap().token_budget,
        None
    );
}

#[test]
fn captures_the_background_flag_and_leaves_it_undefined_when_absent() {
    let bg = "---\nname: watcher\ndescription: A non-blocking background agent.\nbackground: true\n---\nbody";
    assert_eq!(
        parse(bg, AgentSource::Project).0.unwrap().background,
        Some(true)
    );
    let plain = "---\nname: plain\ndescription: A normal foreground agent.\n---\nbody";
    assert_eq!(
        parse(plain, AgentSource::Project).0.unwrap().background,
        None
    );
}

#[test]
fn keeps_background_false_as_a_value_not_an_absence() {
    let raw = "---\nname: fg\ndescription: An agent that stays in the foreground.\nbackground: false\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert_eq!(agent.unwrap().background, Some(false));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn warns_when_background_is_not_a_boolean() {
    let raw = "---\nname: bad-bg\ndescription: Agent with invalid background.\nbackground: yes\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert_eq!(agent.unwrap().background, None);
    assert!(d.iter().any(|m| m.contains("background must be a boolean")));
}

#[test]
fn captures_the_delegate_flag_and_leaves_it_undefined_when_absent() {
    let raw = "---\nname: orchestrator\ndescription: An agent that delegates to other subagents.\ndelegate: true\n---\nbody";
    let agent = parse(raw, AgentSource::Project).0.unwrap();
    assert_eq!(agent.delegate, Some(true));
    assert_eq!(agent.delegate_to, None);
    let plain = "---\nname: plain\ndescription: A normal agent.\n---\nbody";
    assert_eq!(parse(plain, AgentSource::Project).0.unwrap().delegate, None);
}

#[test]
fn captures_a_scoped_delegate_list() {
    let raw = "---\nname: scoped\ndescription: Delegates only to explore and plan.\ndelegate: explore, plan\n---\nbody";
    let agent = parse(raw, AgentSource::Project).0.unwrap();
    assert_eq!(agent.delegate, Some(true));
    assert_eq!(agent.delegate_to.unwrap(), ["explore", "plan"]);
}

#[test]
fn captures_the_fork_flag() {
    let raw = "---\nname: forker\ndescription: An agent that inherits the parent conversation.\nfork: true\n---\nbody";
    assert_eq!(parse(raw, AgentSource::Project).0.unwrap().fork, Some(true));
    let plain = "---\nname: plain\ndescription: A normal agent.\n---\nbody";
    assert_eq!(parse(plain, AgentSource::Project).0.unwrap().fork, None);
}

#[test]
fn warns_when_delegate_is_neither_a_boolean_nor_a_name_list() {
    let raw = "---\nname: bad-delegate\ndescription: Agent with invalid delegate.\ndelegate: 123\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert_eq!(agent.unwrap().delegate, None);
    assert!(d.iter().any(|m| m.contains("delegate must be")));
}

#[test]
fn warns_on_unknown_model_alias() {
    let raw = "---\nname: weird-model\ndescription: Agent using a non-standard model name.\nmodel: gpt-4o\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert_eq!(agent.unwrap().model.as_deref(), Some("gpt-4o"));
    assert!(d.iter().any(|m| m.contains("is not a recognized alias")));
}

#[test]
fn allows_full_claude_model_ids_without_warning() {
    let raw = "---\nname: full-id\ndescription: Agent using a full model ID.\nmodel: claude-sonnet-4-6\n---\nbody";
    let (_, d) = parse(raw, AgentSource::Project);
    assert!(d.iter().all(|m| !m.contains("is not a recognized alias")));
}

#[test]
fn warns_when_tools_is_a_yaml_list() {
    let raw = "---\nname: list-tools\ndescription: Agent declaring tools as a YAML list.\ntools:\n  - read\n  - bash\n---\nbody";
    let (agent, d) = parse(raw, AgentSource::Project);
    assert_eq!(agent.unwrap().tools.unwrap(), ["Read", "Shell"]);
    assert!(d.iter().any(|m| m.contains("comma-separated string")));
}

#[test]
fn keeps_the_plugin_wildcard_as_written() {
    let (t, d) = tools(json!("Read, plugin:*"));
    assert_eq!(t, ["Read", "plugin:*"]);
    assert!(d.is_empty());
}

// Option D (plugins.md, Naming and identity): an agent's selectors are checked when it is read.

#[test]
fn a_bare_name_with_no_alias_is_kept_as_a_selector() {
    // A plugin or MCP tool's model-facing name is not a Claude alias: it stays a `Name` selector,
    // checked against the session's tools at spawn, not dropped here (plugins.md, option D).
    let (t, d) = tools(json!("Read, Lookup, agent"));
    assert_eq!(t, ["Read", "Lookup", "agent"]);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn a_bare_plugin_name_in_an_agent_tools_list_is_kept() {
    let raw = "---\nname: scout\ndescription: Scouts.\ntools: Read, Lookup\ndisallowedTools: Grep, Other\n---\nBody";
    let (agent, d) = parse(raw, AgentSource::ClaudeProject);
    assert!(d.is_empty(), "{d:?}");
    let agent = agent.unwrap();
    assert_eq!(agent.tools.unwrap(), ["Read", "Lookup"]);
    assert_eq!(agent.disallowed_tools.unwrap(), ["CodeSearch", "Other"]);
}

#[test]
fn a_malformed_tools_entry_rejects_the_definition() {
    for tools in ["Read, ext_*", "Read, plugin:", "Read, mcp:a/b/c"] {
        let raw = format!("---\nname: scout\ndescription: Scouts.\ntools: \"{tools}\"\n---\nBody");
        let (agent, d) = parse(&raw, AgentSource::ClaudeProject);
        assert!(agent.is_none(), "{tools}");
        assert!(
            d.iter()
                .any(|m| m.contains("is not a tool name or a selector")),
            "{tools}: {d:?}"
        );
    }
}

#[test]
fn a_malformed_disallowed_tools_entry_rejects_the_definition() {
    let raw =
        "---\nname: scout\ndescription: Scouts.\ndisallowedTools: plugin:*, mcp:*x\n---\nBody";
    let (agent, d) = parse(raw, AgentSource::ClaudeProject);
    assert!(agent.is_none());
    assert!(d.iter().any(|m| m.contains("mcp:*x")), "{d:?}");
}

#[test]
fn qualified_selectors_in_an_agent_are_kept_as_written() {
    let raw = "---\nname: fetcher\ndescription: Fetches.\ntools: Read, plugin:webtools/WebFetch, plugin:*\ndisallowedTools: plugin:*\n---\nBody";
    let (agent, d) = parse(raw, AgentSource::ClaudeProject);
    assert!(d.is_empty(), "{d:?}");
    let agent = agent.unwrap();
    assert_eq!(
        agent.tools.unwrap(),
        ["Read", "plugin:webtools/WebFetch", "plugin:*"]
    );
    assert_eq!(agent.disallowed_tools.unwrap(), ["plugin:*"]);
}
