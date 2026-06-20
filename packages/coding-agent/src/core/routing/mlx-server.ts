/**
 * Local executor server lifecycle manager.
 *
 * When `routing.executor.server` is configured AND local-inference routing is
 * active, the harness can spawn a local OpenAI-compatible server (for example
 * `mlx_lm.server`), wait for it to become healthy, and stop it on shutdown.
 *
 * This is best-effort: if the server fails to start or never becomes healthy,
 * `ensureStarted()` resolves to false and the caller degrades to the primary
 * model (routing falls back). It never throws into the agent loop.
 */

import { type ChildProcess, spawn } from "node:child_process";
import type { ExecutorServerConfig } from "./local-inference.js";
import { logLocalInferenceFallback } from "./metrics.js";

const DEFAULT_COMMAND = "mlx_lm.server";
const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 8080;
const DEFAULT_STARTUP_TIMEOUT_MS = 30_000;
const HEALTH_POLL_INTERVAL_MS = 500;

/** Derive host/port from a baseUrl like "http://127.0.0.1:8080/v1". */
function parseBaseUrl(baseUrl: string | undefined): { host?: string; port?: number } {
	if (!baseUrl) return {};
	try {
		const u = new URL(baseUrl);
		return { host: u.hostname, port: u.port ? Number(u.port) : undefined };
	} catch {
		return {};
	}
}

export interface MlxServerOptions {
	config: ExecutorServerConfig;
	/** Executor model id, passed as --model. */
	modelId: string;
	/** Executor baseUrl, used to derive host/port and health endpoint. */
	baseUrl?: string;
}

export class MlxServerManager {
	private readonly command: string;
	private readonly args: string[];
	private readonly host: string;
	private readonly port: number;
	private readonly startupTimeoutMs: number;
	private readonly modelId: string;
	private child: ChildProcess | undefined;
	private startPromise: Promise<boolean> | undefined;

	constructor(opts: MlxServerOptions) {
		const fromUrl = parseBaseUrl(opts.baseUrl);
		this.command = opts.config.command ?? DEFAULT_COMMAND;
		this.args = opts.config.args ?? [];
		this.host = opts.config.host ?? fromUrl.host ?? DEFAULT_HOST;
		this.port = opts.config.port ?? fromUrl.port ?? DEFAULT_PORT;
		this.startupTimeoutMs = opts.config.startupTimeoutMs ?? DEFAULT_STARTUP_TIMEOUT_MS;
		this.modelId = opts.modelId;
	}

	private healthUrl(): string {
		return `http://${this.host}:${this.port}/v1/models`;
	}

	private async isHealthy(signal?: AbortSignal): Promise<boolean> {
		try {
			const res = await fetch(this.healthUrl(), { method: "GET", signal });
			return res.ok;
		} catch {
			return false;
		}
	}

	/**
	 * Ensure the server is running and healthy. Idempotent: concurrent callers
	 * share one start attempt. Returns false on any failure (caller degrades to
	 * primary). If a server is already healthy (user-started), reuses it without
	 * spawning.
	 */
	async ensureStarted(signal?: AbortSignal): Promise<boolean> {
		if (this.startPromise) return this.startPromise;
		this.startPromise = this.start(signal);
		return this.startPromise;
	}

	private async start(signal?: AbortSignal): Promise<boolean> {
		// Reuse an already-running server (e.g. user-started) without spawning.
		if (await this.isHealthy(signal)) return true;

		try {
			this.child = spawn(
				this.command,
				[...this.args, "--model", this.modelId, "--host", this.host, "--port", String(this.port)],
				{
					stdio: "ignore",
					detached: false,
				},
			);
			this.child.on("error", (error) => {
				logLocalInferenceFallback("primary", error);
				this.child = undefined;
			});
		} catch (error) {
			logLocalInferenceFallback("primary", error);
			this.child = undefined;
			return false;
		}

		const deadline = Date.now() + this.startupTimeoutMs;
		while (Date.now() < deadline) {
			if (signal?.aborted) return false;
			if (!this.child) return false; // spawn errored
			if (await this.isHealthy(signal)) return true;
			await new Promise((r) => setTimeout(r, HEALTH_POLL_INTERVAL_MS));
		}
		// Timed out: stop whatever we spawned and degrade.
		this.stop();
		return false;
	}

	/** Stop the spawned server (no-op if we reused an external one). */
	stop(): void {
		if (this.child && !this.child.killed) {
			this.child.kill("SIGTERM");
		}
		this.child = undefined;
		this.startPromise = undefined;
	}
}
