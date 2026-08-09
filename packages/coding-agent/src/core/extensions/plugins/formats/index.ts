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
import { copilotFormat, copilotReadPaths } from "./copilot.js";
import { JSON_MANIFEST_READ_PATHS } from "./jsonManifest.js";
import { knownUnsupportedSurfaces, modelledManifestKeys } from "./shared.js";
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

/**
 * Whether `root` carries an actual manifest file, in any supported format.
 * Cheap: existence only, no parse.
 */
export function hasAnyManifest(root: string): boolean {
	return PLUGIN_FORMATS.some((format) => format.hasManifest(root));
}

/**
 * Whether the skill scanner should treat `root` as a plugin and stop descending.
 *
 * Keyed on the **manifest**, deliberately not on `detectPlugin`. Claude's
 * manifest-optional rule counts a bare `SKILL.md` as enough to make a directory a
 * plugin — true inside a plugins directory, catastrophic inside a skills
 * directory, where it describes every plain skill folder there is. Using
 * `detectPlugin` here would make the scanner skip all of them.
 */
export function isPluginRoot(root: string): boolean {
	return hasAnyManifest(root);
}

/**
 * Formats that recognize `root`, manifest-bearing ones first.
 *
 * The two-pass order matters. Claude's manifest-optional rule means its adapter
 * recognizes any directory with components in default locations — including a
 * Copilot plugin, which has its own manifest and a lower precedence number. A
 * single precedence-ordered pass would therefore hand every Copilot plugin to
 * the Claude adapter. An explicit manifest always beats an inferred match.
 */
function matchingFormats(root: string): PluginFormatAdapter[] {
	const withManifest = PLUGIN_FORMATS.filter((f) => f.hasManifest(root));
	const inferred = PLUGIN_FORMATS.filter((f) => !f.hasManifest(root) && f.detectPlugin(root));
	return [...withManifest, ...inferred];
}

/**
 * Every platform a plugin directory *offers*, precedence winner first.
 *
 * Manifest-bearing formats only: an inferred match means "Claude would also load
 * this", not "this directory ships a Claude layout", and reporting it would make
 * a Copilot-only plugin claim `["github", "claude"]`. When nothing has a manifest
 * the inferred winner is all there is, so it stands alone.
 */
function detectPlatforms(root: string): MarketplacePlatform[] {
	const declared = PLUGIN_FORMATS.filter((f) => f.hasManifest(root));
	const platforms: MarketplacePlatform[] = [];
	for (const format of declared.length > 0 ? declared : matchingFormats(root).slice(0, 1)) {
		if (!platforms.includes(format.platform)) platforms.push(format.platform);
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
export function parsePluginWithFormats(root: string, options?: { requireManifest?: boolean }): NormalizedPlugin | null {
	// Inside a skills directory the manifest is what promotes a folder from plain
	// skill to plugin, so manifest-less detection must be switched off there or
	// every skill folder becomes a plugin. See §1.6.
	if (options?.requireManifest && !hasAnyManifest(root)) return null;
	for (const format of matchingFormats(root)) {
		const parsed = format.parsePlugin(root);
		if (parsed) {
			// Winner's platform is already first; fold in the rest.
			const supportPlatform = detectPlatforms(root);
			return { ...parsed, supportPlatform };
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

export { claudeFormat, copilotFormat };
export type {
	EmittedFile,
	MarketplacePlatform,
	PluginDraft,
	PluginFormatAdapter,
	PluginFormatId,
} from "./types.js";

/**
 * Everything the adapters claim to understand: manifest keys, plugin-relative
 * paths they read, marketplace index locations, and the surfaces they knowingly
 * skip.
 *
 * This is the input to the Tier 2 drift check (§2.2), and it is a *declaration*
 * on purpose. A checker that inferred coverage by reading the parsers would be
 * validating its own inference; a declared set is a claim the codebase makes,
 * which a vendor reference can falsify.
 *
 * `unsupportedSurfaces` is not a gap list. Those paths are known and
 * deliberately unhandled, so re-flagging them every run would bury the one
 * finding that matters: a vendor path in neither set.
 */
export function declaredVocabulary(): {
	manifestKeys: string[];
	marketplaceKeys: string[];
	readPaths: string[];
	marketplaceFiles: string[];
	unsupportedSurfaces: string[];
} {
	const readPaths = new Set<string>([...JSON_MANIFEST_READ_PATHS, ...copilotReadPaths()]);
	for (const format of PLUGIN_FORMATS) {
		readPaths.add(
			`${format.id === "copilot" ? ".github" : format.id === "claude" ? ".claude-plugin" : ".agents-plugin"}/plugin.json`,
		);
	}
	return {
		manifestKeys: modelledManifestKeys(),
		// Marketplace *index* keys, modelled by `parseMarketplaceDir`. Declared
		// separately from plugin-manifest keys but checked together: a reference
		// page documents both, and a drift check that knew only one of the two
		// vocabularies would report the other's fields as unknown every run.
		marketplaceKeys: ["name", "owner", "metadata", "pluginRoot", "plugins", "source", "supportPlatform", "strict"],
		readPaths: [...readPaths].sort(),
		marketplaceFiles: [...new Set(PLUGIN_FORMATS.flatMap((f) => f.marketplaceFiles))].sort(),
		unsupportedSurfaces: knownUnsupportedSurfaces(),
	};
}
