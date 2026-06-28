/**
 * Shared plumbing for the `browser_flow` and `browser_resume` tools.
 *
 * Unlike `webtools` (one-shot fetch/search subprocesses), `browsertools` drives
 * a *stateful* parent-in-the-loop protocol: a single long-lived `browsertools
 * serve` process holds the live Chromium and the paused-flow state in memory,
 * and the parent (hoocode) talks to it over newline-delimited JSON-RPC on
 * stdin/stdout. A flow can suspend mid-replay with `Outcome::NeedsParent`,
 * yielding a typed `ParentRequest` + `ResumeToken`; the parent reasons about it
 * (often by looking at a screenshot) and calls `flow_resume(token, response)` to
 * continue. See https://github.com/kolisachint/browsertools (contract.rs).
 *
 * Because that single serve process must survive *between* the `browser_flow`
 * call that starts the flow and the `browser_resume` call that answers a
 * `NeedsParent`, this module owns a process-wide {@link sessionRegistry} keyed by
 * `ResumeToken`. Each paused session keeps its serve client alive and is reaped
 * after an idle timeout so an abandoned flow never leaks a Chromium forever.
 *
 * This module owns:
 * - the persistent JSON-RPC serve client (id-correlated request/response),
 * - the locked contract types (mirrors browsertools `src/contract.rs`),
 * - the paused-session registry + lifecycle (idle reap, round cap, teardown),
 * - binary + browser-path resolution.
 */

import { type ChildProcess, spawn } from "node:child_process";
import { ensureTool } from "../../utils/tools-manager.js";

// ============================================================================
// Contract types (locked against browsertools `src/contract.rs`)
// ============================================================================

/** Opaque token identifying a paused flow, returned with every `NeedsParent`. */
export type ResumeToken = string;

/** Reference to bytes (e.g. a screenshot) the parent can fetch via `get_resource`. */
export type ResourceId = string;

/**
 * A typed request the engine yields when replay hits ambiguity only an LLM can
 * resolve. The `request` discriminant matches the serde tag in contract.rs.
 * Fields beyond the discriminant are passed through verbatim to the model.
 */
export interface ParentRequest {
	request: "classify_state" | "verify_visual" | "extract_semantic" | "decide_next_action" | "reidentify_element";
	/** Resource id of the screenshot captured at the suspension point, if any. */
	screenshot_ref?: ResourceId;
	[key: string]: unknown;
}

/** Result of a `flow_start` / `flow_resume` RPC (the `result` field's `outcome`). */
export type FlowOutcome =
	| { outcome: "complete"; result?: unknown }
	| { outcome: "needs_parent"; request: ParentRequest; token: ResumeToken }
	| { outcome: "failed"; step_id?: string; kind?: string; detail?: string };

/** Result of a `get_resource` RPC. */
export interface GetResourceResult {
	ref: ResourceId;
	mime: string;
	len: number;
	png_base64: string;
}

// ============================================================================
// Configuration
// ============================================================================

/** Default per-request RPC timeout. Flow steps drive a real browser, so this is
 *  generous relative to webtools' 15s. */
export const BROWSERTOOLS_REQUEST_TIMEOUT_MS = 120_000;

/** Default idle window a paused flow may sit in the registry before it is reaped
 *  (killing its serve process + Chromium). Reset on every resume round. The
 *  parent may reason / look at a screenshot / make other tool calls between
 *  rounds, so this is minutes, not seconds. */
export const BROWSERTOOLS_DEFAULT_IDLE_MS = 5 * 60 * 1000;

/** Default hard cap on `NeedsParent` rounds for a single flow, bounding a runaway
 *  suspend/resume loop. */
export const BROWSERTOOLS_DEFAULT_MAX_ROUNDS = 12;

/** Options threaded into the browser tools by their factories (tests/overrides). */
export interface BrowsertoolsToolOptions {
	/** Explicit path to the `browsertools` binary; otherwise resolved via {@link ensureTool}. */
	binaryPath?: string;
	/** Chromium/Chrome executable path forwarded to the serve process. */
	browserPath?: string;
	/** Idle reap window for paused sessions (ms). */
	idleTimeoutMs?: number;
	/** Hard cap on NeedsParent rounds per flow. */
	maxParentRounds?: number;
	/** Per-request RPC timeout (ms). */
	requestTimeoutMs?: number;
	/** Extra args appended after `serve` (mainly for tests). */
	serveArgs?: string[];
	/** Launch a real on-screen Chromium window instead of headless (sets
	 *  `BROWSERTOOLS_HEADFUL=1` on the serve process). */
	headful?: boolean;
	/** Default the streamed live viewer on for every flow (the per-call `live_view`
	 *  param still overrides). Set by the --enable-browser-live-preview flag. */
	liveView?: boolean;
}

function envNumber(name: string): number | undefined {
	const raw = process.env[name];
	if (!raw) return undefined;
	const n = Number(raw);
	return Number.isFinite(n) && n > 0 ? n : undefined;
}

/** Resolve effective options from explicit overrides, then env, then defaults. */
export function resolveBrowsertoolsOptions(options?: BrowsertoolsToolOptions): Required<
	Pick<BrowsertoolsToolOptions, "idleTimeoutMs" | "maxParentRounds" | "requestTimeoutMs">
> & {
	binaryPath?: string;
	browserPath?: string;
	serveArgs: string[];
	headful: boolean;
	liveView: boolean;
} {
	return {
		binaryPath: options?.binaryPath,
		browserPath: options?.browserPath ?? (process.env.HOOCODE_BROWSERTOOLS_BROWSER_PATH?.trim() || undefined),
		headful: options?.headful ?? false,
		liveView: options?.liveView ?? false,
		idleTimeoutMs:
			options?.idleTimeoutMs ?? envNumber("HOOCODE_BROWSERTOOLS_IDLE_MS") ?? BROWSERTOOLS_DEFAULT_IDLE_MS,
		maxParentRounds:
			options?.maxParentRounds ?? envNumber("HOOCODE_BROWSERTOOLS_MAX_ROUNDS") ?? BROWSERTOOLS_DEFAULT_MAX_ROUNDS,
		requestTimeoutMs:
			options?.requestTimeoutMs ??
			envNumber("HOOCODE_BROWSERTOOLS_REQUEST_TIMEOUT_MS") ??
			BROWSERTOOLS_REQUEST_TIMEOUT_MS,
		serveArgs: options?.serveArgs ?? [],
	};
}

const BINARY_MISSING_MESSAGE =
	"browsertools binary unavailable and could not be downloaded — the browser tools require the `browsertools` CLI on PATH or a published release for this platform";

/** Resolve the `browsertools` binary path (explicit override, else download/PATH). */
export async function resolveBrowsertoolsBinary(options?: BrowsertoolsToolOptions): Promise<string> {
	if (options?.binaryPath) return options.binaryPath;
	const binaryPath = await ensureTool("browsertools", true);
	if (!binaryPath) throw new Error(BINARY_MISSING_MESSAGE);
	return binaryPath;
}

// ============================================================================
// Persistent JSON-RPC serve client
// ============================================================================

interface PendingRequest {
	resolve: (value: unknown) => void;
	reject: (error: Error) => void;
	timer: ReturnType<typeof setTimeout>;
}

/**
 * A live `browsertools serve` process spoken to over newline-delimited JSON-RPC.
 *
 * Wire format (browsertools `src/serve.rs`):
 *   request:  `{ "id": <n>, "method": <string>, "params": <object> }\n`
 *   response: `{ "id": <n>, "result": <value> }` or `{ "id": <n>, "error": <RpcError> }`\n
 *
 * Responses are correlated by `id`, so concurrent requests are safe, though the
 * tools drive it sequentially.
 */
export class BrowsertoolsServeClient {
	private readonly proc: ChildProcess;
	private readonly pending = new Map<number, PendingRequest>();
	private readonly requestTimeoutMs: number;
	private buffer = "";
	private nextId = 1;
	private closed = false;
	private exitError: Error | undefined;
	private stderrTail = "";

	constructor(
		binaryPath: string,
		opts: {
			cwd?: string;
			browserPath?: string;
			serveArgs: string[];
			requestTimeoutMs: number;
			headful?: boolean;
		},
	) {
		this.requestTimeoutMs = opts.requestTimeoutMs;
		// The exact CLI flag for the browser path is not part of the locked RPC
		// contract; v0.1.3 added a "configurable browser path". We forward it via
		// env vars the binary is likely to read. Passing extra env is harmless if
		// the binary ignores them, and centralizing it here makes it a one-line
		// change once the exact mechanism is confirmed.
		const env = { ...process.env };
		if (opts.browserPath) {
			env.BROWSERTOOLS_BROWSER_PATH = opts.browserPath;
			env.CHROME_PATH = opts.browserPath;
		}
		// Opt into a real on-screen Chromium window. The serve process reads this
		// in `Driver::launch`; harmless if an older binary ignores it.
		if (opts.headful) {
			env.BROWSERTOOLS_HEADFUL = "1";
		}
		// Run the serve process in the project cwd so relative flow paths and the
		// evidence store resolve the same way the user expects.
		this.proc = spawn(binaryPath, ["serve", ...opts.serveArgs], {
			cwd: opts.cwd,
			shell: false,
			stdio: ["pipe", "pipe", "pipe"],
			env,
		});
		this.proc.stdout?.setEncoding("utf8");
		this.proc.stdout?.on("data", (chunk: string) => this.onStdout(chunk));
		this.proc.stderr?.setEncoding("utf8");
		this.proc.stderr?.on("data", (chunk: string) => {
			// Keep only a bounded tail so a chatty serve can't grow unbounded; it is
			// surfaced in the error when the process dies unexpectedly.
			this.stderrTail = (this.stderrTail + chunk).slice(-4000);
		});
		this.proc.on("error", (err) => this.fail(err instanceof Error ? err : new Error(String(err))));
		this.proc.on("exit", (code, sig) => {
			if (this.closed) return;
			const detail = this.stderrTail.trim();
			this.fail(
				new Error(
					`browsertools serve exited unexpectedly (code=${code ?? "null"}, signal=${sig ?? "null"})` +
						(detail ? `: ${detail}` : ""),
				),
			);
		});
	}

	private onStdout(chunk: string): void {
		this.buffer += chunk;
		let newlineIndex = this.buffer.indexOf("\n");
		while (newlineIndex !== -1) {
			const line = this.buffer.slice(0, newlineIndex).trim();
			this.buffer = this.buffer.slice(newlineIndex + 1);
			if (line) this.onLine(line);
			newlineIndex = this.buffer.indexOf("\n");
		}
	}

	private onLine(line: string): void {
		let message: { id?: unknown; result?: unknown; error?: unknown };
		try {
			message = JSON.parse(line);
		} catch {
			// Non-JSON noise on stdout (shouldn't happen; logs go to stderr). Skip it
			// rather than corrupt id correlation.
			return;
		}
		const id = typeof message.id === "number" ? message.id : undefined;
		if (id === undefined) return;
		const entry = this.pending.get(id);
		if (!entry) return;
		this.pending.delete(id);
		clearTimeout(entry.timer);
		if (message.error !== undefined && message.error !== null) {
			entry.reject(new Error(`browsertools ${describeRpcError(message.error)}`));
		} else {
			entry.resolve(message.result);
		}
	}

	/** Reject all in-flight requests and mark the client dead. Idempotent. */
	private fail(error: Error): void {
		if (this.closed) return;
		this.exitError = error;
		for (const [, entry] of this.pending) {
			clearTimeout(entry.timer);
			entry.reject(error);
		}
		this.pending.clear();
	}

	/** Send a JSON-RPC request and resolve with its `result` (or reject on `error`). */
	request<T = unknown>(method: string, params: Record<string, unknown>): Promise<T> {
		if (this.closed) {
			return Promise.reject(this.exitError ?? new Error("browsertools serve client is closed"));
		}
		const id = this.nextId++;
		return new Promise<T>((resolve, reject) => {
			const timer = setTimeout(() => {
				if (this.pending.delete(id)) {
					reject(new Error(`browsertools ${method} timed out after ${this.requestTimeoutMs}ms`));
				}
			}, this.requestTimeoutMs);
			this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
			const payload = `${JSON.stringify({ id, method, params })}\n`;
			this.proc.stdin?.write(payload, (err) => {
				if (err && this.pending.delete(id)) {
					clearTimeout(timer);
					reject(err);
				}
			});
		});
	}

	/** Kill the serve process and reject any outstanding requests. Idempotent. */
	dispose(): void {
		if (this.closed) return;
		this.fail(new Error("browsertools serve client disposed"));
		this.closed = true;
		// Best-effort graceful shutdown, then ensure the process is gone.
		try {
			this.proc.stdin?.write(`${JSON.stringify({ id: this.nextId++, method: "shutdown", params: {} })}\n`);
		} catch {
			// stdin may already be closed; fall through to kill.
		}
		this.proc.stdin?.end();
		this.proc.kill("SIGTERM");
		const proc = this.proc;
		setTimeout(() => {
			if (proc.exitCode === null && proc.signalCode === null) proc.kill("SIGKILL");
		}, 3000).unref?.();
	}
}

function describeRpcError(error: unknown): string {
	if (error && typeof error === "object") {
		const message = (error as { message?: unknown }).message;
		if (typeof message === "string" && message) return message;
	}
	if (typeof error === "string" && error) return error;
	return `RPC error: ${JSON.stringify(error)}`;
}

// ============================================================================
// Paused-session registry (process-wide, keyed by ResumeToken)
// ============================================================================

interface PausedSession {
	client: BrowsertoolsServeClient;
	/** Number of NeedsParent rounds yielded so far for this flow. */
	rounds: number;
	idleTimer: ReturnType<typeof setTimeout>;
	idleTimeoutMs: number;
}

const sessionRegistry = new Map<ResumeToken, PausedSession>();

/**
 * Park a paused flow's live serve client under its resume token and arm the idle
 * reaper. Any previous entry for the token is disposed first.
 */
export function parkSession(
	token: ResumeToken,
	client: BrowsertoolsServeClient,
	rounds: number,
	idleTimeoutMs: number,
): void {
	const existing = sessionRegistry.get(token);
	if (existing && existing.client !== client) {
		clearTimeout(existing.idleTimer);
		existing.client.dispose();
	}
	const idleTimer = setTimeout(() => {
		const session = sessionRegistry.get(token);
		if (session) {
			sessionRegistry.delete(token);
			session.client.dispose();
		}
	}, idleTimeoutMs);
	idleTimer.unref?.();
	sessionRegistry.set(token, { client, rounds, idleTimer, idleTimeoutMs });
}

/**
 * Remove a paused session from the registry and return it, stopping its idle
 * timer. The caller now owns the client (to resume or dispose). Returns
 * undefined if the token is unknown (expired/reaped/already completed).
 */
export function takeSession(token: ResumeToken): PausedSession | undefined {
	const session = sessionRegistry.get(token);
	if (!session) return undefined;
	clearTimeout(session.idleTimer);
	sessionRegistry.delete(token);
	return session;
}

/** Dispose every parked session. Used on shutdown and in tests. */
export function disposeAllSessions(): void {
	for (const [, session] of sessionRegistry) {
		clearTimeout(session.idleTimer);
		session.client.dispose();
	}
	sessionRegistry.clear();
}

/** Number of currently parked sessions (for tests/diagnostics). */
export function pausedSessionCount(): number {
	return sessionRegistry.size;
}
