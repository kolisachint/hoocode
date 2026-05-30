import { EventEmitter } from "node:events";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { CONFIG_DIR_NAME } from "../config.js";

export interface TokenBudgetConfig {
	/** Budget limit in tokens. */
	limit: number;
}

export interface TokenBudgetState {
	task_id: string;
	agent_type: string;
	budget: number;
	used: number;
	warned: boolean;
	exceeded: boolean;
	last_updated: number;
}

/**
 * Default token budgets per agent type (in tokens).
 * - explore: 8 000
 * - edit: 16 000
 * - test: 16 000
 * - review: 12 000
 * - doc: 10 000
 */
const DEFAULT_BUDGETS: Record<string, number> = {
	explore: 8000,
	edit: 16000,
	test: 16000,
	review: 12000,
	doc: 10000,
};

/** Get the default budget for an agent type. */
export function getDefaultBudget(agent_type: string): number {
	return DEFAULT_BUDGETS[agent_type] ?? 16000;
}

/**
 * Tracks cumulative token usage for a single subagent task by parsing
 * newline-delimited JSON events from the subagent's stdout stream.
 *
 * Emits:
 * - "budget_warning" when 80% of the budget is consumed
 * - "budget_exceeded" when 100% of the budget is consumed
 */
export class TokenBudget extends EventEmitter {
	private readonly task_id: string;
	private readonly agent_type: string;
	private readonly limit: number;
	private readonly cwd: string;

	private used = 0;
	private warned = false;
	private exceeded = false;
	private stdoutBuffer = "";

	/** Warning threshold (80%). */
	private readonly warningThreshold: number;
	/** Hard-stop threshold (100%). */
	private readonly exceededThreshold: number;

	constructor(task_id: string, agent_type: string, options: { limit?: number; cwd?: string } = {}) {
		super();
		this.task_id = task_id;
		this.agent_type = agent_type;
		this.limit = options.limit ?? getDefaultBudget(agent_type);
		this.cwd = options.cwd ?? process.cwd();
		this.warningThreshold = Math.floor(this.limit * 0.8);
		this.exceededThreshold = this.limit;
	}

	/** Process a chunk of stdout data from the subagent. */
	processStdout(chunk: string): void {
		this.stdoutBuffer += chunk;

		while (true) {
			const lineEnd = this.stdoutBuffer.indexOf("\n");
			if (lineEnd === -1) break;
			const line = this.stdoutBuffer.slice(0, lineEnd).trimEnd();
			this.stdoutBuffer = this.stdoutBuffer.slice(lineEnd + 1);
			if (line) {
				this.parseLine(line);
			}
		}
	}

	/** Flush any remaining buffered stdout. Call when the stream ends. */
	flush(): void {
		if (this.stdoutBuffer.trim()) {
			this.parseLine(this.stdoutBuffer.trim());
			this.stdoutBuffer = "";
		}
	}

	/** Current cumulative token usage. */
	getUsed(): number {
		return this.used;
	}

	/** Configured token budget limit. */
	getLimit(): number {
		return this.limit;
	}

	/** Whether the budget warning has been triggered. */
	isWarned(): boolean {
		return this.warned;
	}

	/** Whether the budget has been exceeded. */
	isExceeded(): boolean {
		return this.exceeded;
	}

	/** Persist current budget state to disk. */
	persist(): void {
		const state: TokenBudgetState = {
			task_id: this.task_id,
			agent_type: this.agent_type,
			budget: this.limit,
			used: this.used,
			warned: this.warned,
			exceeded: this.exceeded,
			last_updated: Date.now(),
		};

		const path = this.budgetPath();
		try {
			mkdirSync(dirname(path), { recursive: true });
			writeFileSync(path, JSON.stringify(state, null, 2));
		} catch {
			// Persistence is best-effort; silently ignore write failures
		}
	}

	private budgetPath(): string {
		return join(this.cwd, CONFIG_DIR_NAME, "agents", this.task_id, "budget.json");
	}

	private parseLine(line: string): void {
		let event: unknown;
		try {
			event = JSON.parse(line);
		} catch {
			return; // Not valid JSON, ignore
		}

		if (!event || typeof event !== "object") return;

		// Look for message_end events with assistant messages that have usage
		const typedEvent = event as Record<string, unknown>;
		if (typedEvent.type !== "message_end") return;

		const message = typedEvent.message as Record<string, unknown> | undefined;
		if (!message) return;
		if (message.role !== "assistant") return;

		const usage = message.usage as Record<string, unknown> | undefined;
		if (!usage) return;

		const totalTokens = typeof usage.totalTokens === "number" ? usage.totalTokens : undefined;
		if (totalTokens === undefined || totalTokens <= 0) return;

		this.used += totalTokens;

		// Check thresholds
		if (!this.warned && this.used >= this.warningThreshold) {
			this.warned = true;
			this.emit("budget_warning", {
				task_id: this.task_id,
				message: "You are near token limit. Summarize and write result.json now.",
				used: this.used,
				limit: this.limit,
			});
		}

		if (!this.exceeded && this.used >= this.exceededThreshold) {
			this.exceeded = true;
			this.emit("budget_exceeded", {
				task_id: this.task_id,
				used: this.used,
				limit: this.limit,
			});
		}

		this.persist();
	}
}
