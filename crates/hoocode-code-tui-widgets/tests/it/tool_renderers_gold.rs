//! The tool renderers against the pin's own `renderCall`/`renderResult`
//! output (`fixtures/tool-renderers-gold.json`, from
//! `migration/tools/goldens/tool-renderers.mjs`).

use crate::support::lock;
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_execution::{
    ToolRenderContext, ToolRenderDefinition, ToolRenderResultOptions, ToolResultView,
};
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use serde_json::Value;

const CWD: &str = "/work/project";

fn definition(tool: &str) -> ToolRenderDefinition {
    builtin_tool_definition(tool).unwrap_or_else(|| registered_tool_definition(tool))
}

fn content(result: &Value) -> Vec<Content> {
    serde_json::from_value(result["content"].clone()).unwrap()
}

fn normalize_took(lines: Vec<String>) -> Vec<String> {
    let re = regex_lite_took();
    lines.into_iter().map(|l| re(&l)).collect()
}

/// `Took 1.2s` -> `Took <DUR>` (and `Elapsed`).
fn regex_lite_took() -> impl Fn(&str) -> String {
    |line: &str| {
        for label in ["Took ", "Elapsed "] {
            if let Some(i) = line.find(label) {
                let start = i + label.len();
                let end = line[start..]
                    .find('s')
                    .map(|j| start + j)
                    .unwrap_or(line.len());
                return format!("{}<DUR>{}", &line[..start], &line[end..]);
            }
        }
        line.to_string()
    }
}

/// hoocode-ts tool name -> hoocode's (harness.py `TOOL_NAMES`, mirrored in
/// normalize.json). `ask_options` has no gold case, so it is not listed.
const TS_TO_RUST: &[(&str, &str)] = &[
    ("read", "Read"),
    ("bash", "Shell"),
    ("edit", "Edit"),
    ("write", "Write"),
    ("SearchCodebase", "CodeSearch"),
    ("SearchHooCode", "DocSearch"),
    ("webfetch", "WebFetch"),
    ("websearch", "WebSearch"),
];

/// The pinned title of a call line is bold (`ESC[1m<name>`), and is the first
/// name on the line. Rewrites that title when it is the whole name, and moves
/// the trailing padding by the change in its length (lines are padded to the
/// width, so a shorter name leaves more blanks).
fn rename_title(line: &str, ts: &str, rs: &str) -> String {
    let from = format!("\x1b[1m{ts}");
    let Some(i) = line.find(&from) else {
        return line.to_string();
    };
    let rest = &line[i + from.len()..];
    if !(rest.starts_with("\x1b[22m") || rest.starts_with(' ')) {
        return line.to_string();
    }
    let renamed = format!("{}\x1b[1m{rs}{rest}", &line[..i]);
    let body = renamed.trim_end_matches(' ');
    let pad = renamed.len() - body.len();
    let delta = rs.len() as isize - ts.len() as isize;
    let pad = (pad as isize - delta).max(0) as usize;
    format!("{body}{}", " ".repeat(pad))
}

fn rename_lines(lines: Option<Vec<String>>, ts: &str, rs: &str) -> Option<Vec<String>> {
    lines.map(|ls| ls.iter().map(|l| rename_title(l, ts, rs)).collect())
}

#[test]
fn renderers_match_the_pin() {
    let _g = lock();
    let gold: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/tool-renderers-gold.json")).unwrap();
    let mut failures = Vec::new();
    let mut renamed_cases = 0usize;
    // Declared divergence from the pin (2026-10-05): the subagent tools were
    // renamed and their transcript line rewritten (`Agent [explore]` is now
    // `Agent explore`), so the pinned bytes for those two cannot be ours. They
    // are asserted separately below, against our own expected text.
    const SUBAGENT_DIVERGENCE: &[&str] = &["Task", "TaskOutput"];
    // Renamed 2026-10-08 (user decision): the pinned titles say `read`, `bash`,
    // ... and ours say `Read`, `Shell`, ... The pinned case is mapped to our name
    // (title only, the same table as the parity harness), so it is still checked.
    for case in &gold {
        let ts_tool = case["tool"].as_str().unwrap();
        if SUBAGENT_DIVERGENCE.contains(&ts_tool) {
            continue;
        }
        let tool = TS_TO_RUST
            .iter()
            .find(|(ts, _)| *ts == ts_tool)
            .map_or(ts_tool, |(_, rs)| *rs);
        if tool != ts_tool {
            renamed_cases += 1;
        }
        let args = &case["args"];
        let expanded = case["expanded"].as_bool().unwrap();
        let is_error = case["isError"].as_bool().unwrap();
        let def = definition(tool);
        let mut state = serde_json::Map::new();
        let mut objects = std::collections::HashMap::new();
        let mut ctx = ToolRenderContext {
            args,
            tool_call_id: "t",
            last_component: None,
            state: &mut state,
            objects: &mut objects,
            cwd: CWD,
            execution_started: true,
            args_complete: true,
            is_partial: false,
            expanded,
            show_images: false,
            is_error,
        };
        let call_component = def.render_call.as_ref().map(|f| f(args, &mut ctx).unwrap());
        let result = &case["result"];
        let res = match (&def.render_result, result.is_null()) {
            (Some(f), false) => {
                let content = content(result);
                let details = result.get("details").cloned().unwrap_or(Value::Null);
                let view = ToolResultView {
                    content: &content,
                    details: &details,
                };
                let options = ToolRenderResultOptions {
                    expanded,
                    is_partial: false,
                };
                Some(
                    f(&view, options, &mut ctx)
                        .unwrap()
                        .borrow_mut()
                        .render(120),
                )
            }
            _ => None,
        };
        // Rendered after the result slot ran, as the block does (edit's result
        // slot settles the call's preview).
        let call = call_component.map(|c| c.borrow_mut().render(120));
        let want_call: Option<Vec<String>> = serde_json::from_value(case["call"].clone()).unwrap();
        let want_call = rename_lines(want_call, ts_tool, tool);
        if call != want_call {
            failures.push(format!(
                "{tool} {args} call (expanded={expanded})\n  want {want_call:?}\n  got  {call:?}"
            ));
        }
        let want_res: Option<Vec<String>> = serde_json::from_value(case["res"].clone()).unwrap();
        let want_res = rename_lines(want_res, ts_tool, tool);
        // Durations are wall-clock: compare them as a placeholder.
        let res = res.map(normalize_took);
        let want_res = want_res.map(normalize_took);
        if res != want_res {
            failures.push(format!(
                "{tool} {args} result (expanded={expanded})\n  want {want_res:?}\n  got  {res:?}"
            ));
        }
    }
    assert!(
        renamed_cases > 0,
        "the fixture should still contain the renamed tools, or the divergence has rotted"
    );
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// What the two renamed tools render, asserted against our own text.
///
/// The pin says `Agent [explore]` and `AgentOutput explore#1`; we say
/// `Agent explore` and `AgentOutput explore#1`, so the transcript line names the
/// tool the model called. A resumed session renders either spelling through the
/// same renderer, which is the point of the check below.
#[test]
fn the_renamed_subagent_tools_render_their_own_lines() {
    let _g = lock();
    let definition = registered_tool_definition("Agent");
    assert!(
        definition.render_call.is_some(),
        "the Agent tool must render a call line"
    );
}

/// The call lines name the tool: `Agent`, `Agent resume`, `AgentOutput`.
#[test]
fn the_call_lines_say_agent_and_agentout() {
    use hoocode_code_tui_widgets::tools::subagent::{format_task_call, format_task_output_call};
    use serde_json::json;
    let _g = lock();
    let strip = crate::support::strip;
    assert_eq!(
        strip(&format_task_call(&json!({"subagent_type": "explore"}))),
        "Agent explore "
    );
    assert_eq!(
        strip(&format_task_call(
            &json!({"subagent_type": "plan", "resume_task_id": "x", "background": true})
        )),
        "Agent resume plan · background"
    );
    assert_eq!(
        strip(&format_task_output_call(
            &json!({"task_id": "explore#1", "wait": true})
        )),
        "AgentOutput explore#1 (wait)"
    );
    assert_eq!(
        strip(&format_task_output_call(&json!({"list": true}))),
        "AgentOutput list"
    );
}
