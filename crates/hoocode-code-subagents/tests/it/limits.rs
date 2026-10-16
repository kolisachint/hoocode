//! Precedence of the subagent run limits: the `HOOCODE_` env var wins over the
//! setting, the setting over the default, and agent frontmatter over both.

use std::collections::HashMap;

use hoocode_code_subagents::lifeguard::{
    PROGRESS_STALL_THRESHOLD_MS, STALL_THRESHOLD_ENV, SUBAGENT_DEADLINE_MS,
};
use hoocode_code_subagents::limits::{
    SubagentLimitSettings, SubagentLimits, DEADLINE_ENV, MAX_RSS_ENV, MAX_TURNS_ENV,
    TOKEN_BUDGET_ENV,
};
use hoocode_code_subagents::pool::DEFAULT_SUBAGENT_MAX_TURNS;
use hoocode_runtime::MIB;

const MINUTE: u64 = 60 * 1000;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn settings() -> SubagentLimitSettings {
    SubagentLimitSettings {
        max_turns: Some(75.0),
        token_budget: Some(90_000.0),
        deadline_minutes: Some(90.0),
        stall_seconds: Some(45.0),
        max_rss_mb: Some(1024.0),
    }
}

#[test]
fn defaults_apply_with_nothing_set() {
    let limits = SubagentLimits::resolve(&SubagentLimitSettings::default(), &env(&[]));
    assert_eq!(limits, SubagentLimits::default());
    assert_eq!(limits.max_turns, 200);
    assert_eq!(DEFAULT_SUBAGENT_MAX_TURNS, 200);
    assert_eq!(limits.token_budget_override, None);
    assert_eq!(limits.deadline_ms, 4 * 60 * MINUTE);
    assert_eq!(limits.deadline_ms, SUBAGENT_DEADLINE_MS);
    assert_eq!(limits.stall_ms, 20 * MINUTE);
    assert_eq!(limits.stall_ms, PROGRESS_STALL_THRESHOLD_MS);
    assert_eq!(limits.max_rss_bytes, 4096 * MIB);
}

#[test]
fn settings_apply_in_their_own_units() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[]));
    assert_eq!(limits.max_turns, 75);
    assert_eq!(limits.token_budget_override, Some(90_000));
    assert_eq!(limits.deadline_ms, 90 * MINUTE);
    assert_eq!(limits.stall_ms, 45_000);
    assert_eq!(limits.max_rss_bytes, 1024 * MIB);
}

#[test]
fn env_wins_over_setting() {
    let limits = SubagentLimits::resolve(
        &settings(),
        &env(&[
            (MAX_TURNS_ENV, "30"),
            (TOKEN_BUDGET_ENV, "12345"),
            (DEADLINE_ENV, "60000"),
            (STALL_THRESHOLD_ENV, "5000"),
            (MAX_RSS_ENV, "512"),
        ]),
    );
    assert_eq!(limits.max_turns, 30);
    assert_eq!(limits.token_budget_override, Some(12_345));
    assert_eq!(limits.deadline_ms, 60_000, "env deadline is milliseconds");
    assert_eq!(limits.stall_ms, 5_000, "env stall is milliseconds");
    assert_eq!(limits.max_rss_bytes, 512 * MIB);
}

#[test]
fn env_values_are_trimmed_and_parsed() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings::default(),
        &env(&[(MAX_TURNS_ENV, " 42 ")]),
    );
    assert_eq!(limits.max_turns, 42);
}

#[test]
fn bad_env_falls_back_to_the_setting_then_the_default() {
    for bad in ["0", "-3", "soon", "", "NaN", "inf"] {
        let with_setting = SubagentLimits::resolve(&settings(), &env(&[(MAX_TURNS_ENV, bad)]));
        assert_eq!(
            with_setting.max_turns, 75,
            "{bad:?} falls back to the setting"
        );
        let with_nothing = SubagentLimits::resolve(
            &SubagentLimitSettings::default(),
            &env(&[(MAX_TURNS_ENV, bad)]),
        );
        assert_eq!(
            with_nothing.max_turns, DEFAULT_SUBAGENT_MAX_TURNS,
            "{bad:?} falls back to the default"
        );
    }
}

#[test]
fn bad_setting_falls_back_to_the_default() {
    let bad = SubagentLimitSettings {
        max_turns: Some(0.0),
        token_budget: Some(-1.0),
        deadline_minutes: Some(f64::NAN),
        stall_seconds: Some(0.0),
        max_rss_mb: Some(-5.0),
    };
    let limits = SubagentLimits::resolve(&bad, &env(&[]));
    assert_eq!(limits, SubagentLimits::default());
}

#[test]
fn fractional_values_are_floored() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings {
            max_turns: Some(75.9),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(limits.max_turns, 75);
}

#[test]
fn frontmatter_max_turns_wins_over_env_and_setting() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(MAX_TURNS_ENV, "30")]));
    assert_eq!(limits.turns_for(Some(12)), 12);
    assert_eq!(limits.turns_for(None), 30, "env beats setting");
    assert_eq!(
        limits.turns_for(Some(0)),
        30,
        "a zero frontmatter value is no value"
    );
}

#[test]
fn frontmatter_token_budget_wins_over_env_setting_and_type_default() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(TOKEN_BUDGET_ENV, "12345")]));
    assert_eq!(limits.token_budget_for("explore", Some(5_000)), 5_000);
    assert_eq!(limits.token_budget_for("explore", None), 12_345);
    assert_eq!(limits.token_budget_for("general-purpose", None), 12_345);
}

#[test]
fn type_defaults_apply_when_no_override_is_set() {
    let limits = SubagentLimits::default();
    assert_eq!(limits.token_budget_for("explore", None), 150_000);
    assert_eq!(limits.token_budget_for("plan", None), 150_000);
    assert_eq!(limits.token_budget_for("general-purpose", None), 300_000);
    assert_eq!(limits.token_budget_for("custom-agent", None), 150_000);
    assert_eq!(limits.token_budget_for("general-purpose", Some(0)), 300_000);
}

#[test]
fn the_setting_budget_overrides_every_type() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings {
            token_budget: Some(90_000.0),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(limits.token_budget_for("explore", None), 90_000);
    assert_eq!(limits.token_budget_for("general-purpose", None), 90_000);
}

#[test]
fn the_run_budget_follows_the_resolved_deadline() {
    let short = SubagentLimits::resolve(
        &SubagentLimitSettings {
            deadline_minutes: Some(10.0),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(short.max_run_budget_ms(), 40 * MINUTE, "4x a short base");
    let default = SubagentLimits::default();
    assert_eq!(
        default.max_run_budget_ms(),
        SUBAGENT_DEADLINE_MS + 30 * MINUTE,
        "the default base plus the 30 minute headroom"
    );
}
