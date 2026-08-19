/**
 * Canvas extension runner — forks an extension and speaks the provider protocol.
 *
 * Design: `docs/canvas-extensions-design.md` §3.1, §4, §6.1. Two decisions are
 * load-bearing here:
 *
 *   1. **It forks; it does not import.** hoocode's own extensions load in-process
 *      (`core/extensions/loader.ts`), which would put a third-party canvas inside
 *      the permission gate with full access to the tool registry and provider
 *      credentials. Canvas extensions are strangers, so they get a process
 *      boundary — which is also what the Copilot CLI does, so a canvas behaves
 *      the same in both hosts.
 *   2. **Lifecycle mirrors the documented CLI contract**: forked child, JSON-RPC
 *      over stdio, and shutdown by SIGTERM followed by SIGKILL after 5s.
 *
 * stdout is the protocol channel, exactly as in the Copilot CLI, so a stray
 * `console.log` in an extension corrupts it there and here alike. Rather than
 * failing opaquely, unparseable lines are surfaced through `onStray` so a caller
 * can tell the author to use `session.log` instead.
 */

import { type ChildProcessWithoutNullStreams, spawn } from "node:child_process";
import {
	CANVAS_ENVELOPE_VERSION,
	type CanvasChildToHostMessage,
	CanvasMessageDecoder,
	type CanvasProviderCloseRequest,
	type CanvasProviderInvokeActionRequest,
	type CanvasProviderMethod,
	type CanvasProviderOpenRequest,
	type CanvasReadyMessage,
	encodeCanvasMessage,
	isCanvasChildToHostMessage,
	type JsonValue,
} from "./protocol.js";
import { canvasResolverImportArg } from "./resolver.js";

/** Code on the error a cancelled or timed-out provider call rejects with. */
export const CANVAS_ERROR_CODE_ABORTED = "aborted";

/** Per-call options. A signal cancels the wait; the child is reconciled by the caller. */
export interface CanvasCallOptions {
	/**
	 * Stop waiting when this aborts.
	 *
	 * The provider protocol has no cancel verb, so aborting only ends *our* wait —
	 * the child may still be working and may still answer. `registry.ts` is what
	 * reconciles that, by closing the instance it will never see (see its `abandon`).
	 */
	signal?: AbortSignal;
}

/** Grace period between SIGTERM and SIGKILL, matching the documented CLI contract. */
export const CANVAS_SHUTDOWN_GRACE_MS = 5_000;

/**
 * Default ceiling per provider method, so a wedged handler cannot hang a session
 * (design doc §11.4).
 *
 * These differ because the calls do. `canvas.open` may legitimately do real work
 * before it can return a URL — `pr-artifact-explorer` starts a server and
 * `inspect_artifact`-shaped canvases may fetch — whereas an action is a request
 * against an already-open instance, and `close` should be near-instant since the
 * SDK contract makes `onClose` fire-and-forget. A single 30s ceiling for all three
 * was a guess, and wrong at both ends.
 */
export const CANVAS_REQUEST_TIMEOUT_MS: Readonly<Record<CanvasProviderMethod, number>> = {
	"canvas.open": 120_000,
	"canvas.action.invoke": 30_000,
	"canvas.close": 5_000,
};

/** How the child is launched. Injectable so tests can run the TypeScript shim under tsx. */
export interface CanvasRuntime {
	/** Executable to fork. */
	execPath: string;
	/** Arguments that precede the entry file, excluding the resolver `--import`. */
	execArgv: string[];
	/** Absolute `file:` URL of the module the SDK specifier resolves to. */
	shimUrl: string;
}

/** Everything needed to fork one extension. */
export interface CanvasRunnerOptions {
	/** Provider identifier — the extension directory name. */
	extensionId: string;
	/** Absolute path to the extension's `extension.mjs`. */
	entry: string;
	/** Value for the child's `SESSION_ID`. Defaults to the extension id. */
	sessionId?: string;
	runtime: CanvasRuntime;
	/** Working directory for the child. Defaults to the extension directory's parent. */
	cwd?: string;
	env?: NodeJS.ProcessEnv;
	/** Per-method overrides merged over {@link CANVAS_REQUEST_TIMEOUT_MS}. 0 disables. */
	requestTimeoutMs?: Partial<Record<CanvasProviderMethod, number>>;
	/** A `session.log` call from the extension. */
	onLog?: (message: string, level: string | undefined, ephemeral: boolean | undefined) => void;
	/** A stdout line that was not protocol. Almost always a stray `console.log`. */
	onStray?: (line: string) => void;
	/** The child's stderr, which extensions use normally. */
	onStderr?: (chunk: string) => void;
	/** The child exited, expectedly or not. */
	onExit?: (code: number | null, signal: NodeJS.Signals | null) => void;
}

/** A running canvas extension. */
export interface CanvasExtensionProcess {
	readonly extensionId: string;
	/** Resolves with the child's first `ready` message, or rejects if it exits first. */
	readonly ready: Promise<CanvasReadyMessage>;
	/** Whether the child is still running. */
	readonly running: boolean;
	open(params: CanvasProviderOpenRequest, options?: CanvasCallOptions): Promise<JsonValue>;
	close(params: CanvasProviderCloseRequest, options?: CanvasCallOptions): Promise<JsonValue>;
	invokeAction(params: CanvasProviderInvokeActionRequest, options?: CanvasCallOptions): Promise<JsonValue>;
	/** SIGTERM, then SIGKILL after {@link CANVAS_SHUTDOWN_GRACE_MS}. Resolves on exit. */
	terminate(): Promise<void>;
}

interface PendingCall {
	resolve: (value: JsonValue) => void;
	reject: (reason: Error) => void;
	/** Clear the timer and detach the abort listener. Safe to call twice. */
	dispose: () => void;
}

/** Error carrying the `CanvasError.code` an extension handler threw. */
export class CanvasCallError extends Error {
	readonly code: string;

	constructor(code: string, message: string) {
		super(message);
		this.name = "CanvasCallError";
		this.code = code;
	}
}

/** Fork an extension and return a handle to its provider surface. */
export function spawnCanvasExtension(options: CanvasRunnerOptions): CanvasExtensionProcess {
	const timeouts: Record<CanvasProviderMethod, number> = { ...CANVAS_REQUEST_TIMEOUT_MS, ...options.requestTimeoutMs };
	const child = spawn(
		options.runtime.execPath,
		[...options.runtime.execArgv, "--import", canvasResolverImportArg(options.runtime.shimUrl), options.entry],
		{
			cwd: options.cwd,
			env: {
				...(options.env ?? process.env),
				HOOCODE_CANVAS_EXTENSION_ID: options.extensionId,
				// The real SDK's joinSession throws without SESSION_ID (design doc §11.2).
				// Our shim does not read it, but an extension that uses the SDK directly
				// would, so setting it is free fidelity. COPILOT_HOME is deliberately left
				// alone: a canvas should not be able to tell the hosts apart.
				SESSION_ID: options.sessionId ?? options.extensionId,
			},
			stdio: ["pipe", "pipe", "pipe"],
		},
	) as ChildProcessWithoutNullStreams;

	const pending = new Map<number, PendingCall>();
	const decoder = new CanvasMessageDecoder();
	let nextId = 1;
	let exited = false;
	let readyResolve: ((value: CanvasReadyMessage) => void) | undefined;
	let readyReject: ((reason: Error) => void) | undefined;
	const ready = new Promise<CanvasReadyMessage>((resolve, reject) => {
		readyResolve = resolve;
		readyReject = reject;
	});

	const settleAll = (reason: Error): void => {
		for (const call of pending.values()) {
			call.dispose();
			call.reject(reason);
		}
		pending.clear();
	};

	const handle = (message: CanvasChildToHostMessage): void => {
		switch (message.type) {
			case "ready":
				readyResolve?.(message);
				return;
			case "log":
				options.onLog?.(message.message, message.level, message.ephemeral);
				return;
			default: {
				// A response for an id no longer pending is one whose caller stopped
				// waiting — aborted or timed out. Dropping it is the whole point.
				const call = pending.get(message.id);
				if (!call) return;
				pending.delete(message.id);
				call.dispose();
				if (message.type === "response") call.resolve(message.result);
				else call.reject(new CanvasCallError(message.code, message.message));
			}
		}
	};

	child.stdout.setEncoding("utf8");
	child.stdout.on("data", (chunk: string) => {
		const { values, strays } = decoder.push(chunk);
		for (const line of strays) options.onStray?.(line);
		for (const value of values) {
			if (isCanvasChildToHostMessage(value)) handle(value);
			else options.onStray?.(JSON.stringify(value));
		}
	});

	child.stderr.setEncoding("utf8");
	child.stderr.on("data", (chunk: string) => options.onStderr?.(chunk));

	child.on("error", (error: Error) => {
		exited = true;
		readyReject?.(error);
		settleAll(error);
	});

	child.on("exit", (code, signal) => {
		exited = true;
		const reason = new Error(
			`Canvas extension "${options.extensionId}" exited (code ${String(code)}, signal ${String(signal)}).`,
		);
		readyReject?.(reason);
		settleAll(reason);
		options.onExit?.(code, signal);
	});

	const call = (
		method: CanvasProviderMethod,
		params: JsonValue,
		callOptions?: CanvasCallOptions,
	): Promise<JsonValue> => {
		if (exited) {
			return Promise.reject(new Error(`Canvas extension "${options.extensionId}" is not running.`));
		}
		const signal = callOptions?.signal;
		if (signal?.aborted) {
			// Nothing is written to the child: a request nobody awaits should not exist.
			return Promise.reject(
				new CanvasCallError(CANVAS_ERROR_CODE_ABORTED, `Canvas ${method} was cancelled before it started.`),
			);
		}
		const id = nextId;
		nextId += 1;
		const timeoutMs = timeouts[method];
		return new Promise<JsonValue>((resolve, reject) => {
			const abandon = (error: Error): void => {
				const entry = pending.get(id);
				if (!entry) return;
				pending.delete(id);
				entry.dispose();
				reject(error);
			};
			const timer =
				timeoutMs > 0
					? setTimeout(
							() =>
								abandon(
									new CanvasCallError(
										CANVAS_ERROR_CODE_ABORTED,
										`Canvas extension "${options.extensionId}" did not answer ${method} within ${timeoutMs}ms.`,
									),
								),
							timeoutMs,
						)
					: undefined;
			const onAbort = () =>
				abandon(new CanvasCallError(CANVAS_ERROR_CODE_ABORTED, `Canvas ${method} was cancelled.`));
			signal?.addEventListener("abort", onAbort, { once: true });
			pending.set(id, {
				resolve,
				reject,
				dispose: () => {
					if (timer) clearTimeout(timer);
					signal?.removeEventListener("abort", onAbort);
				},
			});
			child.stdin.write(
				encodeCanvasMessage({ envelope: CANVAS_ENVELOPE_VERSION, type: "request", id, method, params }),
			);
		});
	};

	return {
		extensionId: options.extensionId,
		ready,
		get running() {
			return !exited;
		},
		open: (params, callOptions) => call("canvas.open", params as unknown as JsonValue, callOptions),
		close: (params, callOptions) => call("canvas.close", params as unknown as JsonValue, callOptions),
		invokeAction: (params, callOptions) => call("canvas.action.invoke", params as unknown as JsonValue, callOptions),
		terminate: () =>
			new Promise<void>((resolve) => {
				if (exited) {
					resolve();
					return;
				}
				const kill = setTimeout(() => child.kill("SIGKILL"), CANVAS_SHUTDOWN_GRACE_MS);
				child.once("exit", () => {
					clearTimeout(kill);
					resolve();
				});
				child.kill("SIGTERM");
			}),
	};
}
