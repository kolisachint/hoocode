/**
 * Hooks + bundled scripts, end to end through the install path.
 *
 * Real marketplace plugins ship executable scripts referenced from
 * `hooks/hooks.json` via `${CLAUDE_PLUGIN_ROOT}` (e.g. ralph-loop's
 * `hooks/stop-hook.sh`, hookify's python hooks). This verifies the whole
 * chain: install preserves script exec bits, the `{ description, hooks }`
 * wrapper shape parses, the bridge substitutes the plugin root, pipes the
 * event JSON on stdin, and honors the exit-code protocol (exit 2 = block).
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installPluginHooks } from "../src/core/extensions/plugins/hooks-bridge.js";
import { installAvailablePlugin } from "../src/core/extensions/plugins/install.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import { writeMarketplaceStore } from "../src/core/extensions/plugins/marketplace.js";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

/** Minimal ExtensionAPI stub: records handlers per event so tests can fire them. */
function makePi() {
	const handlers = new Map<string, (event: unknown) => Promise<unknown> | unknown>();
	const pi = {
		on(event: string, handler: (event: unknown) => Promise<unknown> | unknown) {
			handlers.set(event, handler);
		},
	} as never;
	return { pi, handlers };
}

describe("plugin hooks + bundled scripts (install → parse → bridge → run)", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-hooks-scripts-"));

		// A marketplace plugin bundling hooks + executable scripts, in the same
		// shape the official marketplace uses (wrapper object, ${CLAUDE_PLUGIN_ROOT}).
		const market = path.join(cwd, "market");
		writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
			name: "local",
			plugins: [{ name: "hooky", source: "./plugins/hooky", description: "hooks + scripts" }],
		});
		const plugin = path.join(market, "plugins", "hooky");
		writeJson(path.join(plugin, ".claude-plugin", "plugin.json"), { name: "hooky" });
		writeJson(path.join(plugin, "hooks", "hooks.json"), {
			description: "wrapper shape, like hookify/ralph-loop",
			hooks: {
				// biome-ignore lint/suspicious/noTemplateCurlyInString: literal placeholder expanded by the hook shell
				Stop: [{ hooks: [{ type: "command", command: 'bash "${CLAUDE_PLUGIN_ROOT}/scripts/mark.sh"' }] }],
				PreToolUse: [
					{
						matcher: "bash",
						// biome-ignore lint/suspicious/noTemplateCurlyInString: literal placeholder expanded by the hook shell
						hooks: [{ type: "command", command: 'bash "${CLAUDE_PLUGIN_ROOT}/scripts/block.sh"' }],
					},
				],
			},
		});
		const scripts = path.join(plugin, "scripts");
		fs.mkdirSync(scripts, { recursive: true });
		// mark.sh proves root substitution + stdin JSON delivery: it writes the
		// event payload it received into a file under the (installed) plugin root.
		fs.writeFileSync(path.join(scripts, "mark.sh"), '#!/usr/bin/env bash\ncat > "$CLAUDE_PLUGIN_ROOT/ran.json"\n');
		fs.writeFileSync(path.join(scripts, "block.sh"), '#!/usr/bin/env bash\necho "nope" >&2\nexit 2\n');
		fs.chmodSync(path.join(scripts, "mark.sh"), 0o755);
		fs.chmodSync(path.join(scripts, "block.sh"), 0o755);

		writeMarketplaceStore(path.join(cwd, ".agents", "marketplaces.json"), [{ location: market, dir: market }]);
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("preserves exec bits, parses the wrapper hooks shape, and runs bundled scripts via the bridge", async () => {
		const outcome = await installAvailablePlugin(cwd, "hooky");
		expect(outcome.installed).toBe(true);
		const dest = outcome.dest!;

		// Bundled scripts survive install as executables.
		for (const script of ["mark.sh", "block.sh"]) {
			const mode = fs.statSync(path.join(dest, "scripts", script)).mode;
			expect(mode & 0o111).not.toBe(0);
		}

		// The { description, hooks: {...} } wrapper unwraps to the event map.
		const parsed = parsePluginDir(dest);
		expect(parsed?.hooks?.Stop).toHaveLength(1);
		expect(parsed?.hooks?.PreToolUse).toHaveLength(1);

		const { pi, handlers } = makePi();
		const errors: string[] = [];
		installPluginHooks(pi, parsed!.hooks!, parsed!.root, (m) => errors.push(m));

		// Stop → agent_end: the script runs from the INSTALLED root and receives
		// the event JSON on stdin.
		await handlers.get("agent_end")!({});
		const ran = JSON.parse(fs.readFileSync(path.join(dest, "ran.json"), "utf8"));
		expect(ran.hook_event_name).toBe("Stop");
		expect(errors).toEqual([]);

		// PreToolUse → tool_call: exit 2 blocks with the script's stderr as reason.
		const decision = await handlers.get("tool_call")!({ toolName: "bash", input: { command: "rm -rf /" } });
		expect(decision).toMatchObject({ block: true, reason: "nope" });

		// Matcher scopes the hook: a non-matching tool is not blocked.
		const pass = await handlers.get("tool_call")!({ toolName: "read", input: {} });
		expect(pass).toBeUndefined();
	});

	it("synthesizes a manifest for a manifest-less marketplace plugin (bare capability tree)", async () => {
		// Like copilot-plugins' "spark": the plugin dir is just a skills/ tree.
		const market2 = path.join(cwd, "market2");
		writeJson(path.join(market2, ".agents-plugin", "marketplace.json"), {
			name: "local2",
			plugins: [{ name: "bare", source: "./plugins/bare", description: "A bare skills tree." }],
		});
		const skillDir = path.join(market2, "plugins", "bare", "skills", "greet");
		fs.mkdirSync(skillDir, { recursive: true });
		fs.writeFileSync(path.join(skillDir, "SKILL.md"), "---\nname: greet\ndescription: greets\n---\n\nSay hi.\n");
		writeMarketplaceStore(path.join(cwd, ".agents", "marketplaces.json"), [
			{ location: path.join(cwd, "market"), dir: path.join(cwd, "market") },
			{ location: market2, dir: market2 },
		]);

		const outcome = await installAvailablePlugin(cwd, "bare");
		expect(outcome.installed).toBe(true);
		const parsed = parsePluginDir(outcome.dest!);
		expect(parsed?.id).toBe("bare");
		expect(parsed?.description).toBe("A bare skills tree.");
		expect(parsed?.skillsDir).toBe(path.join(outcome.dest!, "skills"));
	});
});
