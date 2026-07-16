/**
 * Model categories for subagent model selection.
 *
 * A category (`fast` | `standard` | `capable`) is a provider-neutral indirection
 * that maps to an explicit model id. Precedence:
 *
 *   1. An explicit `settings.modelCategories[category]` always wins.
 *   2. Otherwise, when a set of available models is supplied, the category
 *      resolves to a default *derived* from those models (see
 *      `deriveDefaultModelCategories`) — never a hardcoded provider/model id.
 *   3. Otherwise it resolves to `undefined`, which callers treat as "no override"
 *      and fall back to the agent's or parent's default model.
 *
 * No concrete model names are baked in here, so the feature never assumes a
 * particular provider.
 */

import type { Api, Model } from "@kolisachint/hoocode-ai";
import type { Settings } from "./settings-manager.js";

/** Valid model category names */
export type ModelCategory = "fast" | "standard" | "capable";

/** Check if a string is a valid model category */
export function isModelCategory(value: string): value is ModelCategory {
	return value === "fast" || value === "standard" || value === "capable";
}

/** A category maps to a concrete model reference in `<provider>/<id>` form. */
function modelRef(model: Model<Api>): string {
	return `${model.provider}/${model.id}`;
}

/** Combined per-token price (input + output), used as a capability/cost proxy. */
function combinedPrice(model: Model<Api>): number {
	return model.cost.input + model.cost.output;
}

/** Deterministic tie-break so identical available sets always yield the same pick. */
function compareById(a: Model<Api>, b: Model<Api>): number {
	return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

/**
 * Derive a default model for each tier from the user's available models, used
 * only when a tier is not explicitly configured in `settings.modelCategories`.
 *
 * The rule is deliberately transparent (config, not magic) and provider-neutral:
 * nothing is hardcoded, everything is derived from what the user actually has.
 *
 *   1. `capable` = the user's PRIMARY model: the configured default
 *      (`settings.defaultProvider`/`defaultModel`) when it is in the available
 *      set, otherwise the most capable available model, using combined token
 *      price (input + output cost) as a stand-in for capability.
 *   2. `fast` and `standard` are the cheapest and the upper-median of every
 *      available model priced at or below `capable`, ordered cheapest-first.
 *      Clamping to `capable`'s price keeps the tiers monotonic
 *      (`fast` <= `standard` <= `capable`), and drawing from the whole available
 *      set — not just `capable`'s own provider — still yields a genuinely cheap
 *      `fast` when the primary model's provider has nothing cheaper (a strict
 *      same-provider rule collapses every tier onto a single-model provider).
 *
 * Every ordering breaks ties on a fixed key (context window, then id) so the same
 * available set always yields the same mapping. An empty available set yields an
 * empty map (every tier resolves to `undefined`, i.e. inherit the parent model).
 */
export function deriveDefaultModelCategories(
	availableModels: readonly Model<Api>[],
	settings?: Settings,
): { fast?: string; standard?: string; capable?: string } {
	if (availableModels.length === 0) return {};

	// capable = primary model.
	const configuredDefault =
		settings?.defaultProvider && settings?.defaultModel
			? availableModels.find((m) => m.provider === settings.defaultProvider && m.id === settings.defaultModel)
			: undefined;
	// Most capable = highest combined price; ties -> larger context window, then id.
	const capable =
		configuredDefault ??
		[...availableModels].sort(
			(a, b) => combinedPrice(b) - combinedPrice(a) || b.contextWindow - a.contextWindow || compareById(a, b),
		)[0];

	// fast/standard: every model priced at or below capable, cheapest-first.
	// `capable` is always in this set (its price <= its own price), so it never empties.
	const capablePrice = combinedPrice(capable);
	const candidates = availableModels
		.filter((m) => combinedPrice(m) <= capablePrice)
		.sort((a, b) => combinedPrice(a) - combinedPrice(b) || a.contextWindow - b.contextWindow || compareById(a, b));

	const fast = candidates[0] ?? capable;
	const standard = candidates[Math.floor(candidates.length / 2)] ?? capable;

	return {
		fast: modelRef(fast),
		standard: modelRef(standard),
		capable: modelRef(capable),
	};
}

/**
 * Resolve a model category to a model id. An explicit
 * `settings.modelCategories[category]` wins; otherwise a default is derived from
 * `availableModels` (provider-neutral, see `deriveDefaultModelCategories`); when
 * neither applies the category resolves to `undefined` (a no-op, so the caller
 * keeps its existing model).
 *
 * @param category - The model category (fast, standard, capable)
 * @param settings - The current settings (may contain modelCategories config)
 * @param availableModels - The user's available/configured models to derive from
 */
export function resolveModelCategory(
	category: ModelCategory,
	settings?: Settings,
	availableModels?: readonly Model<Api>[],
): string | undefined {
	const explicit = settings?.modelCategories?.[category];
	if (explicit) return explicit;
	if (availableModels && availableModels.length > 0) {
		return deriveDefaultModelCategories(availableModels, settings)[category];
	}
	return undefined;
}

/**
 * Resolve a model string that might be a category reference. A category resolves
 * to its configured or derived model id (or `undefined` when neither applies);
 * any other string is already a concrete model id or alias and is returned as-is.
 *
 * @param model - The model string (could be a category, alias, or full model ID)
 * @param settings - The current settings (may contain modelCategories config)
 * @param availableModels - The user's available/configured models to derive from
 */
export function resolveModelReference(
	model: string,
	settings?: Settings,
	availableModels?: readonly Model<Api>[],
): string | undefined {
	if (isModelCategory(model)) {
		return resolveModelCategory(model, settings, availableModels);
	}
	return model;
}
