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
 * And `github/awesome-copilot`, which is pinned to a distribution branch rather
 * than a default branch — see the block at the bottom for why that is load
 * bearing and what breaks silently without a check here.
 *
 * Network-dependent: when the marketplace clones fail (offline sandbox) the
 * tests skip rather than fail.
 */

import { existsSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import { pluginCanvasExtensions } from "../src/core/canvas/plugin-canvases.js";
import { ensureWellKnownMarketplaces } from "../src/core/extensions/plugins/install.js";
import { createAgentSession } from "../src/core/sdk.js";
import { SessionManager } from "../src/core/session-manager.js";
import { createInstallPluginToolDefinition, createSearchPluginsToolDefinition } from "../src/core/tools/plugins.js";

let tempDir: string;
let priorAgentDir: string | undefined;
let online = false;

function textOf(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content.map((c) => c.text ?? "").join("\n");
}

beforeAll(async () => {
	tempDir = join(tmpdir(), `hoo-e2e-copilot-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	// The marketplace cache and the install home hang off the agent dir, so the
	// clone and the tools that read it must agree on one — and it must not be the
	// real home.
	priorAgentDir = process.env[ENV_AGENT_DIR];
	process.env[ENV_AGENT_DIR] = join(tempDir, ".hoocode");
	const errors = await ensureWellKnownMarketplaces();
	online = errors.length === 0;
}, 120_000);

afterAll(() => {
	if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
	else process.env[ENV_AGENT_DIR] = priorAgentDir;
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
		// skill is live in the running session's system prompt — namespaced to its
		// plugin, as plugin skills now are.
		const skill = session.resourceLoader
			.getSkills()
			.skills.find((s) => s.name.endsWith(":spark-app-template") || s.name === "spark-app-template");
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

/**
 * `github/awesome-copilot` is the one well-known marketplace hoocode does not
 * read from a default branch, and the reason is not cosmetic. There, a
 * `plugins/<name>/` directory holds only `plugin.json` and a README; the real
 * content lives in top-level trees, is named from the manifest's own
 * `extensions["com.github.awesome-copilot"]` namespace, and is materialized into
 * each plugin directory by CI onto the `marketplace` branch — the branch the
 * vendor's own installer reads.
 *
 * Read from the default branch, every entry in that catalog installs as an empty
 * shell and truthfully reports that it contributes nothing. That is a silent
 * failure in the worst way: the install *succeeds*, and it reads as a missing
 * hoocode feature rather than a wrong ref. Nothing in the unit tests can catch a
 * regression here, because the difference lives entirely in what the remote
 * serves. Hence this.
 *
 * `arcade-canvas` is the case to pin on: it carries no skills, commands or
 * hooks, so a canvas is the *only* thing that can prove the content arrived.
 */
describe("E2E: awesome-copilot marketplace, pinned to its distribution branch", () => {
	it("installs arcade-canvas with its canvas intact, and /canvas can see it", async () => {
		if (!online) return;
		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir: join(tempDir, ".agent-home3"),
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
			"t4",
			{ name: "arcade-canvas", reason: "E2E: canvas surface from a materialized marketplace branch" },
			undefined as never,
			undefined as never,
			ctx,
		);
		expect((result as { details: { installed: boolean } }).details.installed).toBe(true);

		// The stub-branch symptom, asserted directly: an install off the default
		// branch lands a directory with nothing in it but a manifest and a README.
		const dest = join(tempDir, ".agents", "plugins", "arcade-canvas");
		const entry = join(dest, "com.github.copilot", "extensions", "arcade-canvas", CANVAS_ENTRY_FILE);
		expect(existsSync(entry)).toBe(true);

		// And the surface hoocode actually exposes: the canvas is discoverable by
		// the same path `/canvas list` walks.
		const canvases = pluginCanvasExtensions(tempDir, join(tempDir, ".hoocode"));
		expect(canvases.map((c) => c.id)).toContain("arcade-canvas");

		// The install summary has to say so too — a canvas is the one capability
		// that is never loaded on activation, so if it goes unmentioned the install
		// reads as inert.
		expect(textOf(result as never)).toContain("arcade-canvas");

		// ...and "never loaded" is the other half of the contract: activating the
		// plugin must not have started a process or put anything in the prompt.
		expect(session.systemPrompt).not.toContain("arcade-canvas");
	}, 180_000);
});
