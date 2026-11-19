//! Port of the pin's `coding-agent/test/suite/disabled-tools.test.ts`, on the
//! real built-in tools.

use crate::common::{Harness, HarnessOptions};

fn session(tools: Option<&[&str]>, disallowed: &[&str]) -> Harness {
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Harness::new(HarnessOptions {
        real_builtin_tools: true,
        allowed_tool_names: tools.map(names),
        disallowed_tool_names: Some(names(disallowed)),
        ..Default::default()
    })
}

fn all_tools(h: &Harness) -> Vec<String> {
    h.session
        .get_all_tools()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[test]
fn removes_a_disabled_tool_from_the_registry() {
    let h = session(None, &["Shell"]);
    let names = all_tools(&h);
    assert!(names.contains(&"Read".to_string()));
    assert!(!names.contains(&"Shell".to_string()));
    assert!(!h
        .session
        .get_active_tool_names()
        .contains(&"Shell".to_string()));
}

#[test]
fn keeps_a_tool_disabled_even_when_it_is_in_the_allowlist() {
    let h = session(Some(&["Read", "Shell"]), &["Shell"]);
    assert_eq!(all_tools(&h), ["Read"]);
    assert_eq!(h.session.get_active_tool_names(), ["Read"]);
}

/// `noTools: "builtin"` (`--no-builtin-tools`, regression #3592): no built-in
/// starts active, but the extension-registered tools do.
#[test]
fn no_builtin_tools_leaves_the_extension_tools_active() {
    let h = Harness::new(HarnessOptions {
        real_builtin_tools: true,
        initial_active_tool_names: Some(vec![]),
        custom_tools: vec![crate::common::tool("AskUserQuestion", |_| unimplemented!())],
        ..Default::default()
    });
    assert_eq!(h.session.get_active_tool_names(), ["AskUserQuestion"]);
    assert!(all_tools(&h).contains(&"Read".to_string()));
}

/// A custom tool that takes a built-in's name does not replace it in the registry: the agent runs
/// the built-in, as the definitions loop does (the first of a name is kept).
#[test]
fn a_custom_tool_never_replaces_a_built_in_in_the_registry() {
    let h = Harness::new(HarnessOptions {
        real_builtin_tools: true,
        custom_tools: vec![crate::common::tool("Read", |_| unimplemented!())],
        ..Default::default()
    });
    let tools = h.session.agent_tools();
    let read = tools
        .0
        .iter()
        .find(|t| t.name == "Read")
        .expect("Read is active");
    assert_ne!(
        read.description, "Read tool",
        "the custom Read replaced the built-in"
    );
}

/// A deny entry that names no tool is reported (`unmatched_denied_tool_names`), as an allow entry
/// is. A name that matches a tool case-insensitively, an `mcp__` name (its server may start) and
/// an entry in the allow list are not.
#[test]
fn a_denied_name_matching_no_tool_is_reported() {
    let h = session(
        Some(&["Read"]),
        &["shell", "NoSuchTool", "mcp__x__y", "Read"],
    );
    assert_eq!(h.session.unmatched_denied_tool_names(), ["NoSuchTool"]);
    assert!(h.session.unmatched_tool_names().is_empty());
}
