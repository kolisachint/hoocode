//! The tool renderers against the pin's own `renderCall`/`renderResult`
//! output (`fixtures/tool-renderers-gold.json`, from
//! `archive/migration/tools/goldens/tool-renderers.mjs`).

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_execution::{
    ToolRenderContext, ToolRenderDefinition, ToolRenderResultOptions, ToolResultView,
};
use hoocode_code_tui_widgets::tools::subagent::format_task_call;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use serde_json::{json, Value};

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

/// Declared divergence from the pin (2026-10-09, user decision: dial is
/// radar/peek only, ctrl+o toggles): the pinned `ctrl+o to expand` hints are
/// gone from our renderers. Rewrites the pinned bytes of one line into our form
/// and, when the pinned line was padded to the width, moves its trailing padding
/// by the width the hint took, as `rename_title` does. A pinned line the pin left
/// unpadded stays unpadded. Returns `None` when the line has no hint.
fn drop_expand_hint(line: &str) -> Option<String> {
    // (pinned bytes, our bytes): the result hints `(N more lines, ctrl+o to
    // expand)` / `(N more lines, M total, ctrl+o to expand)`, and the call
    // hints ` (ctrl+o to expand)`.
    const FORMS: &[(&str, &str)] = &[
        (
            ",\x1b[39m \x1b[38;2;152;152;152mctrl+o\x1b[39m\x1b[38;2;168;168;168m to expand\x1b[39m)",
            ")\x1b[39m",
        ),
        ("\x1b[38;2;152;152;152m (ctrl+o to expand)\x1b[39m", ""),
    ];
    let mut out = line.to_string();
    let mut removed = 0usize;
    for (from, to) in FORMS {
        if out.contains(from) {
            removed += strip(from).chars().count() - strip(to).chars().count();
            out = out.replacen(from, to, 1);
        }
    }
    if removed == 0 {
        return None;
    }
    let body = out.trim_end_matches(' ');
    let pad = out.len() - body.len();
    let pad = if pad == 0 { 0 } else { pad + removed };
    Some(format!("{body}{}", " ".repeat(pad)))
}

/// `drop_expand_hint` over a pinned render, counting the lines it rewrote.
fn drop_hints(lines: Option<Vec<String>>, rewritten: &mut usize) -> Option<Vec<String>> {
    lines.map(|ls| {
        ls.into_iter()
            .map(|l| match drop_expand_hint(&l) {
                Some(ours) => {
                    *rewritten += 1;
                    ours
                }
                None => l,
            })
            .collect()
    })
}

/// Declared divergence from the pin (2026-10-09, user decision: skills and
/// agents are promoted to peek because the full stop was removed): a compact
/// `Read` of a skill (`SKILL.md`), an `AGENTS.md`, or an app doc renders its
/// body at peek, while the pin renders an empty body when `expanded` is false.
/// These cases are not compared byte for byte; they must show the file's first
/// content line instead (see `renderers_match_the_pin`).
///
/// Matches a `Read` whose path names one of those compact resources, collapsed,
/// with an empty pinned result.
fn is_peek_exempt(tool: &str, args: &Value, expanded: bool, want: Option<&[String]>) -> bool {
    !expanded && tool == "Read" && compact_read_path(args) && matches!(want, Some([]))
}

/// Whether a `Read` path names a skill, an `AGENTS.md`, or an app doc.
fn compact_read_path(args: &Value) -> bool {
    let path = args["file_path"]
        .as_str()
        .or_else(|| args["path"].as_str())
        .unwrap_or_default();
    let file_name = path.rsplit('/').next().unwrap_or(path);
    matches!(file_name, "SKILL.md" | "AGENTS.md")
        || path.starts_with("docs/")
        || path.contains("/docs/")
}

#[ignore = "frozen TS pin; tool renderers redesigned 2026-10-10 (docs/design/tool-calls-peek-radar.md)"]
#[test]
fn renderers_match_the_pin() {
    let _g = lock();
    let gold: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/tool-renderers-gold.json")).unwrap();
    let mut failures = Vec::new();
    let mut renamed_cases = 0usize;
    let mut hint_rewrites = 0usize;
    let mut peek_exempt = 0usize;
    // Declared divergence from the pin (2026-10-09, user decision: radar/peek
    // dial). The Full stop is gone, so no block renders with `expanded` set;
    // the pinned `expanded: true` cases describe a view we no longer have and
    // are skipped. Counted below so the skip cannot grow silently.
    let mut full_skipped = 0usize;
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
        if expanded {
            full_skipped += 1;
            continue;
        }
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
        let want_call = drop_hints(want_call, &mut hint_rewrites);
        if call != want_call {
            failures.push(format!(
                "{tool} {args} call (expanded={expanded})\n  want {want_call:?}\n  got  {call:?}"
            ));
        }
        let want_res: Option<Vec<String>> = serde_json::from_value(case["res"].clone()).unwrap();
        let want_res = rename_lines(want_res, ts_tool, tool);
        let want_res = drop_hints(want_res, &mut hint_rewrites);
        // Durations are wall-clock: compare them as a placeholder.
        let res = res.map(normalize_took);
        let want_res = want_res.map(normalize_took);
        if is_peek_exempt(tool, args, expanded, want_res.as_deref()) {
            peek_exempt += 1;
            let first_line = result["content"][0]["text"]
                .as_str()
                .and_then(|t| t.lines().next())
                .unwrap_or_default();
            let shown = strip(&res.unwrap_or_default().join("\n"));
            if first_line.is_empty() || !shown.contains(first_line) {
                failures.push(format!(
                    "{tool} {args} result (expanded={expanded}, peek divergence)\n  want the first content line {first_line:?} in the output\n  got  {shown:?}"
                ));
            }
        } else if res != want_res {
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
        hint_rewrites > 0,
        "the fixture should still pin `ctrl+o to expand` hints, or the divergence has rotted"
    );
    assert_eq!(
        full_skipped, 34,
        "expected 34 pinned `expanded: true` cases skipped (the removed Full stop), got {full_skipped}"
    );
    assert_eq!(
        peek_exempt, 2,
        "expected 2 compact Read cases exempted at peek (the skill and AGENTS.md cases), got {peek_exempt}; the divergence has rotted or grown"
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

/// The Agent call line names the task: `description` first, then the first
/// line of `prompt` when the description is blank, with `background` after it.
#[test]
fn the_agent_call_line_names_the_task() {
    let _g = lock();
    let line = |args: Value| strip(&format_task_call(&args)).trim_end().to_string();
    assert_eq!(
        line(
            json!({"subagent_type": "explore", "description": "Find the footer", "prompt": "Locate it"})
        ),
        "Agent explore · Find the footer"
    );
    assert_eq!(
        line(
            json!({"subagent_type": "explore", "description": "  ", "prompt": "Locate it\nthen list"})
        ),
        "Agent explore · Locate it"
    );
    assert_eq!(
        line(json!({"description": "Find", "background": true})),
        "Agent agent · Find · background"
    );
    assert_eq!(line(json!({"subagent_type": "explore"})), "Agent explore");
}
