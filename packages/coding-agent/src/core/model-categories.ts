/**
 * Model categories for subagent model selection.
 *
 * A category (`fast` | `standard` | `capable`) is a provider-neutral indirection
 * that maps to an explicit model id via `settings.modelCategories`. No concrete
 * model names are baked in here, so the feature never assumes a particular
 * provider. When a category is not configured, it resolves to `undefined` —
 * callers treat that as "no override" and fall back to the agent's or parent's
 * default model.
 */

import type { Settings } from "./settings-manager.js";

/** Valid model category names */
export type ModelCategory = "fast" | "standard" | "capable";

/** Check if a string is a valid model category */
export function isModelCategory(value: string): value is ModelCategory {
	return value === "fast" || value === "standard" || value === "capable";
}

/**
 * Resolve a model category to the model id configured for it in settings, or
 * `undefined` when the category is not configured. There is no built-in default:
 * an unconfigured category is a no-op so the caller keeps its existing model.
 *
 * @param category - The model category (fast, standard, capable)
 * @param settings - The current settings (may contain modelCategories config)
 */
export function resolveModelCategory(category: ModelCategory, settings?: Settings): string | undefined {
	return settings?.modelCategories?.[category];
}

/**
 * Resolve a model string that might be a category reference. A category resolves
 * to its configured model id (or `undefined` when unconfigured); any other string
 * is already a concrete model id or alias and is returned as-is.
 *
 * @param model - The model string (could be a category, alias, or full model ID)
 * @param settings - The current settings (may contain modelCategories config)
 */
export function resolveModelReference(model: string, settings?: Settings): string | undefined {
	if (isModelCategory(model)) {
		return resolveModelCategory(model, settings);
	}
	return model;
}
