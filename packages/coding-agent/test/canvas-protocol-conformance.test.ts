/**
 * Conformance: hoocode's canvas contract against the real `@github/copilot-sdk`.
 *
 * Design: `docs/canvas-extensions-design.md` §2.3. GitHub marks every canvas type
 * `@experimental` and says it "may change or be removed in future SDK or CLI
 * releases", so drift is stated policy rather than a risk to assess. The response
 * is to make drift a failing build instead of a vigilance problem:
 *
 *   - The type-level assignments below fail `tsgo --noEmit` (part of
 *     `bun run check`) if the SDK's canvas shapes change.
 *   - The protocol-version and wire-method assertions fail this test if the
 *     protocol moves.
 *
 * `@github/copilot-sdk` is a devDependency only, never a runtime one: it pulls
 * `koffi` (native FFI), which must not enter hoocode's runtime tree. Only types
 * are imported from it here; the two runtime facts are read off its published
 * files, which also avoids its `exports` map blocking deep imports.
 */

import { readFileSync } from "node:fs";
import * as path from "node:path";
import type {
	Canvas as SdkCanvas,
	CanvasAction as SdkCanvasAction,
	CanvasDeclaration as SdkCanvasDeclaration,
	CanvasOptions as SdkCanvasOptions,
} from "@github/copilot-sdk/extension";
import { describe, expect, it } from "vitest";
import {
	CANVAS_METHOD_CLOSE,
	CANVAS_METHOD_INVOKE_ACTION,
	CANVAS_METHOD_OPEN,
	CANVAS_PROVIDER_METHODS,
	CANVAS_SDK_PROTOCOL_VERSION,
	type CanvasDeclaration,
	CanvasMessageDecoder,
	encodeCanvasMessage,
	isCanvasChildToHostMessage,
	isCanvasHostToChildMessage,
} from "../src/core/canvas/protocol.js";
import {
	type Canvas,
	type CanvasAction,
	CanvasError,
	type CanvasOptions,
	createCanvas,
} from "../src/core/canvas/sdk-shim/index.js";

/**
 * Type-level conformance. Each assignment is checked by `tsgo --noEmit`; none of
 * them runs. Both directions are asserted, so neither a field the SDK adds nor a
 * field we invent passes unnoticed.
 */
const _sdkOptionsSatisfyShim: CanvasOptions = {} as SdkCanvasOptions;
const _shimOptionsSatisfySdk: SdkCanvasOptions = {} as CanvasOptions;
const _sdkActionSatisfiesShim: CanvasAction = {} as SdkCanvasAction;
const _shimActionSatisfiesSdk: SdkCanvasAction = {} as CanvasAction;
const _sdkDeclarationSatisfiesOurs: CanvasDeclaration = {} as SdkCanvasDeclaration;
const _ourDeclarationSatisfiesSdk: SdkCanvasDeclaration = {} as CanvasDeclaration;
/** `createCanvas` must accept anything the SDK's own options type describes. */
const _canvasFromSdkOptions: Canvas = createCanvas({} as SdkCanvasOptions);
/** Our `Canvas` must remain usable where the SDK's `Canvas` is expected. */
const _shimCanvasSatisfiesSdk: SdkCanvas = {} as Canvas;
void [
	_sdkOptionsSatisfyShim,
	_shimOptionsSatisfySdk,
	_sdkActionSatisfiesShim,
	_shimActionSatisfiesSdk,
	_sdkDeclarationSatisfiesOurs,
	_ourDeclarationSatisfiesSdk,
	_canvasFromSdkOptions,
	_shimCanvasSatisfiesSdk,
];

/** Locate the installed SDK by walking up from this file to the hoisted tree. */
function sdkFile(relative: string): string {
	let dir = import.meta.dirname;
	for (let depth = 0; depth < 6; depth += 1) {
		const candidate = path.join(dir, "node_modules", "@github", "copilot-sdk", relative);
		try {
			readFileSync(candidate, "utf8");
			return candidate;
		} catch {
			dir = path.dirname(dir);
		}
	}
	throw new Error(`Could not locate @github/copilot-sdk/${relative}; is the devDependency installed?`);
}

describe("canvas protocol conformance with @github/copilot-sdk", () => {
	it("targets the SDK's protocol version", () => {
		const source = readFileSync(sdkFile("dist/sdkProtocolVersion.js"), "utf8");
		const match = source.match(/SDK_PROTOCOL_VERSION\s*=\s*(\d+)/);
		expect(match, "SDK_PROTOCOL_VERSION is no longer a plain integer assignment").not.toBeNull();
		expect(Number(match?.[1])).toBe(CANVAS_SDK_PROTOCOL_VERSION);
	});

	it("uses the SDK's provider method names", () => {
		const source = readFileSync(sdkFile("dist/generated/rpc.js"), "utf8");
		for (const method of CANVAS_PROVIDER_METHODS) {
			expect(source, `SDK no longer references the "${method}" provider method`).toContain(`"${method}"`);
		}
	});

	it("keeps the three provider methods and nothing else", () => {
		expect([...CANVAS_PROVIDER_METHODS]).toEqual([
			CANVAS_METHOD_OPEN,
			CANVAS_METHOD_CLOSE,
			CANVAS_METHOD_INVOKE_ACTION,
		]);
	});
});

describe("createCanvas", () => {
	const base = { id: "c", displayName: "C", description: "d", open: () => ({}) } satisfies CanvasOptions;

	it("strips handlers from the wire declaration but keeps them for dispatch", () => {
		const canvas = createCanvas({
			...base,
			actions: [{ name: "go", description: "Go", handler: () => ({ ok: true }) }],
		});
		expect(canvas.declaration.actions).toEqual([{ name: "go", description: "Go", inputSchema: undefined }]);
		expect(canvas.handlers.get("go")).toBeTypeOf("function");
		expect(JSON.stringify(canvas.declaration)).not.toContain("handler");
	});

	it("omits the actions key entirely when a canvas declares none", () => {
		expect(createCanvas(base).declaration.actions).toBeUndefined();
	});

	it("rejects the reserved action prefix the SDK documents", () => {
		expect(() => createCanvas({ ...base, actions: [{ name: "canvas.open", handler: () => null }] })).toThrow(
			CanvasError,
		);
	});

	it("rejects duplicate action names", () => {
		expect(() =>
			createCanvas({
				...base,
				actions: [
					{ name: "go", handler: () => null },
					{ name: "go", handler: () => null },
				],
			}),
		).toThrow(/declared more than once/);
	});
});

describe("CanvasError", () => {
	it("carries a machine-readable code", () => {
		const error = new CanvasError("github_auth_required", "No usable GitHub account was found.");
		expect(error).toBeInstanceOf(Error);
		expect(error.code).toBe("github_auth_required");
		expect(error.name).toBe("CanvasError");
	});

	it("provides the SDK's noHandler default", () => {
		expect(CanvasError.noHandler().code).toBe("no_handler");
	});
});

describe("CanvasMessageDecoder", () => {
	it("reassembles messages split across chunk boundaries", () => {
		const line = encodeCanvasMessage({ envelope: 1, type: "log", message: "hello" });
		const decoder = new CanvasMessageDecoder();
		const half = Math.floor(line.length / 2);
		expect(decoder.push(line.slice(0, half)).values).toEqual([]);
		const rest = decoder.push(line.slice(half));
		expect(rest.values).toHaveLength(1);
		expect(isCanvasChildToHostMessage(rest.values[0])).toBe(true);
	});

	it("reports a stray stdout line instead of failing the stream", () => {
		const decoder = new CanvasMessageDecoder();
		const result = decoder.push(
			`oops a console.log\n${encodeCanvasMessage({ envelope: 1, type: "log", message: "m" })}`,
		);
		expect(result.strays).toEqual(["oops a console.log"]);
		expect(result.values).toHaveLength(1);
	});

	it("flushes a trailing line that arrived without a newline", () => {
		const decoder = new CanvasMessageDecoder();
		expect(decoder.push('{"envelope":1,"type":"log","message":"m"}').values).toEqual([]);
		expect(decoder.flush().values).toHaveLength(1);
	});

	it("rejects messages from a different envelope version", () => {
		expect(isCanvasHostToChildMessage({ envelope: 2, type: "request", id: 1, method: CANVAS_METHOD_OPEN })).toBe(
			false,
		);
	});

	it("rejects a request naming a method outside the provider contract", () => {
		expect(isCanvasHostToChildMessage({ envelope: 1, type: "request", id: 1, method: "canvas.evict" })).toBe(false);
	});
});
