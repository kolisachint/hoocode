import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { getDispatchTaskDir } from "../config.js";
import { waitForChildProcess } from "../utils/child-process.js";
import { MODEL_INHERIT } from "./agent-frontmatter.js";
import { type AgentRegistry, loadAgentRegistry } from "./agent-registry.js";
import { DispatchEvaluator } from "./dispatch-evaluator.js";
import { SubagentLifeguard } from "./lifeguard.js";
import { OutputVerifier } from "./output-verifier.js";
import { TokenBudget } from "./token-budget.js";

export interface SubagentPoolTask {
	task_id: string;
	agent_type: string;
	task: string;
	context?: string;
	token_budget?: number;
	cwd?: string;
	model?: string;
	provider?: string;
	/**
	 * Explicit session file for the child to persist/continue. When omitted the
	 * child uses its own dispatch dir (`<dispatch>/<task_id>/session.jsonl`).
	 * Resume reuses the original task's session file to continue the transcript.
	 */
	sessionFile?: string;
}

export interface SubagentSlot {
	pid: number;
	agent_type: string;
	task_id: string;
	spawned_at: number;
	token_budget: number;
	process: ReturnType<typeof spawn>;
}

export interface SubagentResult {
	task_id: string;
	ok: boolean;
	stdout: string;
	stderr: string;
	exit_code: number | null;
	error?: string;
	/** True when the task exceeded its token budget and was hard-stopped. */
	budget_exceeded?: boolean;
	/** Terminal status derived from how the task finished. */
	status?: "complete" | "partial" | "failed" | "stalled" | "timeout";
	/** Parsed result.json content when available (e.g. on partial completion). */
	result_data?: Record<string, unknown>;
}

export interface TaskResult {
	/** True when the evaluator decided the task is simple enough for inline handling. */
	handled_inline: boolean;
	/** Present when the task was delegated. */
	task_id?: string;
	agent_type?: string;
	reason?: string;
	/** Subagent result when delegated. */
	result?: SubagentResult;
	/** Duration in milliseconds when delegated. */
	duration?: number;
}

export interface DispatchOptions {
	/** Skip evaluation and force this agent type (user/explicit override).
	 *  Accepts any registry-defined agent name, not just the built-in modes. */
	forceAgent?: string;
	/** Context distilled from the calling agent, passed to the subagent. */
	context?: string;
	/** Model id for the subagent (defaults to the child's configured default). */
	model?: string;
	/** Provider for the subagent. */
	provider?: string;
	/** Explicit session file to persist/continue (used by resume). */
	sessionFile?: string;
}

export interface SubagentPoolOptions {
	/** Path to the hoocode executable (or the runtime, e.g. node, when prefixArgs is set). */
	executable: string;
	/** Args inserted before task args (e.g. the CLI entry script for node/tsx). */
	prefixArgs?: string[];
	/** Maximum concurrent child processes. Defaults to 5. */
	maxConcurrency?: number;
	/** Working directory for spawned processes. Defaults to process.cwd(). */
	cwd?: string;
	/** Environment variables. Defaults to process.env. */
	env?: NodeJS.ProcessEnv;
	/** Default token budget per task. Defaults to 0. */
	defaultTokenBudget?: number;
	/**
	 * Non-default skill paths to forward to every spawned subagent via --skill.
	 * Subagents auto-discover skills from standard locations; only paths that
	 * won't be found by default discovery need to be forwarded here.
	 */
	skillPaths?: string[];
}

/**
 * Default hard cap on assistant turns for a spawned subagent when its definition
 * does not set `maxTurns`. The token budget is advisory (it warns but never
 * kills), so this turn cap is the guaranteed hard stop for every subagent.
 */
export const DEFAULT_SUBAGENT_MAX_TURNS = 50;

/**
 * Pool for running hoocode subagents as child processes with bounded concurrency,
 * FIFO queuing with priority support, and automatic slot refill.
 *
 * Events:
 * - "task_done"    – task completed successfully and output was verified
 * - "task_failed"  – task failed (spawn error, bad exit code, verification failure)
 * - "task_stalled" – heartbeat missed for 60s, process was SIGKILLed
 * - "task_timeout" – hard timeout exceeded, process was SIGKILLed
 * - "budget_warning" – token usage crossed 80% threshold (advisory)
 * - "budget_exceeded" – token usage crossed 100% threshold (advisory; never kills)
 */
export class SubagentPool extends EventEmitter {
	private readonly maxConcurrency: number;
	private readonly executable: string;
	private readonly prefixArgs: string[];
	private readonly cwd: string;
	private readonly env: NodeJS.ProcessEnv;
	private readonly defaultTokenBudget: number;
	/** Non-default skill paths forwarded to every spawned subagent via --skill. */
	private skillPaths: string[];

	private slots = new Map<string, SubagentSlot>();
	private queue: SubagentPoolTask[] = [];
	private completed = new Map<string, SubagentResult>();
	private waiters = new Map<string, { resolve: (result: SubagentResult) => void; reject: (err: Error) => void }>();
	private budgets = new Map<string, TokenBudget>();
	private verifier = new OutputVerifier();
	private lifeguard: SubagentLifeguard;
	private disposed = false;
	/** Lazily-loaded agent registry (frontmatter definitions) for this pool's cwd. */
	private registry?: AgentRegistry;
	/** Tracks why a task was killed (stalled / timeout) before exit handler fires. */
	private killReasons = new Map<string, "stalled" | "timeout">();
	/** Persistent terminal status map, survives wait_for consumption. */
	private taskStatus = new Map<string, "done" | "failed" | "stalled" | "timeout">();

	constructor(options: SubagentPoolOptions) {
		super();
		this.maxConcurrency = options.maxConcurrency ?? 5;
		this.executable = options.executable;
		this.prefixArgs = options.prefixArgs ?? [];
		this.cwd = options.cwd ?? process.cwd();
		this.env = options.env ?? process.env;
		this.defaultTokenBudget = options.defaultTokenBudget ?? 0;
		this.skillPaths = options.skillPaths ? [...options.skillPaths] : [];
		this.verifier = new OutputVerifier(this.cwd);
		this.lifeguard = new SubagentLifeguard(this.cwd);
		this.lifeguard.on("stalled", (data: { task_id: string; pid: number }) => {
			this.killReasons.set(data.task_id, "stalled");
			this.emit("task_stalled", data);
		});
		this.lifeguard.on("timeout", (data: { task_id: string; pid: number }) => {
			this.killReasons.set(data.task_id, "timeout");
			this.emit("task_timeout", data);
		});
	}

	/** Update the non-default skill paths forwarded to new subagents. */
	updateSkillPaths(paths: string[]): void {
		this.skillPaths = [...paths];
	}

	/** Lazily load the agent registry for this pool's cwd. */
	private getRegistry(): AgentRegistry {
		if (!this.registry) {
			this.registry = loadAgentRegistry({ cwd: this.cwd });
		}
		return this.registry;
	}

	/** Priority value: higher numbers run first. */
	private priorityOf(agent_type: string): number {
		switch (agent_type) {
			case "explore":
			case "review":
				return 2;
			case "doc":
				return 0;
			default:
				return 1;
		}
	}

	/** Queue a task. It will run when a slot is free. */
	spawn(task: SubagentPoolTask): void {
		if (this.disposed) {
			throw new Error("SubagentPool has been disposed");
		}
		if (
			this.slots.has(task.task_id) ||
			this.queue.some((t) => t.task_id === task.task_id) ||
			this.completed.has(task.task_id)
		) {
			throw new Error(`Duplicate task_id: ${task.task_id}`);
		}

		const p = this.priorityOf(task.agent_type);
		const idx = this.queue.findIndex((t) => this.priorityOf(t.agent_type) < p);
		if (idx === -1) {
			this.queue.push(task);
		} else {
			this.queue.splice(idx, 0, task);
		}
		this.pull();
	}

	/** Current status of a task. */
	get_status(task_id: string): "running" | "queued" | "done" | "failed" | "stalled" | "timeout" {
		if (this.slots.has(task_id)) return "running";
		if (this.queue.some((t) => t.task_id === task_id)) return "queued";
		const persisted = this.taskStatus.get(task_id);
		if (persisted) return persisted;
		const result = this.completed.get(task_id);
		if (result) {
			if (result.status === "stalled") return "stalled";
			if (result.status === "timeout") return "timeout";
			if (result.ok) return "done";
			return "failed";
		}
		return "failed";
	}

	/** Wait for a task to complete and return its result. */
	wait_for(task_id: string): Promise<SubagentResult> {
		if (this.disposed) {
			return Promise.reject(new Error("SubagentPool has been disposed"));
		}

		const existing = this.completed.get(task_id);
		if (existing) {
			this.completed.delete(task_id);
			return Promise.resolve(existing);
		}

		return new Promise((resolve, reject) => {
			this.waiters.set(task_id, { resolve, reject });
		});
	}

	/** Number of currently running subagents. */
	running_count(): number {
		return this.slots.size;
	}

	/** Number of tasks waiting in the queue. */
	queued_count(): number {
		return this.queue.length;
	}

	/**
	 * Dispatch a task through the evaluator.
	 *
	 * - If `options.forceAgent` is provided, skip evaluation and spawn directly.
	 * - Otherwise evaluate the task. If it should be handled inline, return
	 *   `{ handled_inline: true }` immediately.
	 * - If delegating, spawn the subagent, wait for completion, write
	 *   `output.json`, and return the result.
	 */
	async dispatch(task: string, options: DispatchOptions = {}): Promise<TaskResult> {
		if (this.disposed) {
			return Promise.reject(new Error("SubagentPool has been disposed"));
		}
		const begin = this.beginDispatch(task, options);
		if (begin.handled_inline) {
			return { handled_inline: true, reason: begin.reason };
		}
		const result = await this.wait_for(begin.task_id);
		return {
			handled_inline: false,
			task_id: begin.task_id,
			agent_type: begin.agent_type,
			reason: begin.reason,
			result,
			duration: Date.now() - begin.startTime,
		};
	}

	/**
	 * Fire-and-forget dispatch for background agents. Spawns the subagent and
	 * returns its handle immediately; the caller polls get_status()/collect().
	 */
	dispatchDetached(
		task: string,
		options: DispatchOptions = {},
	): { handled_inline: boolean; task_id?: string; agent_type?: string; reason?: string } {
		if (this.disposed) {
			throw new Error("SubagentPool has been disposed");
		}
		const begin = this.beginDispatch(task, options);
		if (begin.handled_inline) {
			return { handled_inline: true, reason: begin.reason };
		}
		return { handled_inline: false, task_id: begin.task_id, agent_type: begin.agent_type, reason: begin.reason };
	}

	/**
	 * Evaluate, log, and spawn a task without waiting. Shared by dispatch()
	 * (blocking) and dispatchDetached() (background).
	 */
	private beginDispatch(
		task: string,
		options: DispatchOptions,
	):
		| { handled_inline: true; reason?: string }
		| { handled_inline: false; task_id: string; agent_type: string; reason?: string; startTime: number } {
		const { forceAgent, context, model, provider, sessionFile } = options;
		const evaluator = new DispatchEvaluator();
		const analysis = evaluator.evaluate(task);

		if (!forceAgent && !analysis.should_delegate) {
			return { handled_inline: true, reason: analysis.reason };
		}

		const agent_type = forceAgent ?? "general-purpose";
		const task_id = `dispatch-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
		const reason = forceAgent ? "user_override" : analysis.reason;
		const complexity = analysis.estimated_complexity;

		// Pre-dispatch logging. Use stderr: stdout is reserved for the JSON event
		// stream / TUI render and must not be polluted.
		console.error(`[DISPATCH] agent=${agent_type} reason=${reason} complexity=${complexity} task_id=${task_id}`);
		this.writeDispatchLog(task_id, agent_type, reason, complexity, task);

		const poolTask: SubagentPoolTask = {
			task_id,
			agent_type,
			task,
			context,
			model,
			provider,
			sessionFile,
			cwd: this.cwd,
		};
		const startTime = Date.now();
		this.spawn(poolTask);
		return { handled_inline: false, task_id, agent_type, reason, startTime };
	}

	/**
	 * Non-destructively read a completed task's result (for background polling).
	 * Returns undefined while the task is still running/queued, or if its result
	 * was already consumed via wait_for().
	 */
	collect(task_id: string): SubagentResult | undefined {
		return this.completed.get(task_id);
	}

	/** Absolute path of the persisted session file for a task. */
	getSessionFile(task_id: string, cwd: string = this.cwd): string {
		return join(getDispatchTaskDir(cwd, task_id), "session.jsonl");
	}

	/**
	 * Resume a previously dispatched subagent, continuing its persisted session
	 * with a follow-up prompt. Recovers the original agent type from its dispatch
	 * log. Rejects if no resumable session exists for the task.
	 */
	async resume(
		task_id: string,
		prompt: string,
		options: Omit<DispatchOptions, "forceAgent" | "sessionFile"> = {},
	): Promise<TaskResult> {
		if (this.disposed) {
			return Promise.reject(new Error("SubagentPool has been disposed"));
		}
		const sessionFile = this.getSessionFile(task_id);
		if (!existsSync(sessionFile)) {
			return Promise.reject(new Error(`No resumable session for task "${task_id}" (expected ${sessionFile}).`));
		}
		const agent_type = this.readDispatchAgentType(task_id) ?? "general-purpose";
		return this.dispatch(prompt, { ...options, forceAgent: agent_type, sessionFile });
	}

	/** Recover the agent type a task was dispatched with, from its dispatch log. */
	private readDispatchAgentType(task_id: string): string | undefined {
		const path = join(getDispatchTaskDir(this.cwd, task_id), "dispatch-log.json");
		if (!existsSync(path)) return undefined;
		try {
			const parsed = JSON.parse(readFileSync(path, "utf-8")) as { agent_type?: string };
			return typeof parsed.agent_type === "string" ? parsed.agent_type : undefined;
		} catch {
			return undefined;
		}
	}

	private writeDispatchLog(
		task_id: string,
		agent_type: string,
		reason: string,
		complexity: string,
		task: string,
	): void {
		const log = {
			timestamp: new Date().toISOString(),
			task_id,
			agent_type,
			reason,
			complexity,
			task,
		};
		const path = join(getDispatchTaskDir(this.cwd, task_id), "dispatch-log.json");
		try {
			mkdirSync(dirname(path), { recursive: true });
			writeFileSync(path, JSON.stringify(log, null, 2));
		} catch {
			// Best-effort persistence
		}
	}

	private writeOutputJson(task_id: string, result: SubagentResult): void {
		const output = {
			task_id: result.task_id,
			ok: result.ok,
			exit_code: result.exit_code,
			status: result.status,
			stdout: result.stdout,
			stderr: result.stderr,
			error: result.error,
			budget_exceeded: result.budget_exceeded,
			result_data: result.result_data,
		};
		const path = join(getDispatchTaskDir(this.cwd, task_id), "output.json");
		try {
			mkdirSync(dirname(path), { recursive: true });
			writeFileSync(path, JSON.stringify(output, null, 2));
		} catch {
			// Best-effort persistence
		}
	}

	/**
	 * Remove a task's dispatch dir after a clean, verified success. Best-effort:
	 * a cleanup failure must never fail an otherwise successful task.
	 */
	private cleanupDispatchDir(task_id: string, cwd: string): void {
		try {
			rmSync(getDispatchTaskDir(cwd, task_id), { recursive: true, force: true });
		} catch {
			// Best-effort cleanup
		}
	}

	/** Kill all running processes, clear the queue, and reject pending waiters. */
	dispose(): void {
		if (this.disposed) return;
		this.disposed = true;

		for (const slot of this.slots.values()) {
			if (!slot.process.killed) {
				slot.process.kill("SIGTERM");
			}
		}
		this.slots.clear();
		this.queue = [];

		for (const [task_id, waiter] of this.waiters) {
			waiter.reject(new Error("SubagentPool disposed"));
			this.waiters.delete(task_id);
		}
		this.completed.clear();
		for (const budget of this.budgets.values()) {
			budget.removeAllListeners();
		}
		this.budgets.clear();
		this.killReasons.clear();
		this.taskStatus.clear();
		this.lifeguard.dispose();
		this.removeAllListeners();
	}

	/** Pull tasks from the queue while slots are available. */
	private pull(): void {
		while (this.slots.size < this.maxConcurrency && this.queue.length > 0) {
			const task = this.queue.shift()!;
			this.startTask(task, false);
		}
	}

	/** Build CLI arguments for a task. */
	private buildArgs(task: SubagentPoolTask): string[] {
		// Persist the child's session so a finished/interrupted subagent can be
		// resumed later (see resume()). SessionManager.open() creates the file on
		// first run and continues it on subsequent runs.
		const sessionFile = task.sessionFile ?? this.getSessionFile(task.task_id, task.cwd ?? this.cwd);
		const args: string[] = [
			...this.prefixArgs,
			"--mode",
			"json",
			"--session",
			sessionFile,
			"--task-id",
			task.task_id,
		];

		// Prefer the data-driven agent definition from the registry; fall back to the
		// built-in mode prompt/allowlist for legacy modes not present in the registry.
		const def = task.agent_type ? this.getRegistry().get(task.agent_type) : undefined;

		if (task.agent_type) {
			const systemPrompt = def?.prompt;
			if (systemPrompt) {
				args.push("--system-prompt", systemPrompt);
			}
			// Tool allowlist comes from the agent definition's frontmatter `tools`
			// field (read-only built-ins declare their own sandbox). When omitted, no
			// --tools is passed and the subagent inherits all parent tools.
			const tools = def?.tools;
			if (tools && tools.length > 0) {
				args.push("--tools", tools.join(","));
			}
		}

		// Model precedence: a definition's explicit model wins (unless it is the
		// `inherit` sentinel), otherwise use the caller-provided model.
		const explicitModel = def?.model && def.model !== MODEL_INHERIT ? def.model : undefined;
		const modelToUse = explicitModel ?? task.model;
		if (modelToUse) {
			args.push("--model", modelToUse);
		}
		if (task.provider) {
			args.push("--provider", task.provider);
		}

		// Always give subagents a hard turn cap. With the token budget now advisory
		// (warn-only), this is the guaranteed hard stop for a runaway subagent.
		const maxTurns = def?.maxTurns && def.maxTurns > 0 ? def.maxTurns : DEFAULT_SUBAGENT_MAX_TURNS;
		args.push("--max-turns", String(maxTurns));

		// Forward non-default skill paths so the subagent has access to all parent skills.
		// Standard discovery locations (~/.hoocode/, .hoocode/, .claude/) are found automatically.
		for (const skillPath of this.skillPaths) {
			args.push("--skill", skillPath);
		}

		const prompt = task.context?.trim()
			? `Context from the calling agent:\n\n${task.context.trim()}\n\nTask: ${task.task.trim()}`
			: `Task: ${task.task.trim()}`;
		args.push(prompt);

		return args;
	}

	/** Start a task in a child process, with one retry on failure. */
	private startTask(task: SubagentPoolTask, isRetry: boolean): void {
		// Get or create a TokenBudget tracker. On retry, reuse the existing one
		// so cumulative usage persists across retries.
		let budget = this.budgets.get(task.task_id);
		if (!budget) {
			budget = new TokenBudget(task.task_id, task.agent_type, {
				limit: task.token_budget,
				cwd: task.cwd ?? this.cwd,
			});
			budget.on("budget_warning", (data: { task_id: string; message: string; used: number; limit: number }) => {
				this.emit("budget_warning", data);
			});
			// The token budget is advisory: surface telemetry but never kill. The
			// guaranteed hard stop is the per-subagent turn cap (--max-turns); see
			// DEFAULT_SUBAGENT_MAX_TURNS.
			budget.on("budget_exceeded", (data: { task_id: string; used: number; limit: number }) => {
				this.emit("budget_exceeded", data);
			});
			this.budgets.set(task.task_id, budget);
		}

		let proc: ReturnType<typeof spawn>;
		try {
			proc = spawn(this.executable, this.buildArgs(task), {
				cwd: task.cwd ?? this.cwd,
				// Mark the child as a subagent so its own DispatchEvaluator refuses to
				// spawn further subagents (depth guard).
				env: { ...this.env, HOOCODE_SUBAGENT_DEPTH: "1" },
				shell: false,
				stdio: ["ignore", "pipe", "pipe"],
			});
		} catch {
			if (!isRetry) {
				this.startTask(task, true);
			} else {
				this.emit("task_failed", {
					task_id: task.task_id,
					error: "Spawn failed synchronously",
				});
				this.resolveWaiter(task.task_id, {
					task_id: task.task_id,
					ok: false,
					stdout: "",
					stderr: "",
					exit_code: null,
					error: "Spawn failed synchronously",
					status: "failed",
				});
				this.pull();
			}
			return;
		}

		const slot: SubagentSlot = {
			pid: proc.pid ?? 0,
			agent_type: task.agent_type,
			task_id: task.task_id,
			spawned_at: Date.now(),
			token_budget: task.token_budget ?? this.defaultTokenBudget,
			process: proc,
		};

		this.slots.set(task.task_id, slot);
		this.lifeguard.monitor(task.task_id, task.agent_type, proc);

		let stdout = "";
		let stderr = "";

		proc.stdout?.on("data", (data: Buffer) => {
			const chunk = data.toString();
			stdout += chunk;
			budget.processStdout(chunk);

			// Heartbeat detection: look for {"ping":true} JSON lines
			for (const raw of chunk.split("\n")) {
				const line = raw.trim();
				if (!line.startsWith("{")) continue;
				try {
					const parsed = JSON.parse(line) as Record<string, unknown>;
					if (parsed.ping === true) {
						this.lifeguard.recordHeartbeat(task.task_id);
					}
				} catch {
					// Not a ping line, ignore
				}
			}
		});
		proc.stderr?.on("data", (data: Buffer) => {
			stderr += data.toString();
		});

		waitForChildProcess(proc)
			.then((code) => {
				this.slots.delete(task.task_id);
				budget.flush();

				const killReason = this.killReasons.get(task.task_id);
				this.killReasons.delete(task.task_id);

				const duration = Date.now() - slot.spawned_at;
				const tokens_used = budget.getUsed();
				const budgetExceeded = budget.isExceeded();

				// If killed by lifeguard, override exit handling
				if (killReason === "stalled" || killReason === "timeout") {
					const result: SubagentResult = {
						task_id: task.task_id,
						ok: false,
						stdout,
						stderr,
						exit_code: code,
						status: killReason,
					};
					this.writeOutputJson(task.task_id, result);
					this.emit(`task_${killReason}`, {
						task_id: task.task_id,
						agent_type: task.agent_type,
						duration,
						tokens_used,
					});
					this.resolveWaiter(task.task_id, result);
					return;
				}

				const result: SubagentResult = {
					task_id: task.task_id,
					ok: code === 0,
					stdout,
					stderr,
					exit_code: code,
					// Advisory telemetry only: exceeding the budget never fails the task.
					budget_exceeded: budgetExceeded,
					status: code === 0 ? "complete" : "failed",
				};

				if (result.ok) {
					const verification = this.verifier.verify(task.task_id, task.cwd ?? this.cwd);
					if (!verification.valid) {
						result.ok = false;
						result.error = verification.reason;
						result.status = "failed";
						this.writeOutputJson(task.task_id, result);
						this.emit("task_failed", {
							task_id: task.task_id,
							agent_type: task.agent_type,
							duration,
							tokens_used,
							error: verification.reason,
						});
						this.resolveWaiter(task.task_id, result);
						return;
					}
					// Attach the verified result.json so callers can read the summary
					// without parsing the raw event stream.
					result.result_data = this.tryReadResultJson(task.task_id, task.cwd ?? this.cwd);

					// Clean success: discard the per-task dispatch dir entirely
					// (session.jsonl, result.json, dispatch-log.json, budget.json). The
					// in-memory result already carries result_data, so callers lose
					// nothing. Trade-off: resume() only works for non-successful tasks.
					this.cleanupDispatchDir(task.task_id, task.cwd ?? this.cwd);

					this.emit("task_done", {
						task_id: task.task_id,
						agent_type: task.agent_type,
						duration,
						tokens_used,
						status: "complete",
					});
					this.resolveWaiter(task.task_id, result);
					return;
				}

				// Failure path: keep the dispatch dir for debugging and persist output.
				this.writeOutputJson(task.task_id, result);
				this.emit("task_failed", {
					task_id: task.task_id,
					agent_type: task.agent_type,
					duration,
					tokens_used,
					error: result.error ?? `Exited with code ${code}`,
				});
				this.resolveWaiter(task.task_id, result);
			})
			.catch((err) => {
				this.slots.delete(task.task_id);
				budget.flush();
				const duration = Date.now() - slot.spawned_at;
				const tokens_used = budget.getUsed();
				if (!isRetry) {
					this.startTask(task, true);
					return;
				}
				const error = err instanceof Error ? err.message : String(err);
				const result: SubagentResult = {
					task_id: task.task_id,
					ok: false,
					stdout,
					stderr,
					exit_code: null,
					error,
					status: "failed",
				};
				this.writeOutputJson(task.task_id, result);
				this.emit("task_failed", {
					task_id: task.task_id,
					agent_type: task.agent_type,
					duration,
					tokens_used,
					error,
				});
				this.resolveWaiter(task.task_id, result);
			})
			.finally(() => {
				budget.removeAllListeners();
				this.budgets.delete(task.task_id);
				this.pull();
			});
	}

	private tryReadResultJson(task_id: string, cwd: string): Record<string, unknown> | undefined {
		const path = join(getDispatchTaskDir(cwd, task_id), "result.json");
		if (!existsSync(path)) return undefined;
		try {
			const raw = readFileSync(path, "utf-8");
			return JSON.parse(raw) as Record<string, unknown>;
		} catch {
			return undefined;
		}
	}

	private resolveWaiter(task_id: string, result: SubagentResult): void {
		// Persist terminal status for get_status() even after wait_for consumes the result
		if (result.status === "stalled") this.taskStatus.set(task_id, "stalled");
		else if (result.status === "timeout") this.taskStatus.set(task_id, "timeout");
		else if (result.ok) this.taskStatus.set(task_id, "done");
		else this.taskStatus.set(task_id, "failed");

		const waiter = this.waiters.get(task_id);
		if (waiter) {
			waiter.resolve(result);
			this.waiters.delete(task_id);
			return;
		}
		this.completed.set(task_id, result);
	}
}
