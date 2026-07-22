import { existsSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createAgentSessionServices } from "../../src/core/agent-session-services.js";
import { SettingsManager } from "../../src/core/settings-manager.js";

/**
 * Persisted extension-flag overrides (settings.json `flags`) are seeded into the
 * runtime at startup for known flags, while an explicit CLI --flag still wins.
 */
describe("persisted flag overrides", () => {
	let tempDir: string;
	let agentDir: string;

	beforeEach(() => {
		tempDir = join(tmpdir(), `hoo-flags-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		agentDir = join(tempDir, "agent");
		mkdirSync(agentDir, { recursive: true });
	});

	afterEach(() => {
		if (tempDir && existsSync(tempDir)) {
			rmSync(tempDir, { recursive: true, force: true });
		}
	});

	async function buildServices(opts: {
		overrides?: Record<string, boolean | string>;
		cliFlags?: Map<string, boolean | string>;
	}) {
		const settingsManager = SettingsManager.create(tempDir, agentDir);
		for (const [name, value] of Object.entries(opts.overrides ?? {})) {
			settingsManager.setFlagOverride(name, value);
		}
		await settingsManager.flush();

		return createAgentSessionServices({
			cwd: tempDir,
			agentDir,
			settingsManager,
			extensionFlagValues: opts.cliFlags,
			resourceLoaderOptions: {
				extensionFactories: [
					(pi) => {
						pi.registerFlag("bool-flag", { type: "boolean", default: false, description: "b" });
						pi.registerFlag("str-flag", { type: "string", description: "s" });
					},
				],
			},
		});
	}

	it("seeds a persisted boolean flag into the runtime", async () => {
		const services = await buildServices({ overrides: { "bool-flag": true } });
		const runtime = services.resourceLoader.getExtensions().runtime;
		expect(runtime.flagValues.get("bool-flag")).toBe(true);
	});

	it("ignores a persisted flag that no extension registered (no startup error)", async () => {
		const services = await buildServices({ overrides: { "ghost-flag": true } });
		const runtime = services.resourceLoader.getExtensions().runtime;
		expect(runtime.flagValues.has("ghost-flag")).toBe(false);
		expect(services.diagnostics.some((d) => d.type === "error")).toBe(false);
	});

	it("lets an explicit CLI flag win over the persisted override", async () => {
		const services = await buildServices({
			overrides: { "str-flag": "from-settings" },
			cliFlags: new Map([["str-flag", "from-cli"]]),
		});
		const runtime = services.resourceLoader.getExtensions().runtime;
		expect(runtime.flagValues.get("str-flag")).toBe("from-cli");
	});
});
