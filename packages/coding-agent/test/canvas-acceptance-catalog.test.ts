/**
 * Acceptance: real catalog canvases open in hoocode, unmodified.
 *
 * Design: `docs/canvas-extensions-design.md` §9. Testing against other people's
 * extensions is the point — a fixture we wrote can only confirm our own
 * assumptions. Two are covered, chosen because they differ in the ways that
 * stress the host:
 *
 *   - `pr-artifact-explorer` — ~10 `.mjs` modules, **no `package.json`**, and a
 *     capability token on its loopback server.
 *   - `arcade-canvas` — a different author, **with** a `package.json` declaring
 *     `@github/copilot-sdk` and a README that says to run `npm install`. Neither
 *     matters: the SDK import is host-resolved (§4.1), so it runs with no
 *     `node_modules` anywhere. It also serves a static game bundle, and it does
 *     *not* token-gate its server — a reminder that the security posture in §8 is
 *     each extension's own choice, not something the host imposes.
 *
 * Neither is vendored here, so each block skips unless a checkout is present.
 * **CI is the real caller**: the `canvas-canary` job (`.github/workflows/ci.yml`)
 * sparse-checks-out both extensions at a pinned SHA and points
 * `HOOCODE_CANVAS_ACCEPTANCE_ROOT` at them — see design §2.3 item 4 for why the
 * revision is pinned. To reproduce it locally:
 *
 *   git clone --depth 1 https://github.com/github/awesome-copilot /tmp/awesome-copilot
 *   HOOCODE_CANVAS_ACCEPTANCE_ROOT=/tmp/awesome-copilot/extensions \
 *     bun run test test/canvas-acceptance-catalog.test.ts
 *
 * Skipping is for the no-checkout case only. Once that variable is set the
 * absence of an extension is an error, because a canary that can report success
 * having run nothing is worse than no canary at all.
 *
 * Only unauthenticated surfaces are exercised, so no GitHub token is needed and
 * nothing is downloaded.
 */

import { existsSync } from "node:fs";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const ROOTS = [
	process.env.HOOCODE_CANVAS_ACCEPTANCE_ROOT,
	"/workspace/github/awesome-copilot/extensions",
	"/tmp/awesome-copilot/extensions",
].filter((root): root is string => typeof root === "string" && root.length > 0);

/**
 * Locate a catalog extension, or undefined when no checkout is available.
 *
 * Setting `HOOCODE_CANVAS_ACCEPTANCE_ROOT` is a statement of intent — CI does it
 * — so a miss *there* is a failure rather than a skip. Without this the canary
 * has the one outcome a canary must never have: a checkout step that silently
 * produced nothing leaves every block skipped and the job green, which is
 * indistinguishable from the catalog still working.
 */
function catalogExtension(id: string): string | undefined {
	const found = ROOTS.map((root) => path.join(root, id)).find((dir) => existsSync(path.join(dir, "extension.mjs")));
	if (!found && process.env.HOOCODE_CANVAS_ACCEPTANCE_ROOT) {
		throw new Error(
			`HOOCODE_CANVAS_ACCEPTANCE_ROOT is set to "${process.env.HOOCODE_CANVAS_ACCEPTANCE_ROOT}" but ` +
				`"${id}/extension.mjs" is not under it. The canary was asked to run and cannot; refusing to skip.`,
		);
	}
	return found;
}

const extensionDir = catalogExtension("pr-artifact-explorer");
const arcadeDir = catalogExtension("arcade-canvas");

describe.skipIf(extensionDir === undefined)("acceptance: pr-artifact-explorer", () => {
	let canvas: CanvasExtensionProcess | undefined;
	const stderr: string[] = [];

	afterEach(async () => {
		await canvas?.terminate();
		canvas = undefined;
		stderr.length = 0;
	});

	function start(): CanvasExtensionProcess {
		canvas = spawnCanvasExtension({
			extensionId: "pr-artifact-explorer",
			entry: path.join(extensionDir as string, "extension.mjs"),
			runtime: canvasTestRuntime(),
			onStderr: (chunk) => stderr.push(chunk),
		});
		return canvas;
	}

	it("declares its canvas and all six documented actions", async () => {
		const ready = await start().ready;
		expect(ready.canvases).toHaveLength(1);
		const [declared] = ready.canvases;
		expect(declared?.id).toBe("pr-artifact-explorer");
		expect(declared?.displayName).toBe("Artifact Explorer");
		expect(declared?.actions?.map((action) => action.name)).toEqual([
			"open_pull_request",
			"inspect_artifact",
			"cache_status",
			"clear_cache",
			"accounts",
			"set_account",
		]);
		expect(ready.unsupported).toEqual([]);
	});

	it("opens its canvas and serves the loopback URL it returned", async () => {
		const running = start();
		await running.ready;
		const opened = (await running.open({
			sessionId: "s1",
			extensionId: "pr-artifact-explorer",
			canvasId: "pr-artifact-explorer",
			instanceId: "i1",
		})) as { url: string; title: string; status: string };

		expect(opened.title).toBe("Artifact Explorer");
		expect(opened.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		const page = await fetch(opened.url);
		expect(page.status).toBe(200);
		expect(page.headers.get("content-security-policy")).toContain("default-src 'self'");
		expect(await page.text()).toContain("Artifact Explorer");

		// Its capability token is enforced, exactly as its README claims.
		expect((await fetch(new URL(opened.url).origin)).status).toBe(403);
	});

	it("answers an unauthenticated action", async () => {
		const running = start();
		await running.ready;
		const summary = (await running.invokeAction({
			sessionId: "s1",
			extensionId: "pr-artifact-explorer",
			canvasId: "pr-artifact-explorer",
			instanceId: "i1",
			actionName: "cache_status",
		})) as Record<string, unknown>;
		expect(summary).toHaveProperty("totalBytes");
		expect(summary).toHaveProperty("count");
	});

	it("runs without writing anything to stderr", async () => {
		const running = start();
		await running.ready;
		expect(stderr.join("")).toBe("");
	});
});

describe.skipIf(arcadeDir === undefined)("acceptance: arcade-canvas", () => {
	let canvas: CanvasExtensionProcess | undefined;
	const stderr: string[] = [];

	afterEach(async () => {
		await canvas?.terminate();
		canvas = undefined;
		stderr.length = 0;
	});

	function start(): CanvasExtensionProcess {
		canvas = spawnCanvasExtension({
			extensionId: "arcade-canvas",
			entry: path.join(arcadeDir as string, "extension.mjs"),
			runtime: canvasTestRuntime(),
			onStderr: (chunk) => stderr.push(chunk),
		});
		return canvas;
	}

	function params(instanceId: string) {
		return { sessionId: "s1", extensionId: "arcade-canvas", canvasId: "arcade-canvas", instanceId };
	}

	it("runs with a package.json and no node_modules, ignoring its README's npm install", async () => {
		// The SDK import is host-resolved, so the declared dependency is decorative and
		// the install step its README asks for is unnecessary (§4.1, §4.2).
		expect(existsSync(path.join(arcadeDir as string, "package.json"))).toBe(true);
		expect(existsSync(path.join(arcadeDir as string, "node_modules"))).toBe(false);
		const ready = await start().ready;
		expect(ready.canvases[0]?.displayName).toBe("Agent Arcade");
		expect(ready.canvases[0]?.actions?.map((action) => action.name)).toEqual([
			"list_games",
			"select_game",
			"restart_game",
		]);
	});

	it("serves its page and its static game bundle", async () => {
		const running = start();
		await running.ready;
		const opened = (await running.open(params("i1"))) as { url: string };
		const page = await fetch(opened.url);
		expect(page.status).toBe(200);
		const html = await page.text();
		expect(html).toContain("Agent Arcade");

		// It ships a Phaser bundle, so the loopback server is a real static host.
		const refs = [...html.matchAll(/(?:src|href)="(\.\/[^"]+)"/g)].map((match) => match[1] as string);
		expect(refs.length).toBeGreaterThan(0);
		for (const ref of refs) {
			expect((await fetch(new URL(ref, opened.url).href)).status, ref).toBe(200);
		}
	});

	it("refuses to walk out of its own directory", async () => {
		const running = start();
		await running.ready;
		const opened = (await running.open(params("i1"))) as { url: string };
		expect((await fetch(new URL("/../../../etc/passwd", opened.url).href)).status).toBe(404);
	});

	it("selects a game by the key its own schema declares", async () => {
		const running = start();
		const ready = await running.ready;
		await running.open(params("i1"));

		// Read the enum from the declaration rather than hardcoding a key — which is
		// exactly why list_canvas_capabilities exposes the schemas (§11.5). An unknown
		// key is silently normalised by this extension, so a guess looks like a pass.
		const schema = ready.canvases[0]?.actions?.find((action) => action.name === "select_game")?.inputSchema as {
			properties: { gameKey: { enum: string[] } };
		};
		const key = schema.properties.gameKey.enum[0] as string;
		expect(
			await running.invokeAction({ ...params("i1"), actionName: "select_game", input: { gameKey: key } }),
		).toEqual({ selectedGame: key });
	});

	it("keeps two instances independent, each on its own port", async () => {
		const running = start();
		const ready = await running.ready;
		const first = (await running.open(params("a"))) as { url: string };
		const second = (await running.open(params("b"))) as { url: string };
		const keys = (
			ready.canvases[0]?.actions?.find((action) => action.name === "select_game")?.inputSchema as {
				properties: { gameKey: { enum: string[] } };
			}
		).properties.gameKey.enum;

		await running.invokeAction({ ...params("a"), actionName: "select_game", input: { gameKey: keys[0] } });
		await running.invokeAction({ ...params("b"), actionName: "select_game", input: { gameKey: keys[1] } });
		expect(await running.invokeAction({ ...params("a"), actionName: "restart_game" })).toEqual({
			selectedGame: keys[0],
		});
		expect(await running.invokeAction({ ...params("b"), actionName: "restart_game" })).toEqual({
			selectedGame: keys[1],
		});
		expect(new URL(first.url).port).not.toBe(new URL(second.url).port);
	});

	it("runs without writing anything to stderr", async () => {
		const running = start();
		await running.ready;
		expect(stderr.join("")).toBe("");
	});
});
