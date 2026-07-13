/**
 * Prompt-reactive reuse policy — the single source of truth for the runtime
 * "plugin reuse nudge".
 *
 * The plugin-productivity guidance in `core/tools/plugins.ts` and
 * `core/tools/propose-plugin.ts` ships as static `promptGuidelines` folded into
 * the system prompt once at session build. That text is never re-surfaced when
 * the work actually shows an actionable cue, so the model tends to repeat the
 * same advice by hand instead of reaching for the plugin layer.
 *
 * This module holds the reactive policy that fixes that:
 *   - {@link REUSE_NUDGES}       the curated cue → nudge table
 *   - {@link matchReuseNudges}   scan runtime text (tool output, turn text) for cues
 *   - the armed registry         nudges that fired this process, so the plugin
 *                                layer can surface them even when no tool asked
 *   - {@link isAutonomousPluginSystemEnabled}
 *                                the one enablement gate shared with the
 *                                tool-attach path in main.ts (settings.json
 *                                `enablePluginTools`)
 *
 * The extension in `./nudges.ts` wires this policy to tool/turn events; the
 * policy itself stays pure and independently testable.
 *
 * Conservative by construction: the patterns are specific (a false positive
 * becoming a nag is the real risk), each nudge fires at most once per category
 * per session, and every nudge points back at the plugin layer rather than
 * re-stating advice.
 */

import { getAgentDir } from "../../../config.js";
import { SettingsManager } from "../../../core/settings-manager.js";

/** A single reactive reuse cue and the plugin-facing nudge it arms. */
export interface ReuseNudge {
	/** Stable id, also used as the de-dupe key so a nudge fires once per session. */
	id: string;
	/** Human-facing category label (grouping for the one-per-category cap). */
	category: string;
	/** Specific cue matched against runtime text. Kept tight to avoid false positives. */
	pattern: RegExp;
	/** The reusable-facing note injected into the turn; always points at the plugin layer. */
	snippet: string;
}

/**
 * Curated cue → nudge table. Seeded from the static plugin-reuse guidance, but
 * keyed on concrete cues that show up in real work (style/policy directives,
 * format standardization, explicit capability gaps). Patterns are deliberately
 * specific — first-hit arming (see nudges.ts) only stays safe if the cue is
 * unambiguous.
 */
export const REUSE_NUDGES: readonly ReuseNudge[] = [
	{
		id: "style-active-voice",
		category: "writing-style",
		pattern: /\b(?:in\s+)?active voice\b|\bavoid(?:ing)?\s+passive voice\b/i,
		snippet:
			'A writing-style rule is in play ("active voice"). If this is a recurring convention, it can live as a reusable skill/command instead of being re-applied by hand — SearchPlugins for one, or author it with ProposePlugin.',
	},
	{
		id: "style-avoid-repetition",
		category: "writing-style",
		pattern: /\bavoid(?:ing)?\s+repetition\b|\bdon['’]?t\s+repeat\b|\bavoid\s+repeat(?:ing|s)?\b/i,
		snippet:
			'A writing-style rule is in play ("avoid repetition"). A reusable skill/command can encode this convention rather than restating it each time — SearchPlugins for one, or author it with ProposePlugin.',
	},
	{
		id: "format-prefer-json",
		category: "output-format",
		pattern: /\bprefer\s+JSON\b|\buse\s+JSON\b|\boutput\s+(?:as\s+)?JSON\b|\breturn\s+(?:as\s+)?JSON\b/i,
		snippet:
			"An output-format convention is in play (JSON). Standardizing a format is exactly the kind of thing a reusable skill/plugin captures — SearchPlugins for one, or author it with ProposePlugin.",
	},
	{
		id: "capability-gap",
		category: "capability-gap",
		pattern: /\bcapability gap\b|\bno tool for\b|\blacks? a tool\b|\bwish (?:I|we) had a tool\b/i,
		snippet:
			"This reads like a capability gap. Before hand-rolling it, SearchPlugins for a plugin that fills it (installable and usable this same turn), or author one with ProposePlugin.",
	},
];

// ── Armed registry (process-scoped) ──────────────────────────────────────────
//
// When a nudge fires, its id is recorded here so the plugin layer (SearchPlugins)
// can surface "there is already a reuse candidate for this work" even if the
// model never explicitly asked. The CLI is single-process per session, so a
// module-level set is the natural home; nudges.ts clears it on session start.

const armed = new Map<string, ReuseNudge>();

/** Record that a nudge fired this session so the plugin layer can surface it. */
export function armReuseNudge(nudge: ReuseNudge): void {
	armed.set(nudge.id, nudge);
}

/** Reuse nudges that have fired this session, in insertion order. */
export function getArmedReuseNudges(): ReuseNudge[] {
	return [...armed.values()];
}

/** Reset the armed registry (called on session start). */
export function clearArmedReuseNudges(): void {
	armed.clear();
}

/**
 * Scan a blob of runtime text (tool output, turn text, user prompt) and return
 * every reuse nudge whose cue matches. Pure — arming/de-duping is the caller's
 * job.
 */
export function matchReuseNudges(text: string | undefined | null): ReuseNudge[] {
	if (!text) return [];
	return REUSE_NUDGES.filter((n) => n.pattern.test(text));
}

/**
 * The one enablement gate for the whole autonomous plugin system.
 *
 * Reads settings.json `enablePluginTools` (default false) — the same setting
 * that gates whether the plugin lifecycle tools are attached to the top-level
 * agent in main.ts. Keeping both the tool surface and the reactive nudge behind
 * a single flag is the "whole autonomous plugin system, off by default" switch.
 */
export function isAutonomousPluginSystemEnabled(cwd: string): boolean {
	try {
		return SettingsManager.create(cwd, getAgentDir()).getEnablePluginTools();
	} catch {
		// Fail closed: if settings can't be read, treat the autonomous system as off.
		return false;
	}
}
