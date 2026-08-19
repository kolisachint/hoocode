/**
 * Phase 1 acceptance: a real catalog canvas opens in hoocode, unmodified.
 *
 * Design: `docs/canvas-extensions-design.md` §9. The stated acceptance criterion
 * for Phase 1 is `pr-artifact-explorer` from `github/awesome-copilot` — ~10 `.mjs`
 * modules, no `package.json`, no `node_modules` — running byte-identical to
 * upstream. Testing against someone else's extension is the point: a fixture we
 * wrote can only confirm our own assumptions.
 *
 * The extension is not vendored into this repo, so the test skips unless a
 * checkout is present. To run it:
 *
 *   git clone --depth 1 https://github.com/github/awesome-copilot /tmp/awesome-copilot
 *   HOOCODE_CANVAS_ACCEPTANCE_DIR=/tmp/awesome-copilot/extensions/pr-artifact-explorer \
 *     npx tsx ../../node_modules/vitest/dist/cli.js --run \
 *     test/canvas-acceptance-pr-artifact-explorer.test.ts
 *
 * Only unauthenticated surfaces are exercised (`canvas.open`, `cache_status`), so
 * no GitHub token is needed and nothing is downloaded.
 */

import { existsSync } from "node:fs";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const CANDIDATES = [
	process.env.HOOCODE_CANVAS_ACCEPTANCE_DIR,
	"/workspace/github/awesome-copilot/extensions/pr-artifact-explorer",
	"/tmp/awesome-copilot/extensions/pr-artifact-explorer",
].filter((candidate): candidate is string => typeof candidate === "string" && candidate.length > 0);

const extensionDir = CANDIDATES.find((candidate) => existsSync(path.join(candidate, "extension.mjs")));

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
