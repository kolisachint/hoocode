/**
 * Child-side `@github/copilot-sdk/extension` implementation.
 *
 * Design: `docs/canvas-extensions-design.md` §4, §4.1. A canvas extension from
 * `github/awesome-copilot` is a directory of plain `.mjs` files whose only
 * external import is `@github/copilot-sdk/extension` — GitHub's own
 * `docs/extensions.md` states the import "is resolved automatically — you don't
 * install it", and none of the 23 catalog extensions ships `node_modules`. This
 * module is what hoocode resolves that specifier to inside the forked child, so a
 * catalog canvas runs byte-identical to upstream with nothing written into its
 * directory.
 *
 * What this is not: the SDK's `joinSession` resolves to a full `CopilotSession`
 * (messaging, events, RPC passthrough, factories). This shim implements the
 * canvas surface and `log` only. `tools`, `hooks`, and `factories` are accepted
 * and reported as unsupported so an extension that uses them fails visibly rather
 * than half-working (design doc §6.2). The canvas types themselves are held
 * structurally identical to the SDK's by
 * `test/canvas-protocol-conformance.test.ts`.
 */

import {
	CANVAS_ENVELOPE_VERSION,
	CANVAS_ERROR_CODE_INTERNAL,
	CANVAS_ERROR_CODE_UNKNOWN_TARGET,
	CANVAS_METHOD_CLOSE,
	CANVAS_METHOD_INVOKE_ACTION,
	CANVAS_METHOD_OPEN,
	CANVAS_RESERVED_ACTION_PREFIX,
	CANVAS_SDK_PROTOCOL_VERSION,
	type CanvasActionDeclaration,
	type CanvasActionInvokeResult,
	type CanvasChildToHostMessage,
	type CanvasDeclaration,
	type CanvasJsonSchema,
	type CanvasLogLevel,
	CanvasMessageDecoder,
	type CanvasProviderCloseRequest,
	type CanvasProviderInvokeActionRequest,
	type CanvasProviderOpenRequest,
	type CanvasProviderOpenResult,
	encodeCanvasMessage,
	isCanvasHostToChildMessage,
	type JsonValue,
} from "../protocol.js";

/**
 * A single agent-callable action contributed by a canvas. Mirrors the SDK's
 * `CanvasAction`: the `handler` closure is stripped before the declaration is
 * sent and dispatched in-process here.
 */
export interface CanvasAction {
	/** Action identifier, unique within the canvas. */
	name: string;
	/** Description shown to the model when picking an action. */
	description?: string;
	/** Optional JSON Schema for the action's `input` payload. */
	inputSchema?: CanvasJsonSchema;
	/** Required per-action dispatch handler. */
	handler: (ctx: CanvasProviderInvokeActionRequest) => Promise<unknown> | unknown;
}

/** Options accepted by {@link createCanvas}. Mirrors the SDK's `CanvasOptions`. */
export interface CanvasOptions {
	/** Canvas id, unique within the declaring connection. */
	id: string;
	/** Human-readable label shown in discovery and host UI chrome. */
	displayName: string;
	/** Short, single-sentence description shown to the agent in canvas catalogs. */
	description: string;
	/** Optional JSON Schema for the `input` payload accepted by `canvas.open`. */
	inputSchema?: CanvasJsonSchema;
	/** Agent-invocable actions, each carrying its own required handler. */
	actions?: CanvasAction[];
	/** Required. Open a new canvas instance. */
	open: (ctx: CanvasProviderOpenRequest) => Promise<CanvasProviderOpenResult> | CanvasProviderOpenResult;
	/**
	 * Optional. Notified when a canvas instance is closed. Fire-and-forget: the
	 * return value is ignored and errors are logged but not surfaced.
	 */
	onClose?: (ctx: CanvasProviderCloseRequest) => Promise<void> | void;
}

/** Structured error returned from canvas handlers. Mirrors the SDK's `CanvasError`. */
export class CanvasError extends Error {
	readonly code: string;

	constructor(code: string, message: string) {
		super(message);
		this.name = "CanvasError";
		this.code = code;
	}

	/** Default error when an action is declared but no `handler` is wired. */
	static noHandler(): CanvasError {
		return new CanvasError("no_handler", "Canvas action has no handler.");
	}
}

/** A registered canvas: declarative metadata plus in-process handler closures. */
export class Canvas {
	readonly declaration: CanvasDeclaration;
	readonly open: NonNullable<CanvasOptions["open"]>;
	readonly onClose?: CanvasOptions["onClose"];
	/** Handlers by action name, kept out of the declaration that crosses the wire. */
	readonly handlers: ReadonlyMap<string, CanvasAction["handler"]>;

	constructor(options: CanvasOptions) {
		const actions: CanvasActionDeclaration[] = [];
		const handlers = new Map<string, CanvasAction["handler"]>();
		for (const action of options.actions ?? []) {
			if (action.name.startsWith(CANVAS_RESERVED_ACTION_PREFIX)) {
				throw new CanvasError(
					"invalid_action_name",
					`Action "${action.name}" must not start with "${CANVAS_RESERVED_ACTION_PREFIX}" — that prefix is reserved for lifecycle verbs.`,
				);
			}
			if (handlers.has(action.name)) {
				throw new CanvasError("duplicate_action", `Action "${action.name}" is declared more than once.`);
			}
			actions.push({ name: action.name, description: action.description, inputSchema: action.inputSchema });
			handlers.set(action.name, action.handler);
		}
		this.declaration = {
			id: options.id,
			displayName: options.displayName,
			description: options.description,
			inputSchema: options.inputSchema,
			actions: actions.length > 0 ? actions : undefined,
		};
		this.open = options.open;
		this.onClose = options.onClose;
		this.handlers = handlers;
	}
}

/** Create a canvas declaration with bound in-process handlers. */
export function createCanvas(options: CanvasOptions): Canvas {
	return new Canvas(options);
}

/** Surfaces a Copilot extension may declare that this shim does not implement. */
const UNSUPPORTED_KEYS = ["tools", "hooks", "factories", "onPermissionRequest"] as const;

/** Configuration accepted by {@link joinSession}. */
export interface JoinSessionConfig {
	canvases?: Canvas[];
	/** Accepted and reported as unsupported (design doc §6.2). */
	tools?: unknown;
	/** Accepted and reported as unsupported (design doc §6.2). */
	hooks?: unknown;
	/** Accepted and reported as unsupported (design doc §6.2). */
	factories?: unknown;
	/** Accepted and reported as unsupported (design doc §6.2). */
	onPermissionRequest?: unknown;
}

/**
 * The subset of the SDK's `CopilotSession` this shim provides. Deliberately small:
 * see the module header.
 */
export interface CanvasShimSession {
	/** Log a message to the session timeline. */
	log(message: string, options?: { level?: CanvasLogLevel; ephemeral?: boolean }): Promise<void>;
}

/** Streams the shim reads from and writes to. Injectable so tests need no child process. */
export interface CanvasShimStreams {
	input: { setEncoding(encoding: "utf8"): unknown; on(event: "data", listener: (chunk: string) => void): unknown };
	write(chunk: string): unknown;
	/** Provider identifier; the runner passes it in the environment. */
	extensionId: string;
}

function defaultStreams(): CanvasShimStreams {
	return {
		input: process.stdin,
		write: (chunk: string) => process.stdout.write(chunk),
		extensionId: process.env.HOOCODE_CANVAS_EXTENSION_ID ?? "unknown-extension",
	};
}

function toJsonValue(value: unknown): JsonValue {
	// Handler return values are author-supplied. Round-tripping through JSON is what
	// the wire does anyway, so do it here where a cycle or a BigInt fails loudly with
	// the action's name attached rather than mid-write.
	return value === undefined ? null : (JSON.parse(JSON.stringify(value)) as JsonValue);
}

function errorOf(cause: unknown): { code: string; message: string } {
	if (cause instanceof CanvasError) return { code: cause.code, message: cause.message };
	return {
		code: CANVAS_ERROR_CODE_INTERNAL,
		message: cause instanceof Error ? cause.message : String(cause),
	};
}

/**
 * Join the host session: announce the declared canvases, then serve provider
 * callbacks until the input stream ends.
 */
export async function joinSession(
	config: JoinSessionConfig = {},
	streams: CanvasShimStreams = defaultStreams(),
): Promise<CanvasShimSession> {
	const canvases = new Map<string, Canvas>();
	for (const canvas of config.canvases ?? []) {
		if (canvases.has(canvas.declaration.id)) {
			throw new CanvasError("duplicate_canvas", `Canvas "${canvas.declaration.id}" is declared more than once.`);
		}
		canvases.set(canvas.declaration.id, canvas);
	}

	const send = (message: CanvasChildToHostMessage): void => {
		streams.write(encodeCanvasMessage(message));
	};

	send({
		envelope: CANVAS_ENVELOPE_VERSION,
		type: "ready",
		protocolVersion: CANVAS_SDK_PROTOCOL_VERSION,
		extensionId: streams.extensionId,
		canvases: [...canvases.values()].map((canvas) => canvas.declaration),
		unsupported: UNSUPPORTED_KEYS.filter((key) => config[key] !== undefined),
	});

	const decoder = new CanvasMessageDecoder();
	streams.input.setEncoding("utf8");
	streams.input.on("data", (chunk: string) => {
		for (const value of decoder.push(chunk).values) {
			if (!isCanvasHostToChildMessage(value)) continue;
			void dispatch(canvases, value.id, value.method, value.params, send);
		}
	});

	return {
		log: async (message, options) => {
			send({
				envelope: CANVAS_ENVELOPE_VERSION,
				type: "log",
				message,
				level: options?.level,
				ephemeral: options?.ephemeral,
			});
		},
	};
}

async function dispatch(
	canvases: ReadonlyMap<string, Canvas>,
	id: number,
	method: string,
	params: JsonValue,
	send: (message: CanvasChildToHostMessage) => void,
): Promise<void> {
	try {
		send({
			envelope: CANVAS_ENVELOPE_VERSION,
			type: "response",
			id,
			result: await invoke(canvases, method, params),
		});
	} catch (cause) {
		const { code, message } = errorOf(cause);
		send({ envelope: CANVAS_ENVELOPE_VERSION, type: "error", id, code, message });
	}
}

async function invoke(canvases: ReadonlyMap<string, Canvas>, method: string, params: JsonValue): Promise<JsonValue> {
	if (method === CANVAS_METHOD_OPEN) {
		const ctx = params as unknown as CanvasProviderOpenRequest;
		return toJsonValue(await resolve(canvases, ctx.canvasId).open(ctx));
	}
	if (method === CANVAS_METHOD_CLOSE) {
		const ctx = params as unknown as CanvasProviderCloseRequest;
		// Fire-and-forget per the SDK contract: a failing onClose must not fail the close.
		await resolve(canvases, ctx.canvasId).onClose?.(ctx);
		return null;
	}
	if (method === CANVAS_METHOD_INVOKE_ACTION) {
		const ctx = params as unknown as CanvasProviderInvokeActionRequest;
		const canvas = resolve(canvases, ctx.canvasId);
		const handler = canvas.handlers.get(ctx.actionName);
		if (!handler) {
			throw new CanvasError(
				CANVAS_ERROR_CODE_UNKNOWN_TARGET,
				`Canvas "${ctx.canvasId}" does not declare an action named "${ctx.actionName}".`,
			);
		}
		const result: CanvasActionInvokeResult = toJsonValue(await handler(ctx));
		return result;
	}
	// Unreachable while the host only sends methods `isCanvasHostToChildMessage`
	// accepts, but naming it keeps this function total if the protocol grows.
	throw new CanvasError(CANVAS_ERROR_CODE_UNKNOWN_TARGET, `Unsupported provider method "${method}".`);
}

function resolve(canvases: ReadonlyMap<string, Canvas>, canvasId: string): Canvas {
	const canvas = canvases.get(canvasId);
	if (!canvas) {
		throw new CanvasError(CANVAS_ERROR_CODE_UNKNOWN_TARGET, `No canvas is registered with id "${canvasId}".`);
	}
	return canvas;
}
