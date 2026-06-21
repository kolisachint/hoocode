/**
 * Model categories for subagent model selection.
 *
 * Categories map to explicit model IDs configured in settings.
 * When a category is not configured, fallback defaults are used.
 */

import type { Settings } from "./settings-manager.js";

/** Valid model category names */
export type ModelCategory = "fast" | "standard" | "capable";

/** Check if a string is a valid model category */
export function isModelCategory(value: string): value is ModelCategory {
	return value === "fast" || value === "standard" || value === "capable";
}

/**
 * Fallback default models for each category when not configured.
 * These are provider-agnostic patterns that will be resolved via model matching.
 */
const CATEGORY_FALLBACKS: Record<ModelCategory, string> = {
	fast: "haiku",
	standard: "sonnet",
	capable: "opus",
};

/**
 * Resolve a model category to an actual model ID.
 *
 * @param category - The model category (fast, standard, capable)
 * @param settings - The current settings (may contain modelCategories config)
 * @returns The resolved model ID or pattern
 */
export function resolveModelCategory(category: ModelCategory, settings?: Settings): string {
	// Check if the category is configured in settings
	const configured = settings?.modelCategories?.[category];
	if (configured) {
		return configured;
	}

	// Use fallback default
	return CATEGORY_FALLBACKS[category];
}

/**
 * Resolve a model string that might be a category reference.
 *
 * @param model - The model string (could be a category, alias, or full model ID)
 * @param settings - The current settings (may contain modelCategories config)
 * @returns The resolved model ID or pattern
 */
export function resolveModelReference(model: string, settings?: Settings): string {
	// If it's a category, resolve it
	if (isModelCategory(model)) {
		return resolveModelCategory(model, settings);
	}

	// Otherwise return as-is (it's already a model ID or alias)
	return model;
}
