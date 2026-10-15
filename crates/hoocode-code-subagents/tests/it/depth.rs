//! subagent-depth.test.ts: the env-driven depth contract and the bounded
//! nested-concurrency rule. Every helper takes an explicit env.

use std::collections::HashMap;

use hoocode_code_subagents::depth::*;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn current_depth_treats_missing_zero_garbage_as_root() {
    assert_eq!(current_subagent_depth(&env(&[])), 0);
    assert_eq!(
        current_subagent_depth(&env(&[("HOOCODE_SUBAGENT_DEPTH", "0")])),
        0
    );
    assert_eq!(
        current_subagent_depth(&env(&[("HOOCODE_SUBAGENT_DEPTH", "nope")])),
        0
    );
}

#[test]
fn current_depth_reads_positive_depths() {
    assert_eq!(
        current_subagent_depth(&env(&[("HOOCODE_SUBAGENT_DEPTH", "2")])),
        2
    );
}

#[test]
fn max_depth_defaults_to_three_levels_of_nesting() {
    assert_eq!(
        resolve_max_subagent_depth(None, &env(&[])),
        DEFAULT_MAX_SUBAGENT_DEPTH
    );
    assert_eq!(DEFAULT_MAX_SUBAGENT_DEPTH, 3);
    assert_eq!(DEFAULT_MAX_SUBAGENT_DEPTH, ABSOLUTE_MAX_SUBAGENT_DEPTH);
}

#[test]
fn max_depth_prefers_the_inherited_env_value_over_the_setting() {
    assert_eq!(
        resolve_max_subagent_depth(Some(3.0), &env(&[("HOOCODE_SUBAGENT_MAX_DEPTH", "2")])),
        2
    );
}

#[test]
fn max_depth_falls_back_to_the_setting_clamped() {
    assert_eq!(resolve_max_subagent_depth(Some(2.0), &env(&[])), 2);
    // A setting below 1 is not a cap: it falls back to the default.
    assert_eq!(
        resolve_max_subagent_depth(Some(0.0), &env(&[])),
        DEFAULT_MAX_SUBAGENT_DEPTH
    );
    assert_eq!(
        resolve_max_subagent_depth(Some(-5.0), &env(&[])),
        DEFAULT_MAX_SUBAGENT_DEPTH
    );
}

#[test]
fn max_depth_clamps_an_over_large_cap() {
    assert_eq!(
        resolve_max_subagent_depth(Some(99.0), &env(&[])),
        ABSOLUTE_MAX_SUBAGENT_DEPTH
    );
    assert_eq!(
        resolve_max_subagent_depth(None, &env(&[("HOOCODE_SUBAGENT_MAX_DEPTH", "50")])),
        ABSOLUTE_MAX_SUBAGENT_DEPTH
    );
}

#[test]
fn clamp_keeps_values_in_range_and_floors() {
    assert_eq!(clamp_max_subagent_depth(0.0), 1);
    assert_eq!(clamp_max_subagent_depth(2.0), 2);
    assert_eq!(clamp_max_subagent_depth(2.9), 2);
    assert_eq!(clamp_max_subagent_depth(999.0), ABSOLUTE_MAX_SUBAGENT_DEPTH);
    assert_eq!(
        clamp_max_subagent_depth(f64::NAN),
        DEFAULT_MAX_SUBAGENT_DEPTH
    );
}

#[test]
fn can_spawn_blocks_at_or_beyond_the_cap() {
    assert!(can_spawn_subagent(None, &env(&[])));
    // Under the default cap of 3: depth 2 spawns depth 3, depth 3 cannot spawn.
    assert!(can_spawn_subagent(
        None,
        &env(&[("HOOCODE_SUBAGENT_DEPTH", "2")])
    ));
    assert!(!can_spawn_subagent(
        None,
        &env(&[("HOOCODE_SUBAGENT_DEPTH", "3")])
    ));
    let lowered = ("HOOCODE_SUBAGENT_MAX_DEPTH", "2");
    assert!(can_spawn_subagent(
        None,
        &env(&[lowered, ("HOOCODE_SUBAGENT_DEPTH", "1")])
    ));
    assert!(!can_spawn_subagent(
        None,
        &env(&[lowered, ("HOOCODE_SUBAGENT_DEPTH", "2")])
    ));
}

#[test]
fn pool_concurrency_is_the_root_default_at_root_and_the_nested_default_below() {
    assert_eq!(
        pool_concurrency_for_depth(None, None, &env(&[])),
        DEFAULT_ROOT_CONCURRENCY
    );
    assert_eq!(DEFAULT_ROOT_CONCURRENCY, 8);
    assert_eq!(
        pool_concurrency_for_depth(None, None, &env(&[("HOOCODE_SUBAGENT_DEPTH", "1")])),
        NESTED_SUBAGENT_CONCURRENCY
    );
    assert_eq!(NESTED_SUBAGENT_CONCURRENCY, 3);
    assert_eq!(
        pool_concurrency_for_depth(None, None, &env(&[("HOOCODE_SUBAGENT_DEPTH", "2")])),
        NESTED_SUBAGENT_CONCURRENCY
    );
}

#[test]
fn pool_concurrency_uses_the_nested_setting_only_below_the_root() {
    let depth1 = env(&[("HOOCODE_SUBAGENT_DEPTH", "1")]);
    assert_eq!(pool_concurrency_for_depth(None, Some(2.0), &depth1), 2);
    assert_eq!(
        pool_concurrency_for_depth(Some(4.0), Some(2.0), &env(&[])),
        4
    );
}

#[test]
fn pool_concurrency_honors_a_configured_nested_concurrency() {
    assert_eq!(
        pool_concurrency_for_depth(
            None,
            None,
            &env(&[
                ("HOOCODE_SUBAGENT_DEPTH", "1"),
                ("HOOCODE_NESTED_SUBAGENT_CONCURRENCY", "4")
            ])
        ),
        4
    );
    // The nested env does not reach the root pool.
    assert_eq!(
        pool_concurrency_for_depth(
            None,
            None,
            &env(&[("HOOCODE_NESTED_SUBAGENT_CONCURRENCY", "4")])
        ),
        DEFAULT_ROOT_CONCURRENCY
    );
}

#[test]
fn root_concurrency_defaults_to_the_cold_pool_default() {
    assert_eq!(
        DEFAULT_ROOT_CONCURRENCY as usize,
        hoocode_code_subagents::pool::DEFAULT_MAX_CONCURRENCY
    );
    assert_eq!(resolve_root_concurrency(None, &env(&[])), 8);
}

#[test]
fn root_concurrency_prefers_env_then_setting_then_default() {
    assert_eq!(resolve_root_concurrency(Some(4.0), &env(&[])), 4);
    assert_eq!(
        resolve_root_concurrency(Some(4.0), &env(&[("HOOCODE_SUBAGENT_CONCURRENCY", "6")])),
        6
    );
    assert_eq!(
        resolve_root_concurrency(None, &env(&[("HOOCODE_SUBAGENT_CONCURRENCY", "12")])),
        12
    );
    // The nested env is a different variable.
    assert_eq!(
        resolve_root_concurrency(None, &env(&[("HOOCODE_NESTED_SUBAGENT_CONCURRENCY", "2")])),
        DEFAULT_ROOT_CONCURRENCY
    );
}

#[test]
fn root_concurrency_is_clamped_to_one_through_sixteen() {
    assert_eq!(resolve_root_concurrency(Some(0.0), &env(&[])), 1);
    assert_eq!(resolve_root_concurrency(Some(-3.0), &env(&[])), 1);
    assert_eq!(resolve_root_concurrency(Some(99.0), &env(&[])), 16);
    assert_eq!(resolve_root_concurrency(Some(2.9), &env(&[])), 2);
    assert_eq!(
        resolve_root_concurrency(None, &env(&[("HOOCODE_SUBAGENT_CONCURRENCY", "0")])),
        1
    );
    assert_eq!(
        resolve_root_concurrency(None, &env(&[("HOOCODE_SUBAGENT_CONCURRENCY", "99")])),
        16
    );
    assert_eq!(
        resolve_root_concurrency(Some(f64::NAN), &env(&[])),
        DEFAULT_ROOT_CONCURRENCY
    );
}

#[test]
fn delegate_scoping_is_unrestricted_without_the_env_var() {
    assert!(is_delegate_allowed("explore", &env(&[])));
    assert_eq!(delegate_allow_list(&env(&[])), None);
}

#[test]
fn delegate_scoping_restricts_to_the_listed_types() {
    let e = env(&[("HOOCODE_DELEGATE_ALLOW", "explore, plan")]);
    assert_eq!(
        delegate_allow_list(&e),
        Some(vec!["explore".to_string(), "plan".to_string()])
    );
    assert!(is_delegate_allowed("explore", &e));
    assert!(!is_delegate_allowed("general-purpose", &e));
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn mcp_needed_when_the_allowlist_is_undefined() {
    assert!(tool_allowlist_needs_mcp(None));
}

#[test]
fn mcp_not_needed_for_an_mcp_free_allowlist() {
    assert!(!tool_allowlist_needs_mcp(Some(&strings(&[
        "Read",
        "CodeSearch"
    ]))));
    assert!(!tool_allowlist_needs_mcp(Some(&[])));
    assert!(!tool_allowlist_needs_mcp(Some(&strings(&[
        "Read",
        "Agent",
        "AgentOutput"
    ]))));
}

#[test]
fn mcp_needed_when_the_allowlist_references_an_mcp_tool() {
    assert!(tool_allowlist_needs_mcp(Some(&strings(&[
        "Read",
        "mcp_github_search"
    ]))));
    assert!(tool_allowlist_needs_mcp(Some(&strings(&[" mcp-foo "]))));
    assert!(tool_allowlist_needs_mcp(Some(&strings(&[
        "MCP_Github_Issue"
    ]))));
}

#[test]
fn subagent_skip_mcp_reads_the_env_flag() {
    assert!(!subagent_skip_mcp(&env(&[])));
    assert!(subagent_skip_mcp(&env(&[("HOOCODE_SKIP_MCP", "1")])));
    assert!(!subagent_skip_mcp(&env(&[("HOOCODE_SKIP_MCP", "0")])));
}

#[test]
fn nested_concurrency_prefers_env_then_setting_then_default() {
    assert_eq!(
        resolve_nested_concurrency(None, &env(&[])),
        NESTED_SUBAGENT_CONCURRENCY
    );
    assert_eq!(resolve_nested_concurrency(Some(3.0), &env(&[])), 3);
    assert_eq!(
        resolve_nested_concurrency(Some(0.0), &env(&[])),
        NESTED_SUBAGENT_CONCURRENCY
    );
    assert_eq!(
        resolve_nested_concurrency(
            Some(2.0),
            &env(&[("HOOCODE_NESTED_SUBAGENT_CONCURRENCY", "5")])
        ),
        5
    );
}
