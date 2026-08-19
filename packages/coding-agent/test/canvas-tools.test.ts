/**
 * The two agent-facing canvas tools, driven against a real forked canvas.
 *
 * Design: `docs/canvas-extensions-design.md` §11.5, §7. Two properties matter most
 * and each has a test that states why:
 *
 *  - **The prompt surface stays flat.** `AGENTS.md` budgets ~4,140 tokens with
 *    ~2,710 of it tool schemas, re-sent every request. The tools are absent when
 *    nothing is open, and there are exactly two of them however many canvases are.
 *  - **The agent cannot open a canvas.** Opening forks a process and binds a socket;
 *    that stays a human decision behind the trust gate.
 */

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { DiscoveredCanvasExtension } from "../src/core/canvas/discovery.js";
import { CanvasRegistry } from "../src/core/canvas/registry.js";
import type { ExtensionContext } from "../src/core/extensions/types.js";
import {
	CANVAS_RESULT_MAX_CHARS,
	createCanvasToolDefinitions,
	INVOKE_CANVAS_ACTION_TOOL_NAME,
	LIST_CANVAS_CAPABILITIES_TOOL_NAME,
} from "../src/core/tools/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board");
const cwdForGate = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-tools-cwd-"));

const EXTENSION: DiscoveredCanvasExtension = {
	id: "plan-board",
	dir: FIXTURE_DIR,
	entry: path.join(FIXTURE_DIR, "extension.mjs"),
	scope: "project",
};

/** Tools receive an ExtensionContext none of these paths touch. */
const NO_CTX = {} as unknown as ExtensionContext;

/** Text of a tool result, for assertions. */
function textOf(result: { content: { type: string; text?: string }[] }): string {
	return result.content.map((part) => part.text ?? "").join("");
}

describe("canvas tools", () => {
	let registry: CanvasRegistry | undefined;

	afterEach(async () => {
		await registry?.shutdown();
		registry = undefined;
	});

	function build(): CanvasRegistry {
		registry = new CanvasRegistry({
			runtime: canvasTestRuntime(),
			cwd: cwdForGate,
			agentDir: path.join(cwdForGate, ".hoocode"),
		});
		return registry;
	}

	function tools(reg: CanvasRegistry) {
		const defs = createCanvasToolDefinitions(reg);
		const byName = new Map(defs.map((def) => [def.name, def]));
		return {
			all: defs,
			list: byName.get(LIST_CANVAS_CAPABILITIES_TOOL_NAME),
			invoke: byName.get(INVOKE_CANVAS_ACTION_TOOL_NAME),
		};
	}

	describe("prompt surface", () => {
		it("contributes no tools while nothing is open", () => {
			expect(createCanvasToolDefinitions(build())).toEqual([]);
		});

		it("contributes exactly two tools, named as Copilot names them", async () => {
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			expect(tools(reg).all.map((def) => def.name)).toEqual([
				LIST_CANVAS_CAPABILITIES_TOOL_NAME,
				INVOKE_CANVAS_ACTION_TOOL_NAME,
			]);
		});

		it("stays at two tools however many canvases are open", async () => {
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			await reg.open(EXTENSION, "plan-board");
			await reg.open(EXTENSION, "plan-board");
			expect(reg.activeActions().length).toBeGreaterThan(3);
			expect(tools(reg).all).toHaveLength(2);
		});

		it("keeps both schemas inside the per-tool token budget", async () => {
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			// AGENTS.md: a built-in should land under ~250 tokens serialized, estimated
			// at chars/4 over {name, description, parameters}. Guarding it here stops a
			// future description from quietly costing every request.
			for (const def of tools(reg).all) {
				const serialized = JSON.stringify({
					name: def.name,
					description: def.description,
					parameters: def.parameters,
				});
				expect(Math.ceil(serialized.length / 4), `${def.name} schema tokens`).toBeLessThan(250);
			}
		});

		it("claims no safety property the runtime does not enforce", async () => {
			// A canvas extension is arbitrary Node code with the user's privileges, and
			// its side effects never reach hoocode's permission gate. An earlier
			// description asserted actions "cannot edit files or run commands", which was
			// simply false — workspace trust is the control, not the schema text.
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			const invoke = tools(reg).invoke;
			expect(invoke?.description).toMatch(/side effects/i);
			expect(invoke?.description).not.toMatch(/cannot (edit|run|touch|write)/i);
		});

		it("exposes no way for the agent to open a canvas", async () => {
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			const names = tools(reg).all.map((def) => def.name);
			expect(names.some((name) => /open|create|spawn|launch/i.test(name))).toBe(false);
		});
	});

	describe("list_canvas_capabilities", () => {
		it("reports each open instance with the actions it declares", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const result = await tools(reg).list?.execute?.("call-1", {}, undefined, undefined, NO_CTX);
			const report = JSON.parse(textOf(result as never));
			expect(report).toHaveLength(1);
			expect(report[0].instanceId).toBe(instance.instanceId);
			expect(report[0].canvas).toBe("plan-board");
			expect(report[0].title).toBe("Plan Board");
			expect(report[0].actions.map((action: { name: string }) => action.name)).toEqual([
				"add_step",
				"needs_auth",
				"crash",
				"flood",
				"leak_stdout",
			]);
		});

		it("carries each action's input schema, so the model can call it correctly", async () => {
			const reg = build();
			await reg.open(EXTENSION, "plan-board");
			const report = JSON.parse(
				textOf((await tools(reg).list?.execute?.("call-1", {}, undefined, undefined, NO_CTX)) as never),
			);
			const addStep = report[0].actions.find((action: { name: string }) => action.name === "add_step");
			expect(addStep.inputSchema).toMatchObject({ type: "object", required: ["text"] });
		});

		it("says a person opens canvases when none is open", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const list = tools(reg).list;
			await reg.close(instance);
			expect(textOf((await list?.execute?.("call-1", {}, undefined, undefined, NO_CTX)) as never)).toMatch(
				/A person opens a canvas/,
			);
		});
	});

	describe("invoke_canvas_action", () => {
		it("runs the action against the named instance", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const result = await tools(reg).invoke?.execute?.(
				"call-1",
				{
					instanceId: instance.instanceId,
					action: "add_step",
					input: { text: "ship it" },
				},
				undefined,
				undefined,
				NO_CTX,
			);
			expect(JSON.parse(textOf(result as never))).toEqual({ steps: 1 });
		});

		it("addresses instances by instanceId alone, keeping the schema to three fields", async () => {
			const reg = build();
			const first = await reg.open(EXTENSION, "plan-board");
			const second = await reg.open(EXTENSION, "plan-board");
			const invoke = tools(reg).invoke;
			await invoke?.execute?.(
				"c1",
				{ instanceId: first.instanceId, action: "add_step", input: { text: "a" } },
				undefined,
				undefined,
				NO_CTX,
			);
			await invoke?.execute?.(
				"c2",
				{ instanceId: first.instanceId, action: "add_step", input: { text: "b" } },
				undefined,
				undefined,
				NO_CTX,
			);
			const onSecond = await invoke?.execute?.(
				"c3",
				{
					instanceId: second.instanceId,
					action: "add_step",
					input: { text: "c" },
				},
				undefined,
				undefined,
				NO_CTX,
			);
			expect(JSON.parse(textOf(onSecond as never))).toEqual({ steps: 1 });
		});

		it("passes a CanvasError's code through instead of flattening it", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			// Only an Error's message reaches the model, so the machine-readable code has
			// to be folded into it rather than left on a field nothing renders.
			await expect(
				tools(reg).invoke?.execute?.(
					"call-1",
					{ instanceId: instance.instanceId, action: "needs_auth" },
					undefined,
					undefined,
					NO_CTX,
				),
			).rejects.toThrow(/^github_auth_required: /);
		});

		it("errors usefully for an unknown instance, listing what is open", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			await expect(
				tools(reg).invoke?.execute?.(
					"call-1",
					{ instanceId: "not-a-real-instance", action: "add_step" },
					undefined,
					undefined,
					NO_CTX,
				),
			).rejects.toThrow(instance.instanceId);
		});

		it("errors for an action the canvas does not declare", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			await expect(
				tools(reg).invoke?.execute?.(
					"call-1",
					{ instanceId: instance.instanceId, action: "nope" },
					undefined,
					undefined,
					NO_CTX,
				),
			).rejects.toThrow(/unknown_target/);
		});

		it("caps a chatty result so one action cannot flood the context window", async () => {
			const reg = build();
			const instance = await reg.open(EXTENSION, "plan-board");
			const invoke = tools(reg).invoke;
			const result = await invoke?.execute?.(
				"call-1",
				{
					instanceId: instance.instanceId,
					action: "flood",
				},
				undefined,
				undefined,
				NO_CTX,
			);
			const text = textOf(result as never);
			expect(text.length).toBeLessThan(CANVAS_RESULT_MAX_CHARS + 200);
			expect(text).toMatch(/truncated at 8000 characters/);
		});
	});
});
