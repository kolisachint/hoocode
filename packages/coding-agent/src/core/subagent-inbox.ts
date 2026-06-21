/**
 * Subagent inbox: the notify-and-pull bookkeeping behind background `Task`
 * dispatch and the `TaskOutput` tool.
 *
 * Background dispatches don't push their full result into the parent's context.
 * Instead the parent gets a compact notification ("explore#1 finished") and the
 * body is retained here, keyed by task id, until the model explicitly pulls it
 * with TaskOutput. This keeps a wide swarm of subagents from flooding the
 * context with N full summaries, and gives the model a queryable liveness view
 * (running / done / failed + last activity) so it can tell a working subagent
 * from a rogue one. The same surface works at every delegation depth.
 *
 * Lifecycle per task:
 *   running ──▶ done   (body retained) ──collect──▶ collected (body dropped)
 *           ├─▶ failed
 *           ├─▶ stalled
 *           └─▶ timeout
 */

import type { SubagentPool, TaskResult } from "./subagent-pool.js";
import type { SubagentResultFile } from "./subagent-result.js";

export type TaskLifecycle = "running" | "done" | "failed" | "stalled" | "timeout" | "collected";

/** A non-terminal task is still doing work; the rest have settled. */
export function isOutstanding(lifecycle: TaskLifecycle): boolean {
	return lifecycle === "running";
}

export interface InboxRecord {
	taskId: string;
	/** Friendly handle shown to the model, e.g. "explore#1". */
	label: string;
	agentType: string;
	lifecycle: TaskLifecycle;
	startedAt: number;
	endedAt?: number;
	/** Tool the subagent is currently running, from the pool's progress stream. */
	lastActivity?: string;
	/** First line of the result/summary, kept even after the body is collected. */
	summaryLine?: string;
	/** Full subagent summary, retained only while `done` and uncollected. */
	body?: string;
	/** Failure reason when the task did not succeed. */
	error?: string;
}

/** Cap on settled (terminal) records kept for late pulls; running records are never pruned. */
const MAX_SETTLED = 50;

/** First non-empty line of a block of text, length-capped. */
function firstLine(text: string, max = 120): string {
	const line = (text.trim().split("\n").find((l) => l.trim()) ?? "").trim();
	return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/** Map a subagent's terminal `status` onto an inbox lifecycle. */
function failLifecycle(status: string | undefined): TaskLifecycle {
	if (status === "stalled") return "stalled";
	if (status === "timeout") return "timeout";
	return "failed";
}

/** Derive the "currently running X" activity label from a forwarded progress event. */
function activityFromEvent(event: Record<string, unknown>): string | undefined {
	if (event.type === "tool_execution_start" && typeof event.toolName === "string") {
		return event.toolName;
	}
	if (event.type === "turn_end") return "thinking";
	return undefined;
}

class SubagentInbox {
	private records = new Map<string, InboxRecord>();
	/** Insertion order of task ids, for stable listing and pruning. */
	private order: string[] = [];
	private labelCounters = new Map<string, number>();
	private observedPools = new WeakSet<SubagentPool>();

	/** Allocate the next friendly label for an agent type (`explore#1`, `explore#2`, …). */
	nextLabel(agentType: string): string {
		const n = (this.labelCounters.get(agentType) ?? 0) + 1;
		this.labelCounters.set(agentType, n);
		return `${agentType}#${n}`;
	}

	/**
	 * Track a pool's `task_progress` events so running records carry a live
	 * `lastActivity`. Idempotent per pool, so callers can wire it on every dispatch.
	 */
	observe(pool: SubagentPool): void {
		// Tolerate a non-EventEmitter stand-in (test fakes): without progress events
		// records simply carry no live activity.
		if (typeof (pool as { on?: unknown }).on !== "function") return;
		if (this.observedPools.has(pool)) return;
		this.observedPools.add(pool);
		pool.on("task_progress", (data: { task_id: string; event: Record<string, unknown> }) => {
			const rec = this.records.get(data.task_id);
			if (!rec || rec.lifecycle !== "running") return;
			const activity = activityFromEvent(data.event);
			if (activity) rec.lastActivity = activity;
		});
	}

	/** Register a freshly dispatched background task as running. */
	start(taskId: string, label: string, agentType: string): InboxRecord {
		const rec: InboxRecord = { taskId, label, agentType, lifecycle: "running", startedAt: Date.now() };
		this.records.set(taskId, rec);
		this.order.push(taskId);
		this.prune();
		return rec;
	}

	/** Settle a task from its dispatch result: retain the body on success, the reason on failure. */
	finish(taskId: string, result: TaskResult): InboxRecord | undefined {
		const rec = this.records.get(taskId);
		if (!rec) return undefined;
		rec.endedAt = Date.now();
		rec.lastActivity = undefined;
		const r = result.result;
		const data = r?.result_data as SubagentResultFile | undefined;
		if (r?.ok) {
			rec.lifecycle = "done";
			rec.body = data?.summary?.trim() || "(subagent returned no output)";
			rec.summaryLine = firstLine(rec.body);
		} else {
			rec.lifecycle = failLifecycle(r?.status);
			rec.error = r?.error ?? (r?.status ? `subagent ${r.status}` : "unknown error");
			rec.summaryLine = rec.error;
		}
		return rec;
	}

	/** Settle a task that never produced a TaskResult (e.g. a thrown dispatch error). */
	fail(taskId: string, reason: string, lifecycle: TaskLifecycle = "failed"): InboxRecord | undefined {
		const rec = this.records.get(taskId);
		if (!rec) return undefined;
		rec.endedAt = Date.now();
		rec.lastActivity = undefined;
		rec.lifecycle = lifecycle;
		rec.error = reason;
		rec.summaryLine = reason;
		return rec;
	}

	/** Look up a record by task id or by friendly label. */
	get(handle: string): InboxRecord | undefined {
		const byId = this.records.get(handle);
		if (byId) return byId;
		for (const rec of this.records.values()) {
			if (rec.label === handle) return rec;
		}
		return undefined;
	}

	/**
	 * Read a done task's body and mark it collected, dropping the body so it is not
	 * re-fed to the model. Returns the record (with `body` still populated for this
	 * one read) or undefined for an unknown handle.
	 */
	collect(handle: string): { record: InboxRecord; body: string } | undefined {
		const rec = this.get(handle);
		if (!rec || rec.lifecycle !== "done" || rec.body === undefined) return undefined;
		const body = rec.body;
		rec.lifecycle = "collected";
		rec.body = undefined;
		return { record: rec, body };
	}

	/** All records, oldest first. */
	list(): InboxRecord[] {
		return this.order.map((id) => this.records.get(id)).filter((r): r is InboxRecord => r !== undefined);
	}

	/** Records still doing work. */
	outstanding(): InboxRecord[] {
		return this.list().filter((r) => isOutstanding(r.lifecycle));
	}

	/** Test/teardown helper. */
	clear(): void {
		this.records.clear();
		this.order = [];
		this.labelCounters.clear();
	}

	/** Drop the oldest settled records once past the cap; running records are kept. */
	private prune(): void {
		let settled = this.order.filter((id) => {
			const r = this.records.get(id);
			return r !== undefined && !isOutstanding(r.lifecycle);
		}).length;
		if (settled <= MAX_SETTLED) return;
		const kept: string[] = [];
		for (const id of this.order) {
			const r = this.records.get(id);
			if (r && !isOutstanding(r.lifecycle) && settled > MAX_SETTLED) {
				this.records.delete(id);
				settled--;
				continue;
			}
			kept.push(id);
		}
		this.order = kept;
	}
}

/** Process-wide subagent inbox shared by the Task and TaskOutput tools. */
export const subagentInbox = new SubagentInbox();
