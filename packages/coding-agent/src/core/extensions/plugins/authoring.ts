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

import { cpSync, existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import * as path from "node:path";
import { CLAUDE_TOOL_ALIASES } from "../../agent-frontmatter.js";
import { PLUGIN_SYSTEM_TOOL_NAMES } from "../../tools/plugin-tool-names.js";
import { emitForPlatforms } from "./formats/index.js";
import { isPluginPlatform, resolvePluginPlatforms } from "./formats/platform-targets.js";
import { slug } from "./formats/shared.js";
import type { AuthoredHook, AuthoredMcpServer, MarketplacePlatform, PluginDraft } from "./formats/types.js";
import { candidatePluginDirs, discardDraftDir, makeDraftDir, productionPluginDir } from "./locations.js";
import { type NormalizedPlugin, type PluginHooksConfig, parsePluginDir } from "./manifest.js";

// Re-exported so existing importers keep one vocabulary; the resolution chain
// (explicit → session --platform → default) lives in platform-targets.
export { resolvePluginPlatforms } from "./formats/platform-targets.js";

/**
 * Layout for an *edit* of an existing plugin. Unlike creation, this never
 * re-resolves against the session's `--platform`: a merge or a removal must keep
 * the plugin in whatever layout it already has on disk, or it would move the
 * manifest out from under a plugin the user (or an older hoocode) already
 * published. Plugins authored before plugin production became platform-specific
 * carry the native `agents` layout, and those must keep working.
 */
function existingLayout(existing: NormalizedPlugin, explicit?: readonly MarketplacePlatform[]): MarketplacePlatform[] {
	if (explicit && explicit.length > 0) return [...explicit];
	if (existing.supportPlatform.length > 0) return [...existing.supportPlatform];
	return resolvePluginPlatforms();
}

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
	/** Where the plugin ended up: a production home, or the draft dir when not promoted. */
	dest: string;
	/** False when the caller asked to stop at the draft (gates still to run). */
	promoted: boolean;
	/** Platforms the draft was emitted for. */
	targets: MarketplacePlatform[];
	/** Written file paths, relative to the plugin root. */
	files: string[];
	/** Re-parsed plugin (confirms the scaffold round-trips). */
	plugin: NormalizedPlugin | null;
}

/**
 * Provenance marker written at the root of every authored plugin. A production
 * home can also hold artifacts hoocode did not write — `~/.claude/skills/` is a
 * directory Claude Code and the user both write into — so this marker is what
 * separates "ours to re-emit or delete" from "someone else's". UpdatePlugin
 * (which rewrites manifests and hook/MCP files) is gated on it. Existence is the
 * signal; the content is informational.
 */
const AUTHORED_MARKER_FILE = ".authored.json";

/** Locate an existing plugin by id across every home, or undefined. */
export function findPluginDir(cwd: string, id: string): string | undefined {
	return candidatePluginDirs(cwd, id).find((dir) => existsSync(dir));
}

/** Whether the plugin at `id` was authored here (carries the provenance marker), vs. installed or hand-written. */
export function isAuthoredPlugin(cwd: string, id: string): boolean {
	const dir = findPluginDir(cwd, id);
	return !!dir && existsSync(path.join(dir, AUTHORED_MARKER_FILE));
}

/**
 * Render `draft` into the requested platform layouts and write it out.
 *
 * Two modes, and the distinction matters:
 *
 * - **Creation** (default): emit into an ephemeral draft dir, then promote into
 *   the target platform's production home, replacing whatever was there. A
 *   production home is live — `~/.claude/skills/<id>/` is loaded by Claude Code
 *   on its next session — so writing there directly would mean a plugin that
 *   failed validation, or whose confirmation was declined, is already running.
 *   Pass `promote: false` to stop at the draft and promote later with
 *   {@link promoteDraft}.
 * - **In-place edit** (`dest` given): write the emitted files over an existing
 *   plugin directory, wherever it already lives. An edit must never relocate the
 *   plugin — a merge or a removal on a plugin sitting in the consumption home or
 *   the legacy project home would otherwise silently move it into a production
 *   home. Files not named by the emit are left alone, which is what makes the
 *   directory-scanned merge semantics work.
 */
export function writePluginDraft(
	draft: PluginDraft,
	platforms?: MarketplacePlatform[],
	options?: { promote?: boolean; dest?: string },
): WriteResult {
	// Creation picks a platform; `agents` is refused here (see resolvePluginPlatforms).
	const targets = resolvePluginPlatforms(platforms ?? draft.supportPlatform?.filter(isPluginPlatform));
	const files = emitForPlatforms({ ...draft, supportPlatform: targets }, targets);

	// Formats share the capability tree (only marker manifests differ), so
	// dedupe by path — later formats overwrite with identical content.
	const byPath = new Map(files.map((f) => [f.path, f]));
	const writeInto = (root: string) => {
		for (const f of byPath.values()) {
			const abs = path.join(root, f.path);
			mkdirSync(path.dirname(abs), { recursive: true });
			writeFileSync(abs, f.content);
		}
		writeFileSync(path.join(root, AUTHORED_MARKER_FILE), `${JSON.stringify({ authored: true }, null, 2)}\n`);
	};
	const written = { files: [...byPath.keys(), AUTHORED_MARKER_FILE], targets };

	if (options?.dest) {
		mkdirSync(options.dest, { recursive: true });
		writeInto(options.dest);
		return { dest: options.dest, promoted: true, ...written, plugin: parsePluginDir(options.dest) };
	}

	const draftDir = makeDraftDir();
	writeInto(draftDir);
	if (options?.promote === false) {
		return { dest: draftDir, promoted: false, ...written, plugin: parsePluginDir(draftDir) };
	}
	const dest = promoteDraft(draftDir, draft.id, targets);
	return { dest, promoted: true, ...written, plugin: parsePluginDir(dest) };
}

/**
 * Move a draft into the production home of its first target platform, replacing
 * whatever is there. The draft directory is consumed either way.
 *
 * A cross-device rename fails when temp and home sit on different filesystems,
 * so this falls back to copy-then-delete rather than assuming `rename` works.
 */
export function promoteDraft(draftDir: string, id: string, targets: readonly MarketplacePlatform[]): string {
	const platform = targets.filter(isPluginPlatform)[0] ?? resolvePluginPlatforms()[0];
	const dest = productionPluginDir(platform, id);
	rmSync(dest, { recursive: true, force: true });
	mkdirSync(path.dirname(dest), { recursive: true });
	try {
		renameSync(draftDir, dest);
	} catch {
		cpSync(draftDir, dest, { recursive: true });
		discardDraftDir(draftDir);
	}
	return dest;
}

/**
 * Whether authoring this id would clobber something **in the target platform's
 * production home**.
 *
 * Scoped to the target rather than every home on purpose: a `claude` `foo` and a
 * `github` `foo` are artifacts for two different ecosystems, not duplicates of
 * each other, and a marketplace-installed `foo` in the consumption home is a
 * third unrelated thing. Only a collision in the directory this write would
 * actually land in is a collision.
 */
export function pluginExists(id: string, platforms?: readonly MarketplacePlatform[]): boolean {
	const target = resolvePluginPlatforms(platforms?.filter(isPluginPlatform))[0];
	return existsSync(productionPluginDir(target, id));
}

/** Load an installed/authored plugin by id from whichever home holds it, or null. */
export function getPlugin(cwd: string, id: string): NormalizedPlugin | null {
	const dir = findPluginDir(cwd, id);
	return dir ? parsePluginDir(dir) : null;
}

/** Reverse of {@link authoredHooksToConfig}: flatten a parsed hook event-map back to authored hooks. */
function hooksConfigToAuthored(config: PluginHooksConfig): AuthoredHook[] {
	const out: AuthoredHook[] = [];
	for (const [event, groups] of Object.entries(config)) {
		for (const group of groups) {
			for (const cmd of group.hooks) {
				if (typeof cmd.command !== "string" || !cmd.command) continue;
				out.push({
					event,
					...(group.matcher ? { matcher: group.matcher } : {}),
					command: cmd.command,
					...(cmd.timeout ? { timeout: cmd.timeout } : {}),
				});
			}
		}
	}
	return out;
}

/**
 * Convert a parsed `mcpServers` record back to authored form. Throws on a
 * non-stdio (url/http-type) server rather than silently dropping it from the
 * re-emit — a merge must never quietly lose a capability (only reachable via a
 * hand-edited authored plugin; our own schema always writes `command` servers).
 */
function mcpRecordToAuthored(record: Record<string, unknown>): AuthoredMcpServer[] {
	const out: AuthoredMcpServer[] = [];
	for (const [name, value] of Object.entries(record)) {
		if (!value || typeof value !== "object") continue;
		const server = value as { command?: unknown; args?: unknown; env?: unknown };
		if (typeof server.command !== "string") {
			throw new Error(
				`Cannot merge: MCP server "${name}" has no command (url/http-type servers don't round-trip through authoring). ` +
					"Edit the plugin's .mcp.json directly instead.",
			);
		}
		out.push({
			name,
			command: server.command,
			...(Array.isArray(server.args) ? { args: server.args.map(String) } : {}),
			...(server.env && typeof server.env === "object" ? { env: server.env as Record<string, string> } : {}),
		});
	}
	return out;
}

/** Dedupe authored hooks by (event, matcher, command) so a re-supplied hook doesn't stack. */
function dedupeHooks(hooks: AuthoredHook[]): AuthoredHook[] {
	const seen = new Set<string>();
	const out: AuthoredHook[] = [];
	for (const h of hooks) {
		const key = `${h.event} ${h.matcher ?? ""} ${h.command}`;
		if (seen.has(key)) continue;
		seen.add(key);
		out.push(h);
	}
	return out;
}

/**
 * Merge inline-authored `delta` capabilities into the existing local plugin `id`
 * and re-emit. Unlike a marketplace `UpdatePlugin`, nothing is fetched from a
 * remote source — the new content comes from the caller — so the supply-chain
 * "benign v1 → hostile v2" vector the spec guards against is structurally absent.
 *
 * Merge semantics:
 *  - **Skills / commands / agents** are directory-scanned, so existing ones are
 *    left on disk untouched; a delta entry with a matching name overwrites just
 *    that file (an update), a new name is added.
 *  - **Hooks** and **MCP servers** live in single files that a re-emit rewrites,
 *    so they are re-emitted as the *union* of existing + delta (MCP keyed by
 *    server name with delta winning; hooks deduped by event/matcher/command).
 *    Hooks have no name, so there is deliberately no modify-in-place: a delta
 *    hook with the same event/matcher but a different command is a NEW hook
 *    added alongside the old one, never a replacement. (Keying replacement by
 *    event+matcher would silently drop legitimate sibling hooks that share
 *    them.) Changing a hook = {@link removeFromPlugin} the old one + merge the
 *    new one.
 *  - **Metadata** (version, description, author) takes the delta's value when
 *    provided, else keeps the existing one.
 *
 * Platforms default to the plugin's existing `supportPlatform` so a merge never
 * silently adds or drops a vendor layout.
 */
export function mergePluginDraft(
	cwd: string,
	id: string,
	delta: Partial<PluginDraft>,
	platforms?: MarketplacePlatform[],
): WriteResult {
	const existing = getPlugin(cwd, id);
	if (!existing) {
		throw new Error(`Cannot update plugin "${id}": it does not exist. Use ProposePlugin to create it first.`);
	}
	// Authored-only: merging re-emits manifests and hook/MCP files through our
	// writer, which only round-trips what PluginDraft can represent. Running that
	// over a marketplace install could silently drop fields it carries (capability
	// -dir overrides, url-type MCP servers, extra manifest keys).
	if (!isAuthoredPlugin(cwd, id)) {
		throw new Error(
			`Cannot update plugin "${id}": it was not authored here (no ${AUTHORED_MARKER_FILE} marker). ` +
				"Only locally authored plugins can be merged.",
		);
	}

	const existingHooks = existing.hooks ? hooksConfigToAuthored(existing.hooks) : [];
	const mergedHooks = dedupeHooks([...existingHooks, ...(delta.hooks ?? [])]);

	const mcpByName = new Map<string, AuthoredMcpServer>();
	for (const s of existing.mcpServers ? mcpRecordToAuthored(existing.mcpServers) : []) mcpByName.set(s.name, s);
	for (const s of delta.mcpServers ?? []) mcpByName.set(s.name, s);

	const targets = existingLayout(existing, platforms);
	const merged: PluginDraft = {
		id,
		version: delta.version ?? existing.version,
		description: delta.description ?? existing.description,
		author: delta.author ?? existing.author,
		supportPlatform: targets,
		// Keys we do not model survive the manifest rewrite.
		unknownFields: existing.unknownFields,
		// Directory-scanned capabilities: delta-only; existing files stay on disk.
		skills: delta.skills,
		commands: delta.commands,
		agents: delta.agents,
		// Single-file capabilities: re-emit the union so a merge never drops them.
		hooks: mergedHooks.length ? mergedHooks : undefined,
		mcpServers: mcpByName.size ? [...mcpByName.values()] : undefined,
	};
	// An edit stays where the plugin already lives; it never relocates it.
	return writePluginDraft(merged, targets, { dest: findPluginDir(cwd, id) });
}

/** A hook to remove: `event` is required; `matcher`/`command` narrow the match when provided. */
interface HookRemovalSpec {
	event: string;
	matcher?: string;
	command?: string;
}

/** Named capabilities to remove from an authored plugin. */
export interface RemovalSpec {
	skills?: string[];
	commands?: string[];
	subagents?: string[];
	mcpServers?: string[];
	hooks?: HookRemovalSpec[];
}

export interface RemoveResult {
	dest: string;
	/** Human-readable descriptions of what was removed. */
	removed: string[];
	/** Requested capabilities that were not found (nothing was removed for these). */
	missing: string[];
}

function describeHookSpec(h: HookRemovalSpec): string {
	return `hook [${h.event}${h.matcher !== undefined ? ` matcher=${h.matcher}` : ""}${h.command !== undefined ? ` command=${h.command}` : ""}]`;
}

/**
 * Remove named capabilities from the authored plugin `id`. The inverse of the
 * additive merge, and — like {@link mergePluginDraft} — authored-only.
 *
 * Removal is the low-risk direction (deleting capabilities cannot execute
 * code), which is why callers may run it without a confirmation gate.
 *
 *  - **Skills / commands / subagents** are directory-scanned, so removal is a
 *    surgical file delete at our emit conventions; no re-emit needed.
 *  - **Hooks** (matched by event, narrowed by matcher/command when given) and
 *    **MCP servers** (by name) live in single files, so the remaining set is
 *    re-emitted — and when a set empties, its file is DELETED, because the
 *    parser falls back to `hooks/hooks.json` / `.mcp.json` on disk and a stale
 *    file would resurrect the removed capability on the next parse.
 */
export function removeFromPlugin(cwd: string, id: string, spec: RemovalSpec): RemoveResult {
	const existing = getPlugin(cwd, id);
	if (!existing) {
		throw new Error(`Cannot remove from plugin "${id}": it does not exist.`);
	}
	if (!isAuthoredPlugin(cwd, id)) {
		throw new Error(
			`Cannot remove from plugin "${id}": it was not authored here (no ${AUTHORED_MARKER_FILE} marker). ` +
				"Only locally authored plugins can be edited.",
		);
	}
	const dest = findPluginDir(cwd, id);
	if (!dest) throw new Error(`Cannot remove from plugin "${id}": it is not on disk.`);
	const removed: string[] = [];
	const missing: string[] = [];

	// Directory-scanned capabilities: surgical deletes at our emit conventions.
	const fileTargets: Array<[kind: string, name: string, relPath: string]> = [
		...(spec.skills ?? []).map((n): [string, string, string] => ["skill", n, path.join("skills", slug(n))]),
		...(spec.commands ?? []).map((n): [string, string, string] => [
			"command",
			n,
			path.join("commands", `${slug(n)}.md`),
		]),
		...(spec.subagents ?? []).map((n): [string, string, string] => [
			"subagent",
			n,
			path.join("agents", `${slug(n)}.md`),
		]),
	];
	for (const [kind, name, rel] of fileTargets) {
		const abs = path.join(dest, rel);
		if (existsSync(abs)) {
			rmSync(abs, { recursive: true, force: true });
			removed.push(`${kind} "${name}"`);
		} else {
			missing.push(`${kind} "${name}"`);
		}
	}

	// Single-file capabilities: filter the reconstructed sets, then re-emit.
	let singleFileChanged = false;
	let remainingHooks = existing.hooks ? hooksConfigToAuthored(existing.hooks) : [];
	for (const h of spec.hooks ?? []) {
		const before = remainingHooks.length;
		remainingHooks = remainingHooks.filter(
			(x) =>
				!(
					x.event === h.event &&
					(h.matcher === undefined || (x.matcher ?? "") === h.matcher) &&
					(h.command === undefined || x.command === h.command)
				),
		);
		const n = before - remainingHooks.length;
		if (n > 0) {
			removed.push(`${n} ${describeHookSpec(h)}`);
			singleFileChanged = true;
		} else {
			missing.push(describeHookSpec(h));
		}
	}
	let remainingMcp = existing.mcpServers ? mcpRecordToAuthored(existing.mcpServers) : [];
	for (const name of spec.mcpServers ?? []) {
		const before = remainingMcp.length;
		remainingMcp = remainingMcp.filter((s) => s.name !== name);
		if (remainingMcp.length < before) {
			removed.push(`mcp server "${name}"`);
			singleFileChanged = true;
		} else {
			missing.push(`mcp server "${name}"`);
		}
	}

	if (singleFileChanged) {
		const targets = existingLayout(existing);
		writePluginDraft(
			{
				id,
				version: existing.version,
				description: existing.description,
				author: existing.author,
				supportPlatform: targets,
				unknownFields: existing.unknownFields,
				hooks: remainingHooks.length ? remainingHooks : undefined,
				mcpServers: remainingMcp.length ? remainingMcp : undefined,
			},
			targets,
			{ dest },
		);
		// Emit skips empty sets, so a stale file from the previous emit survives
		// and the parser's on-disk fallback would resurrect it — delete explicitly.
		if (remainingHooks.length === 0) rmSync(path.join(dest, "hooks", "hooks.json"), { force: true });
		if (remainingMcp.length === 0) rmSync(path.join(dest, ".mcp.json"), { force: true });
	}

	return { dest, removed, missing };
}
