import { existsSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { getModel } from "@kolisachint/hoocode-ai";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createAgentSessionFromServices, createAgentSessionServices } from "../../src/core/agent-session-services.js";
import { SessionManager } from "../../src/core/session-manager.js";
import { SettingsManager } from "../../src/core/settings-manager.js";

/**
 * Guards the `disabledTools` feature end-to-end on the interactive path:
 *  - `createAgentSessionFromServices` forwards `disallowedTools` (it previously
 *    dropped it), so a disabled tool is removed from the session's registry.
 *  - the denylist subtracts even from an explicit `--tools` allowlist.
 */
describe("disabled tools (services path)", () => {
	let tempDir: string;
	let agentDir: string;

	beforeEach(() => {
		tempDir = join(tmpdir(), `hoo-disabled-tools-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		agentDir = join(tempDir, "agent");
		mkdirSync(agentDir, { recursive: true });
	});

	afterEach(() => {
		if (tempDir && existsSync(tempDir)) {
			rmSync(tempDir, { recursive: true, force: true });
		}
	});

	async function createSession(opts: { tools?: string[]; disallowedTools?: string[] }) {
		const settingsManager = SettingsManager.create(tempDir, agentDir);
		const sessionManager = SessionManager.inMemory(tempDir);
		const services = await createAgentSessionServices({ cwd: tempDir, agentDir, settingsManager });
		const { session } = await createAgentSessionFromServices({
			services,
			sessionManager,
			model: getModel("anthropic", "claude-sonnet-4-5")!,
			tools: opts.tools,
			disallowedTools: opts.disallowedTools,
		});
		await session.bindExtensions({});
		return session;
	}

	it("removes a disabled tool from the registry", async () => {
		const session = await createSession({ disallowedTools: ["bash"] });
		const names = session.getAllTools().map((t) => t.name);
		expect(names).toContain("read");
		expect(names).not.toContain("bash");
		expect(session.getActiveToolNames()).not.toContain("bash");
		session.dispose();
	});

	it("keeps a tool disabled even when it is also in the --tools allowlist", async () => {
		const session = await createSession({ tools: ["read", "bash"], disallowedTools: ["bash"] });
		expect(session.getAllTools().map((t) => t.name)).toEqual(["read"]);
		expect(session.getActiveToolNames()).toEqual(["read"]);
		session.dispose();
	});
});
