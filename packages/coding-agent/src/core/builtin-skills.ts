/**
 * Skills hoocode itself ships.
 *
 * hoocode reads skills from `~/.agents/skills`, `.hoocode/skills`, `.claude/skills`
 * and installed packages — every source except its own. That gap is why it
 * shipped three subagents and zero skills while telling users skills are the
 * extension unit: there was simply nowhere for a first-party skill to live.
 *
 * The obstacle is that a skill's `<location>` has to be a real readable path —
 * the model loads a skill by `read`ing it — and the Bun-compiled binary has no
 * `templates/` beside it. So rather than resolve the package directory (which
 * differs across npm/pnpm/source/binary layouts and would give the compiled
 * binary a silently degraded skill set), every install materializes the same
 * embedded copy into a cache directory. One code path, same behaviour
 * everywhere.
 *
 * The cache is keyed by content hash, so an upgrade writes a new directory and
 * a dev build that changes a skill without changing the version still takes
 * effect. It is a cache, not user-editable state: `~/.agents/skills` is where a
 * user's own skills go, and nothing here ever writes there.
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { getAgentDir } from "../config.js";
import { EMBEDDED_SKILLS } from "../init-templates.generated.js";

/** What decides whether a built-in skill is registered this session. */
export interface BuiltinSkillGate {
	/** The `enablePluginTools` setting (the plugin system's master switch). */
	enablePluginTools: boolean;
}

export interface BuiltinSkill {
	/** Directory name under `templates/skills`, and the skill's own name. */
	name: string;
	/** Why hoocode ships it. Documentation, and the catalog test reads it. */
	summary: string;
	/**
	 * Registered only when this returns true. A skill costs its description on
	 * every turn, so one that only makes sense alongside a feature rides that
	 * feature's switch rather than the default user's token budget.
	 *
	 * Omit for a skill that should always be available.
	 */
	gate?: (options: BuiltinSkillGate) => boolean;
}

export const BUILTIN_SKILLS: readonly BuiltinSkill[] = [
	{
		name: "plugin-authoring",
		summary:
			"The craft half of ProposePlugin/UpdatePlugin: when a capability is worth extracting, naming it so it triggers again, portability, and the hook trap.",
		// Useless without the tools it describes, and those are off by default,
		// so this costs nothing for a user who never enables the plugin system.
		gate: (options) => options.enablePluginTools,
	},
];

/** Stable short hash of the embedded skill tree; the cache directory's name. */
function contentHash(): string {
	const hash = createHash("sha256");
	for (const key of Object.keys(EMBEDDED_SKILLS).sort()) {
		hash.update(key);
		hash.update("\0");
		hash.update(EMBEDDED_SKILLS[key] ?? "");
		hash.update("\0");
	}
	return hash.digest("hex").slice(0, 12);
}

/** Root of the materialized copy for the current content. */
export function builtinSkillsCacheDir(agentDir: string = getAgentDir()): string {
	return join(agentDir, "cache", "builtin-skills", contentHash());
}

/**
 * Write the embedded skills to the cache directory if they are not already
 * there, and return its path.
 *
 * Returns null when nothing could be written — a read-only home, a full disk.
 * That is a degraded session, not a broken one: the caller contributes no skill
 * paths and hoocode runs exactly as it did before these existed.
 */
export function materializeBuiltinSkills(agentDir: string = getAgentDir()): string | null {
	const root = builtinSkillsCacheDir(agentDir);
	try {
		for (const [relativePath, content] of Object.entries(EMBEDDED_SKILLS)) {
			const target = join(root, relativePath);
			// Content is hash-addressed, so an existing file with the right size is
			// already correct; re-reading beats re-writing on every startup.
			if (existsSync(target) && readFileSync(target, "utf-8") === content) continue;
			mkdirSync(dirname(target), { recursive: true });
			// Write-then-rename so a killed process never leaves a half-written
			// SKILL.md that would parse as a malformed skill on the next run.
			const temp = `${target}.${process.pid}.tmp`;
			writeFileSync(temp, content, "utf-8");
			renameSync(temp, target);
		}
		return root;
	} catch {
		// Drop a partial tree so the next run rebuilds it rather than loading a
		// half-written skill. The cleanup gets its own guard: `force` swallows
		// ENOENT but not ENOTDIR, and a cleanup that throws would turn the
		// degraded path back into a crash — which is the failure this whole
		// branch exists to prevent.
		try {
			rmSync(root, { recursive: true, force: true });
		} catch {}
		return null;
	}
}

/**
 * The skill directories to load this session, after gating.
 *
 * Returns per-skill directories rather than the root so a gated-off skill is
 * genuinely absent rather than loaded and filtered later — the load is what
 * costs the description on every turn.
 */
export function builtinSkillPaths(gate: BuiltinSkillGate, agentDir: string = getAgentDir()): string[] {
	const enabled = BUILTIN_SKILLS.filter((skill) => !skill.gate || skill.gate(gate));
	if (enabled.length === 0) return [];

	const root = materializeBuiltinSkills(agentDir);
	if (!root) return [];

	return enabled.map((skill) => join(root, skill.name)).filter((dir) => existsSync(dir));
}
