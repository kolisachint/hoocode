import type { ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, readdirSync, readFileSync, rmdirSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { getDispatchRoot } from "../config.js";

const TIMEOUTS_MS: Record<string, number> = {
	explore: 5 * 60 * 1000,
	edit: 10 * 60 * 1000,
	test: 10 * 60 * 1000,
	review: 8 * 60 * 1000,
	doc: 5 * 60 * 1000,
};

const HEARTBEAT_MISS_THRESHOLD_MS = 60000;
const HEARTBEAT_CHECK_INTERVAL_MS = 5000;
const PARENT_SHUTDOWN_GRACE_MS = 5000;

/**
 * Concurrency tolerance. Each *additional* concurrently-monitored subagent adds
 * this fraction to the heartbeat-miss and hard-timeout budgets.
 *
 * When several subagents run at once (plus background MCP tools), they saturate
 * the CPU and starve the parent's event loop: its `setInterval` heartbeat check
 * fires late, and it cannot read a child's `{"ping":true}` line in time even
 * though the child is healthy and still working. Scaling the budgets by load
 * stops that contention from false-positive SIGKILLing healthy subagents — the
 * failure the demo hit when running many agents + MCP tools in the background.
 */
const LOAD_TOLERANCE_PER_PROCESS = 0.5;

/**
 * Hard ceiling on the load tolerance multiplier, so a genuinely stuck subagent
 * is still eventually reaped no matter how busy the pool is.
 */
const MAX_LOAD_MULTIPLIER = 4;

export interface LifeguardProcess {
	pid: number;
	task_id: string;
	agent_type: string;
	process: ChildProcess;
}

/**
 * Monitors running subagent processes for heartbeats, hard timeouts,
 * and parent-exit cleanup. Emits "stalled" and "timeout" events when
 * processes are terminated.
 */
export class SubagentLifeguard extends EventEmitter {
	private processes = new Map<string, LifeguardProcess>();
	private lastHeartbeat = new Map<string, number>();
	private timeouts = new Map<string, NodeJS.Timeout>();
	/** When each task started, used to compute the load-scaled hard timeout. */
	private startedAt = new Map<string, number>();
	/** Per-agent base hard timeout (before load scaling), captured at monitor(). */
	private baseTimeoutMs = new Map<string, number>();
	private checkInterval: NodeJS.Timeout | null = null;
	/** Wall-clock time the heartbeat check last ran, to measure event-loop lag. */
	private lastCheckAt = Date.now();
	private disposed = false;
	private readonly cwd: string;
	private parentShutdownHandler?: () => void;

	constructor(cwd: string) {
		super();
		this.cwd = cwd;
		this.setupParentExitHandlers();
		this.sweepOldAgents();
		this.lastCheckAt = Date.now();
		this.checkInterval = setInterval(() => this.checkHeartbeats(), HEARTBEAT_CHECK_INTERVAL_MS);
	}

	/**
	 * Tolerance multiplier for the current load. 1 process → 1x; each extra
	 * concurrent process adds LOAD_TOLERANCE_PER_PROCESS, capped at MAX_LOAD_MULTIPLIER.
	 */
	private loadMultiplier(): number {
		const concurrent = this.processes.size;
		const mult = 1 + Math.max(0, concurrent - 1) * LOAD_TOLERANCE_PER_PROCESS;
		return Math.min(mult, MAX_LOAD_MULTIPLIER);
	}

	/**
	 * Begin monitoring a child process. The process must emit a
	 * `{"ping":true}` JSON line on stdout every 30 seconds.
	 */
	monitor(task_id: string, agent_type: string, proc: ChildProcess): void {
		if (this.disposed) return;

		const pid = proc.pid ?? 0;
		this.processes.set(task_id, { pid, task_id, agent_type, process: proc });
		this.lastHeartbeat.set(task_id, Date.now());

		const timeoutMs = TIMEOUTS_MS[agent_type] ?? TIMEOUTS_MS.explore;
		this.startedAt.set(task_id, Date.now());
		this.baseTimeoutMs.set(task_id, timeoutMs);
		// Arm the hard timeout scaled by current load. When it fires, handleTimeout
		// re-checks load and re-arms (up to MAX_LOAD_MULTIPLIER) if the pool is still
		// busy, so a slow-but-progressing subagent isn't killed for CPU contention.
		const timeout = setTimeout(
			() => {
				this.handleTimeout(task_id);
			},
			Math.round(timeoutMs * this.loadMultiplier()),
		);
		this.timeouts.set(task_id, timeout);

		proc.once("exit", () => {
			this.untrack(task_id);
		});
	}

	/** Record a heartbeat for a monitored task. */
	recordHeartbeat(task_id: string): void {
		if (this.processes.has(task_id)) {
			this.lastHeartbeat.set(task_id, Date.now());
		}
	}

	/** Get the last recorded heartbeat timestamp, or null. */
	lastHeartbeatAt(task_id: string): number | null {
		return this.lastHeartbeat.get(task_id) ?? null;
	}

	/** True if the task is currently being monitored. */
	isMonitoring(task_id: string): boolean {
		return this.processes.has(task_id);
	}

	/** Kill all monitored processes and clean up. */
	dispose(): void {
		if (this.disposed) return;
		this.disposed = true;

		if (this.checkInterval) {
			clearInterval(this.checkInterval);
			this.checkInterval = null;
		}

		for (const timeout of this.timeouts.values()) {
			clearTimeout(timeout);
		}
		this.timeouts.clear();

		for (const monitored of this.processes.values()) {
			if (!monitored.process.killed) {
				monitored.process.kill("SIGKILL");
			}
		}
		this.processes.clear();
		this.lastHeartbeat.clear();
		this.startedAt.clear();
		this.baseTimeoutMs.clear();
		this.removeAllListeners();

		if (this.parentShutdownHandler) {
			process.removeListener("SIGINT", this.parentShutdownHandler);
			process.removeListener("SIGTERM", this.parentShutdownHandler);
		}
	}

	private checkHeartbeats(): void {
		const now = Date.now();
		// Event-loop lag: how much later than scheduled this check actually ran.
		// Heavy CPU load (many concurrent subagents + background MCP tools) starves
		// the loop, delaying both this check *and* our reading of child heartbeats.
		// Forgive that gap so the parent's own starvation isn't charged against the
		// children as a missed heartbeat.
		const loopLag = Math.max(0, now - this.lastCheckAt - HEARTBEAT_CHECK_INTERVAL_MS);
		this.lastCheckAt = now;

		const threshold = HEARTBEAT_MISS_THRESHOLD_MS * this.loadMultiplier() + loopLag;
		for (const [task_id] of this.processes) {
			const last = this.lastHeartbeat.get(task_id);
			if (last === undefined) continue;
			if (now - last > threshold) {
				this.handleStalled(task_id);
			}
		}
	}

	private handleStalled(task_id: string): void {
		const monitored = this.processes.get(task_id);
		if (!monitored) return;

		// Record why we reaped this child so a recurrence is diagnosable rather than
		// just "stalled": how long since the last heartbeat, and the load factors
		// (concurrent monitored subagents, threshold) that fed the decision.
		const last = this.lastHeartbeat.get(task_id);
		const silentMs = last === undefined ? -1 : Date.now() - last;
		console.error(
			`[LIFEGUARD] stalled task_id=${task_id} agent=${monitored.agent_type} ` +
				`silent_ms=${silentMs} concurrent=${this.processes.size} ` +
				`load_mult=${this.loadMultiplier().toFixed(2)} base_threshold_ms=${HEARTBEAT_MISS_THRESHOLD_MS}`,
		);

		if (!monitored.process.killed) {
			monitored.process.kill("SIGKILL");
		}

		this.emit("stalled", { task_id, pid: monitored.pid });
		// Process exit handler will call untrack()
	}

	private handleTimeout(task_id: string): void {
		const monitored = this.processes.get(task_id);
		if (!monitored) return;

		// Under load the wall-clock timer can fire while the subagent is still doing
		// real work — its turns are just slow because the CPU is shared. Re-arm rather
		// than kill, up to a hard ceiling (base * MAX_LOAD_MULTIPLIER) so a genuinely
		// stuck agent still terminates.
		const started = this.startedAt.get(task_id) ?? Date.now();
		const base = this.baseTimeoutMs.get(task_id) ?? TIMEOUTS_MS[monitored.agent_type] ?? TIMEOUTS_MS.explore;
		const elapsed = Date.now() - started;
		const ceiling = base * MAX_LOAD_MULTIPLIER;
		if (this.loadMultiplier() > 1 && elapsed < ceiling) {
			const remaining = ceiling - elapsed;
			const next = Math.min(
				Math.round(base * this.loadMultiplier()),
				Math.max(HEARTBEAT_CHECK_INTERVAL_MS, remaining),
			);
			this.timeouts.set(
				task_id,
				setTimeout(() => this.handleTimeout(task_id), next),
			);
			return;
		}

		if (!monitored.process.killed) {
			monitored.process.kill("SIGKILL");
		}

		this.emit("timeout", { task_id, pid: monitored.pid });
		this.timeouts.delete(task_id);
		// Process exit handler will call untrack()
	}

	private untrack(task_id: string): void {
		const timeout = this.timeouts.get(task_id);
		if (timeout) {
			clearTimeout(timeout);
			this.timeouts.delete(task_id);
		}
		this.processes.delete(task_id);
		this.lastHeartbeat.delete(task_id);
		this.startedAt.delete(task_id);
		this.baseTimeoutMs.delete(task_id);
	}

	private setupParentExitHandlers(): void {
		const shutdown = () => this.gracefulShutdown();
		this.parentShutdownHandler = shutdown;
		process.setMaxListeners(Math.max(process.getMaxListeners(), 20));
		process.once("SIGINT", shutdown);
		process.once("SIGTERM", shutdown);
	}

	private gracefulShutdown(): void {
		// SIGTERM all children
		for (const monitored of this.processes.values()) {
			if (!monitored.process.killed) {
				monitored.process.kill("SIGTERM");
			}
		}

		// SIGKILL after grace period
		setTimeout(() => {
			for (const monitored of this.processes.values()) {
				if (!monitored.process.killed) {
					monitored.process.kill("SIGKILL");
				}
			}
		}, PARENT_SHUTDOWN_GRACE_MS).unref();
	}

	private sweepOldAgents(): void {
		const dispatchDir = getDispatchRoot(this.cwd);
		if (!existsSync(dispatchDir)) return;

		const now = Date.now();
		const cutoff = 24 * 60 * 60 * 1000; // 24 hours

		for (const entry of readdirSync(dispatchDir)) {
			const entryPath = join(dispatchDir, entry);
			try {
				const stats = statSync(entryPath);
				if (!stats.isDirectory()) continue;

				if (now - stats.mtimeMs > cutoff) {
					const hasRunningPid = this.hasRunningPid(entryPath);
					if (!hasRunningPid) {
						this.rmrf(entryPath);
					}
				}
			} catch {
				// Ignore errors for individual entries
			}
		}
	}

	private hasRunningPid(dir: string): boolean {
		const pidFile = join(dir, "pid");
		if (!existsSync(pidFile)) return false;

		try {
			const pid = Number.parseInt(readFileSync(pidFile, "utf-8"), 10);
			if (Number.isNaN(pid)) return false;
			process.kill(pid, 0); // Check if process exists
			return true;
		} catch {
			return false;
		}
	}

	private rmrf(dir: string): void {
		for (const entry of readdirSync(dir)) {
			const entryPath = join(dir, entry);
			try {
				const stats = statSync(entryPath);
				if (stats.isDirectory()) {
					this.rmrf(entryPath);
				} else {
					unlinkSync(entryPath);
				}
			} catch {
				// Ignore
			}
		}
		try {
			rmdirSync(dir);
		} catch {
			// Ignore
		}
	}
}
