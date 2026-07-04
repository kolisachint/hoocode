/**
 * Task scheduler — cron-driven recurring/one-shot prompts.
 *
 * Backs the `/loop` command and the Cron* tools. Tasks are matched against a
 * standard 5-field cron expression (minute hour day-of-month month day-of-week)
 * in local time and fired by re-submitting their prompt as a user message.
 * Firing is idle-gated so a task never interrupts an in-flight turn; a task due
 * while the agent is busy is picked up on a later tick within the same minute.
 *
 * Recurring tasks persist; one-shot tasks delete themselves after firing.
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export interface ScheduledTask {
	id: string;
	/** 5-field cron expression in local time. */
	cron: string;
	/** Prompt re-submitted on each fire. */
	prompt: string;
	/** Recurring (fire on every match) vs one-shot (fire once, then delete). */
	recurring: boolean;
	createdAt: number;
	/** Minute-bucket key of the last fire, to avoid double-firing within a minute. */
	lastRunMinute?: number;
}

/** Parse one cron field into a predicate over its numeric value. */
function parseField(field: string, min: number, max: number): (value: number) => boolean {
	if (field === "*") return () => true;

	const allowed = new Set<number>();
	for (const part of field.split(",")) {
		// step: */n or a-b/n or a/n
		const [range, stepStr] = part.split("/");
		const step = stepStr ? Number.parseInt(stepStr, 10) : 1;
		if (!Number.isFinite(step) || step < 1) continue;

		let lo = min;
		let hi = max;
		if (range && range !== "*") {
			const [a, b] = range.split("-");
			lo = Number.parseInt(a, 10);
			hi = b !== undefined ? Number.parseInt(b, 10) : lo;
			if (!Number.isFinite(lo) || !Number.isFinite(hi)) continue;
		}
		for (let v = lo; v <= hi; v += step) {
			if (v >= min && v <= max) allowed.add(v);
		}
	}
	return (value: number) => allowed.has(value);
}

/** True when `date` matches the 5-field cron expression. Invalid expressions never match. */
export function matchesCron(expr: string, date: Date): boolean {
	const fields = expr.trim().split(/\s+/);
	if (fields.length !== 5) return false;

	const [m, h, dom, mon, dow] = fields;
	const minute = parseField(m, 0, 59)(date.getMinutes());
	const hour = parseField(h, 0, 23)(date.getHours());
	const month = parseField(mon, 1, 12)(date.getMonth() + 1);
	// day-of-week: cron allows 0 or 7 for Sunday; normalize 7→0.
	const dowVal = date.getDay();
	const domField = parseField(dom, 1, 31);
	const dowField = parseField(dow.replace(/7/g, "0"), 0, 6);

	if (!(minute && hour && month)) return false;

	// Standard cron semantics: when both DOM and DOW are restricted, match either.
	const domRestricted = dom !== "*";
	const dowRestricted = dow !== "*";
	if (domRestricted && dowRestricted) {
		return domField(date.getDate()) || dowField(dowVal);
	}
	return domField(date.getDate()) && dowField(dowVal);
}

let idCounter = 0;
function newId(): string {
	idCounter += 1;
	return `task_${Date.now().toString(36)}${idCounter.toString(36)}`;
}

export interface TaskSchedulerOptions {
	/** Path to the durable JSON store (written on every mutation). */
	storePath: string;
	/**
	 * Legacy store path read only when {@link storePath} does not yet exist, so
	 * tasks scheduled under an older location migrate forward on first persist.
	 */
	legacyStorePath?: string;
	/** Submit a due task's prompt (e.g. via sendUserMessage). */
	fire: (prompt: string) => void;
	/** Whether the agent is idle; tasks only fire when true. Defaults to always-idle. */
	isIdle?: () => boolean;
	/** Tick interval in ms. Defaults to 30s. */
	intervalMs?: number;
}

export class TaskScheduler {
	private tasks: ScheduledTask[] = [];
	private timer: ReturnType<typeof setInterval> | undefined;
	private readonly opts: Required<Pick<TaskSchedulerOptions, "storePath" | "fire">> &
		Pick<TaskSchedulerOptions, "isIdle" | "intervalMs" | "legacyStorePath">;

	constructor(opts: TaskSchedulerOptions) {
		this.opts = opts;
		this.load();
	}

	/** Load persisted tasks from disk (best-effort). Falls back to the legacy
	 *  store path when the primary one does not exist yet. */
	load(): void {
		const source =
			existsSync(this.opts.storePath) || !this.opts.legacyStorePath
				? this.opts.storePath
				: existsSync(this.opts.legacyStorePath)
					? this.opts.legacyStorePath
					: this.opts.storePath;
		try {
			if (existsSync(source)) {
				const parsed = JSON.parse(readFileSync(source, "utf8")) as { tasks?: ScheduledTask[] };
				this.tasks = Array.isArray(parsed.tasks) ? parsed.tasks : [];
			}
		} catch {
			this.tasks = [];
		}
	}

	private persist(): void {
		try {
			mkdirSync(dirname(this.opts.storePath), { recursive: true });
			writeFileSync(this.opts.storePath, `${JSON.stringify({ tasks: this.tasks }, null, 2)}\n`, "utf8");
		} catch {
			// best-effort persistence
		}
	}

	/** Begin the tick loop. Safe to call once. */
	start(): void {
		if (this.timer) return;
		this.timer = setInterval(() => this.tick(new Date()), this.opts.intervalMs ?? 30_000);
		this.timer.unref?.();
	}

	stop(): void {
		if (this.timer) {
			clearInterval(this.timer);
			this.timer = undefined;
		}
	}

	create(input: { cron: string; prompt: string; recurring?: boolean }): ScheduledTask {
		const task: ScheduledTask = {
			id: newId(),
			cron: input.cron,
			prompt: input.prompt,
			recurring: input.recurring ?? true,
			createdAt: Date.now(),
		};
		this.tasks.push(task);
		this.persist();
		return task;
	}

	list(): ScheduledTask[] {
		return [...this.tasks];
	}

	delete(id: string): boolean {
		const before = this.tasks.length;
		this.tasks = this.tasks.filter((t) => t.id !== id);
		const removed = this.tasks.length < before;
		if (removed) this.persist();
		return removed;
	}

	clear(): void {
		if (this.tasks.length === 0) return;
		this.tasks = [];
		this.persist();
	}

	/** Evaluate all tasks against `now` and fire those that are due (when idle). */
	tick(now: Date): void {
		const idle = this.opts.isIdle ? this.opts.isIdle() : true;
		if (!idle) return;

		const minuteKey = Math.floor(now.getTime() / 60_000);
		let mutated = false;
		const toDelete: string[] = [];

		for (const task of this.tasks) {
			if (task.lastRunMinute === minuteKey) continue;
			if (!matchesCron(task.cron, now)) continue;

			task.lastRunMinute = minuteKey;
			mutated = true;
			this.opts.fire(task.prompt);
			if (!task.recurring) toDelete.push(task.id);
		}

		if (toDelete.length > 0) {
			this.tasks = this.tasks.filter((t) => !toDelete.includes(t.id));
		}
		if (mutated || toDelete.length > 0) this.persist();
	}
}
