/**
 * Cancelling an open, and honouring a turn's abort signal.
 *
 * Design: `docs/canvas-extensions-design.md` §11.4 and §11.6. The provider protocol has
 * no cancel verb, so aborting only ends *our* wait — the child may have finished
 * opening and be holding a port. What makes this safe is that the instance id is
 * generated before the open call, so the registry can close an instance it never saw
 * open. One `abandon` path serves both a person's cancel and a timeout.
 *
 * The fixture reports its `onClose` through `session.log`, so these tests observe the
 * close actually reaching the child rather than assuming it did.
 */

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { DiscoveredCanvasExtension } from "../src/core/canvas/discovery.js";
import { CanvasRegistry, type CanvasRegistryOptions } from "../src/core/canvas/registry.js";
import { CANVAS_ERROR_CODE_ABORTED } from "../src/core/canvas/runner.js";
import type { ExtensionContext } from "../src/core/extensions/types.js";
import { createCanvasToolDefinitions } from "../src/core/tools/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board");
const cwdForGate = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-cancel-cwd-"));
const NO_CTX = {} as unknown as ExtensionContext;

const EXTENSION: DiscoveredCanvasExtension = {
	id: "plan-board",
	dir: FIXTURE_DIR,
	entry: path.join(FIXTURE_DIR, "extension.mjs"),
	scope: "project",
};

/** Wait until `predicate` holds, so tests observe async child traffic without sleeping blindly. */
async function until(predicate: () => boolean, timeoutMs = 8_000): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (!predicate()) {
		if (Date.now() > deadline) throw new Error("condition not met in time");
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
}

describe("cancelling canvas work", () => {
	let registry: CanvasRegistry | undefined;
	const logs: string[] = [];
	const diagnostics: string[] = [];

	afterEach(async () => {
		await registry?.shutdown();
		registry = undefined;
		logs.length = 0;
		diagnostics.length = 0;
	});

	function build(overrides: Partial<CanvasRegistryOptions> = {}): CanvasRegistry {
		registry = new CanvasRegistry({
			runtime: canvasTestRuntime(),
			cwd: cwdForGate,
			agentDir: path.join(cwdForGate, ".hoocode"),
			onLog: (_id, message) => logs.push(message),
			onDiagnostic: (_id, message) => diagnostics.push(message),
			...overrides,
		});
		return registry;
	}

	describe("open", () => {
		it("rejects with an aborted code when a person cancels", async () => {
			const reg = build();
			const controller = new AbortController();
			const opening = reg.open(EXTENSION, "plan-board", { delayMs: 4_000 }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toMatchObject({ code: CANVAS_ERROR_CODE_ABORTED });
		});

		it("registers no instance for an open it stopped waiting on", async () => {
			const reg = build();
			const controller = new AbortController();
			const opening = reg.open(EXTENSION, "plan-board", { delayMs: 4_000 }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toThrow();
			expect(reg.listInstances()).toEqual([]);
			expect(reg.activeActions()).toEqual([]);
		});

		it("closes the instance it never saw open, so the extension can release its port", async () => {
			const reg = build();
			const controller = new AbortController();
			const opening = reg.open(EXTENSION, "plan-board", { delayMs: 300 }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toThrow();
			// The close is what the instance id being pre-generated buys us.
			await until(() => logs.some((line) => line.startsWith("closed ")));
			expect(logs.some((line) => line.startsWith("closed "))).toBe(true);
		});

		it("refuses immediately when the signal is already aborted, writing nothing to the child", async () => {
			const reg = build();
			await expect(
				reg.open(EXTENSION, "plan-board", undefined, { signal: AbortSignal.abort() }),
			).rejects.toMatchObject({ code: CANVAS_ERROR_CODE_ABORTED });
			expect(logs.some((line) => line.startsWith("opened "))).toBe(false);
		});

		it("takes the same abandon path on a timeout as on a cancel", async () => {
			const reg = build({ requestTimeoutMs: { "canvas.open": 150 } });
			await expect(reg.open(EXTENSION, "plan-board", { delayMs: 4_000 })).rejects.toMatchObject({
				code: CANVAS_ERROR_CODE_ABORTED,
			});
			expect(reg.listInstances()).toEqual([]);
			await until(() => logs.some((line) => line.startsWith("closed ")));
		});

		it("leaves a sibling canvas untouched when one open is cancelled", async () => {
			const reg = build();
			const healthy = await reg.open(EXTENSION, "plan-board");
			const controller = new AbortController();
			const opening = reg.open(EXTENSION, "plan-board", { delayMs: 4_000 }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toThrow();

			// Terminating the child would have killed this one too.
			expect(reg.listInstances().map((i) => i.instanceId)).toEqual([healthy.instanceId]);
			expect(await reg.invokeAction(healthy, "add_step", { text: "still fine" })).toEqual({ steps: 1 });
			expect((await fetch(healthy.url ?? "")).status).toBe(200);
		});

		it("keeps working after a cancelled open", async () => {
			const reg = build();
			const controller = new AbortController();
			const opening = reg.open(EXTENSION, "plan-board", { delayMs: 2_000 }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toThrow();
			const next = await reg.open(EXTENSION, "plan-board");
			expect((await fetch(next.url ?? "")).status).toBe(200);
		});
	});

	describe("actions", () => {
		it("rejects an in-flight action when the signal aborts", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const controller = new AbortController();
			const running = reg.invokeAction(instance, "add_step", { text: "x" }, { signal: controller.signal });
			controller.abort();
			await expect(running).rejects.toMatchObject({ code: CANVAS_ERROR_CODE_ABORTED });
		});

		it("leaves the instance usable after an aborted action", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const controller = new AbortController();
			const running = reg.invokeAction(instance, "add_step", { text: "x" }, { signal: controller.signal });
			controller.abort();
			await expect(running).rejects.toThrow();
			// A late answer for the abandoned call must be dropped, not mistaken for this one.
			expect(await reg.invokeAction(instance, "add_step", { text: "y" })).toMatchObject({
				steps: expect.any(Number),
			});
		});
	});

	describe("invoke_canvas_action", () => {
		it("passes the turn's abort signal through to the canvas", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const invoke = createCanvasToolDefinitions(reg).find((def) => def.name === "invoke_canvas_action");
			const controller = new AbortController();
			const running = invoke?.execute?.(
				"call-1",
				{ instanceId: instance.instanceId, action: "add_step", input: { text: "x" } },
				controller.signal,
				undefined,
				NO_CTX,
			);
			controller.abort();
			await expect(running).rejects.toThrow(new RegExp(CANVAS_ERROR_CODE_ABORTED));
		});
	});
});
