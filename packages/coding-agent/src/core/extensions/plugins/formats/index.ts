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
import type { EmittedFile, MarketplacePlatform, PluginDraft, PluginFormatAdapter, PluginFormatId } from "./types.js";

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

/** Every platform present in a plugin directory, precedence winner first. */
export function detectPlatforms(root: string): MarketplacePlatform[] {
	const platforms: MarketplacePlatform[] = [];
	for (const format of PLUGIN_FORMATS) {
		if (format.detectPlugin(root) && !platforms.includes(format.platform)) platforms.push(format.platform);
	}
	return platforms;
}

/**
 * Parse a plugin directory using the highest-precedence format present.
 * Returns null when no registered format recognizes the directory.
 *
 * `supportPlatform` records *every* format present (not just the winner), so a
 * directory carrying more than one vendor layout reports all of them.
 */
export function parsePluginWithFormats(root: string): NormalizedPlugin | null {
	for (const format of PLUGIN_FORMATS) {
		if (format.detectPlugin(root)) {
			const parsed = format.parsePlugin(root);
			if (parsed) {
				// Winner's platform is already first via precedence order; fold in the rest.
				const supportPlatform = detectPlatforms(root);
				return { ...parsed, supportPlatform };
			}
		}
	}
	return null;
}

/**
 * Render a draft into the on-disk files for the given platforms (defaulting to
 * the draft's own `supportPlatform`, then to every format). Files from different
 * formats live under distinct marker directories, so the merge never collides.
 */
export function emitForPlatforms(draft: PluginDraft, platforms?: MarketplacePlatform[]): EmittedFile[] {
	const targets = platforms ?? draft.supportPlatform ?? PLUGIN_FORMATS.map((f) => f.platform);
	const files: EmittedFile[] = [];
	const seen = new Set<MarketplacePlatform>();
	for (const platform of targets) {
		if (seen.has(platform)) continue;
		seen.add(platform);
		const adapter = getFormatByPlatform(platform);
		if (adapter) files.push(...adapter.emit(draft));
	}
	return files;
}

export { agentsFormat, claudeFormat, copilotFormat };
export {
	DEFAULT_AUTHORING_PLATFORMS,
	getSupportPlatforms,
	normalizePlatformToken,
	parseSupportPlatforms,
	resolveAuthoringPlatforms,
	type SupportPlatformParse,
	setSupportPlatforms,
} from "./platform-targets.js";
export type {
	EmittedFile,
	MarketplacePlatform,
	PluginDraft,
	PluginFormatAdapter,
	PluginFormatId,
	WorkspaceLayout,
} from "./types.js";
