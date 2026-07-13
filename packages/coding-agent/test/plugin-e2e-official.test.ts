/**
 * E2E for the single-turn capability loop against the REAL official marketplace
 * (https://github.com/anthropics/claude-plugins-official):
 *
 *   SearchPlugins → InstallPlugin → live activation → capability usable NOW,
 *
 * with no human /reload in between. The marketplace index is cloned once into
 * the temp project's `.agents/marketplace-cache` (network); when the clone
 * fails (offline sandbox), the network-dependent tests are skipped rather than
 * failed.
 */

import { existsSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ensureWellKnownMarketplaces, installedPluginsDir } from "../src/core/extensions/plugins/install.js";
import { createAgentSession } from "../src/core/sdk.js";
import { SessionManager } from "../src/core/session-manager.js";
import {
	createInstallPluginToolDefinition,
	createSearchPluginsToolDefinition,
	createUninstallPluginToolDefinition,
} from "../src/core/tools/plugins.js";

const PLUGIN = "skill-creator"; // official, skills-only, sourced locally within the marketplace repo

let tempDir: string;
let online = false;

function textOf(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content.map((c) => c.text ?? "").join("\n");
}

beforeAll(async () => {
	tempDir = join(tmpdir(), `hoo-e2e-official-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	const errors = await ensureWellKnownMarketplaces(tempDir);
	online = errors.length === 0;
}, 120_000);

afterAll(() => {
	if (tempDir) rmSync(tempDir, { recursive: true, force: true });
});

describe("E2E: search → install → use in a single turn (official marketplace)", () => {
	it("SearchPlugins fetches the official index lazily and finds skill-creator", async () => {
		if (!online) return; // offline sandbox — nothing to assert
		const search = createSearchPluginsToolDefinition();
		const ctx = { cwd: tempDir, hasUI: false, ui: { notify: () => {} } } as never;
		const result = await search.execute("t1", { query: PLUGIN }, undefined as never, undefined as never, ctx);
		const text = textOf(result as never);
		expect(text).toContain(PLUGIN);
		expect(text).toContain("claude-plugins-official");
	});

	it("InstallPlugin live-activates the plugin: the skill is usable in the SAME turn", async () => {
		if (!online) return;
		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir: join(tempDir, ".agent-home"),
			sessionManager: SessionManager.inMemory(),
		});

		// The skill must not be known before install.
		expect(session.resourceLoader.getSkills().skills.some((s) => s.name === PLUGIN)).toBe(false);

		const notifications: string[] = [];
		const ctx = {
			cwd: tempDir,
			hasUI: false,
			ui: { notify: (m: string) => notifications.push(m) },
			// Same delegation the real ExtensionContext performs.
			activatePlugin: (dir: string) => session.activatePlugin(dir),
			requestReloadWhenIdle: () => session.requestReloadWhenIdle(),
		} as never;

		const install = createInstallPluginToolDefinition();
		const result = await install.execute(
			"t2",
			{ name: PLUGIN, reason: "E2E: acquire the skill-creator capability" },
			undefined as never,
			undefined as never,
			ctx,
		);
		const text = textOf(result as never);
		expect((result as { details: { installed: boolean } }).details.installed).toBe(true);
		expect(text).toContain(`Installed "${PLUGIN}"`);
		// Live activation happened and reported the capability as usable now.
		expect(text).toContain("activated in the live session");
		expect(text).toContain("Active NOW");
		expect(text).toContain(PLUGIN);

		// The capability is registered in the running session…
		const skills = session.resourceLoader.getSkills().skills;
		const skill = skills.find((s) => s.name === PLUGIN);
		expect(skill).toBeDefined();

		// …injected into the system prompt the model sees…
		expect(session.systemPrompt).toContain(PLUGIN);

		// …and the mid-turn context refresh hands the running loop the new prompt
		// before its next provider request (the "same turn" guarantee).
		const refreshed = await session.agent.prepareNextTurn?.(
			{ context: { systemPrompt: "stale", messages: [], tools: [] } } as never,
			undefined,
		);
		expect(refreshed?.context?.systemPrompt).toContain(PLUGIN);

		// "Use" the capability the way the model does: read the skill body on demand.
		const body = readFileSync(skill!.filePath, "utf8");
		expect(body.length).toBeGreaterThan(100);

		// Reversibility: uninstall removes it from disk.
		const uninstall = createUninstallPluginToolDefinition();
		const unResult = await uninstall.execute("t3", { name: PLUGIN }, undefined as never, undefined as never, ctx);
		expect((unResult as { details: { removed: boolean } }).details.removed).toBe(true);
		expect(existsSync(join(installedPluginsDir(tempDir), PLUGIN))).toBe(false);
	}, 120_000);
});
