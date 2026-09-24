/**
 * Canvas wire protocol — the entire GitHub-facing surface.
 *
 * Design: `docs/canvas-extensions-design.md` §2. The short version: GitHub's own
 * types (`@github/copilot-sdk` `dist/canvas.d.ts`) state that the Node
 * `createCanvas`/`joinSession` API is one of five language wrappers over the same
 * JSON-RPC wire protocol, and that "the divergence is API ergonomics only". So
 * hoocode binds to the wire protocol, not to the Node sugar, and the drift
 * surface is the three provider methods below plus one version integer — and,
 * since envelope 2, the slice of `CopilotSession` a canvas uses to talk back:
 * `send` and `on` (see {@link CanvasSendMessage}, {@link CanvasSessionEvent}).
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
 *
 * `send`, `subscribe` and `event` were added without a bump: they are new
 * message types, not a change to existing ones, and the shim and the runner
 * ship together (`attach` likewise). They are the part of the SDK's `CopilotSession` that lets a
 * canvas talk to the agent and watch what it is doing; before them a canvas
 * could answer the agent but never start a conversation.
 */
export const CANVAS_ENVELOPE_VERSION = 1;

/**
 * Delivery mode for {@link CanvasSendMessage}, mirroring the SDK's
 * `MessageOptions.mode`: `enqueue` waits for the agent to finish what it is
 * doing, `immediate` steers the turn in progress.
 */
export type CanvasSendMode = "enqueue" | "immediate";

/**
 * Session event types a canvas may subscribe to, a subset of the SDK's
 * `SessionEventType`. Names and payload fields are GitHub's; hoocode emits only
 * these because they are the ones it can fill honestly, and a canvas that
 * subscribes to another type simply never hears it — as it would from a host
 * that never emits it.
 */
export const CANVAS_SESSION_EVENT_TYPES = [
	"assistant.turn_start",
	"assistant.intent",
	"tool.execution_start",
	"tool.execution_complete",
	"session.idle",
	"session.todos_changed",
] as const;

/** One of {@link CANVAS_SESSION_EVENT_TYPES}. */
export type CanvasSessionEventType = (typeof CANVAS_SESSION_EVENT_TYPES)[number];

/** Subscribe to every event type the host emits. */
export const CANVAS_EVENT_WILDCARD = "*";

/**
 * A session event as it reaches a canvas: the SDK's `SessionEvent` envelope
 * (`id`, `timestamp`, `parentId`, `ephemeral`, `type`, `data`).
 *
 * `data` carries GitHub's field names. Where hoocode adds a field upstream does
 * not have, it is additive and documented at the emit site — for example
 * `session.todos_changed`, which upstream sends empty and follows with an RPC
 * read hoocode does not have, carries the list itself.
 */
export interface CanvasSessionEvent {
	id: string;
	timestamp: string;
	parentId: string | null;
	ephemeral: boolean;
	type: CanvasSessionEventType;
	data: { [key: string]: JsonValue };
}

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

/**
 * Child forwards a `session.send` call: a message for the agent.
 *
 * Whether and when it reaches the model is the host's decision, not the
 * canvas's — see `core/canvas/inbox.ts` for the policy that keeps a canvas
 * from flooding a session.
 */
export interface CanvasSendMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "send";
	/** Id the shim returned to the caller, so the two can be matched in logs. */
	messageId: string;
	prompt: string;
	mode?: CanvasSendMode;
}

/**
 * Child states the full set of event types it listens for.
 *
 * The whole set rather than a delta, so a lost or reordered message cannot
 * leave the host forwarding events nobody wants. Sent whenever `session.on`
 * adds the first handler for a type or removes the last one.
 */
export interface CanvasSubscribeMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "subscribe";
	/** Event types, or {@link CANVAS_EVENT_WILDCARD}. */
	events: string[];
}

/**
 * One `extension_context` attachment: a titled piece of context the host shows
 * as a pill in its prompt and sends with the person's next message. The SDK's
 * `ExtensionContextPushInput`, minus the `type` discriminator.
 */
export interface CanvasAttachment {
	title: string;
	payload: JsonValue;
}

/**
 * Child forwards `session.rpc.extensions.sendAttachmentsToMessage`: context for
 * the person's next message. The set replaces what this extension pushed before;
 * an empty set withdraws it.
 */
export interface CanvasAttachMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "attach";
	/** The canvas instance the context came from, when the extension named one. */
	instanceId?: string;
	attachments: CanvasAttachment[];
}

/** Host delivers a session event the child subscribed to. */
export interface CanvasEventMessage {
	envelope: typeof CANVAS_ENVELOPE_VERSION;
	type: "event";
	event: CanvasSessionEvent;
}

/** Anything the host may send to a child. */
export type CanvasHostToChildMessage = CanvasRequestMessage | CanvasEventMessage;

/** Anything a child may send to the host. */
export type CanvasChildToHostMessage =
	| CanvasReadyMessage
	| CanvasLogMessage
	| CanvasResponseMessage
	| CanvasErrorMessage
	| CanvasSendMessage
	| CanvasSubscribeMessage
	| CanvasAttachMessage;

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
	if (value.type === "event") return isRecord(value.event) && typeof value.event.type === "string";
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
		case "send":
			return (
				typeof value.messageId === "string" &&
				typeof value.prompt === "string" &&
				(value.mode === undefined || value.mode === "enqueue" || value.mode === "immediate")
			);
		case "subscribe":
			return Array.isArray(value.events) && value.events.every((event) => typeof event === "string");
		case "attach":
			return (
				(value.instanceId === undefined || typeof value.instanceId === "string") &&
				Array.isArray(value.attachments) &&
				value.attachments.every((item) => isRecord(item) && typeof item.title === "string")
			);
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
