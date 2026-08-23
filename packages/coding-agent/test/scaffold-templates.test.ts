/**
 * `/new-skill`, `/new-agent` and `/new-command` scaffold the same content
 * whichever path they take.
 *
 * Each command has two: the `--platform` emitters (structured fields handed to
 * a per-vendor writer) and the plain `.hoocode/` writer (a hand-assembled
 * markdown file). Each path used to carry its own copy of the body, and they
 * had drifted — the `.hoocode/` command body documented the `${@:N}` slice
 * placeholders that the platform one omitted, so what `/new-command` taught you
 * depended on whether `--platform` was set.
 *
 * These tests read the emitted files rather than the constants, because the
 * constants agreeing is not the property that matters — the files a user ends
 * up with is.
 */

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setPlatforms } from "../src/core/extensions/plugins/formats/platform-targets.js";
import { setupScaffold } from "../src/extensions/core/scaffold.js";

let cwd = "";

/** Collect the commands setupScaffold registers, keyed by name. */
function registerCommands() {
	const handlers = new Map<string, (args: string, ctx: any) => Promise<void>>();
	const pi = {
		registerCommand: (name: string, def: { handler: (args: string, ctx: any) => Promise<void> }) => {
			handlers.set(name, def.handler);
		},
	} as never;
	setupScaffold(pi);
	return handlers;
}

function ctxFor(dir: string) {
	return { cwd: dir, ui: { notify: () => {} } } as never;
}

beforeEach(() => {
	cwd = mkdtempSync(join(tmpdir(), "scaffold-"));
});

afterEach(() => {
	setPlatforms(undefined);
	vi.restoreAllMocks();
	if (cwd) rmSync(cwd, { recursive: true, force: true });
	cwd = "";
});

/** Run one scaffold command with platforms unset (the `.hoocode/` path). */
async function scaffoldLocal(command: string, name: string): Promise<void> {
	setPlatforms(undefined);
	const handlers = registerCommands();
	await handlers.get(command)?.(name, ctxFor(cwd));
}

/** Run one scaffold command targeting the claude layout (the platform path). */
async function scaffoldPlatform(command: string, name: string): Promise<void> {
	setPlatforms(["claude"]);
	const handlers = registerCommands();
	await handlers.get(command)?.(name, ctxFor(cwd));
}

function read(...parts: string[]): string {
	return readFileSync(join(cwd, ...parts), "utf-8");
}

describe("scaffold bodies agree across the two write paths", () => {
	it("/new-command documents the slice placeholders on both paths", async () => {
		await scaffoldLocal("new-command", "alpha");
		const local = read(".hoocode", "commands", "alpha.md");
		// The line the platform path used to silently omit.
		expect(local).toContain("bash-style slices");

		await scaffoldPlatform("new-command", "beta");
		const platform = read(".claude", "commands", "beta.md");
		expect(platform).toContain("bash-style slices");
		expect(platform).toContain("$1, $2, ... for positional arguments");
	});

	it("/new-agent writes the same body on both paths", async () => {
		await scaffoldLocal("new-agent", "alpha");
		const local = read(".hoocode", "agents", "alpha.md");
		expect(local).toContain("You are a alpha subagent running inside hoocode.");

		await scaffoldPlatform("new-agent", "beta");
		const platform = read(".claude", "agents", "beta.md");
		// Previously "You are a beta subagent." — no mention of hoocode.
		expect(platform).toContain("You are a beta subagent running inside hoocode.");
	});

	it("/new-skill writes the same body and description on both paths", async () => {
		await scaffoldLocal("new-skill", "alpha");
		const local = read(".hoocode", "skills", "alpha", "SKILL.md");
		expect(local).toContain("the agent reads this to decide whether to load it");
		expect(local).toContain("# alpha");

		await scaffoldPlatform("new-skill", "beta");
		const platform = read(".claude", "skills", "beta", "SKILL.md");
		expect(platform).toContain("the agent reads this to decide whether to load it");
		expect(platform).toContain("# beta");
	});
});
