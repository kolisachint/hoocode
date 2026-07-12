import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { getDispatchTaskDir } from "../config.js";
import { attachJsonlLineReader } from "../modes/rpc/jsonl.js";
import { writeFileAtomicSync } from "../utils/atomic-file.js";
import { waitForChildProcess } from "../utils/child-process.js";
import { killProcessTree } from "../utils/shell.js";
import { MODEL_INHERIT } from "./agent-frontmatter.js";
import { type AgentRegistry, loadAgentRegistry } from "./agent-registry.js";
import { DispatchEvaluator } from "./dispatch-evaluator.js";
import { SubagentLifeguard } from "./lifeguard.js";
import { resolveModelReference } from "./model-categories.js";
import { OutputVerifier } from "./output-verifier.js";
import type { Settings } from "./settings-manager.js";
import {
	currentSubagentDepth,
	DEFER_MCP_SCHEMAS_ENV,
	resolveMaxSubagentDepth,
	SUBAGENT_DEPTH_ENV,
	SUBAGENT_SKIP_MCP_ENV,
	toolAllowlistNeedsMcp,
} from "./subagent-depth.js";
import { SUBAGENT_PROGRESS_EVENTS } from "./subagent-events.js";
import { TokenBudget } from "./token-budget.js";

/**
 * Provider/model failure signatures where inheriting the parent model can
 * recover. Compiled once at module load instead of on every error check.
 */
const INHERITED_MODEL_FALLBACK_ERROR =
	/usage[_\s-]?limit|subscription|quota|rate.?limit|too many requests|429|insufficient|out of credit|credit balance|billing|payment required|402|model[^\n]*(not found|unavailable|not available|not supported|does not exist|invalid|unsupported)|no api key|no auth configured|authentication|unauthorized|forbidden|permission/i;

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
	/** Internal: retry using the caller's model when a built-in agent's preferred model fails. */
	useInheritedModelFallback?: boolean;
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
	status?: "complete" | "partial" | "failed" | "stalled" | "timeout" | "cancelled";
	/** Parsed result.json content when available (e.g. on partial completion). */
	result_data?: Record<string, unknown>;
	/** True when this run used the inherited-model fallback (preferred model failed first). */
	usedInheritedModelFallback?: boolean;
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
	/** Caller-supplied task id. Defaults to a generated `dispatch-…` id. Lets a
	 *  caller register liveness/inbox state under the id before dispatch resolves. */
	taskId?: string;
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
	/** Settings for model category resolution. */
	settings?: Settings;
}

/**
 * Default hard cap on assistant turns for a spawned subagent when its definition
 * does not set `maxTurns`. The token budget is advisory (it warns but never
 * kills), so this turn cap is the guaranteed hard stop for every subagent.
 */
export const DEFAULT_SUBAGENT_MAX_TURNS = 50;

/**
 * AgentSession event `type`s forwarded from a subagent's json event stream as
 * `task_progress` events. Deliberately coarse; the child now also filters its
 * stdout to this set plus `message_end` at the source (see
 * SUBAGENT_STDOUT_EVENT_TYPES), so the per-delta firehose no longer crosses the
 * pipe. Re-exported from the shared module so the child emitter and this
 * consumer stay in lockstep.
 */
export const FORWARDED_SUBAGENT_EVENTS = SUBAGENT_PROGRESS_EVENTS;

/**
 * Cap on the captured stdout/stderr text kept per task (tail-truncated). The
 * capture exists for diagnostics (failure reasons, output.json), so keeping the
 * most recent tail is enough; without a cap a chatty child could grow the
 * parent's memory without bound across a long swarm.
 */
export const MAX_CAPTURED_STREAM_CHARS = 256 * 1024;

/**
 * Cap on a single un-terminated JSONL line buffered from a child's stdout. The
 * forwarded events are small; anything approaching this is a runaway writer,
 * and the reader drops the line rather than buffering toward OOM.
 */
export const MAX_SUBAGENT_EVENT_LINE_CHARS = 8 * 1024 * 1024;

/** Append a chunk to a capped capture buffer, keeping the most recent tail. */
function appendTail(current: string, chunk: string, cap: number = MAX_CAPTURED_STREAM_CHARS): string {
	const next = current + chunk;
	return next.length > cap ? next.slice(next.length - cap) : next;
}

/** The action the pool should take for one JSONL line from a subagent's stdout. */
export type SubagentStdoutLine =
	| { kind: "heartbeat" }
	| { kind: "progress"; event: Record<string, unknown> }
	| { kind: "ignore" };

/**
 * Classify one JSONL line from a subagent's stdout into the action to take.
 * Pure (no side effects) so the ping/forward/drop policy is unit-testable without
 * spawning a child. Line framing — UTF-8-safe reassembly of chunks split mid-line
 * — is handled upstream by attachJsonlLineReader; this only sees complete lines.
 */
export function classifySubagentLine(line: string): SubagentStdoutLine {
	const trimmed = line.trim();
	if (!trimmed.startsWith("{")) return { kind: "ignore" };
	let parsed: Record<string, unknown>;
	try {
		parsed = JSON.parse(trimmed) as Record<string, unknown>;
	} catch {
		return { kind: "ignore" };
	}
	if (parsed.ping === true) return { kind: "heartbeat" };
	if (typeof parsed.type === "string" && FORWARDED_SUBAGENT_EVENTS.has(parsed.type)) {
		return { kind: "progress", event: parsed };
	}
	return { kind: "ignore" };
}

/**
 * Pool for running hoocode subagents as child processes with bounded concurrency,
 * FIFO queuing with priority support, and automatic slot refill.
 *
 * Events:
 * - "task_done"    – task completed successfully and output was verified
 * - "task_failed"  – task failed (spawn error, bad exit code, verification failure)
 * - "task_stalled" – heartbeat missed past the load-scaled threshold (60s base,
 *                    widened under concurrency/event-loop lag), process SIGKILLed
 * - "task_timeout" – hard timeout exceeded, process was SIGKILLed
 * - "task_cancelled" – user-initiated cancel (see cancel()); process tree killed
 * - "budget_warning" – token usage crossed 80% threshold (advisory)
 * - "budget_exceeded" – token usage crossed 100% threshold (advisory; never kills)
 * - "task_progress" – coarse lifecycle event (turn_end, tool start/end) parsed
 *                    from the child's json event stream, for live UI updates
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
	/** Tracks why a task was killed (stalled / timeout / user cancel) before exit handler fires. */
	private killReasons = new Map<string, "stalled" | "timeout" | "cancelled">();
	/** Persistent terminal status map, survives wait_for consumption. */
	private taskStatus = new Map<string, "done" | "failed" | "stalled" | "timeout" | "cancelled">();
	/** Settings for model category resolution. */
	private readonly settings?: Settings;

	constructor(options: SubagentPoolOptions) {
		super();
		this.maxConcurrency = options.maxConcurrency ?? 5;
		this.executable = options.executable;
		this.prefixArgs = options.prefixArgs ?? [];
		this.cwd = options.cwd ?? process.cwd();
		this.env = options.env ?? process.env;
		this.defaultTokenBudget = options.defaultTokenBudget ?? 0;
		this.skillPaths = options.skillPaths ? [...options.skillPaths] : [];
		this.settings = options.settings;
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

	/**
	 * Report external in-process load (e.g. the number of background MCP tools
	 * currently executing in the parent) to the lifeguard. This widens its
	 * heartbeat/timeout tolerance so monitored subagents aren't false-positive
	 * reaped when the parent's event loop is busy with concurrent background work.
	 */
	setExternalLoad(count: number): void {
		this.lifeguard.setExternalLoad(count);
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
		// Read-only investigation (explore/plan) often unblocks downstream work, so
		// it runs ahead of other agents.
		return agent_type === "explore" || agent_type === "plan" ? 2 : 1;
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
	get_status(
		task_id: string,
	): "running" | "queued" | "done" | "failed" | "stalled" | "timeout" | "cancelled" | "unknown" {
		if (this.slots.has(task_id)) return "running";
		if (this.queue.some((t) => t.task_id === task_id)) return "queued";
		const persisted = this.taskStatus.get(task_id);
		if (persisted) return persisted;
		const result = this.completed.get(task_id);
		if (result) {
			if (result.status === "stalled") return "stalled";
			if (result.status === "timeout") return "timeout";
			if (result.status === "cancelled") return "cancelled";
			if (result.ok) return "done";
			return "failed";
		}
		return "unknown";
	}

	/**
	 * Cancel a task on the user's behalf (Esc/abort mid-turn). A queued task is
	 * removed and settles immediately; a running task's whole process tree is
	 * killed and settles with status "cancelled" when its exit is observed
	 * (unless it already wrote a valid result.json — completed work is honored).
	 * Returns false for unknown/settled task ids.
	 */
	cancel(task_id: string): boolean {
		const queued = this.queue.findIndex((t) => t.task_id === task_id);
		if (queued !== -1) {
			this.queue.splice(queued, 1);
			const result: SubagentResult = {
				task_id,
				ok: false,
				stdout: "",
				stderr: "",
				exit_code: null,
				error: "cancelled before start",
				status: "cancelled",
			};
			this.emit("task_cancelled", { task_id });
			this.resolveWaiter(task_id, result);
			return true;
		}
		const slot = this.slots.get(task_id);
		if (!slot) return false;
		this.killReasons.set(task_id, "cancelled");
		if (slot.pid > 0) {
			killProcessTree(slot.pid);
		} else if (!slot.process.killed) {
			slot.process.kill("SIGKILL");
		}
		return true;
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
		const task_id = options.taskId ?? `dispatch-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
		const reason = forceAgent ? "user_override" : analysis.reason;
		const complexity = analysis.estimated_complexity;
		// Depth of the child about to be spawned (this process's depth + 1). Surfaced
		// so a delegation tree's nesting is visible in logs without extra tooling.
		const childDepth = currentSubagentDepth(this.env) + 1;

		// Pre-dispatch logging. Use stderr: stdout is reserved for the JSON event
		// stream / TUI render and must not be polluted.
		console.error(
			`[DISPATCH] agent=${agent_type} depth=${childDepth} reason=${reason} complexity=${complexity} task_id=${task_id}`,
		);
		this.writeDispatchLog(task_id, agent_type, reason, complexity, task, childDepth);

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
		depth: number,
	): void {
		const log = {
			timestamp: new Date().toISOString(),
			task_id,
			agent_type,
			depth,
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
			// Atomic like result.json: background pollers may read output.json while
			// it is being (re)written.
			writeFileAtomicSync(path, JSON.stringify(output, null, 2));
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
			// Kill the whole process group/tree, not just the direct child: a
			// subagent's own children (bash commands, nested subagents) must not
			// outlive the pool.
			if (slot.pid > 0) {
				killProcessTree(slot.pid);
			} else if (!slot.process.killed) {
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

			// A `delegate: true` agent may itself dispatch via the Task tool, but only
			// while the child it becomes can still nest (childDepth < cap) — so the
			// deepest permitted level cannot delegate further. Gating here keeps the
			// authorization explicit and bounded by the same cap as the depth guard.
			const childDepth = currentSubagentDepth(this.env) + 1;
			const canChildDelegate = def?.delegate === true && childDepth < resolveMaxSubagentDepth(undefined, this.env);

			// Tool allowlist comes from the agent definition's frontmatter `tools`
			// field (read-only built-ins declare their own sandbox). When omitted, no
			// --tools is passed and the subagent inherits all parent tools (so the Task
			// tool already survives). A delegating agent with an explicit allowlist must
			// have Task/TaskOutput added, or the child would filter them out.
			const tools = def?.tools ? [...def.tools] : undefined;
			if (canChildDelegate && tools) {
				for (const t of ["Task", "TaskOutput"]) {
					if (!tools.includes(t)) tools.push(t);
				}
			}
			if (tools && tools.length > 0) {
				args.push("--tools", tools.join(","));
			}
			if (def?.disallowedTools && def.disallowedTools.length > 0) {
				args.push("--disallowed-tools", def.disallowedTools.join(","));
			}

			// Propagate subagent enablement so the child registers the Task tool; without
			// this the flag-based enablement would not reach a spawned child.
			if (canChildDelegate) {
				args.push("--enable-subagents");
				// Scoped delegation: restrict which agent types this child may spawn.
				if (def?.delegateTo && def.delegateTo.length > 0) {
					args.push("--delegate-allow", def.delegateTo.join(","));
				}
			}
		}

		// Model precedence: a definition's explicit model wins (unless it is the
		// `inherit` sentinel), otherwise use the caller-provided model. Built-in
		// agents can retry with the inherited model when their preferred model is
		// unavailable or quota-limited.
		const explicitModel =
			!task.useInheritedModelFallback && def?.model && def.model !== MODEL_INHERIT ? def.model : undefined;
		// Resolve a model-category reference (fast/standard/capable) to its
		// configured model id. An unconfigured category resolves to undefined, so no
		// `--model` is passed and the child keeps its default model.
		const rawModel = explicitModel ?? task.model;
		const modelToUse = rawModel ? resolveModelReference(rawModel, this.settings) : undefined;
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

	/**
	 * Environment for a spawned child.
	 *
	 * Stamps the child's depth (parent depth + 1) so its own guard knows where it
	 * sits in the tree; the tree-wide cap (HOOCODE_SUBAGENT_MAX_DEPTH) is inherited
	 * via the spread, so at the default cap of 1 the child lands at depth 1 and
	 * cannot spawn further subagents. Also flags the child to skip MCP server
	 * connection when its tool allowlist is explicit and MCP-free — connecting
	 * external servers it can never call is pure boot latency.
	 */
	private childSpawnEnv(task: SubagentPoolTask): NodeJS.ProcessEnv {
		const env: NodeJS.ProcessEnv = {
			...this.env,
			[SUBAGENT_DEPTH_ENV]: String(currentSubagentDepth(this.env) + 1),
		};
		// Use the agent's own frontmatter allowlist (the same source buildArgs passes
		// via --tools). A delegating child also gets Task/TaskOutput appended there,
		// but neither is an MCP tool, so the decision is unchanged by that.
		const def = task.agent_type ? this.getRegistry().get(task.agent_type) : undefined;
		if (!toolAllowlistNeedsMcp(def?.tools)) {
			env[SUBAGENT_SKIP_MCP_ENV] = "1";
		}
		// A child never defers MCP schemas: if it needs MCP it eager-registers its
		// allowlisted tools at dispatch so they are immediately callable (spec §2).
		delete env[DEFER_MCP_SCHEMAS_ENV];
		return env;
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
				env: this.childSpawnEnv(task),
				shell: false,
				stdio: ["ignore", "pipe", "pipe"],
				// POSIX: the child leads its own process group so a kill can reach its
				// whole tree (see lifeguard/dispose). A single-PID SIGKILL orphans any
				// grandchildren the subagent spawned (its own bash commands, nested
				// subagents), which kept burning CPU after their parent was reaped.
				detached: process.platform !== "win32",
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
		let detachStdoutReader: (() => void) | undefined;

		proc.stdout?.on("data", (data: Buffer) => {
			// Capture (tail-capped) for diagnostics, and treat any output as a
			// heartbeat: a child busily streaming events is alive even when the
			// parent's starved event loop hasn't parsed its {"ping":true} line yet.
			// All *parsing* happens in the single JSONL reader below.
			stdout = appendTail(stdout, data.toString());
			this.lifeguard.recordHeartbeat(task.task_id);
		});

		// The one parser on the child's stdout: UTF-8-safe, LF-only framing —
		// multi-byte characters and large events split across pipe chunks are
		// reassembled before parsing. Each complete line feeds the token budget
		// (usage telemetry) and the ping/forward/drop classifier; previously the
		// budget ran its own naive chunk-splitting parser on the same stream, which
		// corrupted lines split mid-character. The line buffer is capped so a
		// runaway writer cannot OOM the parent. Detached when the child exits (see
		// cleanup below); the reader also self-detaches on stream error/close.
		if (proc.stdout) {
			detachStdoutReader = attachJsonlLineReader(
				proc.stdout,
				(line) => {
					budget.processLine(line);
					const action = classifySubagentLine(line);
					if (action.kind === "heartbeat") {
						this.lifeguard.recordHeartbeat(task.task_id);
					} else if (action.kind === "progress") {
						this.emit("task_progress", {
							task_id: task.task_id,
							agent_type: task.agent_type,
							event: action.event,
						});
					}
				},
				{ maxBuffer: MAX_SUBAGENT_EVENT_LINE_CHARS },
			);
		}
		proc.stderr?.on("data", (data: Buffer) => {
			stderr = appendTail(stderr, data.toString());
		});

		// True when this attempt scheduled another run of the same task id (spawn
		// retry or inherited-model fallback). The retry reuses the task's TokenBudget
		// so usage accumulates across attempts; the .finally below must then leave
		// the budget (and its listeners) alone instead of tearing it down under the
		// retry's feet.
		let retryScheduled = false;

		waitForChildProcess(proc)
			.then((code) => {
				this.slots.delete(task.task_id);
				budget.flush();

				const killReason = this.killReasons.get(task.task_id);
				this.killReasons.delete(task.task_id);

				const duration = Date.now() - slot.spawned_at;
				const tokens_used = budget.getUsed();
				const budgetExceeded = budget.isExceeded();

				// A subagent's success is defined by a valid, verified result.json, not by
				// its exit code. A child that finished its work and wrote a valid result can
				// still be SIGKILLed by the lifeguard before it exits on its own (lingering
				// open handles delay a natural exit past the heartbeat threshold), which forces
				// exit_code === null. Keying completion off the verified result, not code === 0,
				// honors that genuine success instead of discarding it as a false stall.
				const verification = this.verifier.verify(task.task_id, task.cwd ?? this.cwd);
				// A well-formed result.json counts as clean completion unless its own
				// status field declares failure (e.g. "failed" from a provider quota
				// error). Without this check the pool would treat a child that wrote a
				// valid-but-failed result.json and exited non-zero as a success.
				let cleanlyCompleted = code === 0 || verification.valid;
				if (cleanlyCompleted && verification.valid) {
					const rd = this.tryReadResultJson(task.task_id, task.cwd ?? this.cwd);
					if (rd && (rd as Record<string, unknown>).status === "failed") {
						cleanlyCompleted = false;
					}
				}

				// If killed (lifeguard reap or user cancel) before producing a valid
				// result, honor the kill; a child that already wrote a verified
				// result.json completed its work and settles as a success below.
				if (killReason !== undefined && !verification.valid) {
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
					ok: cleanlyCompleted,
					stdout,
					stderr,
					exit_code: code,
					// Advisory telemetry only: exceeding the budget never fails the task.
					budget_exceeded: budgetExceeded,
					status: cleanlyCompleted ? "complete" : "failed",
					usedInheritedModelFallback: task.useInheritedModelFallback === true,
				};

				if (result.ok) {
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
				// Attach the child's result.json (if any) and derive a concrete failure
				// reason so callers see the real cause (e.g. a provider usage/quota
				// error) instead of a generic "subagent failed".
				result.result_data = this.tryReadResultJson(task.task_id, task.cwd ?? this.cwd);
				if (!result.error) {
					result.error = this.deriveFailureReason(result);
				}
				if (this.shouldRetryWithInheritedModel(task, result)) {
					console.error(
						`[DISPATCH] agent=${task.agent_type} task_id=${task.task_id} preferred model failed; retrying with inherited model`,
					);
					this.cleanupRetryArtifacts(task);
					retryScheduled = true;
					this.queue.unshift({ ...task, useInheritedModelFallback: true });
					return;
				}
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
					// The retry (started synchronously here) looks the budget up from
					// this.budgets and reuses it; flag it so the .finally below doesn't
					// strip the budget's listeners or delete the entry the retry now owns.
					retryScheduled = true;
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
					usedInheritedModelFallback: task.useInheritedModelFallback === true,
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
				// Runs on success AND failure: the stdout reader used to be detached only
				// on the success path, leaking listeners (and their buffers) whenever a
				// child failed or the wait rejected.
				detachStdoutReader?.();
				proc.stdout?.removeAllListeners("data");
				proc.stderr?.removeAllListeners("data");
				proc.stdout?.destroy();
				proc.stderr?.destroy();
				if (!retryScheduled) {
					budget.removeAllListeners();
					this.budgets.delete(task.task_id);
				}
				this.pull();
			});
	}

	/** Whether a failed built-in subagent should be retried with `model: inherit`. */
	private shouldRetryWithInheritedModel(task: SubagentPoolTask, result: SubagentResult): boolean {
		if (task.useInheritedModelFallback) return false;
		if (task.sessionFile) return false;
		// Only the parent model is required: the provider may be unset when the
		// harness routes through a gateway. The retry inherits the parent model and
		// lets the child resolve the provider from its own default when none was threaded through.
		if (!task.model) return false;

		const def = task.agent_type ? this.getRegistry().get(task.agent_type) : undefined;
		// Built-in agents always inherit; project agents may pin an explicit model in
		// frontmatter, so let them fall back too when that model is rejected.
		if (def?.source !== "builtin" && def?.source !== "project") return false;
		if (!def.model || def.model === MODEL_INHERIT) return false;

		return this.isInheritedModelFallbackError(result);
	}

	/** Detect provider/model failures where inheriting the parent model can recover. */
	private isInheritedModelFallbackError(result: SubagentResult): boolean {
		const text = [result.error, result.stderr, JSON.stringify(result.result_data ?? {})]
			.filter((part): part is string => typeof part === "string" && part.length > 0)
			.join("\n");

		return INHERITED_MODEL_FALLBACK_ERROR.test(text);
	}

	/** Remove failed attempt artifacts before rerunning the same task id. */
	private cleanupRetryArtifacts(task: SubagentPoolTask): void {
		const cwd = task.cwd ?? this.cwd;
		const taskDir = getDispatchTaskDir(cwd, task.task_id);
		const sessionFile = task.sessionFile ?? this.getSessionFile(task.task_id, cwd);
		try {
			rmSync(sessionFile, { force: true });
			rmSync(join(taskDir, "result.json"), { force: true });
			rmSync(join(taskDir, "output.json"), { force: true });
		} catch {
			// Best-effort cleanup; retry can still proceed with existing artifacts.
		}
	}

	/**
	 * Best-effort concrete failure reason for a non-zero-exit subagent. Prefers
	 * the child's result.json summary (which carries the provider/model error
	 * message on failure), then the tail of stderr, then the exit code.
	 */
	private deriveFailureReason(result: SubagentResult): string {
		const summary = (result.result_data as { summary?: string } | undefined)?.summary?.trim();
		if (summary) {
			return summary;
		}
		const stderrTail = result.stderr
			.split("\n")
			.map((line) => line.trim())
			.filter((line) => line.length > 0)
			.slice(-5)
			.join("\n");
		if (stderrTail) {
			return stderrTail;
		}
		return `Exited with code ${result.exit_code}`;
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
		else if (result.status === "cancelled") this.taskStatus.set(task_id, "cancelled");
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
