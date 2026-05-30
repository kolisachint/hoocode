import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { waitForChildProcess } from "../utils/child-process.js";
import { getSubagentSystemPrompt, type SubagentMode } from "./subagent.js";

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
	private disposed = false;

	constructor(options: SubagentPoolOptions) {
		super();
		this.maxConcurrency = options.maxConcurrency ?? 5;
		this.executable = options.executable;
		this.cwd = options.cwd ?? process.cwd();
		this.env = options.env ?? process.env;
		this.defaultTokenBudget = options.defaultTokenBudget ?? 0;
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
				this.emit("subagent_failed", {
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

		let stdout = "";
		let stderr = "";

		proc.stdout?.on("data", (data: Buffer) => {
			stdout += data.toString();
		});
		proc.stderr?.on("data", (data: Buffer) => {
			stderr += data.toString();
		});

		waitForChildProcess(proc)
			.then((code) => {
				this.slots.delete(task.task_id);
				this.resolveWaiter(task.task_id, {
					task_id: task.task_id,
					ok: code === 0,
					stdout,
					stderr,
					exit_code: code,
				});
			})
			.catch((err) => {
				this.slots.delete(task.task_id);
				if (!isRetry) {
					this.startTask(task, true);
					return;
				}
				const error = err instanceof Error ? err.message : String(err);
				this.emit("subagent_failed", { task_id: task.task_id, error });
				this.resolveWaiter(task.task_id, {
					task_id: task.task_id,
					ok: false,
					stdout,
					stderr,
					exit_code: null,
					error,
				});
			})
			.finally(() => {
				this.pull();
			});
	}

	private resolveWaiter(task_id: string, result: SubagentResult): void {
		const waiter = this.waiters.get(task_id);
		if (waiter) {
			waiter.resolve(result);
			this.waiters.delete(task_id);
			return;
		}
		this.completed.set(task_id, result);
	}
}
