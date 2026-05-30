import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CONFIG_DIR_NAME } from "../config.js";
import { waitForChildProcess } from "../utils/child-process.js";
import { SubagentLifeguard } from "./lifeguard.js";
import { OutputVerifier } from "./output-verifier.js";
import { getSubagentSystemPrompt, type SubagentMode } from "./subagent.js";
import { TokenBudget } from "./token-budget.js";

export interface SubagentPoolTask {
	task_id: string;
	agent_type: SubagentMode | string;
	task: string;
	context?: string;
	token_budget?: number;
	cwd?: string;
	model?: string;
	provider?: string;
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

export interface SubagentPoolOptions {
	/** Path to the hoocode executable. */
	executable: string;
	/** Maximum concurrent child processes. Defaults to 5. */
	maxConcurrency?: number;
	/** Working directory for spawned processes. Defaults to process.cwd(). */
	cwd?: string;
	/** Environment variables. Defaults to process.env. */
	env?: NodeJS.ProcessEnv;
	/** Default token budget per task. Defaults to 0. */
	defaultTokenBudget?: number;
}

/**
 * Pool for running hoocode subagents as child processes with bounded concurrency,
 * FIFO queuing with priority support, and automatic slot refill.
 *
 * Events:
 * - "task_done"    – task completed successfully and output was verified
 * - "task_failed"  – task failed (spawn error, bad exit code, verification failure)
 * - "task_stalled" – heartbeat missed for 60s, process was SIGKILLed
 * - "task_timeout" – hard timeout exceeded, process was SIGKILLed
 * - "budget_warning" – token usage crossed 80% threshold
 */
export class SubagentPool extends EventEmitter {
	private readonly maxConcurrency: number;
	private readonly executable: string;
	private readonly cwd: string;
	private readonly env: NodeJS.ProcessEnv;
	private readonly defaultTokenBudget: number;

	private slots = new Map<string, SubagentSlot>();
	private queue: SubagentPoolTask[] = [];
	private completed = new Map<string, SubagentResult>();
	private waiters = new Map<string, { resolve: (result: SubagentResult) => void; reject: (err: Error) => void }>();
	private budgets = new Map<string, TokenBudget>();
	private verifier = new OutputVerifier();
	private lifeguard: SubagentLifeguard;
	private disposed = false;
	/** Tracks why a task was killed (stalled / timeout) before exit handler fires. */
	private killReasons = new Map<string, "stalled" | "timeout">();
	/** Persistent terminal status map, survives wait_for consumption. */
	private taskStatus = new Map<string, "done" | "failed" | "stalled" | "timeout">();

	constructor(options: SubagentPoolOptions) {
		super();
		this.maxConcurrency = options.maxConcurrency ?? 5;
		this.executable = options.executable;
		this.cwd = options.cwd ?? process.cwd();
		this.env = options.env ?? process.env;
		this.defaultTokenBudget = options.defaultTokenBudget ?? 0;
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

	/** Priority value: higher numbers run first. */
	private priorityOf(agent_type: string): number {
		switch (agent_type) {
			case "explore":
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
		const args: string[] = ["--mode", "json", "--no-session"];

		if (task.agent_type) {
			try {
				const systemPrompt = getSubagentSystemPrompt(task.agent_type as SubagentMode);
				args.push("--system-prompt", systemPrompt);
			} catch {
				// Unknown mode, skip custom system prompt
			}
		}

		if (task.model) {
			args.push("--model", task.model);
		}
		if (task.provider) {
			args.push("--provider", task.provider);
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
			budget.on("budget_exceeded", () => {
				const slot = this.slots.get(task.task_id);
				if (slot && !slot.process.killed) {
					slot.process.kill("SIGTERM");
				}
			});
			this.budgets.set(task.task_id, budget);
		}

		let proc: ReturnType<typeof spawn>;
		try {
			proc = spawn(this.executable, this.buildArgs(task), {
				cwd: task.cwd ?? this.cwd,
				env: this.env,
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
		let budgetExceeded = false;

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

		budget.once("budget_exceeded", () => {
			budgetExceeded = true;
		});

		waitForChildProcess(proc)
			.then((code) => {
				this.slots.delete(task.task_id);
				budget.flush();

				const killReason = this.killReasons.get(task.task_id);
				this.killReasons.delete(task.task_id);

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
					this.emit(`task_${killReason}`, { task_id: task.task_id });
					this.resolveWaiter(task.task_id, result);
					return;
				}

				const result: SubagentResult = {
					task_id: task.task_id,
					ok: code === 0 && !budgetExceeded,
					stdout,
					stderr,
					exit_code: code,
					budget_exceeded: budgetExceeded,
					status: code === 0 && !budgetExceeded ? "complete" : "failed",
				};

				if (budgetExceeded) {
					// Force-return whatever exists in result.json, mark partial
					const resultData = this.tryReadResultJson(task.task_id, task.cwd ?? this.cwd);
					result.status = resultData ? "partial" : "failed";
					result.result_data = resultData;
					if (resultData) {
						result.ok = true; // partial is considered success with data
					}
					this.emit("task_done", { task_id: task.task_id, status: "partial" });
					this.resolveWaiter(task.task_id, result);
					return;
				}

				if (result.ok) {
					const verification = this.verifier.verify(task.task_id, task.cwd ?? this.cwd);
					if (!verification.valid) {
						result.ok = false;
						result.error = verification.reason;
						result.status = "failed";
						this.emit("task_failed", {
							task_id: task.task_id,
							error: verification.reason,
						});
						this.resolveWaiter(task.task_id, result);
						return;
					}
				}

				if (result.ok) {
					this.emit("task_done", { task_id: task.task_id, status: "complete" });
				} else {
					this.emit("task_failed", {
						task_id: task.task_id,
						error: result.error ?? `Exited with code ${code}`,
					});
				}

				this.resolveWaiter(task.task_id, result);
			})
			.catch((err) => {
				this.slots.delete(task.task_id);
				budget.flush();
				if (!isRetry) {
					this.startTask(task, true);
					return;
				}
				const error = err instanceof Error ? err.message : String(err);
				this.emit("task_failed", { task_id: task.task_id, error });
				this.resolveWaiter(task.task_id, {
					task_id: task.task_id,
					ok: false,
					stdout,
					stderr,
					exit_code: null,
					error,
					status: "failed",
				});
			})
			.finally(() => {
				this.budgets.delete(task.task_id);
				this.pull();
			});
	}

	private tryReadResultJson(task_id: string, cwd: string): Record<string, unknown> | undefined {
		const path = join(cwd, CONFIG_DIR_NAME, "agents", task_id, "result.json");
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
