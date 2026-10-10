//! The sync rules, offline. The fixtures are small hand-written catalogs in the
//! models.dev shape; `tests/fixtures/models-dev.json` covers every rule.

use hoocode_models_sync::{
    parse_catalog, sync, to_json, CatalogModel, Overrides, Sync, UpProvider, Upstream,
};

fn baseline() -> Vec<CatalogModel> {
    parse_catalog(include_str!("../fixtures/baseline.json")).expect("baseline fixture")
}

fn upstream() -> Upstream {
    serde_json::from_str(include_str!("../fixtures/models-dev.json")).expect("models.dev fixture")
}

fn overrides() -> Overrides {
    serde_json::from_str(include_str!("../fixtures/overrides.json")).expect("overrides fixture")
}

fn run() -> Sync {
    sync(&baseline(), &upstream(), &overrides()).expect("the fixture syncs")
}

fn ids<'a>(models: &'a [CatalogModel], provider: &str) -> Vec<&'a str> {
    models
        .iter()
        .filter(|m| m.provider == provider)
        .map(|m| m.id.as_str())
        .collect()
}

fn find<'a>(models: &'a [CatalogModel], provider: &str, id: &str) -> &'a CatalogModel {
    models
        .iter()
        .find(|m| m.provider == provider && m.id == id)
        .unwrap_or_else(|| panic!("{provider}/{id} is in the output"))
}

fn report_for<'a>(out: &'a Sync, provider: &str) -> &'a hoocode_models_sync::ProviderReport {
    out.report
        .providers
        .iter()
        .find(|p| p.name == provider)
        .unwrap_or_else(|| panic!("{provider} has a report"))
}

#[test]
fn adds_refreshes_and_keeps_in_baseline_order() {
    let out = run();
    assert_eq!(
        ids(&out.models, "anthropic"),
        ["claude-opus-5", "claude-retired", "claude-haiku-6"]
    );
    assert_eq!(
        ids(&out.models, "github-copilot"),
        ["claude-sonnet-5.5", "gpt-old", "claude-haiku-5.5"]
    );
    assert_eq!(
        ids(&out.models, "google-vertex"),
        ["gemini-old", "gemini-new"]
    );
    assert_eq!(
        ids(&out.models, "minimax"),
        ["MiniMax-M2.7", "MiniMax-M2.7-highspeed"]
    );
    assert_eq!(
        ids(&out.models, "openrouter"),
        ["vendor/zero-price", "vendor/gone", "vendor/fresh"]
    );
    assert_eq!(ids(&out.models, "together"), ["MiniMaxAI/MiniMax-M2.7"]);
    assert_eq!(
        ids(&out.models, "zai"),
        ["glm-5", "glm-5.2", "override-only"]
    );
    // Not on models.dev: kept verbatim.
    assert_eq!(
        ids(&out.models, "google-antigravity"),
        ["gemini-3.8-flash-tiered"]
    );

    assert_eq!(out.report.before, 11);
    assert_eq!(out.report.after, 18);
}

#[test]
fn a_real_price_is_refreshed_one_field_at_a_time() {
    let out = run();
    let opus = find(&out.models, "anthropic", "claude-opus-5");
    assert_eq!(opus.cost.cache_read, 0.4, "models.dev's new cache read");
    assert_eq!(opus.cost.input, 5.0, "unchanged fields stay");
    let sonnet = find(&out.models, "github-copilot", "claude-sonnet-5.5");
    assert_eq!(sonnet.cost.cache_read, 0.1);
    assert_eq!(
        report_for(&out, "anthropic").cost_changed,
        ["claude-opus-5"]
    );
}

#[test]
fn an_added_model_takes_its_api_base_url_and_headers_from_the_template() {
    let out = run();
    let haiku = find(&out.models, "github-copilot", "claude-haiku-5.5");
    assert_eq!(haiku.api, "anthropic-messages", "model-level npm wins");
    assert_eq!(haiku.base_url, "https://api.individual.githubcopilot.com");
    assert!(
        haiku.headers.is_some(),
        "copilot headers come from the template"
    );
    assert!(haiku.compat.is_none());
    assert!(
        haiku.thinking_level_map.is_none(),
        "new entries are not tuned"
    );

    let fresh = find(&out.models, "openrouter", "vendor/fresh");
    assert_eq!(
        fresh.api, "openai-completions",
        "provider npm, then the template"
    );
    assert_eq!(fresh.context_window, 262_144);
    assert_eq!(fresh.max_tokens, 32_768);
    assert_eq!(fresh.input, ["text"]);
    assert_eq!(fresh.cost.output, 2.0);
}

#[test]
fn a_model_without_a_price_is_added_at_zero_and_marked() {
    let out = run();
    let haiku = find(&out.models, "anthropic", "claude-haiku-6");
    assert_eq!(haiku.cost.input, 0.0);
    assert!(
        report_for(&out, "anthropic")
            .added
            .contains(&"claude-haiku-6 (no price on models.dev: 0)".to_string()),
        "the report flags it for review"
    );
}

#[test]
fn zero_and_missing_prices_never_overwrite_a_real_one() {
    // models.dev has 0 and no cache price for opus; hoocode's 5 / 25 / 0.5 / 6.25 stay.
    // Only anthropic is replaced; the other mapped sources must still exist.
    let anthropic: UpProvider = serde_json::from_str(
        r#"{"npm": "@ai-sdk/anthropic", "models": {
            "claude-opus-5": {"tool_call": true, "reasoning": true,
              "modalities": {"input": ["text", "image"], "output": ["text"]},
              "limit": {"context": 1000000, "output": 128000},
              "cost": {"input": 0, "output": 0}}}}"#,
    )
    .unwrap();
    let mut upstream = upstream();
    upstream.insert("anthropic".into(), anthropic);
    let out = sync(&baseline(), &upstream, &Overrides::default()).unwrap();
    let opus = find(&out.models, "anthropic", "claude-opus-5");
    assert_eq!(opus.cost.input, 5.0);
    assert_eq!(opus.cost.output, 25.0);
    assert_eq!(opus.cost.cache_read, 0.5);
    assert!(report_for(&out, "anthropic").cost_changed.is_empty());
}

#[test]
fn keep_prices_leaves_the_price_and_reports_the_difference() {
    let out = run();
    let zero_price = find(&out.models, "openrouter", "vendor/zero-price");
    assert_eq!(zero_price.cost.input, 0.28, "openrouter's own price stays");
    let report = report_for(&out, "openrouter");
    assert!(report.prices_kept);
    assert_eq!(report.price_drift, ["vendor/zero-price"]);
}

#[test]
fn curated_fields_are_reported_as_drift_and_not_applied() {
    let out = run();
    let vertex = find(&out.models, "google-vertex", "gemini-old");
    assert!(
        vertex.reasoning,
        "models.dev says false; hoocode keeps true"
    );
    assert_eq!(
        report_for(&out, "google-vertex").drift,
        ["gemini-old: reasoning true -> false"]
    );
}

#[test]
fn a_model_needs_a_known_api_type_and_a_sane_limit() {
    let out = run();
    let vertex = report_for(&out, "google-vertex");
    assert_eq!(
        vertex
            .skipped
            .get("npm @ai-sdk/google-vertex/anthropic has no hoocode api type"),
        Some(&1),
        "Claude on Vertex is not a google-vertex model"
    );
    assert!(!ids(&out.models, "google-vertex").contains(&"claude-opus-5@default"));

    let openrouter = report_for(&out, "openrouter");
    assert_eq!(openrouter.skipped.get("output is not text only"), Some(&1));
    assert_eq!(
        openrouter
            .skipped
            .get("output limit above the context window"),
        Some(&1)
    );
    assert!(!ids(&out.models, "openrouter").contains(&"vendor/capped"));
}

#[test]
fn skips_non_tool_and_deprecated_models() {
    let out = run();
    let anthropic = report_for(&out, "anthropic");
    assert_eq!(anthropic.skipped.get("not a tool-calling model"), Some(&1));
    assert_eq!(anthropic.skipped.get("deprecated upstream"), Some(&1));
    assert!(!ids(&out.models, "anthropic").contains(&"claude-embed"));
    assert!(!ids(&out.models, "anthropic").contains(&"claude-legacy"));
}

#[test]
fn allow_list_limits_new_entries_but_not_existing_ones() {
    let out = run();
    assert_eq!(
        report_for(&out, "minimax")
            .skipped
            .get("not in overrides.json allow list"),
        Some(&1),
        "MiniMax-M3 is not allowed"
    );
    assert!(ids(&out.models, "minimax").contains(&"MiniMax-M2.7-highspeed"));
    assert!(!ids(&out.models, "minimax").contains(&"MiniMax-M3"));
}

#[test]
fn models_that_disappear_upstream_are_kept_and_counted() {
    let out = run();
    assert!(ids(&out.models, "openrouter").contains(&"vendor/gone"));
    assert_eq!(report_for(&out, "openrouter").kept, ["vendor/gone"]);
}

#[test]
fn exclude_blocks_an_upstream_model_for_good() {
    let out = run();
    assert!(!ids(&out.models, "openrouter").contains(&"vendor/excluded-new"));
}

#[test]
fn overrides_add_only_what_is_missing() {
    let out = run();
    assert!(ids(&out.models, "zai").contains(&"override-only"));
    let second = sync(&out.models, &upstream(), &overrides()).unwrap();
    assert_eq!(
        ids(&second.models, "zai")
            .iter()
            .filter(|id| **id == "override-only")
            .count(),
        1,
        "an override already in the catalog is not added twice"
    );
}

#[test]
fn a_missing_mapped_source_is_an_error_not_a_quiet_no_op() {
    let mut data = upstream();
    data.remove("togetherai");
    let err = sync(&baseline(), &data, &overrides()).unwrap_err();
    assert!(err.contains("togetherai"), "{err}");
}

#[test]
fn an_override_for_an_unknown_provider_is_an_error() {
    let mut o = overrides();
    o.allow.insert("no-such-provider".into(), vec!["x".into()]);
    let err = sync(&baseline(), &upstream(), &o).unwrap_err();
    assert!(err.contains("no-such-provider"), "{err}");
}

#[test]
fn a_second_run_over_its_own_output_changes_nothing() {
    let first = run();
    let second = sync(&first.models, &upstream(), &overrides()).unwrap();
    assert_eq!(second.models, first.models);
    // keep_prices drift is reported on every run by design (the price is never
    // applied), so only additions and price changes must be empty here.
    assert!(second
        .report
        .providers
        .iter()
        .all(|p| p.added.is_empty() && p.cost_changed.is_empty()));
}

#[test]
fn numbers_are_written_like_the_committed_file() {
    let text = to_json(&baseline());
    // Whole numbers without ".0", and the same spacing as the Python writer.
    assert!(text.contains("\"input\": 5,"), "{text}");
    assert!(!text.contains("5.0"), "{text}");
    assert!(text.starts_with("[\n {\n  \"id\": \"claude-opus-5\","));
    assert!(text.ends_with("]\n"));
}

#[test]
fn the_committed_catalog_is_already_in_canonical_form() {
    // If this fails, the file was edited or written by something other than
    // models-sync. Regenerate it with models-sync instead.
    let committed = include_str!("../../../hoocode-ai-models-catalog/data/models.json");
    let models = parse_catalog(committed).expect("the committed catalog parses");
    assert_eq!(to_json(&models), committed);
}

#[test]
fn the_report_names_each_change() {
    let text = run().report.render();
    assert!(
        text.starts_with("models.json: 11 -> 18 entries (+7 added, -0 excluded, 2 prices changed)")
    );
    assert!(text.contains("added 1: claude-haiku-6 (no price on models.dev: 0)"));
    assert!(
        text.contains("together (from models.dev `togetherai`)"),
        "{text}"
    );
    assert!(text.contains("not on models.dev: kept as is"));
}
