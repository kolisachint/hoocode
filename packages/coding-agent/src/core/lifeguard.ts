import type { ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, readdirSync, readFileSync, rmdirSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { CONFIG_DIR_NAME } from "../config.js";

const TIMEOUTS_MS: Record<string, number> = {
	explore: 5 * 60 * 1000,
	edit: 10 * 60 * 1000,
	test: 10 * 60 * 1000,
	review: 8 * 60 * 1000,
	doc: 5 * 60 * 1000,
};

const HEARTBEAT_MISS_THRESHOLD_MS = 60000;
const PARENT_SHUTDOWN_GRACE_MS = 5000;

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
	private checkInterval: NodeJS.Timeout | null = null;
	private disposed = false;
	private readonly cwd: string;
	private parentShutdownHandler?: () => void;

	constructor(cwd: string) {
		super();
		this.cwd = cwd;
		this.setupParentExitHandlers();
		this.sweepOldAgents();
		this.checkInterval = setInterval(() => this.checkHeartbeats(), 5000);
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
		const timeout = setTimeout(() => {
			this.handleTimeout(task_id);
		}, timeoutMs);
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
		this.removeAllListeners();

		if (this.parentShutdownHandler) {
			process.removeListener("SIGINT", this.parentShutdownHandler);
			process.removeListener("SIGTERM", this.parentShutdownHandler);
		}
	}

	private checkHeartbeats(): void {
		const now = Date.now();
		for (const [task_id] of this.processes) {
			const last = this.lastHeartbeat.get(task_id);
			if (last === undefined) continue;
			if (now - last > HEARTBEAT_MISS_THRESHOLD_MS) {
				this.handleStalled(task_id);
			}
		}
	}

	private handleStalled(task_id: string): void {
		const monitored = this.processes.get(task_id);
		if (!monitored) return;

		if (!monitored.process.killed) {
			monitored.process.kill("SIGKILL");
		}

		this.emit("stalled", { task_id, pid: monitored.pid });
		// Process exit handler will call untrack()
	}

	private handleTimeout(task_id: string): void {
		const monitored = this.processes.get(task_id);
		if (!monitored) return;

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
		const agentsDir = join(this.cwd, CONFIG_DIR_NAME, "agents");
		if (!existsSync(agentsDir)) return;

		const now = Date.now();
		const cutoff = 24 * 60 * 60 * 1000; // 24 hours

		for (const entry of readdirSync(agentsDir)) {
			const entryPath = join(agentsDir, entry);
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
