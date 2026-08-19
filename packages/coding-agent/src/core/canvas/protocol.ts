/**
 * Canvas wire protocol — the entire GitHub-facing surface.
 *
 * Design: `docs/canvas-extensions-design.md` §2. The short version: GitHub's own
 * types (`@github/copilot-sdk` `dist/canvas.d.ts`) state that the Node
 * `createCanvas`/`joinSession` API is one of five language wrappers over the same
 * JSON-RPC wire protocol, and that "the divergence is API ergonomics only". So
 * hoocode binds to the wire protocol, not to the Node sugar, and the drift
 * surface is the three provider methods below plus one version integer.
 *
 * Everything a third-party canvas can observe lives in this file. If GitHub
 * moves the protocol, this file and `sdk-shim/` move; nothing else does.
 *
 * Two layers are deliberately separated here:
 *
 *   1. **Provider contract** (`CanvasProvider*`, `CANVAS_METHOD_*`) — GitHub's.
 *      These payload shapes reach extension code as the `ctx` argument of
 *      `open`, `onClose`, and action handlers, so they must stay structurally
 *      identical to the SDK's. `test/canvas-protocol-conformance.test.ts` fails
 *      the build if they drift.
 *   2. **Host envelope** (`Canvas*Message`) — ours. It carries provider calls
 *      between hoocode's runner and the child-side shim. In the Copilot case the
 *      equivalent layer sits between their CLI and their SDK and an extension
 *      never sees it, so we are free to keep it simple: newline-delimited JSON.
 */

/** JSON value, mirroring the SDK's `JsonValue`. */
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };

/** JSON Schema carried as data, mirroring the SDK's `CanvasJsonSchema`. */
export type CanvasJsonSchema = JsonValue;

/** Result of an action invocation, mirroring the SDK's `CanvasActionInvokeResult`. */
export type CanvasActionInvokeResult = JsonValue;

/**
 * SDK protocol version this implementation targets, mirroring the SDK's
 * `SDK_PROTOCOL_VERSION`. The conformance test asserts the two are equal, so a
 * protocol bump surfaces as a failing test rather than a runtime mismatch.
 */
export const CANVAS_SDK_PROTOCOL_VERSION = 3;

/** Provider callback: open a canvas instance. */
export const CANVAS_METHOD_OPEN = "canvas.open";
/** Provider callback: a canvas instance was closed. */
export const CANVAS_METHOD_CLOSE = "canvas.close";
/** Provider callback: invoke an agent-callable action. */
export const CANVAS_METHOD_INVOKE_ACTION = "canvas.action.invoke";

/** The three provider callbacks, in the order the design doc lists them. */
export const CANVAS_PROVIDER_METHODS = [CANVAS_METHOD_OPEN, CANVAS_METHOD_CLOSE, CANVAS_METHOD_INVOKE_ACTION] as const;

/** One of the three provider callback names. */
export type CanvasProviderMethod = (typeof CANVAS_PROVIDER_METHODS)[number];

/** Host capabilities advertised to the provider. */
export interface CanvasHostContextCapabilities {
	/** Whether canvas rendering is supported. */
	canvases?: boolean;
}

/** Host context supplied by the runtime. */
export interface CanvasHostContext {
	capabilities?: CanvasHostContextCapabilities;
}

/** Session context supplied by the runtime. */
export interface CanvasSessionContext {
	/** Active session working directory, when known. */
	workingDirectory?: string;
}

/** Fields shared by every provider callback payload. */
interface CanvasProviderRequestBase {
	/** Target session identifier. */
	sessionId: string;
	/** Owning provider identifier. */
	extensionId: string;
	/** Provider-local canvas identifier. */
	canvasId: string;
	/** Canvas instance identifier. */
	instanceId: string;
	host?: CanvasHostContext;
	session?: CanvasSessionContext;
}

/** `canvas.open` payload. */
export interface CanvasProviderOpenRequest extends CanvasProviderRequestBase {
	/** Canvas open input. */
	input?: JsonValue;
}

/** `canvas.open` result. A web-rendered canvas returns the URL the host loads. */
export interface CanvasProviderOpenResult {
	/** URL for web-rendered canvases. */
	url?: string;
	/** Provider-supplied title. */
	title?: string;
	/** Provider-supplied status text. */
	status?: string;
}

/** `canvas.close` payload. */
export interface CanvasProviderCloseRequest extends CanvasProviderRequestBase {}

/** `canvas.action.invoke` payload. */
export interface CanvasProviderInvokeActionRequest extends CanvasProviderRequestBase {
	/** Action name to invoke. */
	actionName: string;
	/** Action input. */
	input?: JsonValue;
}

/**
 * Action metadata as it crosses the wire. The SDK strips each action's `handler`
 * closure before sending the declaration, so this is `CanvasAction` minus the
 * handler.
 */
export interface CanvasActionDeclaration {
	/** Action identifier, unique within the canvas. */
	name: string;
	/** Description shown to the model when picking an action. */
	description?: string;
	/** Optional JSON Schema for the action's `input` payload. */
	inputSchema?: CanvasJsonSchema;
}

/** Declarative metadata for a single canvas. */
export interface CanvasDeclaration {
	/** Canvas id, unique within the declaring connection. */
	id: string;
	/** Human-readable label shown in discovery and host UI chrome. */
	displayName: string;
	/** Short, single-sentence description shown to the agent in canvas catalogs. */
	description: string;
	/** Optional JSON Schema for the `input` payload accepted by `canvas.open`. */
	inputSchema?: CanvasJsonSchema;
	/** Agent-invocable actions. */
	actions?: CanvasActionDeclaration[];
}

/**
 * Reserved action-name prefix. The SDK is explicit: "Names MUST NOT start with
 * `canvas.` — that prefix is reserved for lifecycle verbs."
 */
export const CANVAS_RESERVED_ACTION_PREFIX = "canvas.";

/** Log levels accepted by `session.log`. */
export type CanvasLogLevel = "info" | "warning" | "error";

/**
 * Host envelope version. Ours, not GitHub's — bumped only when the
 * runner↔shim framing changes.
 */
export const CANVAS_ENVELOPE_VERSION = 1;

/** Child announces itself and its canvases. Always the first message. */
export interface CanvasReadyMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "ready";
	/** The `CANVAS_SDK_PROTOCOL_VERSION` the child was built against. */
	protocolVersion: number;
	/** Provider identifier, derived by the runner from the extension directory. */
	extensionId: string;
	canvases: CanvasDeclaration[];
	/**
	 * Surfaces declared by the extension that this shim does not implement, so the
	 * runner can warn once instead of letting the extension half-work
	 * (design doc §6.2).
	 */
	unsupported?: string[];
}

/** Child forwards a `session.log` call. */
export interface CanvasLogMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "log";
	message: string;
	level?: CanvasLogLevel;
	ephemeral?: boolean;
}

/** Host asks the child to run one provider callback. */
export interface CanvasRequestMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "request";
	id: number;
	method: CanvasProviderMethod;
	params: JsonValue;
}

/** Child returns a provider callback result. */
export interface CanvasResponseMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "response";
	id: number;
	result: JsonValue;
}

/**
 * Child returns a failure. `code` carries `CanvasError.code` when the handler threw
 * one, so the agent gets a machine-readable code rather than a string to parse.
 */
export interface CanvasErrorMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "error";
	id: number;
	code: string;
	message: string;
}

/** Anything the host may send to a child. */
export type CanvasHostToChildMessage = CanvasRequestMessage;

/** Anything a child may send to the host. */
export type CanvasChildToHostMessage =
	| CanvasReadyMessage
	| CanvasLogMessage
	| CanvasResponseMessage
	| CanvasErrorMessage;

/** Error code used when a handler throws something that is not a `CanvasError`. */
export const CANVAS_ERROR_CODE_INTERNAL = "internal_error";
/** Error code used when the host asks for a canvas or action the child does not declare. */
export const CANVAS_ERROR_CODE_UNKNOWN_TARGET = "unknown_target";

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasCurrentEnvelope(value: Record<string, unknown>): boolean {
	return value.envelope === CANVAS_ENVELOPE_VERSION;
}

/** Whether `value` is a provider callback name. */
export function isCanvasProviderMethod(value: unknown): value is CanvasProviderMethod {
	return typeof value === "string" && (CANVAS_PROVIDER_METHODS as readonly string[]).includes(value);
}

/** Narrow a decoded value to a host→child message. */
export function isCanvasHostToChildMessage(value: unknown): value is CanvasHostToChildMessage {
	if (!isRecord(value) || !hasCurrentEnvelope(value)) return false;
	return value.type === "request" && typeof value.id === "number" && isCanvasProviderMethod(value.method);
}

/** Narrow a decoded value to a child→host message. */
export function isCanvasChildToHostMessage(value: unknown): value is CanvasChildToHostMessage {
	if (!isRecord(value) || !hasCurrentEnvelope(value)) return false;
	switch (value.type) {
		case "ready":
			return typeof value.extensionId === "string" && Array.isArray(value.canvases);
		case "log":
			return typeof value.message === "string";
		case "response":
			return typeof value.id === "number";
		case "error":
			return typeof value.id === "number" && typeof value.code === "string" && typeof value.message === "string";
		default:
			return false;
	}
}

/** Serialize one message as a single NDJSON line, newline included. */
export function encodeCanvasMessage(message: CanvasHostToChildMessage | CanvasChildToHostMessage): string {
	return `${JSON.stringify(message)}\n`;
}

/** One decode pass: parsed JSON values plus any lines that were not JSON at all. */
export interface CanvasDecodeResult {
	values: unknown[];
	/**
	 * Lines that failed to parse. In the Copilot CLI stdout is the JSON-RPC channel,
	 * so a stray `console.log` corrupts it; we keep the same discipline but report
	 * the stray text so the runner can tell the author to use `session.log`
	 * instead of leaving them with a silent protocol error.
	 */
	strays: string[];
}

/**
 * Incremental NDJSON decoder. Chunk boundaries do not respect line boundaries, so
 * a partial trailing line is held until the rest arrives.
 */
export class CanvasMessageDecoder {
	private buffer = "";

	/** Decode everything complete in `chunk`, buffering any partial trailing line. */
	push(chunk: string): CanvasDecodeResult {
		this.buffer += chunk;
		const result: CanvasDecodeResult = { values: [], strays: [] };
		let newline = this.buffer.indexOf("\n");
		while (newline !== -1) {
			this.take(this.buffer.slice(0, newline), result);
			this.buffer = this.buffer.slice(newline + 1);
			newline = this.buffer.indexOf("\n");
		}
		return result;
	}

	/** Decode whatever is left, for use when the stream ends without a trailing newline. */
	flush(): CanvasDecodeResult {
		const result: CanvasDecodeResult = { values: [], strays: [] };
		const rest = this.buffer;
		this.buffer = "";
		this.take(rest, result);
		return result;
	}

	private take(line: string, result: CanvasDecodeResult): void {
		const trimmed = line.trim();
		if (trimmed.length === 0) return;
		try {
			result.values.push(JSON.parse(trimmed));
		} catch {
			result.strays.push(trimmed);
		}
	}
}
