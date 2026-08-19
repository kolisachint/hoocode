/**
 * Registry behaviour: children, instances, reaping, and the action inventory.
 *
 * Design: `docs/canvas-extensions-design.md` §4, §6, §7. Driven against the same
 * real fixture extension the runner tests use, so child reuse and instance
 * isolation are exercised across a genuine process boundary. The clock and the
 * instance-id generator are injected, so nothing here sleeps.
 */

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { DiscoveredCanvasExtension } from "../src/core/canvas/discovery.js";
import { CanvasRegistry, type CanvasRegistryOptions, canvasInstanceKeyOf } from "../src/core/canvas/registry.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board");

/** A directory containing no extension scopes, so nothing here is repository-supplied. */
const cwdForGate = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-registry-cwd-"));

const EXTENSION: DiscoveredCanvasExtension = {
	id: "plan-board",
	dir: FIXTURE_DIR,
	entry: path.join(FIXTURE_DIR, "extension.mjs"),
	scope: "project",
};

describe("canvas registry", () => {
	let registry: CanvasRegistry | undefined;
	let clock = 1_000;
	let counter = 0;
	const diagnostics: string[] = [];

	function build(overrides: Partial<CanvasRegistryOptions> = {}) {
		clock = 1_000;
		counter = 0;
		registry = new CanvasRegistry({
			runtime: canvasTestRuntime(),
			// The fixture lives inside this repo but outside any `<cwd>/.github/extensions`,
			// so the trust gate treats it as locally installed. Gating itself is covered
			// by canvas-trust.test.ts.
			cwd: cwdForGate,
			agentDir: path.join(cwdForGate, ".hoocode"),
			now: () => clock,
			newInstanceId: () => {
				counter += 1;
				return `i${counter}`;
			},
			onDiagnostic: (_id, message) => diagnostics.push(message),
			...overrides,
		});
		return registry;
	}

	afterEach(async () => {
		await registry?.shutdown();
		registry = undefined;
		diagnostics.length = 0;
	});

	it("forks on first use and reports the extension's declarations", async () => {
		const declarations = await build().declarations(EXTENSION);
		expect(declarations.map((declaration) => declaration.id)).toEqual(["plan-board"]);
	});

	it("reuses one child across opens of the same extension", async () => {
		const reg = build();
		const first = await reg.open(EXTENSION, "plan-board");
		const second = await reg.open(EXTENSION, "plan-board");
		expect(first.instanceId).toBe("i1");
		expect(second.instanceId).toBe("i2");
		// Same process, but the extension gave each instance its own port.
		expect(new URL(first.url ?? "").port).not.toBe(new URL(second.url ?? "").port);
	});

	it("rejects a canvas id the extension does not declare, naming what it has", async () => {
		await expect(build().open(EXTENSION, "ghost")).rejects.toThrow(/declares no canvas "ghost".*plan-board/s);
	});

	it("enforces the per-canvas instance cap", async () => {
		const reg = build({ maxInstancesPerCanvas: 2 });
		await reg.open(EXTENSION, "plan-board");
		await reg.open(EXTENSION, "plan-board");
		await expect(reg.open(EXTENSION, "plan-board")).rejects.toThrow(/limit 2/);
	});

	it("routes an action to the right instance and refreshes its touch time", async () => {
		const reg = build();
		const instance = await reg.open(EXTENSION, "plan-board");
		clock += 5_000;
		expect(await reg.invokeAction(instance, "add_step", { text: "ship" })).toEqual({ steps: 1 });
		expect(reg.listInstances()[0]?.lastTouchedAt).toBe(clock);
	});

	it("keeps action state separate per instance", async () => {
		const reg = build();
		const a = await reg.open(EXTENSION, "plan-board");
		const b = await reg.open(EXTENSION, "plan-board");
		await reg.invokeAction(a, "add_step", { text: "one" });
		await reg.invokeAction(a, "add_step", { text: "two" });
		expect(await reg.invokeAction(b, "add_step", { text: "only" })).toEqual({ steps: 1 });
	});

	it("refuses an action on an instance that is not open", async () => {
		const reg = build();
		const instance = await reg.open(EXTENSION, "plan-board");
		await reg.close(instance);
		await expect(reg.invokeAction(instance, "add_step", { text: "x" })).rejects.toThrow(/No open canvas instance/);
	});

	it("exposes no actions until something is open, and drops them on close", async () => {
		const reg = build();
		expect(reg.activeActions()).toEqual([]);
		const instance = await reg.open(EXTENSION, "plan-board");
		expect(reg.activeActions().map((binding) => binding.action.name)).toEqual([
			"add_step",
			"needs_auth",
			"crash",
			"flood",
			"leak_stdout",
		]);
		await reg.close(instance);
		expect(reg.activeActions()).toEqual([]);
	});

	it("binds each open instance's actions separately", async () => {
		const reg = build();
		await reg.open(EXTENSION, "plan-board");
		await reg.open(EXTENSION, "plan-board");
		expect(reg.activeActions()).toHaveLength(10);
		expect(new Set(reg.activeActions().map((binding) => binding.instanceId))).toEqual(new Set(["i1", "i2"]));
	});

	it("is idempotent on close", async () => {
		const reg = build();
		const instance = await reg.open(EXTENSION, "plan-board");
		await reg.close(instance);
		await expect(reg.close(instance)).resolves.toBeUndefined();
		expect(reg.listInstances()).toEqual([]);
	});

	it("reaps an instance hoocode has not touched within the idle timeout", async () => {
		const reg = build({ idleTimeoutMs: 10_000 });
		const instance = await reg.open(EXTENSION, "plan-board");
		clock += 9_999;
		expect(await reg.reapIdle()).toEqual([]);
		clock += 1;
		expect(await reg.reapIdle()).toEqual([canvasInstanceKeyOf(instance)]);
		expect(reg.listInstances()).toEqual([]);
	});

	it("counts an action as a touch, so an active instance survives reaping", async () => {
		const reg = build({ idleTimeoutMs: 10_000 });
		const instance = await reg.open(EXTENSION, "plan-board");
		clock += 8_000;
		await reg.invokeAction(instance, "add_step", { text: "still here" });
		clock += 8_000;
		expect(await reg.reapIdle()).toEqual([]);
		expect(reg.listInstances()).toHaveLength(1);
	});

	it("terminates a child once it has had no instances for the linger period", async () => {
		const reg = build({ idleTimeoutMs: 1_000, childLingerMs: 5_000 });
		const instance = await reg.open(EXTENSION, "plan-board");
		await reg.close(instance);
		clock += 4_999;
		await reg.reapIdle();
		// Still warm: reopening does not have to pay for a fork.
		const reopened = await reg.open(EXTENSION, "plan-board");
		expect(reopened.url).toBeDefined();

		await reg.close(reopened);
		clock += 5_000;
		await reg.reapIdle();
		// Child reaped; the next open forks again and still works.
		expect((await reg.open(EXTENSION, "plan-board")).url).toBeDefined();
	});

	it("releases the extension's port when an instance is reaped", async () => {
		const reg = build({ idleTimeoutMs: 1_000 });
		const instance = await reg.open(EXTENSION, "plan-board");
		const url = instance.url ?? "";
		expect((await fetch(url)).status).toBe(200);
		clock += 1_000;
		await reg.reapIdle();
		await expect(fetch(url)).rejects.toThrow();
	});

	it("forgets instances when the extension crashes, and forks a fresh one on demand", async () => {
		const reg = build();
		const instance = await reg.open(EXTENSION, "plan-board");
		expect(reg.listInstances()).toHaveLength(1);

		// The fixture's `crash` action calls process.exit, so the child dies mid-call.
		// The in-flight request must reject rather than hang.
		await expect(reg.invokeAction(instance, "crash")).rejects.toThrow(/exited|not running/);

		// No stale instance or action binding survives a dead child.
		expect(reg.listInstances()).toEqual([]);
		expect(reg.activeActions()).toEqual([]);

		// And the next open forks a replacement rather than reusing the corpse.
		const revived = await reg.open(EXTENSION, "plan-board");
		expect(revived.url).toBeDefined();
		expect((await fetch(revived.url ?? "")).status).toBe(200);
	});

	it("shuts down cleanly when called twice", async () => {
		const reg = build();
		await reg.open(EXTENSION, "plan-board");
		await reg.shutdown();
		await expect(reg.shutdown()).resolves.toBeUndefined();
	});
});
