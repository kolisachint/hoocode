//! Maps models.dev (`https://models.dev/api.json`) onto hoocode's model catalog.
//!
//! The committed `crates/hoocode-ai-models-catalog/data/models.json` is the
//! baseline, and the output keeps its shape and order:
//!
//! - An entry hoocode already has keeps what models.dev cannot tell us (`api`,
//!   `baseUrl`, `headers`, `compat`, `thinkingLevelMap`, `name`). Its `cost` is
//!   refreshed from models.dev, one price at a time. A missing or zero price on
//!   models.dev never overwrites a non-zero one: models.dev often has `0` where
//!   it has no price. The providers in `keep_prices` (overrides.json) are never
//!   refreshed, because their prices came from their own API and models.dev's
//!   copy differs. Its `reasoning`, `input` and limits are curated, so a
//!   disagreement is reported as drift, not applied.
//!   models.dev's `limit.output` is not reliable for the aggregators (it reports
//!   the context window as the output limit for some OpenRouter models), and a
//!   wrong `maxTokens` gets sent to the API.
//! - A tool-calling model that models.dev lists for a mapped provider, and that
//!   hoocode does not have yet, is added. It is added only when its API type,
//!   base URL and headers can be determined (see [`sync`]). Anything else is
//!   counted as skipped.
//! - Nothing is removed automatically. Entries models.dev no longer lists are
//!   counted in the report. `exclude` in `overrides.json` removes one for good.
//!
//! Output order is the baseline's: providers in their order, entries in their
//! order, new entries appended to their provider's block. A second run over the
//! output with the same input changes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

/// The models.dev catalog endpoint.
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// hoocode provider -> models.dev provider, where the ids differ. Any other
/// hoocode provider has the same id on models.dev. A hoocode provider that is
/// not on models.dev at all (google-antigravity, google-gemini-cli, openai-codex)
/// is kept as it is.
pub const SOURCES: &[(&str, &str)] = &[
    ("fireworks", "fireworks-ai"),
    ("kimi-coding", "kimi-code-plan-global"),
    ("together", "togetherai"),
    ("vercel-ai-gateway", "vercel"),
    ("zai", "zai-coding-plan"),
];

/// models.dev `provider.npm` (per model, else per provider) -> hoocode `api`.
/// A package not listed here means the wire format is unknown, so the model is
/// skipped (for example Claude on Vertex, `@ai-sdk/google-vertex/anthropic`).
const NPM_APIS: &[(&str, &str)] = &[
    ("@ai-sdk/anthropic", "anthropic-messages"),
    ("@ai-sdk/cerebras", "openai-completions"),
    ("@ai-sdk/google", "google-generative-ai"),
    ("@ai-sdk/google-vertex", "google-vertex"),
    ("@ai-sdk/groq", "openai-completions"),
    ("@ai-sdk/openai", "openai-responses"),
    ("@ai-sdk/openai-compatible", "openai-completions"),
    ("@ai-sdk/togetherai", "openai-completions"),
    ("@ai-sdk/xai", "openai-completions"),
    // Vercel AI Gateway serves every model over the Anthropic Messages wire;
    // all 252 existing vercel-ai-gateway entries say so.
    ("@ai-sdk/gateway", "anthropic-messages"),
    ("@openrouter/ai-sdk-provider", "openai-completions"),
];

/// Providers hoocode has dropped. They are never emitted (Azure, see the
/// archive decisions).
const DROPPED_PROVIDERS: &[&str] = &["azure-openai-responses"];

// ---------------------------------------------------------------------------
// The catalog file
// ---------------------------------------------------------------------------

/// One entry of `models.json`: the `Model` shape, in the file's key order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<Value>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<Value>,
    pub input: Vec<String>,
    pub cost: Cost,
    pub context_window: u64,
    pub max_tokens: u64,
}

/// Pricing per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    #[serde(serialize_with = "js_number")]
    pub input: f64,
    #[serde(serialize_with = "js_number")]
    pub output: f64,
    #[serde(serialize_with = "js_number")]
    pub cache_read: f64,
    #[serde(serialize_with = "js_number")]
    pub cache_write: f64,
}

impl Cost {
    fn from_upstream(c: &UpCost) -> Self {
        Self {
            input: c.input.unwrap_or(0.0),
            output: c.output.unwrap_or(0.0),
            cache_read: c.cache_read.unwrap_or(0.0),
            cache_write: c.cache_write.unwrap_or(0.0),
        }
    }
}

/// Writes a whole number without serde_json's `.0` (`10`, not `10.0`), like JS
/// and the Python writer that produced the committed file.
fn js_number<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

/// Parses `models.json`.
pub fn parse_catalog(json: &str) -> Result<Vec<CatalogModel>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Writes the catalog in the committed file's format: pretty JSON with a
/// one-space indent, UTF-8 as is, and a trailing newline. Same bytes as the
/// Python writer that made the file, so diffs only show real changes.
pub fn to_json(models: &[CatalogModel]) -> String {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    models
        .serialize(&mut ser)
        .expect("catalog entries serialize to JSON");
    let mut out = String::from_utf8(buf).expect("serde_json writes UTF-8");
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// models.dev
// ---------------------------------------------------------------------------

/// models.dev `api.json`: provider id -> provider. Unknown fields are ignored.
pub type Upstream = BTreeMap<String, UpProvider>;

#[derive(Debug, Deserialize)]
pub struct UpProvider {
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, UpModel>,
}

#[derive(Debug, Deserialize)]
pub struct UpModel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub modalities: Option<UpModalities>,
    #[serde(default)]
    pub limit: Option<UpLimit>,
    #[serde(default)]
    pub cost: Option<UpCost>,
    #[serde(default)]
    pub provider: Option<UpModelProvider>,
}

#[derive(Debug, Deserialize)]
pub struct UpModalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpLimit {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct UpCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct UpModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
}

/// `overrides.json`: hand-maintained rules next to the generated file.
#[derive(Debug, Default, Deserialize)]
pub struct Overrides {
    /// Entries hoocode ships that models.dev does not list. Added when missing,
    /// so the file can rebuild the catalog from nothing.
    #[serde(default)]
    pub add: Vec<CatalogModel>,
    /// Provider -> the only upstream ids that may be added as new entries.
    /// Existing entries are not affected.
    #[serde(default)]
    pub allow: BTreeMap<String, Vec<String>>,
    /// Entries that are never emitted, even when models.dev lists them.
    #[serde(default)]
    pub exclude: Vec<ModelKey>,
    /// Providers whose existing prices are never refreshed from models.dev.
    #[serde(default)]
    pub keep_prices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelKey {
    pub provider: String,
    pub id: String,
}

// ---------------------------------------------------------------------------
// The sync
// ---------------------------------------------------------------------------

/// What one sync did, provider by provider.
#[derive(Debug, Default)]
pub struct ProviderReport {
    pub name: String,
    /// The models.dev provider the entries came from. `None` when models.dev
    /// has no such provider, so the entries were kept as they are.
    pub source: Option<String>,
    /// Added from models.dev (or from overrides). An id is marked when its price
    /// is missing or zero on models.dev, so a review can check it.
    pub added: Vec<String>,
    /// Existing entries whose pricing changed.
    pub cost_changed: Vec<String>,
    /// Existing entries whose models.dev price differs, not applied because the
    /// provider is in `keep_prices`.
    pub price_drift: Vec<String>,
    /// The provider is in `keep_prices`.
    pub prices_kept: bool,
    /// Existing entries that models.dev no longer lists. Kept.
    pub kept: Vec<String>,
    /// Entries removed by `exclude`.
    pub excluded: Vec<String>,
    /// Upstream models not added, by reason.
    pub skipped: BTreeMap<String, usize>,
    /// Curated fields that disagree with models.dev. Not applied.
    pub drift: Vec<String>,
}

/// What a sync did, for every provider in the output.
#[derive(Debug, Default)]
pub struct Report {
    pub before: usize,
    pub after: usize,
    pub providers: Vec<ProviderReport>,
}

/// The output of [`sync`].
#[derive(Debug)]
pub struct Sync {
    pub models: Vec<CatalogModel>,
    pub report: Report,
}

/// A copyable (base URL, headers, compat) triple for one (provider, api).
#[derive(Debug, Clone, PartialEq)]
struct Template {
    base_url: String,
    headers: Option<Value>,
    compat: Option<Value>,
}

/// Merges models.dev into the baseline. See the module docs for the rules.
///
/// Errors (the output is not produced) when a mapped models.dev provider is
/// missing, which means the upstream shape or the [`SOURCES`] table changed.
/// A silent no-op would look like a clean sync.
pub fn sync(
    baseline: &[CatalogModel],
    upstream: &Upstream,
    overrides: &Overrides,
) -> Result<Sync, String> {
    let providers = provider_order(baseline);
    for (hoocode, source) in SOURCES {
        if providers.iter().any(|p| p.as_str() == *hoocode) && !upstream.contains_key(*source) {
            return Err(format!(
                "models.dev has no provider `{source}` (hoocode `{hoocode}`); \
                 check SOURCES in crates/hoocode-models-sync/src/lib.rs"
            ));
        }
    }
    for add in &overrides.add {
        if !providers.contains(&add.provider) {
            return Err(format!(
                "overrides.json adds {}/{}, but the catalog has no such provider",
                add.provider, add.id
            ));
        }
    }
    for provider in overrides.allow.keys() {
        if !providers.contains(provider) {
            return Err(format!(
                "overrides.json allows models for unknown provider `{provider}`"
            ));
        }
    }

    let templates = templates(baseline);
    let apis = provider_apis(baseline);
    let mut models = Vec::new();
    let mut report = Report {
        before: baseline.len(),
        ..Report::default()
    };

    for provider in &providers {
        let source = source_of(provider);
        let upstream_provider = upstream.get(source);
        let mut rep = ProviderReport {
            name: provider.clone(),
            source: upstream_provider.map(|_| source.to_string()),
            ..ProviderReport::default()
        };
        let excluded = |id: &str| {
            overrides
                .exclude
                .iter()
                .any(|k| k.provider == *provider && k.id == id)
        };

        rep.prices_kept = overrides.keep_prices.contains(provider);
        let mut block = Vec::new();
        let mut known: BTreeSet<&str> = BTreeSet::new();
        for entry in baseline.iter().filter(|m| m.provider == *provider) {
            known.insert(entry.id.as_str());
            if excluded(&entry.id) {
                rep.excluded.push(entry.id.clone());
                continue;
            }
            match upstream_provider.and_then(|p| p.models.get(&entry.id)) {
                Some(upstream_model) => block.push(refresh(entry, upstream_model, &mut rep)),
                None => {
                    if upstream_provider.is_some() {
                        rep.kept.push(entry.id.clone());
                    }
                    block.push(entry.clone());
                }
            }
        }

        if let Some(up) = upstream_provider {
            for (id, upstream_model) in &up.models {
                if known.contains(id.as_str()) || excluded(id) {
                    continue;
                }
                let allow = overrides.allow.get(provider);
                match new_entry(
                    provider,
                    id,
                    upstream_model,
                    up.npm.as_deref(),
                    allow,
                    &templates,
                    &apis,
                ) {
                    Ok(model) => {
                        let mark = match upstream_model.cost {
                            None => " (no price on models.dev: 0)",
                            Some(_) if model.cost == Cost::default() => " (zero price)",
                            Some(_) => "",
                        };
                        rep.added.push(format!("{id}{mark}"));
                        block.push(model);
                    }
                    Err(reason) => *rep.skipped.entry(reason).or_default() += 1,
                }
            }
        }

        for add in overrides.add.iter().filter(|a| a.provider == *provider) {
            let present = block.iter().any(|m| m.id == add.id);
            if !present && !excluded(&add.id) {
                rep.added.push(add.id.clone());
                block.push(add.clone());
            }
        }

        report.after += block.len();
        models.extend(block);
        report.providers.push(rep);
    }

    Ok(Sync { models, report })
}

/// Existing entry + models.dev entry -> the entry to write.
fn refresh(entry: &CatalogModel, up: &UpModel, rep: &mut ProviderReport) -> CatalogModel {
    let mut model = entry.clone();
    if let Some(cost) = &up.cost {
        let fresh = Cost {
            input: pick_price(model.cost.input, cost.input),
            output: pick_price(model.cost.output, cost.output),
            cache_read: pick_price(model.cost.cache_read, cost.cache_read),
            cache_write: pick_price(model.cost.cache_write, cost.cache_write),
        };
        if !same_cost(&fresh, &model.cost) {
            if rep.prices_kept {
                rep.price_drift.push(model.id.clone());
            } else {
                rep.cost_changed.push(model.id.clone());
                model.cost = fresh;
            }
        }
    }
    let mut drift = Vec::new();
    if up.reasoning != model.reasoning {
        drift.push(format!("reasoning {} -> {}", model.reasoning, up.reasoning));
    }
    let input = input_of(up);
    if input != model.input {
        drift.push(format!("input {:?} -> {:?}", model.input, input));
    }
    if let Some(ctx) = up.limit.as_ref().and_then(|l| l.context) {
        if ctx != model.context_window {
            drift.push(format!("contextWindow {} -> {ctx}", model.context_window));
        }
    }
    if let Some(max) = up.limit.as_ref().and_then(|l| l.output) {
        if max != model.max_tokens {
            drift.push(format!("maxTokens {} -> {max}", model.max_tokens));
        }
    }
    for d in drift {
        rep.drift.push(format!("{}: {d}", model.id));
    }
    model
}

/// A models.dev model hoocode does not have yet -> a new entry, or the reason
/// it is skipped.
///
/// The API type comes from the model's `npm` package (or the provider's). The
/// base URL, headers and compat come from the existing entries with the same
/// provider and api, most common first. `thinkingLevelMap` is left unset: it is
/// hand-tuned per model, so review new reasoning models for it.
fn new_entry(
    provider: &str,
    id: &str,
    up: &UpModel,
    provider_npm: Option<&str>,
    allow: Option<&Vec<String>>,
    templates: &BTreeMap<(String, String), Template>,
    apis: &BTreeMap<String, BTreeSet<String>>,
) -> Result<CatalogModel, String> {
    if let Some(allowed) = allow {
        if !allowed.iter().any(|a| a == id) {
            return Err("not in overrides.json allow list".into());
        }
    }
    if !up.tool_call {
        return Err("not a tool-calling model".into());
    }
    if up.status.as_deref() == Some("deprecated") {
        return Err("deprecated upstream".into());
    }
    let text_only = up
        .modalities
        .as_ref()
        .and_then(|m| m.output.as_ref())
        .is_none_or(|out| out.iter().all(|m| m == "text"));
    if !text_only {
        return Err("output is not text only".into());
    }
    let npm = up
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .or(provider_npm);
    let api = match npm {
        Some(npm) => NPM_APIS
            .iter()
            .find(|(name, _)| *name == npm)
            .map(|(_, api)| (*api).to_string())
            .ok_or_else(|| format!("npm {npm} has no hoocode api type"))?,
        None => match apis.get(provider) {
            Some(set) if set.len() == 1 => set.iter().next().cloned().unwrap_or_default(),
            _ => return Err("no npm package to tell the api type".into()),
        },
    };
    let template = templates
        .get(&(provider.to_string(), api.clone()))
        .ok_or_else(|| {
            format!("no existing {api} entry in {provider} to copy the base URL from")
        })?;
    let context = up.limit.as_ref().and_then(|l| l.context);
    let max = up.limit.as_ref().and_then(|l| l.output);
    let (Some(context_window), Some(max_tokens)) = (context, max) else {
        return Err("no context or output limit".into());
    };
    if max_tokens > context_window {
        return Err("output limit above the context window".into());
    }
    Ok(CatalogModel {
        id: id.to_string(),
        name: up.name.clone().unwrap_or_else(|| id.to_string()),
        api,
        provider: provider.to_string(),
        base_url: template.base_url.clone(),
        headers: template.headers.clone(),
        compat: template.compat.clone(),
        reasoning: up.reasoning,
        thinking_level_map: None,
        input: input_of(up),
        cost: up
            .cost
            .as_ref()
            .map_or_else(Cost::default, Cost::from_upstream),
        context_window,
        max_tokens,
    })
}

/// One price: models.dev's value when it is a real, different price; otherwise
/// hoocode's. Missing and zero mean "no data" here (see the module docs).
fn pick_price(existing: f64, upstream: Option<f64>) -> f64 {
    match upstream {
        Some(v) if v != 0.0 && !same_number(existing, v) => v,
        _ => existing,
    }
}

fn same_cost(a: &Cost, b: &Cost) -> bool {
    same_number(a.input, b.input)
        && same_number(a.output, b.output)
        && same_number(a.cache_read, b.cache_read)
        && same_number(a.cache_write, b.cache_write)
}

/// Equal up to float noise (`0.7999999999999999` and `0.8` are one price).
fn same_number(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// models.dev input modalities -> hoocode's `input`. hoocode only knows text and image.
fn input_of(up: &UpModel) -> Vec<String> {
    let image = up
        .modalities
        .as_ref()
        .is_some_and(|m| m.input.iter().any(|i| i == "image"));
    if image {
        vec!["text".into(), "image".into()]
    } else {
        vec!["text".into()]
    }
}

/// The models.dev provider for a hoocode provider.
fn source_of(provider: &str) -> &str {
    SOURCES
        .iter()
        .find(|(hoocode, _)| *hoocode == provider)
        .map_or(provider, |(_, source)| *source)
}

/// Providers in the baseline's order, without the dropped ones.
fn provider_order(baseline: &[CatalogModel]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in baseline {
        if DROPPED_PROVIDERS.contains(&m.provider.as_str()) || out.contains(&m.provider) {
            continue;
        }
        out.push(m.provider.clone());
    }
    out
}

/// The most common (base URL, headers, compat) per (provider, api). Ties go to
/// the first one in the file.
fn templates(baseline: &[CatalogModel]) -> BTreeMap<(String, String), Template> {
    let mut counts: BTreeMap<(String, String), Vec<(Template, usize)>> = BTreeMap::new();
    for m in baseline {
        let t = Template {
            base_url: m.base_url.clone(),
            headers: m.headers.clone(),
            compat: m.compat.clone(),
        };
        let list = counts
            .entry((m.provider.clone(), m.api.clone()))
            .or_default();
        match list.iter_mut().find(|(seen, _)| *seen == t) {
            Some((_, n)) => *n += 1,
            None => list.push((t, 1)),
        }
    }
    counts
        .into_iter()
        .filter_map(|(key, list)| {
            let mut best: Option<(Template, usize)> = None;
            for (t, n) in list {
                if best.as_ref().is_none_or(|(_, b)| n > *b) {
                    best = Some((t, n));
                }
            }
            best.map(|(t, _)| (key, t))
        })
        .collect()
}

/// The api types each provider uses in the baseline.
fn provider_apis(baseline: &[CatalogModel]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in baseline {
        out.entry(m.provider.clone())
            .or_default()
            .insert(m.api.clone());
    }
    out
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

impl Report {
    /// The text the tool prints and the PR body quotes. Plain lines, no colour.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let added: usize = self.providers.iter().map(|p| p.added.len()).sum();
        let removed: usize = self.providers.iter().map(|p| p.excluded.len()).sum();
        let changed: usize = self.providers.iter().map(|p| p.cost_changed.len()).sum();
        let _ = writeln!(
            out,
            "models.json: {} -> {} entries (+{added} added, -{removed} excluded, {changed} prices changed)",
            self.before, self.after
        );
        for p in &self.providers {
            let source = match &p.source {
                Some(s) if *s == p.name => String::new(),
                Some(s) => format!(" (from models.dev `{s}`)"),
                None => " (not on models.dev: kept as is)".to_string(),
            };
            let _ = writeln!(out, "\n{}{source}", p.name);
            if p.prices_kept {
                let _ = writeln!(out, "  prices kept: listed in keep_prices");
            }
            if !p.added.is_empty() {
                let _ = writeln!(out, "  added {}: {}", p.added.len(), list(&p.added));
            }
            if !p.cost_changed.is_empty() {
                let _ = writeln!(
                    out,
                    "  prices changed {}: {}",
                    p.cost_changed.len(),
                    list(&p.cost_changed)
                );
            }
            if !p.price_drift.is_empty() {
                let _ = writeln!(
                    out,
                    "  prices differ upstream, not applied ({}): {}",
                    p.price_drift.len(),
                    list(&p.price_drift)
                );
            }
            if !p.excluded.is_empty() {
                let _ = writeln!(
                    out,
                    "  excluded {}: {}",
                    p.excluded.len(),
                    list(&p.excluded)
                );
            }
            if !p.kept.is_empty() {
                let _ = writeln!(
                    out,
                    "  kept {} not listed upstream any more: {}",
                    p.kept.len(),
                    list(&p.kept)
                );
            }
            for (reason, n) in &p.skipped {
                let _ = writeln!(out, "  skipped {n}: {reason}");
            }
            if !p.drift.is_empty() {
                let _ = writeln!(out, "  differs upstream, not applied ({}):", p.drift.len());
                for d in &p.drift {
                    let _ = writeln!(out, "    {d}");
                }
            }
        }
        out
    }
}

/// A comma list, cut at 12 items so a provider's line stays readable.
fn list(items: &[String]) -> String {
    const SHOWN: usize = 12;
    let mut out = items
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > SHOWN {
        let _ = write!(out, ", and {} more", items.len() - SHOWN);
    }
    out
}
