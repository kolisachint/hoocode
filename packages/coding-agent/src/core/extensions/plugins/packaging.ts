/**
 * Packaging — everything that has to happen before a plugin can be published,
 * and nothing that publishes it.
 *
 * The split is the point. Preparing a release is mechanical: run the gates at
 * their strictest, write a README, produce the marketplace index entry. All of
 * that is automated. *Publishing* — pushing an artifact into a marketplace that
 * other agents install from autonomously — stays a human act, because an agent
 * that can do it unattended is a supply-chain compromise primitive. See
 * docs/plugin-system-architecture.md §3.2–§3.3.
 *
 * Packaging works **in place**, in the plugin's production home. There is no
 * separate output location: by the time a plugin reaches here it is already in
 * its target platform's layout, and copying it somewhere else would just create
 * a fourth directory role to confuse with the other three (§8.3).
 *
 * The two platforms are not symmetric here, and the lane is built for the harder
 * one. A `claude` plugin is already live locally from `~/.claude/skills/<id>/`,
 * so publishing is optional and additive. For `github`, local installs are
 * deprecated — publishing is the *only* route into the ecosystem — so the
 * GitHub flow (a PR against a marketplace repo's index) is the primary case and
 * the Claude submission is the variant.
 */

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import * as path from "node:path";
import type { CapabilityDoc } from "../../capabilities/registry.js";
import { isPluginPlatform, type PluginPlatform, resolvePluginPlatforms } from "./formats/platform-targets.js";
import { formatGateFindings, type GateFinding, type GateResult, runStaticGates, withFindings } from "./gates.js";
import { productionRoot } from "./locations.js";
import type { NormalizedPlugin } from "./manifest.js";
import { runSmokeGate } from "./smoke.js";
import {
	distractorCandidates,
	loadTriggerCases,
	ownCandidates,
	runTriggerEval,
	type TriggerJudge,
	triggerFindings,
} from "./trigger-eval.js";

/**
 * Where packaging records what it produced, at the plugin root.
 *
 * A dotfile beside the existing `.authored.json` provenance marker rather than a
 * visible `marketplace-entry.json`: the plugin directory is the thing that gets
 * copied into a marketplace repo, and a build record is not part of the artifact.
 */
export const PUBLISH_RECORD_FILE = ".hoocode-publish.json";

/** Placeholder `source` for a plugin that is not in a git checkout — deliberately unusable as-is. */
const SOURCE_PLACEHOLDER = "<REPLACE: git URL, or path within the marketplace repo>";

/** A `marketplace.json` `plugins[]` entry, ready to paste into an index. */
export interface MarketplaceEntry {
	name: string;
	description?: string;
	version?: string;
	source: string | { source: "git"; url: string } | { source: "github"; repo: string };
}

export interface PublishRecord {
	packagedAt: string;
	platform: PluginPlatform;
	entry: MarketplaceEntry;
	/** Whether the gates were green when this record was written. */
	ok: boolean;
}

export interface PackageResult {
	ok: boolean;
	dir: string;
	platform: PluginPlatform;
	/** Files packaging created or refreshed, relative to the plugin root. */
	written: string[];
	entry: MarketplaceEntry;
	evaluation: GateResult;
	/** Human-facing next steps; deliberately instructions, not actions. */
	instructions: string;
}

/**
 * Which ecosystem this plugin is being packaged for.
 *
 * Where it lives is the strongest evidence — the production homes are per
 * platform, so a plugin under `~/.agents/publish/github/` is a github artifact
 * whatever its manifests say. A plugin outside a production home (installed, or
 * authored by an older version) falls back to what it declares, then to the
 * session default.
 */
export function resolvePackagePlatform(plugin: NormalizedPlugin): PluginPlatform {
	const root = path.resolve(plugin.root);
	for (const platform of ["claude", "github"] as const) {
		const home = path.resolve(productionRoot(platform));
		if (root === home || root.startsWith(`${home}${path.sep}`)) return platform;
	}
	const declared = plugin.supportPlatform.filter(isPluginPlatform);
	return declared[0] ?? resolvePluginPlatforms()[0];
}

/** Best-effort origin URL of the repo containing `dir`, if it is in one. */
function gitOriginUrl(dir: string): string | undefined {
	try {
		const url = execFileSync("git", ["-C", dir, "remote", "get-url", "origin"], {
			encoding: "utf8",
			timeout: 5_000,
			stdio: ["ignore", "pipe", "ignore"],
		}).trim();
		return url || undefined;
	} catch {
		return undefined;
	}
}

function capabilityLines(plugin: NormalizedPlugin): string[] {
	const lines: string[] = [];
	const add = (label: string, present: unknown) => {
		if (present) lines.push(`- ${label}`);
	};
	add("Skills", plugin.skillsDir);
	add("Slash commands", plugin.commandsDir);
	add("Subagents", plugin.agentsDir);
	add("Hooks", plugin.hooks);
	add("MCP servers", plugin.mcpServers);
	return lines;
}

function installSection(plugin: NormalizedPlugin, platform: PluginPlatform): string[] {
	if (platform === "claude") {
		return [
			"## Install",
			"",
			"```bash",
			"claude plugin marketplace add <marketplace-repo>",
			`claude plugin install ${plugin.id}@<marketplace>`,
			"```",
			"",
			`Or copy this directory to \`~/.claude/skills/${plugin.id}/\` — a folder there`,
			"carrying a `.claude-plugin/plugin.json` loads with no install step.",
		];
	}
	return [
		"## Install",
		"",
		"```bash",
		"copilot plugin marketplace add <marketplace-repo>",
		`copilot plugin install ${plugin.id}@<marketplace>`,
		"```",
	];
}

/**
 * Generate a README from what the plugin actually contains.
 *
 * Never overwrites: a hand-written README is the author's description of their
 * own plugin, and a generated one derived from the directory listing is strictly
 * worse. This only fills the gap where there is nothing at all.
 */
function writeReadme(plugin: NormalizedPlugin, platform: PluginPlatform): string | undefined {
	const file = path.join(plugin.root, "README.md");
	if (existsSync(file)) return undefined;

	const capabilities = capabilityLines(plugin);
	const body = [
		`# ${plugin.id}`,
		"",
		plugin.description ?? "_No description provided._",
		"",
		...(capabilities.length > 0 ? ["## Provides", "", ...capabilities, ""] : []),
		...installSection(plugin, platform),
		"",
		...(plugin.unsupportedSurfaces?.length
			? [
					"## Notes",
					"",
					`This plugin ships surfaces hoocode does not load: ${plugin.unsupportedSurfaces.join(", ")}.`,
					"They are preserved as-is and are the target platform's concern.",
					"",
				]
			: []),
	].join("\n");

	writeFileSync(file, `${body.trimEnd()}\n`, "utf8");
	return "README.md";
}

/**
 * Build the marketplace index entry.
 *
 * `source` is where the *published* plugin will live, which packaging cannot
 * know: the artifact is on this machine and has not been pushed anywhere. When
 * the plugin sits in a git checkout its origin is a plausible starting point;
 * otherwise the field is a clearly-marked placeholder for the human to fill.
 * Guessing here would produce an index entry that resolves to nothing, which is
 * worse than an obvious blank.
 */
function buildEntry(plugin: NormalizedPlugin): MarketplaceEntry {
	const origin = gitOriginUrl(plugin.root);
	return {
		name: plugin.id,
		...(plugin.description ? { description: plugin.description } : {}),
		...(plugin.version ? { version: plugin.version } : {}),
		source: origin ? { source: "git", url: origin } : SOURCE_PLACEHOLDER,
	};
}

/** True when the entry still needs a human to say where the plugin will be hosted. */
export function entryNeedsSource(entry: MarketplaceEntry): boolean {
	return entry.source === SOURCE_PLACEHOLDER;
}

function publishSteps(plugin: NormalizedPlugin, platform: PluginPlatform): string[] {
	if (platform === "github") {
		return [
			"Publish (GitHub Copilot) — this is the only route into the ecosystem,",
			"local installs being deprecated:",
			"",
			"  1. Copy this directory into your marketplace repo.",
			"  2. Add the entry above to its `.github/marketplace.json` `plugins` array.",
			"  3. Open a pull request; the plugin is installable once it merges.",
		];
	}
	return [
		`Publish (Claude Code) — optional: the plugin is already live locally from`,
		`\`~/.claude/skills/${plugin.id}/\`. To share it:`,
		"",
		"  1. Copy this directory into your marketplace repo.",
		"  2. Add the entry above to its `.claude-plugin/marketplace.json` `plugins` array.",
		"  3. Open a pull request, or submit to the community directory for review",
		"     (approved plugins are pinned to a commit SHA).",
	];
}

/**
 * Package a plugin for publication: gate it at full strictness, write a README,
 * produce its marketplace entry, and record what it produced. Touches no remote.
 */
export async function packagePlugin(
	plugin: NormalizedPlugin,
	platformOverride?: PluginPlatform,
	options: { judge?: TriggerJudge; distractors?: readonly CapabilityDoc[] } = {},
): Promise<PackageResult> {
	const platform = platformOverride ?? resolvePackagePlatform(plugin);

	// A plugin can only be published into an ecosystem whose manifest it carries.
	// `claude plugin validate` says so too, but as a wall of validator output about
	// a missing file; a plugin in the native layout has simply been authored for
	// the wrong target, and that is worth saying in one sentence. Nothing is
	// written — there is no artifact to package.
	if (!plugin.supportPlatform.includes(platform)) {
		const finding: GateFinding = {
			gate: "G1",
			severity: "error",
			message:
				`Plugin "${plugin.id}" carries no ${platform} manifest (it offers: ${plugin.supportPlatform.join(", ")}), ` +
				`so it cannot be published to that ecosystem. Re-author it with --platform ${platform}.`,
		};
		const evaluation: GateResult = { ok: false, findings: [finding], conformance: "round-trip", plugin };
		return {
			ok: false,
			dir: plugin.root,
			platform,
			written: [],
			entry: buildEntry(plugin),
			evaluation,
			instructions: `Plugin "${plugin.id}" cannot be packaged for ${platform}.\n\n${formatGateFindings(evaluation)}`,
		};
	}

	// Strict: warnings that are tolerable in a local plugin — an unresolvable
	// binary, a vendor-validator nit — are not tolerable in something other people
	// will install on the strength of the marketplace vouching for it.
	let evaluation = runStaticGates(plugin.root, { platform, strict: true });
	if (evaluation.plugin) {
		evaluation = withFindings(evaluation, await runSmokeGate(evaluation.plugin), true);
	}

	// G4 is the publish path's gate (§4.3) and nothing else's: it costs a model
	// call, and the question it asks — is this description actually worth the
	// context it occupies — only becomes other people's problem at publication.
	// With no judge or no gold set it reports `not-run`, which is `info`, so a
	// machine without a model is never blocked from publishing by a check it
	// could not perform.
	const own = ownCandidates(plugin);
	if (own.length > 0) {
		const outcome = await runTriggerEval(
			plugin.id,
			[...own, ...distractorCandidates(plugin.id, options.distractors ?? [])],
			loadTriggerCases(plugin.root),
			options.judge,
		);
		evaluation = withFindings(evaluation, triggerFindings(outcome), true);
	}

	const written: string[] = [];
	const readme = writeReadme(plugin, platform);
	if (readme) written.push(readme);

	const entry = buildEntry(plugin);
	const record: PublishRecord = {
		packagedAt: new Date().toISOString(),
		platform,
		entry,
		ok: evaluation.ok,
	};
	writeFileSync(path.join(plugin.root, PUBLISH_RECORD_FILE), `${JSON.stringify(record, null, 2)}\n`, "utf8");
	written.push(PUBLISH_RECORD_FILE);

	const instructions = [
		evaluation.ok
			? `Plugin "${plugin.id}" is ready to publish (${platform}). The remaining step is yours.`
			: `Plugin "${plugin.id}" is NOT ready to publish — these must pass first:`,
		"",
		formatGateFindings(evaluation),
		"",
		"Marketplace entry:",
		"```json",
		JSON.stringify(entry, null, 2),
		"```",
		...(entryNeedsSource(entry) ? ["", "Replace `source` with where the plugin will be hosted."] : []),
		"",
		...publishSteps(plugin, platform),
		"",
		"Publishing is deliberately not automated: an agent that can push executable",
		"code into a marketplace other agents install from unattended is a supply-chain",
		"risk, so the last step stays with you.",
	].join("\n");

	return { ok: evaluation.ok, dir: plugin.root, platform, written, entry, evaluation, instructions };
}

// ── Staging into a local marketplace checkout ────────────────────────────────

/**
 * Files that exist for hoocode's benefit and have no business in a published
 * artifact: the provenance marker and packaging's own build record.
 */
const INTERNAL_FILES = new Set([PUBLISH_RECORD_FILE, ".authored.json"]);

/** Where each platform's marketplace index lives, and the subdirectory plugins are vendored into. */
const INDEX_FILE: Record<PluginPlatform, string> = {
	claude: path.join(".claude-plugin", "marketplace.json"),
	github: path.join(".github", "marketplace.json"),
};
const VENDOR_DIR = "plugins";

export interface StageResult {
	ok: boolean;
	message: string;
	/** Path the plugin was copied to, relative to the marketplace root. */
	pluginPath?: string;
	/** The index that was written, relative to the marketplace root. */
	indexPath?: string;
	/** True when the marketplace had no index for this platform and one was created. */
	createdIndex?: boolean;
}

interface RawIndex {
	name?: string;
	plugins?: Array<Record<string, unknown>>;
	[key: string]: unknown;
}

/**
 * Copy a packaged plugin into a **local** marketplace checkout and add its index
 * entry, leaving the result uncommitted.
 *
 * This is the far side of the automation line drawn in §3.3, and it is where the
 * line actually falls. Everything here is local, reversible, and shows up in
 * `git status` — the human reviews a diff and decides. What stays manual is the
 * step that leaves the machine: no commit, no push, no PR. Those are what make a
 * plugin installable by other people's agents, and that is the act install is
 * trusting a human to have performed.
 *
 * Only reachable from `/plugin publish`, never from a tool: a human typed the
 * command, which is the whole point.
 */
export function stageIntoMarketplace(
	plugin: NormalizedPlugin,
	platform: PluginPlatform,
	marketplaceDir: string,
): StageResult {
	const root = path.resolve(marketplaceDir);
	if (!existsSync(root)) {
		return { ok: false, message: `Marketplace directory not found: ${root}` };
	}
	const pluginRoot = path.resolve(plugin.root);
	if (pluginRoot === root || pluginRoot.startsWith(`${root}${path.sep}`)) {
		return { ok: false, message: `Plugin "${plugin.id}" already lives inside ${root}; nothing to stage.` };
	}

	const indexRel = INDEX_FILE[platform];
	const indexAbs = path.join(root, indexRel);
	let index: RawIndex;
	let createdIndex = false;
	if (existsSync(indexAbs)) {
		try {
			index = JSON.parse(readFileSync(indexAbs, "utf8")) as RawIndex;
		} catch (error) {
			return { ok: false, message: `${indexRel} is not valid JSON (${(error as Error).message}); fix it first.` };
		}
	} else {
		// A marketplace repo that does not have an index yet is a normal starting
		// point, and refusing here would mean the first plugin can never be staged.
		index = { name: path.basename(root), plugins: [] };
		createdIndex = true;
	}
	if (!Array.isArray(index.plugins)) index.plugins = [];

	const dest = path.join(root, VENDOR_DIR, plugin.id);
	rmSync(dest, { recursive: true, force: true });
	mkdirSync(path.dirname(dest), { recursive: true });
	cpSync(pluginRoot, dest, {
		recursive: true,
		filter: (src) => !INTERNAL_FILES.has(path.basename(src)),
	});

	// Vendored in the repo, so the source is a path relative to it — not the
	// git origin `packagePlugin` guessed, which points at wherever the plugin was
	// authored rather than at the marketplace.
	const entry: MarketplaceEntry = { ...buildEntry(plugin), source: `./${VENDOR_DIR}/${plugin.id}` };
	const existingAt = index.plugins.findIndex((p) => p?.name === plugin.id);
	if (existingAt >= 0) index.plugins[existingAt] = { ...index.plugins[existingAt], ...entry };
	else index.plugins.push(entry as unknown as Record<string, unknown>);

	mkdirSync(path.dirname(indexAbs), { recursive: true });
	writeFileSync(indexAbs, `${JSON.stringify(index, null, 2)}\n`, "utf8");

	const verb = existingAt >= 0 ? "Updated" : "Added";
	return {
		ok: true,
		message:
			`${verb} "${plugin.id}" in ${path.basename(root)}: copied to ${VENDOR_DIR}/${plugin.id}/ and ` +
			`${createdIndex ? "created" : "updated"} ${indexRel}. Nothing was committed or pushed.`,
		pluginPath: `${VENDOR_DIR}/${plugin.id}`,
		indexPath: indexRel,
		createdIndex,
	};
}

/** Read back the record packaging left, for callers that want to show it. */
export function readPublishRecord(dir: string): PublishRecord | undefined {
	try {
		const file = path.join(dir, PUBLISH_RECORD_FILE);
		return existsSync(file) ? (JSON.parse(readFileSync(file, "utf8")) as PublishRecord) : undefined;
	} catch {
		return undefined;
	}
}
