/**
 * Workspace trust: which plugins are allowed to run code, and on whose say-so.
 *
 * The property under test is the one the record exists for — a plugin in the
 * working tree runs its skills either way, but starts hooks and MCP servers only
 * where a person on this machine said the directory was theirs to run code from.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { createEventBus } from "../src/core/event-bus.js";
import {
	createExtensionRuntime,
	defaultPluginDirs,
	isProjectSuppliedPlugin,
	loadPlugins,
	shouldWithholdExecutables,
} from "../src/core/extensions/loader.js";
import { pluginScopeOf } from "../src/core/extensions/plugins/locations.js";
import {
	isWorkspaceTrusted,
	listTrustedWorkspaces,
	trustStorePath,
	trustWorkspace,
	untrustWorkspace,
} from "../src/core/extensions/plugins/trust.js";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

/** A plugin carrying both a passive capability and an executable one. */
function seedPlugin(root: string, id: string): void {
	writeJson(path.join(root, ".agents-plugin", "plugin.json"), { name: id });
	fs.mkdirSync(path.join(root, "skills", "s"), { recursive: true });
	fs.writeFileSync(path.join(root, "skills", "s", "SKILL.md"), "---\nname: s\ndescription: does s\n---\n\nDo s.\n");
	writeJson(path.join(root, "hooks", "hooks.json"), {
		hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "echo hi" }] }] },
	});
}

async function loadAndCollectIssues(cwd: string) {
	const res = await loadPlugins(defaultPluginDirs(cwd), cwd, createEventBus(), createExtensionRuntime());
	return res.errors;
}

async function loadAndCollectErrors(cwd: string): Promise<string[]> {
	return (await loadAndCollectIssues(cwd)).map((e) => e.error);
}

describe("workspace trust", () => {
	let home: string;
	let cwd: string;
	let prior: string | undefined;

	beforeEach(() => {
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-trust-home-"));
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-trust-cwd-"));
		prior = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(home, ".hoocode");
	});

	afterEach(() => {
		if (prior === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = prior;
		fs.rmSync(home, { recursive: true, force: true });
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("stores trust outside the repository, so repo content cannot grant it", () => {
		trustWorkspace(cwd);
		const store = trustStorePath();
		expect(fs.existsSync(store)).toBe(true);
		// The whole design in one assertion: a file the working tree can write is
		// not a trust record, because everything in the tree travels with a clone.
		expect(store.startsWith(path.resolve(cwd))).toBe(false);
		expect(store.startsWith(path.resolve(home))).toBe(true);
	});

	it("grants, revokes, and reports trust", () => {
		expect(isWorkspaceTrusted(cwd)).toBe(false);
		trustWorkspace(cwd);
		expect(isWorkspaceTrusted(cwd)).toBe(true);
		expect(listTrustedWorkspaces().map((w) => w.path)).toContain(path.resolve(cwd));

		expect(untrustWorkspace(cwd)).toBe(true);
		expect(isWorkspaceTrusted(cwd)).toBe(false);
		// Revoking something that was never trusted is a no-op, not an error.
		expect(untrustWorkspace(cwd)).toBe(false);
	});

	it("does not let trusting a parent directory trust everything cloned beneath it", () => {
		const child = path.join(cwd, "nested-repo");
		fs.mkdirSync(child, { recursive: true });
		trustWorkspace(cwd);
		// A prefix match here would silently trust every repository someone clones
		// into their projects folder the first time they trust one level too high.
		expect(isWorkspaceTrusted(child)).toBe(false);
	});

	it("treats all three working-tree plugin homes as repository-supplied", () => {
		for (const rel of [".claude/skills/p", ".agents/plugins/p", ".hoocode/plugins/p"]) {
			expect(isProjectSuppliedPlugin(path.join(cwd, rel), cwd)).toBe(true);
		}
		// The user homes are not in the working tree at all.
		expect(isProjectSuppliedPlugin(path.join(home, ".agents", "plugins", "p"), cwd)).toBe(false);
	});

	it("withholds executables from a working-tree plugin until the workspace is trusted", async () => {
		seedPlugin(path.join(cwd, ".agents", "plugins", "hooky"), "hooky");

		expect(shouldWithholdExecutables(path.join(cwd, ".agents", "plugins", "hooky"), cwd)).toBe(true);
		const before = await loadAndCollectErrors(cwd);
		expect(before.some((e) => e.includes("hooky") && e.includes("not loaded"))).toBe(true);
		expect(before.some((e) => e.includes("/plugin trust"))).toBe(true);

		trustWorkspace(cwd);
		expect(shouldWithholdExecutables(path.join(cwd, ".agents", "plugins", "hooky"), cwd)).toBe(false);
		const after = await loadAndCollectErrors(cwd);
		expect(after.some((e) => e.includes("hooky"))).toBe(false);
	});

	it("reports withholding as a warning, so the session that runs `/plugin trust` can start", async () => {
		seedPlugin(path.join(cwd, ".agents", "plugins", "hooky"), "hooky");

		const issues = await loadAndCollectIssues(cwd);
		const notice = issues.find((e) => e.error.includes("hooky"));
		expect(notice).toBeDefined();
		// An "error" here aborts startup (main.ts exits on any error diagnostic),
		// which would put the only remedy — /plugin trust, a slash command — behind
		// a prompt the user can never reach.
		expect(notice?.severity).toBe("warning");
	});

	it("never withholds from a user-scoped plugin, trusted workspace or not", async () => {
		seedPlugin(path.join(home, ".agents", "plugins", "userplug"), "userplug");
		expect(shouldWithholdExecutables(path.join(home, ".agents", "plugins", "userplug"), cwd)).toBe(false);
		expect(await loadAndCollectErrors(cwd)).toEqual([]);
	});

	it("classifies each plugin home into the scope ListPlugins reports", () => {
		expect(pluginScopeOf(path.join(home, ".agents", "plugins", "p"), cwd)).toBe("user");
		expect(pluginScopeOf(path.join(cwd, ".agents", "plugins", "p"), cwd)).toBe("project");
		expect(pluginScopeOf(path.join(cwd, ".hoocode", "plugins", "p"), cwd)).toBe("project");
		// Arrived with the repo rather than through an install — project-located,
		// but nobody here chose it.
		expect(pluginScopeOf(path.join(cwd, ".claude", "skills", "p"), cwd)).toBe("repo");
	});
});
