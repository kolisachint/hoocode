/**
 * Warm subagent worker pool (experimental, opt-in via `--warm-subagents` /
 * settings.warmSubagents; default off).
 *
 * The cold path (SubagentPool) re-execs the whole CLI for every dispatch, paying
 * a full module-load + resource-graph boot each time. A warm worker instead keeps
 * a long-lived child running in RPC mode and hands it one task at a time over the
 * existing JSON-line protocol: `new_session` resets its conversation between
 * tasks, `prompt` runs the task, and `agent_end` on the event stream signals
 * completion, after which the answer + usage are pulled inline (no result.json
 * disk round-trip, no OutputVerifier). The first task per worker still pays the
 * boot; every reuse after that skips it.
 *
 * Scope/limits (deliberately conservative — see the dispatch integration):
 * - Workers are pinned per agent type: RPC has no per-prompt system-prompt/tools
 *   swap, so each (agentType, model, provider) is its own worker config.
 * - Only non-resume, non-fork dispatches are eligible; resume/fork need a
 *   persisted/forked session the warm path does not own.
 * - Any worker/infra failure falls back to the cold pool, so enabling this can
 *   only change latency, never whether a task can run.
 */

import type { AgentEvent } from "@kolisachint/hoocode-agent-core";
import type { Api, Model } from "@kolisachint/hoocode-ai";
import { getSubagentSpawnCommand } from "../config.js";
import { RpcClient } from "../modes/rpc/rpc-client.js";
import { MODEL_INHERIT } from "./agent-frontmatter.js";
import { type AgentRegistry, loadAgentRegistry } from "./agent-registry.js";
import { resolveModelReference } from "./model-categories.js";
import type { Settings } from "./settings-manager.js";
import {
	currentSubagentDepth,
	DEFER_MCP_SCHEMAS_ENV,
	resolveMaxSubagentDepth,
	SUBAGENT_DEPTH_ENV,
	SUBAGENT_SKIP_MCP_ENV,
	toolAllowlistNeedsMcp,
} from "./subagent-depth.js";
import { DEFAULT_SUBAGENT_MAX_TURNS } from "./subagent-pool.js";

/** Usage totals pulled from a worker after a task, shaped like SubagentResultFile.usage. */
export interface WarmUsage {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cost: number;
}

/** Outcome of running one task on a warm worker. */
export interface WarmRunResult {
	ok: boolean;
	status: "complete" | "failed";
	/** The subagent's final assistant text (its answer to the caller). */
	summary: string;
	usage?: WarmUsage;
	error?: string;
}

/** Per-dispatch inputs that select and configure a worker. */
export interface WarmDispatchOptions {
	agentType: string;
	cwd: string;
	/** Model id or category (fast/standard/capable); resolved to a concrete id. */
	model?: string;
	provider?: string;
}

/** Reports the tool a warm worker is currently running ("" = idle between tools). */
export type WarmProgressCallback = (activity: string) => void;

/** A run failed at the infrastructure level (worker crash, timeout, protocol error). */
export class WarmWorkerError extends Error {}

/** Default per-task wait before declaring a warm run stalled and failing over to cold. */
const WARM_RUN_TIMEOUT_MS = 180_000;

/**
 * One long-lived RPC child pinned to a single agent-type configuration. Reused
 * across tasks via reset(); a crash makes it not-alive so the pool discards it.
 */
export class WarmSubagentWorker {
	private readonly client: RpcClient;
	private alive = true;

	constructor(
		readonly key: string,
		options: WarmDispatchOptions,
		private readonly env: NodeJS.ProcessEnv,
		registry: AgentRegistry,
		skillPaths: readonly string[],
		settings: Settings | undefined,
		availableModels: readonly Model<Api>[],
		/** Spawn command override (tests inject a fake RPC child); defaults to the real spawn command. */
		spawnCommand?: { executable: string; prefixArgs: string[] },
	) {
		const { executable, prefixArgs } = spawnCommand ?? getSubagentSpawnCommand();
		this.client = new RpcClient({
			executable,
			prefixArgs,
			cwd: options.cwd,
			env: this.env as Record<string, string>,
			args: buildWorkerArgs(options, registry, skillPaths, settings, availableModels),
		});
	}

	/** Boot the child. Throws (as WarmWorkerError) if it fails to come up. */
	async start(): Promise<void> {
		try {
			await this.client.start();
		} catch (error) {
			this.alive = false;
			throw new WarmWorkerError(error instanceof Error ? error.message : String(error));
		}
	}

	isAlive(): boolean {
		return this.alive;
	}

	/**
	 * Run one task to completion and return its answer + usage. Throws
	 * WarmWorkerError on an infra failure (crash/timeout/protocol) so the caller
	 * can fall back to the cold pool; a task that ran but reported failure returns
	 * `{ ok: false }` instead (no fall back — the work was actually done).
	 */
	async run(
		prompt: string,
		onActivity?: WarmProgressCallback,
		timeoutMs = WARM_RUN_TIMEOUT_MS,
	): Promise<WarmRunResult> {
		if (!this.alive) throw new WarmWorkerError("worker is not alive");
		// Mirror the cold pool's coarse progress: report the tool the child is
		// currently running so a warm dispatch's task row reads "⋯ grep" rather than a
		// static "running…". Cleared between tools and at turn end.
		const detachProgress = onActivity ? this.tapProgress(onActivity) : undefined;
		try {
			const events = await this.client.promptAndWait(prompt, undefined, timeoutMs);
			const failure = firstTurnError(events);
			const summary = (await this.client.getLastAssistantText()) ?? "";
			const usage = await this.readUsage();
			if (failure) {
				return { ok: false, status: "failed", summary, usage, error: failure };
			}
			return { ok: true, status: "complete", summary, usage };
		} catch (error) {
			// A prompt/await failure means the child is no longer trustworthy: mark it
			// dead so the pool discards it, and signal infra failure for fallback.
			this.alive = false;
			throw new WarmWorkerError(error instanceof Error ? error.message : String(error));
		} finally {
			detachProgress?.();
			onActivity?.("");
		}
	}

	/** Forward the child's coarse tool-lifecycle events to an activity callback. */
	private tapProgress(onActivity: WarmProgressCallback): () => void {
		return this.client.onEvent((event) => {
			const e = event as { type?: string; toolName?: string };
			if (e.type === "tool_execution_start") onActivity(typeof e.toolName === "string" ? e.toolName : "");
			else if (e.type === "tool_execution_end" || e.type === "turn_end") onActivity("");
		});
	}

	/** Reset the worker's conversation so it can take the next task cleanly. */
	async reset(): Promise<void> {
		if (!this.alive) return;
		try {
			await this.client.newSession();
		} catch (error) {
			// A reset failure leaves the worker in an unknown state — retire it.
			this.alive = false;
			throw new WarmWorkerError(error instanceof Error ? error.message : String(error));
		}
	}

	async dispose(): Promise<void> {
		this.alive = false;
		await this.client.stop().catch(() => {});
	}

	private async readUsage(): Promise<WarmUsage | undefined> {
		try {
			const stats = await this.client.getSessionStats();
			return {
				input: stats.tokens.input,
				output: stats.tokens.output,
				cacheRead: stats.tokens.cacheRead,
				cacheWrite: stats.tokens.cacheWrite,
				cost: stats.cost,
			};
		} catch {
			return undefined;
		}
	}
}

/**
 * Pool of warm workers keyed by agent-type configuration. Hands out an idle
 * worker (or boots a new one up to a per-key cap), and on release resets the
 * worker and returns it to the idle set with an idle-TTL reclaim timer.
 */
export class WarmSubagentPool {
	private idle = new Map<string, WarmSubagentWorker[]>();
	private reclaimTimers = new Map<WarmSubagentWorker, ReturnType<typeof setTimeout>>();
	private liveCount = new Map<string, number>();
	private disposed = false;
	private registry?: AgentRegistry;

	constructor(
		private readonly cwd: string,
		private readonly settings: Settings | undefined,
		private skillPaths: string[] = [],
		/**
		 * Available models used to derive default model-category mappings when a tier
		 * is not explicitly set in `settings.modelCategories` (snapshot at creation).
		 */
		private readonly availableModels: readonly Model<Api>[] = [],
		private readonly maxPerKey = 2,
		private readonly idleTtlMs = 30_000,
		/** Spawn command override (tests inject a fake RPC child); defaults to the real spawn command. */
		private readonly spawnCommand?: { executable: string; prefixArgs: string[] },
	) {}

	updateSkillPaths(paths: string[]): void {
		this.skillPaths = [...paths];
	}

	private getRegistry(): AgentRegistry {
		if (!this.registry) this.registry = loadAgentRegistry({ cwd: this.cwd });
		return this.registry;
	}

	/** A registry agent is poolable when it is not a fork agent (fork needs a forked session). */
	isPoolable(agentType: string): boolean {
		const def = this.getRegistry().get(agentType);
		return def !== undefined && def.fork !== true;
	}

	/** Stable key for one worker configuration. */
	private keyFor(options: WarmDispatchOptions): string {
		const resolved = options.model
			? resolveModelReference(options.model, this.settings, this.availableModels)
			: undefined;
		return `${options.agentType}::${resolved ?? "default"}::${options.provider ?? "default"}`;
	}

	/** Environment for a warm child: depth stamp + MCP skip, mirroring SubagentPool.childSpawnEnv. */
	private childEnv(agentType: string): NodeJS.ProcessEnv {
		const env: NodeJS.ProcessEnv = {
			...process.env,
			[SUBAGENT_DEPTH_ENV]: String(currentSubagentDepth(process.env) + 1),
		};
		const def = this.getRegistry().get(agentType);
		if (!toolAllowlistNeedsMcp(def?.tools)) env[SUBAGENT_SKIP_MCP_ENV] = "1";
		// A child never defers MCP schemas: if it needs MCP it eager-registers its
		// allowlisted tools at dispatch so they are immediately callable (spec §2).
		delete env[DEFER_MCP_SCHEMAS_ENV];
		return env;
	}

	/**
	 * Run a task on a warm worker end to end: acquire (reuse or boot), run, then
	 * release back to the pool. Throws WarmWorkerError on infra failure so the
	 * caller can fall back to the cold pool.
	 */
	async dispatch(
		prompt: string,
		options: WarmDispatchOptions,
		onActivity?: WarmProgressCallback,
	): Promise<WarmRunResult> {
		if (this.disposed) throw new WarmWorkerError("warm pool disposed");
		const worker = await this.acquire(options);
		try {
			const result = await worker.run(prompt, onActivity);
			await this.release(worker);
			return result;
		} catch (error) {
			// Infra failure: the worker already marked itself dead; drop it entirely.
			await this.discard(worker);
			throw error instanceof WarmWorkerError ? error : new WarmWorkerError(String(error));
		}
	}

	private async acquire(options: WarmDispatchOptions): Promise<WarmSubagentWorker> {
		const key = this.keyFor(options);
		const pool = this.idle.get(key);
		while (pool && pool.length > 0) {
			const worker = pool.pop()!;
			this.clearReclaim(worker);
			if (worker.isAlive()) return worker;
			// Dead idle worker (child exited while parked): drop and try the next.
			this.decLive(key);
			await worker.dispose();
		}

		const worker = new WarmSubagentWorker(
			key,
			options,
			this.childEnv(options.agentType),
			this.getRegistry(),
			this.skillPaths,
			this.settings,
			this.availableModels,
			this.spawnCommand,
		);
		this.incLive(key);
		try {
			await worker.start();
		} catch (error) {
			this.decLive(key);
			throw error;
		}
		return worker;
	}

	private async release(worker: WarmSubagentWorker): Promise<void> {
		if (this.disposed || !worker.isAlive()) {
			await this.discard(worker);
			return;
		}
		try {
			await worker.reset();
		} catch {
			await this.discard(worker);
			return;
		}
		const key = worker.key;
		const pool = this.idle.get(key) ?? [];
		if (pool.length >= this.maxPerKey) {
			await this.discard(worker);
			return;
		}
		pool.push(worker);
		this.idle.set(key, pool);
		this.armReclaim(worker);
	}

	private async discard(worker: WarmSubagentWorker): Promise<void> {
		this.clearReclaim(worker);
		this.decLive(worker.key);
		await worker.dispose();
	}

	private armReclaim(worker: WarmSubagentWorker): void {
		const timer = setTimeout(() => {
			const pool = this.idle.get(worker.key);
			if (pool) {
				const idx = pool.indexOf(worker);
				if (idx !== -1) pool.splice(idx, 1);
			}
			void this.discard(worker);
		}, this.idleTtlMs);
		timer.unref?.();
		this.reclaimTimers.set(worker, timer);
	}

	private clearReclaim(worker: WarmSubagentWorker): void {
		const timer = this.reclaimTimers.get(worker);
		if (timer) {
			clearTimeout(timer);
			this.reclaimTimers.delete(worker);
		}
	}

	private incLive(key: string): void {
		this.liveCount.set(key, (this.liveCount.get(key) ?? 0) + 1);
	}

	private decLive(key: string): void {
		const n = (this.liveCount.get(key) ?? 1) - 1;
		if (n <= 0) this.liveCount.delete(key);
		else this.liveCount.set(key, n);
	}

	/** Number of currently idle (parked) workers — exposed for tests/diagnostics. */
	idleCount(): number {
		let total = 0;
		for (const pool of this.idle.values()) total += pool.length;
		return total;
	}

	async dispose(): Promise<void> {
		if (this.disposed) return;
		this.disposed = true;
		const all: WarmSubagentWorker[] = [];
		for (const pool of this.idle.values()) all.push(...pool);
		this.idle.clear();
		for (const timer of this.reclaimTimers.values()) clearTimeout(timer);
		this.reclaimTimers.clear();
		this.liveCount.clear();
		await Promise.all(all.map((w) => w.dispose()));
	}
}

/**
 * Build the RPC child's CLI args for a warm worker. Mirrors the agent-config
 * portion of SubagentPool.buildArgs (system prompt, tools, model/provider, turn
 * cap, skills) but omits the one-shot `--mode json` / `--session` / `--task-id`
 * bits, since the worker runs persistently in RPC mode and resets via new_session.
 */
function buildWorkerArgs(
	options: WarmDispatchOptions,
	registry: AgentRegistry,
	skillPaths: readonly string[],
	settings: Settings | undefined,
	availableModels: readonly Model<Api>[],
): string[] {
	const args: string[] = [];
	const def = registry.get(options.agentType);

	if (def?.prompt) args.push("--system-prompt", def.prompt);

	const childDepth = currentSubagentDepth(process.env) + 1;
	const canChildDelegate = def?.delegate === true && childDepth < resolveMaxSubagentDepth(undefined, process.env);

	const tools = def?.tools ? [...def.tools] : undefined;
	if (canChildDelegate && tools) {
		for (const t of ["Task", "TaskOutput"]) if (!tools.includes(t)) tools.push(t);
	}
	if (tools && tools.length > 0) args.push("--tools", tools.join(","));
	if (def?.disallowedTools && def.disallowedTools.length > 0) {
		args.push("--disallowed-tools", def.disallowedTools.join(","));
	}
	if (canChildDelegate) {
		args.push("--enable-subagents");
		if (def?.delegateTo && def.delegateTo.length > 0) args.push("--delegate-allow", def.delegateTo.join(","));
	}

	// Model precedence matches the cold path: a pinned (non-inherit) agent model
	// wins, else the requested model/category, resolved to a concrete id.
	const explicitModel = def?.model && def.model !== MODEL_INHERIT ? def.model : undefined;
	const rawModel = explicitModel ?? options.model;
	const modelToUse = rawModel ? resolveModelReference(rawModel, settings, availableModels) : undefined;
	if (modelToUse) args.push("--model", modelToUse);
	if (options.provider) args.push("--provider", options.provider);

	const maxTurns = def?.maxTurns && def.maxTurns > 0 ? def.maxTurns : DEFAULT_SUBAGENT_MAX_TURNS;
	args.push("--max-turns", String(maxTurns));

	for (const skillPath of skillPaths) args.push("--skill", skillPath);

	return args;
}

/**
 * Scan a completed run's events for a turn that ended in error/abort, returning
 * its message (or a generic marker) so the caller can report a task failure
 * without re-reading the transcript. Returns undefined for a clean run.
 */
function firstTurnError(events: readonly AgentEvent[]): string | undefined {
	for (const event of events) {
		const e = event as { type?: string; message?: { stopReason?: string; errorMessage?: string } };
		if (e.type === "turn_end" && (e.message?.stopReason === "error" || e.message?.stopReason === "aborted")) {
			return e.message.errorMessage || `turn ${e.message.stopReason}`;
		}
	}
	return undefined;
}
