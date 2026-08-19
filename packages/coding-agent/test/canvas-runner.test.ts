/**
 * Runner end-to-end: fork a real extension, drive the provider protocol, tear it down.
 *
 * Design: `docs/canvas-extensions-design.md` §3.1, §4.1, §6.1. These tests fork
 * `test/fixtures/canvas/plan-board`, which is a genuine canvas extension — it
 * imports only `@github/copilot-sdk/extension` and `node:` builtins, and serves
 * its UI from a loopback port behind a capability token, exactly as
 * `pr-artifact-explorer` does. Nothing here is mocked: if the module resolver, the
 * NDJSON framing, or the lifecycle is wrong, these fail.
 *
 * The child runs the TypeScript shim under `tsx`, which is why `canvasTestRuntime`
 * injects `--import tsx/esm`. Production resolves the built JavaScript instead;
 * the shim URL is a runtime parameter precisely so that swap needs no code change.
 */

import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CanvasCallError, type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board", "extension.mjs");

function providerParams(instanceId = "i1") {
	return { sessionId: "s1", extensionId: "plan-board", canvasId: "plan-board", instanceId };
}

describe("canvas runner", () => {
	let running: CanvasExtensionProcess | undefined;
	const strays: string[] = [];
	const logs: string[] = [];

	afterEach(async () => {
		await running?.terminate();
		running = undefined;
		strays.length = 0;
		logs.length = 0;
	});

	function start(): CanvasExtensionProcess {
		running = spawnCanvasExtension({
			extensionId: "plan-board",
			entry: FIXTURE,
			runtime: canvasTestRuntime(),
			requestTimeoutMs: { "canvas.open": 20_000, "canvas.action.invoke": 20_000 },
			onStray: (line) => strays.push(line),
			onLog: (message) => logs.push(message),
		});
		return running;
	}

	it("resolves the SDK specifier inside the child without touching the extension", async () => {
		const ready = await start().ready;
		expect(ready.extensionId).toBe("plan-board");
		expect(ready.protocolVersion).toBe(3);
		expect(ready.canvases).toHaveLength(1);
		expect(ready.canvases[0]?.displayName).toBe("Plan Board");
	});

	it("sends action metadata but never handlers", async () => {
		const ready = await start().ready;
		expect(ready.canvases[0]?.actions?.map((action) => action.name)).toEqual([
			"add_step",
			"needs_auth",
			"crash",
			"leak_stdout",
		]);
		expect(JSON.stringify(ready)).not.toContain("handler");
	});

	it("opens an instance and serves the URL it returned", async () => {
		const canvas = start();
		await canvas.ready;
		const opened = (await canvas.open(providerParams())) as { url: string; title: string };
		expect(opened.title).toBe("Plan Board");
		expect(opened.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		const response = await fetch(opened.url);
		expect(response.status).toBe(200);
		expect(await response.text()).toContain("Plan Board");

		// The capability token is not decoration: without it the canvas refuses.
		const withoutToken = await fetch(new URL(opened.url).origin);
		expect(withoutToken.status).toBe(403);
	});

	it("invokes an action against the open instance", async () => {
		const canvas = start();
		await canvas.ready;
		await canvas.open(providerParams());
		expect(
			await canvas.invokeAction({ ...providerParams(), actionName: "add_step", input: { text: "ship" } }),
		).toEqual({
			steps: 1,
		});
		expect(
			await canvas.invokeAction({ ...providerParams(), actionName: "add_step", input: { text: "again" } }),
		).toEqual({
			steps: 2,
		});
	});

	it("surfaces a CanvasError's code across the process boundary", async () => {
		const canvas = start();
		await canvas.ready;
		await expect(canvas.invokeAction({ ...providerParams(), actionName: "needs_auth" })).rejects.toMatchObject({
			name: "CanvasCallError",
			code: "github_auth_required",
		});
	});

	it("reports a stray stdout line instead of corrupting the protocol", async () => {
		const canvas = start();
		await canvas.ready;
		await canvas.open(providerParams());
		expect(await canvas.invokeAction({ ...providerParams(), actionName: "leak_stdout" })).toEqual({ wrote: true });
		expect(strays).toContain("stray diagnostic from the extension");
	});

	it("forwards session.log from the child", async () => {
		const canvas = start();
		await canvas.ready;
		await canvas.open(providerParams());
		expect(logs).toContain("opened i1");
	});

	it("closes an instance and lets the extension release its port", async () => {
		const canvas = start();
		await canvas.ready;
		const opened = (await canvas.open(providerParams())) as { url: string };
		await canvas.close(providerParams());
		await expect(fetch(opened.url)).rejects.toThrow();
	});

	it("keeps instances independent", async () => {
		const canvas = start();
		await canvas.ready;
		const first = (await canvas.open(providerParams("a"))) as { url: string };
		const second = (await canvas.open(providerParams("b"))) as { url: string };
		expect(new URL(first.url).port).not.toBe(new URL(second.url).port);
	});

	it("fails a call for an action the extension does not declare", async () => {
		const canvas = start();
		await canvas.ready;
		await expect(canvas.invokeAction({ ...providerParams(), actionName: "nope" })).rejects.toBeInstanceOf(
			CanvasCallError,
		);
	});

	it("terminates the child and refuses further calls", async () => {
		const canvas = start();
		await canvas.ready;
		await canvas.terminate();
		expect(canvas.running).toBe(false);
		await expect(canvas.open(providerParams())).rejects.toThrow(/is not running/);
	});

	it("rejects in-flight calls when the child dies", async () => {
		const canvas = start();
		await canvas.ready;
		const inFlight = canvas.open(providerParams());
		await canvas.terminate();
		await expect(inFlight).rejects.toThrow(/exited|not running/);
	});
});
