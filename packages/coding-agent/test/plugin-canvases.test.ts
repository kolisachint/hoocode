/**
 * Canvases that arrive inside a plugin.
 *
 * The gap this closes, in one sentence: canvas discovery searched the three
 * places a person *puts* a canvas by hand and none of the places a package
 * manager puts one, so `/plugin install arcade-canvas@awesome-copilot` copied a
 * working extension onto the disk and `/canvas list` then reported nothing.
 *
 * Both real catalog layouts are covered, because they are genuinely different
 * shapes and only one of them is documented anywhere:
 *
 *   - `"extensions": "extensions"` + `extensions/<id>/extension.mjs` — the
 *     Copilot manifest key, as `Redth/mobile-canvas-ghcp` ships it.
 *   - `com.github.copilot/extensions/<id>/extension.mjs` — the Agent Plugins
 *     vendor content namespace, which is what `github/awesome-copilot` publishes
 *     for all 24 of its canvas entries (`eng/materialize-plugins.mjs`).
 *
 * The second layout also carries subagents at `com.github.copilot/agents/`, so
 * that is asserted here too: it is the same namespace, missed for the same
 * reason.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import { pluginCanvasExtensions } from "../src/core/canvas/plugin-canvases.js";
import { CanvasSession } from "../src/core/canvas/session.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";

const FIXTURE = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board", CANVAS_ENTRY_FILE);

describe("plugin-shipped canvases", () => {
	let cwd: string;
	let home: string;
	let agentDir: string;
	let session: CanvasSession | undefined;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-canvas-repo-"));
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-canvas-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });
	});

	afterEach(async () => {
		await session?.dispose();
		session = undefined;
		untrustWorkspace(cwd, agentDir);
		fs.rmSync(cwd, { recursive: true, force: true });
		fs.rmSync(home, { recursive: true, force: true });
	});

	/** Write a plugin directory, with a real canvas entry at each given relative path. */
	function writePlugin(
		root: string,
		manifest: { rel: string; body: Record<string, unknown> },
		canvasRelDirs: string[],
	): string {
		const manifestPath = path.join(root, manifest.rel);
		fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
		fs.writeFileSync(manifestPath, JSON.stringify(manifest.body, null, 2), "utf8");
		for (const rel of canvasRelDirs) {
			const dir = path.join(root, rel);
			fs.mkdirSync(dir, { recursive: true });
			fs.copyFileSync(FIXTURE, path.join(dir, CANVAS_ENTRY_FILE));
		}
		return root;
	}

	/** A plugin in the user consumption home, `~/.agents/plugins/<name>`. */
	function userPluginDir(name: string): string {
		return path.join(home, ".agents", "plugins", name);
	}

	/** A plugin in the project consumption home, `<cwd>/.agents/plugins/<name>`. */
	function projectPluginDir(name: string): string {
		return path.join(cwd, ".agents", "plugins", name);
	}

	function build(): CanvasSession {
		session = new CanvasSession({
			cwd,
			homeDir: home,
			agentDir,
			resolveRuntime: async () => ({ available: false, reason: "not needed for listing" }),
		});
		return session;
	}

	describe("manifest reading", () => {
		it("resolves the Copilot `extensions` path key", () => {
			const root = writePlugin(
				userPluginDir("mobile-canvas"),
				{ rel: "plugin.json", body: { name: "mobile-canvas", extensions: "extensions" } },
				[path.join("extensions", "mobile-canvas")],
			);
			const parsed = parsePluginDir(root);
			expect(parsed?.canvasExtensions?.map((e) => e.id)).toEqual(["mobile-canvas"]);
			expect(parsed?.canvasExtensions?.[0]?.dir).toBe(path.join(root, "extensions", "mobile-canvas"));
		});

		it("resolves the vendor content namespace, and the subagents beside it", () => {
			const root = writePlugin(
				userPluginDir("arcade-canvas"),
				{
					rel: "plugin.json",
					// The awesome-copilot served manifest: `extensions` here is a vendor
					// *metadata map*, not a path. Both readings of the key must coexist.
					body: {
						name: "arcade-canvas",
						extensions: { "com.github.copilot": { logo: "assets/preview.png" } },
					},
				},
				[path.join("com.github.copilot", "extensions", "arcade-canvas")],
			);
			const agentFile = path.join(root, "com.github.copilot", "agents", "arcade-expert.md");
			fs.mkdirSync(path.dirname(agentFile), { recursive: true });
			fs.writeFileSync(agentFile, "---\nname: arcade-expert\n---\nHi.\n", "utf8");

			const parsed = parsePluginDir(root);
			expect(parsed?.canvasExtensions?.map((e) => e.id)).toEqual(["arcade-canvas"]);
			expect(parsed?.agentsDir).toBe(path.join(root, "com.github.copilot", "agents"));
			// The metadata map is not a path, so it must survive into unknownFields
			// rather than being consumed as one.
			expect(parsed?.unknownFields?.extensions).toEqual({ "com.github.copilot": { logo: "assets/preview.png" } });
		});

		it("treats a plugin root that carries extension.mjs as one canvas named for the plugin", () => {
			const root = writePlugin(userPluginDir("solo"), { rel: "plugin.json", body: { name: "solo" } }, ["."]);
			expect(parsePluginDir(root)?.canvasExtensions).toEqual([{ id: "solo", dir: root }]);
		});

		it("reports no canvases for a plugin that ships none", () => {
			const root = writePlugin(userPluginDir("plain"), { rel: "plugin.json", body: { name: "plain" } }, []);
			expect(parsePluginDir(root)?.canvasExtensions).toBeUndefined();
		});
	});

	describe("discovery", () => {
		it("finds a user-scope plugin's canvas", () => {
			writePlugin(userPluginDir("arcade-canvas"), { rel: "plugin.json", body: { name: "arcade-canvas" } }, [
				path.join("com.github.copilot", "extensions", "arcade-canvas"),
			]);
			const found = pluginCanvasExtensions(cwd, agentDir);
			expect(found.map((e) => [e.id, e.scope])).toEqual([["arcade-canvas", "user"]]);
			expect(fs.existsSync(found[0].entry)).toBe(true);
		});

		it("scopes a project-installed plugin's canvas as project, not user", () => {
			writePlugin(projectPluginDir("arcade-canvas"), { rel: "plugin.json", body: { name: "arcade-canvas" } }, [
				path.join("extensions", "arcade-canvas"),
			]);
			expect(pluginCanvasExtensions(cwd, agentDir).map((e) => e.scope)).toEqual(["project"]);
		});
	});

	describe("/canvas", () => {
		it("lists a canvas that arrived through a plugin install", async () => {
			writePlugin(userPluginDir("arcade-canvas"), { rel: "plugin.json", body: { name: "arcade-canvas" } }, [
				path.join("com.github.copilot", "extensions", "arcade-canvas"),
			]);
			const overview = await build().list();
			expect(overview.listings.map((l) => [l.extensionId, l.scope])).toEqual([["arcade-canvas", "user"]]);
			expect(overview.withheldCount).toBe(0);
		});

		it("withholds a project-installed plugin's canvas until the workspace is trusted", async () => {
			writePlugin(projectPluginDir("arcade-canvas"), { rel: "plugin.json", body: { name: "arcade-canvas" } }, [
				path.join("extensions", "arcade-canvas"),
			]);

			const untrusted = await build().list();
			expect(untrusted.withheldCount).toBe(1);
			expect(untrusted.listings[0]?.withheld).toBe("untrusted-workspace");

			await session?.dispose();
			session = undefined;
			trustWorkspace(cwd, agentDir);
			const trusted = await build().list();
			expect(trusted.withheldCount).toBe(0);
			expect(trusted.listings[0]?.withheld).toBeUndefined();
		});

		it("lets a hand-placed extension shadow a plugin's canvas of the same id", async () => {
			writePlugin(userPluginDir("arcade-canvas"), { rel: "plugin.json", body: { name: "arcade-canvas" } }, [
				path.join("extensions", "arcade-canvas"),
			]);
			const local = path.join(cwd, ".agents", "extensions", "arcade-canvas");
			fs.mkdirSync(local, { recursive: true });
			fs.copyFileSync(FIXTURE, path.join(local, CANVAS_ENTRY_FILE));
			trustWorkspace(cwd, agentDir);

			const overview = await build().list();
			expect(overview.listings).toHaveLength(1);
			expect(overview.listings[0]?.scope).toBe("agents");
		});
	});
});
