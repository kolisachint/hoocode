/**
 * Plugin authoring engine (spec §3) — shared by the two ProposePlugin tools.
 *
 * Completes the discover → acquire → author spectrum: when no marketplace plugin
 * fits a gap, the model can scaffold one. Authoring is gated on the *content /
 * capability-grant* trust axis (what the plugin can do), not the *source* axis
 * used for install. This module carries the risk classification, the
 * privilege-amplification guardrail, and the file writer; the tools own the two
 * escalating-risk *paths* (autonomous scaffold vs. confirm-then-activate).
 *
 * Everything is written through the format registry's {@link emitForPlatforms},
 * so an authored plugin lands in the requested vendor layouts (Claude Code and
 * GitHub Copilot by default) and round-trips back through {@link parsePluginDir}.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import * as path from "node:path";
import { CLAUDE_TOOL_ALIASES } from "../../agent-frontmatter.js";
import { PLUGIN_SYSTEM_TOOL_NAMES } from "../../tools/plugin-tool-names.js";
import { emitForPlatforms } from "./formats/index.js";
import type { MarketplacePlatform, PluginDraft } from "./formats/types.js";
import { installedPluginsDir, sanitizeForDir } from "./install.js";
import { type NormalizedPlugin, parsePluginDir } from "./manifest.js";

/** Default authoring targets: the two external vendor formats the model can author into. */
export const DEFAULT_AUTHORING_PLATFORMS: MarketplacePlatform[] = ["claude", "github"];

/** hoocode tool names that only read (no mutation, no exec). Grants limited to these are low-risk. */
const READONLY_TOOLS = new Set(["read", "grep", "find", "ls", "webfetch", "websearch"]);

export interface AllowlistClassification {
	/** read-only grants are as safe as a skill; mutating/exec/network/`*` grants need confirmation. */
	risk: "read-only" | "mutating";
	/** Human-readable explanation of what drove the classification. */
	reason: string;
	/** Any plugin-system (capability-acquisition) tools found — always forbidden in an authored allowlist. */
	pluginTools: string[];
	/** The raw allowlist tokens. */
	tokens: string[];
}

/**
 * Classify an authored subagent `tools:` allowlist as read-only vs. mutating,
 * reusing the same Claude-alias vocabulary as the agent-frontmatter normalizer
 * (spec §3 "compute the risk, don't guess it"). Anything unrecognized — an MCP
 * tool, a bare `*`, an unknown name — is treated as mutating (fail-safe).
 */
export function classifyAllowlist(tools: string | undefined): AllowlistClassification {
	const tokens = (tools ?? "")
		.split(/[,\s]+/)
		.map((t) => t.trim())
		.filter(Boolean);
	const pluginTools = tokens.filter((t) => PLUGIN_SYSTEM_TOOL_NAMES.some((n) => n.toLowerCase() === t.toLowerCase()));

	if (tokens.length === 0) {
		return { risk: "read-only", reason: "no tools granted", pluginTools, tokens };
	}

	const reasons: string[] = [];
	let mutating = false;
	for (const t of tokens) {
		const low = t.toLowerCase();
		if (t === "*" || low === "all") {
			mutating = true;
			reasons.push("grants all tools (*)");
			continue;
		}
		if (pluginTools.some((p) => p.toLowerCase() === low)) continue; // reported separately as a guardrail violation
		const mapped = CLAUDE_TOOL_ALIASES[low];
		if (!mapped) {
			mutating = true;
			reasons.push(`grants "${t}" (unrecognized or MCP tool — treated as mutating)`);
			continue;
		}
		if (!READONLY_TOOLS.has(mapped)) {
			mutating = true;
			reasons.push(`grants "${mapped}" (mutating/exec)`);
		}
	}

	return {
		risk: mutating ? "mutating" : "read-only",
		reason: reasons.join("; ") || "read-only tools only",
		pluginTools,
		tokens,
	};
}

export interface WriteResult {
	dest: string;
	/** Written file paths, relative to the plugin root. */
	files: string[];
	/** Re-parsed plugin (confirms the scaffold round-trips). */
	plugin: NormalizedPlugin | null;
}

/**
 * Render `draft` into the requested platform layouts and write it under
 * `.agents/plugins/<id>/`. Returns the destination, the emitted files, and the
 * re-parsed plugin so callers can confirm the round-trip.
 */
export function writePluginDraft(cwd: string, draft: PluginDraft, platforms?: MarketplacePlatform[]): WriteResult {
	const targets = platforms ?? draft.supportPlatform ?? DEFAULT_AUTHORING_PLATFORMS;
	const dest = path.join(installedPluginsDir(cwd), sanitizeForDir(draft.id));
	const files = emitForPlatforms({ ...draft, supportPlatform: targets }, targets);

	// Formats share the capability tree (only marker manifests differ), so
	// dedupe by path — later formats overwrite with identical content.
	const byPath = new Map(files.map((f) => [f.path, f]));
	mkdirSync(dest, { recursive: true });
	for (const f of byPath.values()) {
		const abs = path.join(dest, f.path);
		mkdirSync(path.dirname(abs), { recursive: true });
		writeFileSync(abs, f.content);
	}

	return { dest, files: [...byPath.keys()], plugin: parsePluginDir(dest) };
}

/** Whether a plugin id already exists on disk (so authoring never silently clobbers). */
export function pluginExists(cwd: string, id: string): boolean {
	return existsSync(path.join(installedPluginsDir(cwd), sanitizeForDir(id)));
}
