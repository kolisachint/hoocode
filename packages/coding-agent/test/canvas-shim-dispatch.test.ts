/**
 * The shim's dispatch loop: what a third-party canvas actually experiences.
 *
 * Design: `docs/canvas-extensions-design.md` §4.1. These tests drive the shim
 * through injected streams rather than a forked child, so they cover the provider
 * contract itself — the part that must behave the same in hoocode as in the
 * Copilot app — without depending on the runner.
 */

import { beforeEach, describe, expect, it } from "vitest";
import {
	CANVAS_ENVELOPE_VERSION,
	CANVAS_ERROR_CODE_UNKNOWN_TARGET,
	CANVAS_METHOD_CLOSE,
	CANVAS_METHOD_INVOKE_ACTION,
	CANVAS_METHOD_OPEN,
	CANVAS_SDK_PROTOCOL_VERSION,
	type CanvasProviderMethod,
	encodeCanvasMessage,
	type JsonValue,
} from "../src/core/canvas/protocol.js";
import { type FakeCanvasStreams, fakeCanvasStreams } from "../src/core/canvas/sdk-shim/dispatch.test-helpers.js";
import { CanvasError, createCanvas, joinSession } from "../src/core/canvas/sdk-shim/index.js";

/** Let the shim's async dispatch settle before asserting on what it wrote. */
const settle = () => new Promise((resolve) => setImmediate(resolve));

function request(id: number, method: CanvasProviderMethod, params: Record<string, JsonValue>): string {
	return encodeCanvasMessage({
		envelope: CANVAS_ENVELOPE_VERSION,
		type: "request",
		id,
		method,
		params: { sessionId: "s1", extensionId: "test-extension", canvasId: "board", instanceId: "i1", ...params },
	});
}

describe("canvas shim dispatch", () => {
	let streams: FakeCanvasStreams;

	beforeEach(() => {
		streams = fakeCanvasStreams();
	});

	function board() {
		const closed: string[] = [];
		const canvas = createCanvas({
			id: "board",
			displayName: "Board",
			description: "A board.",
			actions: [
				{
					name: "add_step",
					description: "Add a step.",
					inputSchema: { type: "object", properties: { text: { type: "string" } } },
					handler: (ctx) => ({ added: (ctx.input as { text?: string } | undefined)?.text ?? null }),
				},
				{
					name: "explode",
					handler: () => {
						throw new CanvasError("github_auth_required", "No usable GitHub account was found.");
					},
				},
				{ name: "returns_undefined", handler: () => undefined },
			],
			open: (ctx) => ({ url: `http://127.0.0.1:1/?i=${ctx.instanceId}`, title: "Board" }),
			onClose: (ctx) => {
				closed.push(ctx.instanceId);
			},
		});
		return { canvas, closed };
	}

	it("announces protocol version, extension id and stripped declarations", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		expect(streams.written[0]).toMatchObject({
			envelope: CANVAS_ENVELOPE_VERSION,
			type: "ready",
			protocolVersion: CANVAS_SDK_PROTOCOL_VERSION,
			extensionId: "test-extension",
			canvases: [{ id: "board", displayName: "Board", description: "A board." }],
			unsupported: [],
		});
		expect(JSON.stringify(streams.written[0])).not.toContain("handler");
	});

	it("reports declared surfaces the shim does not implement", async () => {
		await joinSession({ canvases: [board().canvas], tools: [], hooks: {} }, streams);
		expect(streams.written[0]).toMatchObject({ unsupported: ["tools", "hooks"] });
	});

	it("routes canvas.open and returns the provider's url", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(1, CANVAS_METHOD_OPEN, { input: { repository: "a/b" } }));
		await settle();
		expect(streams.written[1]).toEqual({
			envelope: CANVAS_ENVELOPE_VERSION,
			type: "response",
			id: 1,
			result: { url: "http://127.0.0.1:1/?i=i1", title: "Board" },
		});
	});

	it("routes an action to its handler with the full provider context", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(7, CANVAS_METHOD_INVOKE_ACTION, { actionName: "add_step", input: { text: "ship" } }));
		await settle();
		expect(streams.written[1]).toMatchObject({ type: "response", id: 7, result: { added: "ship" } });
	});

	it("propagates a CanvasError's code to the host", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(2, CANVAS_METHOD_INVOKE_ACTION, { actionName: "explode" }));
		await settle();
		expect(streams.written[1]).toMatchObject({
			type: "error",
			id: 2,
			code: "github_auth_required",
			message: "No usable GitHub account was found.",
		});
	});

	it("fails an unknown action with a machine-readable code", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(3, CANVAS_METHOD_INVOKE_ACTION, { actionName: "nope" }));
		await settle();
		expect(streams.written[1]).toMatchObject({ type: "error", id: 3, code: CANVAS_ERROR_CODE_UNKNOWN_TARGET });
	});

	it("fails a request for a canvas id that was never declared", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(4, CANVAS_METHOD_OPEN, { canvasId: "ghost" }));
		await settle();
		expect(streams.written[1]).toMatchObject({ type: "error", id: 4, code: CANVAS_ERROR_CODE_UNKNOWN_TARGET });
	});

	it("normalises an undefined handler result to null so the wire stays valid JSON", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(request(5, CANVAS_METHOD_INVOKE_ACTION, { actionName: "returns_undefined" }));
		await settle();
		expect(streams.written[1]).toEqual({
			envelope: CANVAS_ENVELOPE_VERSION,
			type: "response",
			id: 5,
			result: null,
		});
	});

	it("delivers canvas.close to onClose and still answers the host", async () => {
		const { canvas, closed } = board();
		await joinSession({ canvases: [canvas] }, streams);
		streams.input.feed(request(6, CANVAS_METHOD_CLOSE, {}));
		await settle();
		expect(closed).toEqual(["i1"]);
		expect(streams.written[1]).toMatchObject({ type: "response", id: 6, result: null });
	});

	it("forwards session.log to the host instead of stdout", async () => {
		const session = await joinSession({ canvases: [board().canvas] }, streams);
		await session.log("indexing", { level: "warning" });
		expect(streams.written[1]).toMatchObject({ type: "log", message: "indexing", level: "warning" });
	});

	it("handles two requests arriving in one chunk", async () => {
		await joinSession({ canvases: [board().canvas] }, streams);
		streams.input.feed(
			request(10, CANVAS_METHOD_OPEN, {}) + request(11, CANVAS_METHOD_INVOKE_ACTION, { actionName: "add_step" }),
		);
		await settle();
		expect(streams.written.slice(1)).toMatchObject([
			{ type: "response", id: 10 },
			{ type: "response", id: 11 },
		]);
	});

	it("rejects two canvases declaring the same id", async () => {
		await expect(joinSession({ canvases: [board().canvas, board().canvas] }, streams)).rejects.toThrow(
			/declared more than once/,
		);
	});
});
