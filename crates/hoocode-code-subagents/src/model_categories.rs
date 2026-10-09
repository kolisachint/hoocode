//! `core/model-categories.ts`: `fast` | `standard` | `capable` model
//! categories for subagent model selection, and the scoped-model selection
//! that sits on top of them ([`select_model`]).
//!
//! Without a scope, the category resolves to a default derived from the
//! available models (never a hardcoded provider/model); otherwise it is `None`
//! ("no override"). With a scope (`scopedModels`), subagents may only run on a
//! scoped model, and categories pick from the scope (see [`select_model`]).
//! The retired `modelCategories` setting is no longer read; the migration
//! moved it into `scopedModels`.

use std::cmp::Ordering;

use hoocode_ai_types::{Model, ThinkingLevel};
use hoocode_code_models::{
    clamp_effort, match_scoped_model, parse_thinking_level, pick_by_category, ResolvedScoped,
};
use hoocode_code_settings::ModelCategoryName;

/// `ModelCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCategory {
    Fast,
    Standard,
    Capable,
}

impl ModelCategory {
    /// `isModelCategory`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fast" => Some(Self::Fast),
            "standard" => Some(Self::Standard),
            "capable" => Some(Self::Capable),
            _ => None,
        }
    }

    /// The tier's name, as it appears in settings and in the tool schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Standard => "standard",
            Self::Capable => "capable",
        }
    }
}

/// `isModelCategory`.
pub fn is_model_category(value: &str) -> bool {
    ModelCategory::parse(value).is_some()
}

/// The settings the resolution reads (`Settings` fields).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategorySettings {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
}

impl CategorySettings {
    /// From a settings object (`defaultProvider`, `defaultModel`).
    pub fn from_settings(settings: &serde_json::Map<String, serde_json::Value>) -> Self {
        let text = |value: Option<&serde_json::Value>| {
            value
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        Self {
            default_provider: text(settings.get("defaultProvider")),
            default_model: text(settings.get("defaultModel")),
        }
    }
}

/// `deriveDefaultModelCategories` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DerivedCategories {
    pub fast: Option<String>,
    pub standard: Option<String>,
    pub capable: Option<String>,
}

impl DerivedCategories {
    fn get(&self, category: ModelCategory) -> Option<String> {
        match category {
            ModelCategory::Fast => self.fast.clone(),
            ModelCategory::Standard => self.standard.clone(),
            ModelCategory::Capable => self.capable.clone(),
        }
    }
}

fn model_ref(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

/// Combined per-token price, a capability/cost proxy.
fn combined_price(model: &Model) -> f64 {
    model.cost.input + model.cost.output
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Derive a default per tier from the available models:
/// `capable` is the configured default model when available, else the
/// priciest (ties: larger context window, then id); `fast` and `standard` are
/// the cheapest and the upper median of the models priced at or below
/// `capable` (ties: smaller context window, then id).
pub fn derive_default_model_categories(
    available: &[Model],
    settings: Option<&CategorySettings>,
) -> DerivedCategories {
    if available.is_empty() {
        return DerivedCategories::default();
    }
    let configured = settings.and_then(|s| match (&s.default_provider, &s.default_model) {
        (Some(provider), Some(model)) => available
            .iter()
            .find(|m| &m.provider == provider && &m.id == model),
        _ => None,
    });
    let capable = configured.unwrap_or_else(|| {
        let mut sorted: Vec<&Model> = available.iter().collect();
        sorted.sort_by(|a, b| {
            cmp_f64(combined_price(b), combined_price(a))
                .then(b.context_window.cmp(&a.context_window))
                .then(a.id.cmp(&b.id))
        });
        sorted[0]
    });
    let capable_price = combined_price(capable);
    let mut candidates: Vec<&Model> = available
        .iter()
        .filter(|m| combined_price(m) <= capable_price)
        .collect();
    candidates.sort_by(|a, b| {
        cmp_f64(combined_price(a), combined_price(b))
            .then(a.context_window.cmp(&b.context_window))
            .then(a.id.cmp(&b.id))
    });
    let fast = candidates.first().copied().unwrap_or(capable);
    let standard = candidates
        .get(candidates.len() / 2)
        .copied()
        .unwrap_or(capable);
    DerivedCategories {
        fast: Some(model_ref(fast)),
        standard: Some(model_ref(standard)),
        capable: Some(model_ref(capable)),
    }
}

/// `resolveModelCategory`: the tier's derived default from the available models,
/// or `None` when there are none.
pub fn resolve_model_category(
    category: ModelCategory,
    settings: Option<&CategorySettings>,
    available: Option<&[Model]>,
) -> Option<String> {
    match available {
        Some(models) if !models.is_empty() => {
            derive_default_model_categories(models, settings).get(category)
        }
        _ => None,
    }
}

/// `resolveModelReference`: a category resolves (or is `None`); anything else
/// is a concrete model id or alias and passes through.
pub fn resolve_model_reference(
    model: &str,
    settings: Option<&CategorySettings>,
    available: Option<&[Model]>,
) -> Option<String> {
    match ModelCategory::parse(model) {
        Some(category) => resolve_model_category(category, settings, available),
        None => Some(model.to_string()),
    }
}

/// What a dispatch asks for, and what its agent definition and the dispatching
/// session offer. The pool fills `pin` from the agent definition itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelRequest {
    /// The Agent tool's `model`: a category, a scoped alias or id, or a model id.
    pub ask: Option<String>,
    /// The Agent tool's `effort`: a thinking level name.
    pub effort: Option<String>,
    /// The agent definition's `model:`. A default, not a pin: an explicit `ask`
    /// beats it. `inherit` and an empty value are `None`.
    pub pin: Option<String>,
    /// The dispatching session's own model (`provider/id`): the default when
    /// nothing else applies, and the model an inherited-model retry runs on.
    pub inherited: Option<String>,
}

/// The model a dispatch runs on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelSelection {
    /// `provider/id` for `--model`. `None` only when there is no scope and
    /// nothing was asked for: the child then uses its own default.
    pub model: Option<String>,
    /// The scoped alias of `model`, when it is a scoped model.
    pub alias: Option<String>,
    /// The thinking level for `--thinking`: the explicit `effort`, else the
    /// scoped entry's effort, clamped to `model`.
    pub effort: Option<ThinkingLevel>,
    /// Why the choice is not the ask as written: a tier fallback, or a pin or
    /// the session's model that is outside the scope.
    pub note: Option<String>,
    /// The ask could not be used (unknown name, or no model in the asked tier).
    /// The selection then holds the default, which is still within the scope.
    pub error: Option<String>,
}

/// The `cheap` tier has no derived default of its own; without a scope it
/// behaves as `fast`.
fn derived_category(tier: ModelCategoryName) -> ModelCategory {
    match tier {
        ModelCategoryName::Cheap | ModelCategoryName::Fast => ModelCategory::Fast,
        ModelCategoryName::Standard => ModelCategory::Standard,
        ModelCategoryName::Capable => ModelCategory::Capable,
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// `provider/id` of a scoped model.
fn scoped_ref(scoped: &ResolvedScoped) -> String {
    format!("{}/{}", scoped.model.provider, scoped.model.id)
}

/// The entry a tier ask lands on (decisions 10, 13 and 14). When some scoped
/// entry carries a category, the tagged entries decide ([`pick_by_category`]).
/// When none does (e.g. a `--models` list), the tier is derived from the scoped
/// models alone, the same way the unscoped default is derived from the
/// available models. The flag is true when a neighbouring tier was used.
fn pick_tier<'a>(
    tier: ModelCategoryName,
    settings: Option<&CategorySettings>,
    scope: &'a [ResolvedScoped],
) -> Option<(&'a ResolvedScoped, bool)> {
    if scope.iter().any(|s| s.category.is_some()) {
        return pick_by_category(tier, scope);
    }
    let models: Vec<Model> = scope.iter().map(|s| s.model.clone()).collect();
    let reference =
        derive_default_model_categories(&models, settings).get(derived_category(tier))?;
    scope
        .iter()
        .find(|s| scoped_ref(s) == reference)
        .map(|s| (s, false))
}

/// A pick before effort is applied.
struct Pick<'a> {
    model: Option<String>,
    alias: Option<String>,
    /// The scoped entry's effort (scoped picks only).
    entry_effort: Option<ThinkingLevel>,
    /// The model the effort is clamped to, when it is known.
    info: Option<&'a Model>,
    note: Option<String>,
}

fn tier_note(tier: ModelCategoryName, scoped: &ResolvedScoped) -> String {
    let used = scoped.category.map(|c| c.as_str()).unwrap_or("untagged");
    format!("no scoped {tier} model, so the {used} tier was used")
}

fn scoped_pick(scoped: &ResolvedScoped, note: Option<String>) -> Pick<'_> {
    Pick {
        model: Some(scoped_ref(scoped)),
        alias: scoped.alias.clone(),
        entry_effort: scoped.effort.clone(),
        info: Some(&scoped.model),
        note,
    }
}

/// A model reference outside any scope, with its `Model` when it is available.
fn concrete_pick(reference: Option<String>, available: &[Model]) -> Pick<'_> {
    let info = reference.as_deref().and_then(|r| {
        available
            .iter()
            .find(|m| m.id == r || format!("{}/{}", m.provider, m.id) == r)
    });
    Pick {
        model: reference,
        alias: None,
        entry_effort: None,
        info,
        note: None,
    }
}

/// Whether an unscoped ask names an available model (by id, `provider/id`, or a
/// substring of the id). Only checked when the available models are known.
fn names_available_model(ask: &str, available: &[Model]) -> bool {
    available
        .iter()
        .any(|m| m.id == ask || format!("{}/{}", m.provider, m.id) == ask || m.id.contains(ask))
}

/// The explicit ask: a category, a scoped name, or a model reference.
fn pick_ask<'a>(
    ask: &str,
    settings: Option<&CategorySettings>,
    available: &'a [Model],
    scope: &'a [ResolvedScoped],
) -> Result<Pick<'a>, String> {
    if let Some(tier) = ModelCategoryName::parse(ask) {
        if scope.is_empty() {
            let reference =
                resolve_model_category(derived_category(tier), settings, Some(available));
            return Ok(concrete_pick(reference, available));
        }
        return match pick_tier(tier, settings, scope) {
            Some((scoped, fallback)) => Ok(scoped_pick(
                scoped,
                fallback.then(|| tier_note(tier, scoped)),
            )),
            None => Err(format!(
                "no scoped model has the \"{tier}\" category. Ask for a scoped model by alias or id, or omit model."
            )),
        };
    }
    if scope.is_empty() {
        if !available.is_empty() && !names_available_model(ask, available) {
            return Err(format!(
                "unknown model \"{ask}\". Use a category (cheap, fast, standard, capable), or a model id or alias."
            ));
        }
        return Ok(concrete_pick(Some(ask.to_string()), available));
    }
    match_scoped_model(ask, scope).map(|scoped| scoped_pick(scoped, None))
}

/// The agent's `model:` pin, used when nothing was asked. Inside a scope a pin
/// that is not a scoped model yields `None` and the caller falls back.
fn pick_pin<'a>(
    pin: &str,
    settings: Option<&CategorySettings>,
    available: &'a [Model],
    scope: &'a [ResolvedScoped],
) -> Option<Pick<'a>> {
    if scope.is_empty() {
        // `cheap` has no derived default of its own, so it reads as `fast`.
        let reference = match ModelCategoryName::parse(pin) {
            Some(tier) => resolve_model_category(derived_category(tier), settings, Some(available)),
            None => resolve_model_reference(pin, settings, Some(available)),
        };
        return Some(concrete_pick(reference, available));
    }
    if let Some(tier) = ModelCategoryName::parse(pin) {
        return pick_tier(tier, settings, scope).map(|(scoped, fallback)| {
            scoped_pick(scoped, fallback.then(|| tier_note(tier, scoped)))
        });
    }
    match_scoped_model(pin, scope)
        .ok()
        .map(|scoped| scoped_pick(scoped, None))
}

/// The default: without a scope, the session's model (or the child's own
/// default when there is none), as before scopes existed. Inside a scope, the
/// session's model when it is scoped, else the standard tier, else the first
/// scoped model.
fn pick_default<'a>(
    inherited: Option<&str>,
    available: &'a [Model],
    scope: &'a [ResolvedScoped],
) -> Pick<'a> {
    if scope.is_empty() {
        return concrete_pick(inherited.map(String::from), available);
    }
    if let Some(scoped) = inherited.and_then(|m| scope.iter().find(|s| scoped_ref(s) == m)) {
        return scoped_pick(scoped, None);
    }
    let note = inherited.map(|m| format!("the session's model {m} is not scoped"));
    let scoped = pick_by_category(ModelCategoryName::Standard, scope)
        .map(|(scoped, _)| scoped)
        .unwrap_or(&scope[0]);
    scoped_pick(scoped, note)
}

/// Chooses the model and effort for one dispatch (decisions 6 to 14 and 17 of
/// the scoped-models design).
///
/// Order: an explicit ask beats the agent's `model:` pin, and the pin beats the
/// session's model. A non-empty `scope` is a hard limit: subagents run only on
/// scoped models. An ask that cannot be used sets `error`, and the selection
/// then holds the default. Effort is the explicit `effort`, else the scoped
/// entry's effort, clamped to the model.
pub fn select_model(
    request: &ModelRequest,
    settings: Option<&CategorySettings>,
    available: &[Model],
    scope: &[ResolvedScoped],
) -> ModelSelection {
    let mut selection = ModelSelection::default();
    let mut found = None;
    if let Some(ask) = non_empty(request.ask.as_deref()) {
        match pick_ask(ask, settings, available, scope) {
            Ok(pick) => found = Some(pick),
            Err(error) => selection.error = Some(error),
        }
    }
    let pick = match found {
        Some(pick) => pick,
        None => {
            let mut pin_note = None;
            let pinned = non_empty(request.pin.as_deref()).and_then(|pin| {
                let pick = pick_pin(pin, settings, available, scope);
                if pick.is_none() {
                    pin_note = Some(format!("the agent's model {pin} is not a scoped model"));
                }
                pick
            });
            let mut pick = pinned
                .unwrap_or_else(|| pick_default(request.inherited.as_deref(), available, scope));
            pick.note = match (pin_note, pick.note.take()) {
                (Some(a), Some(b)) => Some(format!("{a}; {b}")),
                (a, b) => a.or(b),
            };
            pick
        }
    };
    let explicit = non_empty(request.effort.as_deref()).and_then(parse_thinking_level);
    let level = explicit.or(pick.entry_effort.clone());
    selection.effort = match (level, pick.info) {
        (Some(level), Some(model)) => Some(clamp_effort(level, model)),
        (level, _) => level,
    };
    selection.model = pick.model;
    selection.alias = pick.alias;
    selection.note = pick.note;
    selection
}
