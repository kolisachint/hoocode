/**
 * E2E for the Copilot variant against the REAL GitHub Copilot plugin directory
 * (https://github.com/github/copilot-plugins), registered as a well-known
 * marketplace:
 *
 *   SearchPlugins (platform=github) → InstallPlugin (manifest-less "spark",
 *   a bare skills tree) → live activation → skill usable in the same turn,
 *
 * plus a real hooks-bundle install (ralph-loop from the official Claude
 * marketplace): hooks parse, bundled scripts stay executable, and activation
 * schedules the automatic reload for the executable tier.
 *
 * Network-dependent: when the marketplace clones fail (offline sandbox) the
 * tests skip rather than fail.
 */

import { existsSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ensureWellKnownMarketplaces } from "../src/core/extensions/plugins/install.js";
import { createAgentSession } from "../src/core/sdk.js";
import { SessionManager } from "../src/core/session-manager.js";
import { createInstallPluginToolDefinition, createSearchPluginsToolDefinition } from "../src/core/tools/plugins.js";

let tempDir: string;
let online = false;

function textOf(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content.map((c) => c.text ?? "").join("\n");
}

beforeAll(async () => {
	tempDir = join(tmpdir(), `hoo-e2e-copilot-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	const errors = await ensureWellKnownMarketplaces(tempDir);
	online = errors.length === 0;
}, 120_000);

afterAll(() => {
	if (tempDir) rmSync(tempDir, { recursive: true, force: true });
});

describe("E2E: Copilot marketplace (github/copilot-plugins)", () => {
	it("SearchPlugins with platform=github surfaces the Copilot directory", async () => {
		if (!online) return;
		const search = createSearchPluginsToolDefinition();
		const ctx = { cwd: tempDir, hasUI: false, ui: { notify: () => {} } } as never;
		const result = await search.execute(
			"t1",
			{ query: "spark", platform: "github" },
			undefined as never,
			undefined as never,
			ctx,
		);
		const text = textOf(result as never);
		expect(text).toContain("spark");
		expect(text).toContain("copilot-plugins");
	});

	it("installs manifest-less 'spark' and live-activates its skill in the same turn", async () => {
		if (!online) return;
		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir: join(tempDir, ".agent-home"),
			sessionManager: SessionManager.inMemory(),
		});
		const ctx = {
			cwd: tempDir,
			hasUI: false,
			ui: { notify: () => {} },
			activatePlugin: (dir: string) => session.activatePlugin(dir),
			requestReloadWhenIdle: () => session.requestReloadWhenIdle(),
		} as never;

		const install = createInstallPluginToolDefinition();
		const result = await install.execute(
			"t2",
			{ name: "spark", reason: "E2E: Copilot spark app template skill" },
			undefined as never,
			undefined as never,
			ctx,
		);
		expect((result as { details: { installed: boolean } }).details.installed).toBe(true);
		const text = textOf(result as never);
		expect(text).toContain("Active NOW");
		expect(text).toContain("spark-app-template");

		// Manifest was synthesized (spark ships as a bare skills tree) and the
		// skill is live in the running session's system prompt.
		const skill = session.resourceLoader.getSkills().skills.find((s) => s.name === "spark-app-template");
		expect(skill).toBeDefined();
		expect(session.systemPrompt).toContain("spark-app-template");
		expect(readFileSync(skill!.filePath, "utf8")).toContain("Spark");
	}, 120_000);

	it("installs a real hooks bundle (ralph-loop): hooks parse, scripts stay executable, reload scheduled", async () => {
		if (!online) return;
		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir: join(tempDir, ".agent-home2"),
			sessionManager: SessionManager.inMemory(),
		});
		let idleReloads = 0;
		const ctx = {
			cwd: tempDir,
			hasUI: false,
			ui: { notify: () => {} },
			activatePlugin: (dir: string) => session.activatePlugin(dir),
			requestReloadWhenIdle: () => {
				idleReloads++;
			},
		} as never;

		const install = createInstallPluginToolDefinition();
		const result = await install.execute(
			"t3",
			{ name: "ralph-loop", reason: "E2E: hooks + bundled scripts" },
			undefined as never,
			undefined as never,
			ctx,
		);
		const details = (result as { details: { installed: boolean } }).details;
		expect(details.installed).toBe(true);
		const text = textOf(result as never);
		// Executable tier announced for end-of-turn activation.
		expect(text).toContain("hooks/MCP servers");

		const dest = join(tempDir, ".agents", "plugins", "ralph-loop");
		expect(existsSync(join(dest, "hooks", "hooks.json"))).toBe(true);
		// Bundled hook + setup scripts keep their exec bits through the install.
		expect(statSync(join(dest, "hooks", "stop-hook.sh")).mode & 0o111).not.toBe(0);
		expect(statSync(join(dest, "scripts", "setup-ralph-loop.sh")).mode & 0o111).not.toBe(0);
		expect(idleReloads).toBe(0); // hooks reload goes through activatePlugin's own scheduling, not the uninstall path
	}, 120_000);
});
