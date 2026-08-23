/**
 * The bundled `plugin-authoring` plugin.
 *
 * ProposePlugin/UpdatePlugin shed their how-to-author-well guidance from the
 * always-on prompt surface on the understanding that it is reachable as a
 * skill instead. That trade is only sound while the plugin actually ships and
 * its skill actually parses — a malformed SKILL.md would silently drop the
 * guidance rather than fail loudly, leaving the tools thinner with nothing
 * behind them. These tests are what make the trade checkable.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { defaultMarketplaceRecord } from "../src/core/extensions/plugins/install.js";
import { parseMarketplaceDir } from "../src/core/extensions/plugins/marketplace.js";
import { loadSkillsFromDir } from "../src/core/skills.js";

const PLUGIN_NAME = "plugin-authoring";

function pluginDir(): string {
	return join(defaultMarketplaceRecord().dir, "plugins", PLUGIN_NAME);
}

describe("bundled plugin-authoring plugin", () => {
	it("is listed in the default marketplace", () => {
		const manifest = parseMarketplaceDir(defaultMarketplaceRecord().dir);
		expect(manifest?.plugins.map((p) => p.name)).toContain(PLUGIN_NAME);
	});

	it("ships a skill that parses, with a description that says when to use it", () => {
		const result = loadSkillsFromDir({ dir: join(pluginDir(), "skills"), source: "user" });
		expect(result.diagnostics.filter((d) => d.type === "error")).toEqual([]);

		const skill = result.skills.find((s) => s.name === PLUGIN_NAME);
		expect(skill).toBeDefined();
		// The description is the only part loaded every turn and the whole basis
		// for the skill being chosen, so it has to name the trigger, not just the
		// topic. "Use before calling ProposePlugin" is that trigger.
		expect(skill?.description).toMatch(/ProposePlugin/);
		expect(skill?.description.length).toBeGreaterThan(80);
	});

	it("carries the guidance the plugin tools stopped shipping", () => {
		const body = readFileSync(join(pluginDir(), "skills", PLUGIN_NAME, "SKILL.md"), "utf-8");
		// Each of these left a tool's promptGuidelines and has to land somewhere.
		expect(body).toMatch(/portab/i);
		expect(body).toMatch(/capability, not the/i);
		// The hook trap is the one that silently doubles a side effect.
		expect(body).toMatch(/second hook/i);
		// The two prohibitions stay in the tools as well; losing them here would
		// still be a regression in the explanation of why they exist.
		expect(body).toMatch(/never grant a subagent/i);
		expect(body).toMatch(/publish/i);
	});
});
