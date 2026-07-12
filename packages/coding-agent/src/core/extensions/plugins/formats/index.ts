/**
 * Plugin format registry.
 *
 * The single ordered list of every supported plugin format. Readers, the
 * marketplace parser, and `ProposePlugin` all go through here instead of
 * branching on a format, so adding or evolving a vendor layout is a one-file
 * change (write/adjust an adapter, add it to {@link PLUGIN_FORMATS}).
 *
 * Order is precedence order — native `.agents-plugin` first, then Claude, then
 * Copilot — so "first match wins" when a directory carries more than one format.
 */

import type { NormalizedPlugin } from "../manifest.js";
import { agentsFormat } from "./agents.js";
import { claudeFormat } from "./claude.js";
import { copilotFormat } from "./copilot.js";
import type { MarketplacePlatform, PluginFormatAdapter, PluginFormatId } from "./types.js";

/** All registered formats, in precedence order (lower `precedence` first). */
export const PLUGIN_FORMATS: readonly PluginFormatAdapter[] = [agentsFormat, claudeFormat, copilotFormat]
	.slice()
	.sort((a, b) => a.precedence - b.precedence);

/** Look up an adapter by its internal format id. */
export function getFormat(id: PluginFormatId): PluginFormatAdapter | undefined {
	return PLUGIN_FORMATS.find((f) => f.id === id);
}

/** Look up an adapter by its public platform token (`"github"` → Copilot). */
export function getFormatByPlatform(platform: MarketplacePlatform): PluginFormatAdapter | undefined {
	return PLUGIN_FORMATS.find((f) => f.platform === platform);
}

/**
 * Parse a plugin directory using the highest-precedence format present.
 * Returns null when no registered format recognizes the directory.
 */
export function parsePluginWithFormats(root: string): NormalizedPlugin | null {
	for (const format of PLUGIN_FORMATS) {
		if (format.detectPlugin(root)) {
			const parsed = format.parsePlugin(root);
			if (parsed) return parsed;
		}
	}
	return null;
}

export { agentsFormat, claudeFormat, copilotFormat };
export type { EmittedFile, MarketplacePlatform, PluginDraft, PluginFormatAdapter, PluginFormatId } from "./types.js";
