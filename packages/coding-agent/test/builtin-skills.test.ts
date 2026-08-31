/**
 * Skills hoocode ships itself.
 *
 * hoocode read skills from every source except its own, which is why it shipped
 * three subagents and zero skills while telling users skills are the extension
 * unit. These tests hold the three properties that make the mechanism safe to
 * have: the skills actually parse, a gated one costs nothing when its feature is
 * off, and a failure to materialize degrades to "no built-in skills" rather than
 * to a broken session.
 */

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	BUILTIN_SKILLS,
	builtinSkillPaths,
	builtinSkillsCacheDir,
	materializeBuiltinSkills,
} from "../src/core/builtin-skills.js";
import { loadSkillsFromDir } from "../src/core/skills.js";
import { EMBEDDED_SKILLS } from "../src/init-templates.generated.js";

let agentDir = "";

beforeEach(() => {
	agentDir = mkdtempSync(join(tmpdir(), "builtin-skills-"));
});

afterEach(() => {
	if (agentDir) rmSync(agentDir, { recursive: true, force: true });
	agentDir = "";
});

const ON = { enablePluginTools: true };
const OFF = { enablePluginTools: false };

describe("the built-in skill catalog", () => {
	it("has an embedded SKILL.md for every catalog entry", () => {
		// A catalog entry with no file materializes an empty directory and the
		// skill silently does not exist; a file with no entry never loads.
		for (const skill of BUILTIN_SKILLS) {
			expect(EMBEDDED_SKILLS[`${skill.name}/SKILL.md`], skill.name).toBeTruthy();
		}
		const embeddedNames = new Set(Object.keys(EMBEDDED_SKILLS).map((path) => path.split("/")[0]));
		expect([...embeddedNames].sort()).toEqual(BUILTIN_SKILLS.map((s) => s.name).sort());
	});

	it("parses every shipped skill, with the frontmatter name matching its directory", () => {
		const root = materializeBuiltinSkills(agentDir);
		expect(root).not.toBeNull();
		for (const skill of BUILTIN_SKILLS) {
			const result = loadSkillsFromDir({ dir: join(root as string, skill.name), source: "user" });
			expect(
				result.diagnostics.filter((d) => d.type === "error"),
				skill.name,
			).toEqual([]);
			expect(
				result.skills.map((s) => s.name),
				skill.name,
			).toContain(skill.name);
		}
	});
});

describe("the plugin-authoring skill", () => {
	it("says when to use it, not just what it is", () => {
		const root = materializeBuiltinSkills(agentDir) as string;
		const skill = loadSkillsFromDir({ dir: join(root, "plugin-authoring"), source: "user" }).skills[0];
		// The description is the only part loaded every turn and the whole basis
		// for the skill being chosen, so it has to name the trigger.
		expect(skill?.description).toMatch(/ProposePlugin/);
	});

	it("carries the guidance the plugin tools stopped shipping", () => {
		// ProposePlugin/UpdatePlugin shed ~325 tok/turn on the understanding that
		// it is reachable here. Each line below left a tool's promptGuidelines.
		const body = EMBEDDED_SKILLS["plugin-authoring/SKILL.md"] ?? "";
		expect(body).toMatch(/portab/i);
		expect(body).toMatch(/capability, not the/i);
		// The hook trap is the one that silently doubles a side effect.
		expect(body).toMatch(/second hook/i);
		expect(body).toMatch(/never grant a subagent/i);
		expect(body).toMatch(/publish/i);
	});
});

describe("materializing", () => {
	it("writes the embedded content to a content-addressed cache dir", () => {
		const root = materializeBuiltinSkills(agentDir);
		expect(root).toBe(builtinSkillsCacheDir(agentDir));
		for (const [relativePath, content] of Object.entries(EMBEDDED_SKILLS)) {
			expect(readFileSync(join(root as string, relativePath), "utf-8")).toBe(content);
		}
	});

	it("repairs a file that was corrupted in the cache", () => {
		const root = materializeBuiltinSkills(agentDir) as string;
		const [first] = Object.keys(EMBEDDED_SKILLS);
		writeFileSync(join(root, first), "corrupted", "utf-8");
		materializeBuiltinSkills(agentDir);
		expect(readFileSync(join(root, first), "utf-8")).toBe(EMBEDDED_SKILLS[first]);
	});

	it("is idempotent", () => {
		expect(materializeBuiltinSkills(agentDir)).toBe(materializeBuiltinSkills(agentDir));
	});
});

describe("gating", () => {
	it("withholds a gated skill when its feature is off", () => {
		// plugin-authoring rides enablePluginTools, which is off by default, so
		// the default user pays no per-turn description for it. artifact-design is
		// ungated and is the only thing a default session contributes.
		expect(builtinSkillPaths(OFF, agentDir)).toEqual([join(builtinSkillsCacheDir(agentDir), "artifact-design")]);
	});

	it("contributes the skill directory when the feature is on", () => {
		const paths = builtinSkillPaths(ON, agentDir);
		expect(paths).toContain(join(builtinSkillsCacheDir(agentDir), "plugin-authoring"));
	});

	it("materializes the whole tree but contributes only enabled skills", () => {
		const contributed = builtinSkillPaths(OFF, agentDir);
		// materializeBuiltinSkills is hash-addressed over the entire embedded
		// tree, so a gated-off skill's file still lands in the cache. What gating
		// controls is whether the loader is pointed at it — the file on disk costs
		// nothing per turn; a contributed path costs its description.
		expect(readFileSync(join(builtinSkillsCacheDir(agentDir), "plugin-authoring", "SKILL.md"), "utf-8")).toContain(
			"plugin-authoring",
		);
		expect(contributed).not.toContain(join(builtinSkillsCacheDir(agentDir), "plugin-authoring"));
	});
});

describe("degrading", () => {
	/**
	 * An agent dir that cannot hold a cache directory, in a way that holds for
	 * root too: the path is a regular file, so `mkdirSync` under it fails with
	 * ENOTDIR for every user. Permissions would not do — CI and containers often
	 * run as root, where a 0500 directory is still writable and the test would
	 * pass without exercising anything.
	 */
	function unwritableAgentDir(): string {
		const blocked = join(agentDir, "not-a-directory");
		writeFileSync(blocked, "", "utf-8");
		return blocked;
	}

	it("returns no paths rather than throwing when the cache cannot be written", () => {
		// A read-only home is a degraded session, not a broken one: hoocode should
		// run exactly as it did before built-in skills existed.
		const blocked = unwritableAgentDir();
		let paths: string[] = [];
		expect(() => {
			paths = builtinSkillPaths(ON, blocked);
		}).not.toThrow();
		expect(paths).toEqual([]);
	});

	it("reports the failure as null rather than a half-written root", () => {
		expect(materializeBuiltinSkills(unwritableAgentDir())).toBeNull();
	});
});
