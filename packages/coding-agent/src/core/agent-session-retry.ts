/**
 * Auto-retry controller for AgentSession.
 *
 * Owns the retry lifecycle for transient assistant errors (overloaded, rate
 * limit, server/network/transport failures): deciding whether an error is
 * retryable, arming the retry promise synchronously on agent_end, backing off
 * exponentially, and re-driving the agent via continue(). Context-overflow
 * errors are intentionally excluded here — those are handled by compaction.
 */

import type { AgentEvent, AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage, Model } from "@kolisachint/hoocode-ai";
import { isContextOverflow } from "@kolisachint/hoocode-ai";
import { sleep } from "../utils/sleep.js";
import type { AgentSessionEvent } from "./agent-session.js";

/**
 * Retryable error signatures (overloaded, rate limit, server/network errors,
 * transport closes). Compiled once at module load instead of on every assistant
 * response. Context-overflow errors are handled separately by compaction.
 */
const RETRYABLE_ERROR_PATTERN =
	/overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|connection.?lost|websocket.?closed|websocket.?error|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|http2 request did not get a response|timed? out|timeout|terminated|retry delay/i;

/** Narrow dependencies the retry controller needs from AgentSession. */
export interface AutoRetryDeps {
	getRetrySettings(): { enabled: boolean; maxRetries: number; baseDelayMs: number };
	getModel(): Model<any> | undefined;
	getAgentMessages(): AgentMessage[];
	setAgentMessages(messages: AgentMessage[]): void;
	/** Fire-and-forget continue() on the agent (errors surface on the next agent_end). */
	continueAgent(): void;
	waitForAgentIdle(): Promise<void>;
	emit(event: AgentSessionEvent): void;
}

export class AutoRetryController {
	private _abortController: AbortController | undefined = undefined;
	private _attempt = 0;
	private _promise: Promise<void> | undefined = undefined;
	private _resolve: (() => void) | undefined = undefined;

	constructor(private readonly deps: AutoRetryDeps) {}

	/** Current retry attempt (0 if not retrying) */
	get attempt(): number {
		return this._attempt;
	}

	/** Whether a retry is currently in progress */
	get isRetrying(): boolean {
		return this._promise !== undefined;
	}

	/**
	 * Check if an error is retryable (overloaded, rate limit, server errors).
	 * Context overflow errors are NOT retryable (handled by compaction instead).
	 */
	isRetryableError(message: AssistantMessage): boolean {
		if (message.stopReason !== "error" || !message.errorMessage) return false;

		// Context overflow is handled by compaction, not retry
		const contextWindow = this.deps.getModel()?.contextWindow ?? 0;
		if (isContextOverflow(message, contextWindow)) return false;

		return RETRYABLE_ERROR_PATTERN.test(message.errorMessage);
	}

	/**
	 * Create the retry promise synchronously when an agent_end carries a
	 * retryable error. Agent.emit() runs handlers synchronously and prompt()
	 * calls waitForRetry() as soon as agent.prompt() resolves; arming the promise
	 * here (rather than inside async event processing) ensures waitForRetry()
	 * never misses an in-flight retry.
	 */
	createPromiseForAgentEnd(event: AgentEvent): void {
		if (event.type !== "agent_end" || this._promise) {
			return;
		}

		const settings = this.deps.getRetrySettings();
		if (!settings.enabled) {
			return;
		}

		const lastAssistant = this._findLastAssistantInMessages(event.messages);
		if (!lastAssistant || !this.isRetryableError(lastAssistant)) {
			return;
		}

		this._promise = new Promise((resolve) => {
			this._resolve = resolve;
		});
	}

	private _findLastAssistantInMessages(messages: AgentMessage[]): AssistantMessage | undefined {
		for (let i = messages.length - 1; i >= 0; i--) {
			const message = messages[i];
			if (message.role === "assistant") {
				return message as AssistantMessage;
			}
		}
		return undefined;
	}

	/**
	 * Reset the attempt counter after a successful assistant response.
	 * Callers invoke this only for non-error responses; it emits a success event
	 * when a retry was in progress.
	 */
	onSuccessfulAssistantResponse(): void {
		if (this._attempt > 0) {
			this.deps.emit({
				type: "auto_retry_end",
				success: true,
				attempt: this._attempt,
			});
			this._attempt = 0;
		}
	}

	/** Resolve the pending retry promise */
	resolve(): void {
		if (this._resolve) {
			this._resolve();
			this._resolve = undefined;
			this._promise = undefined;
		}
	}

	/**
	 * Handle retryable errors with exponential backoff.
	 * @returns true if retry was initiated, false if max retries exceeded or disabled
	 */
	async handleRetryableError(message: AssistantMessage): Promise<boolean> {
		const settings = this.deps.getRetrySettings();
		if (!settings.enabled) {
			this.resolve();
			return false;
		}

		// Retry promise is created synchronously in createPromiseForAgentEnd for agent_end.
		// Keep a defensive fallback here in case a future refactor bypasses that path.
		if (!this._promise) {
			this._promise = new Promise((resolve) => {
				this._resolve = resolve;
			});
		}

		this._attempt++;

		if (this._attempt > settings.maxRetries) {
			// Max retries exceeded, emit final failure and reset
			this.deps.emit({
				type: "auto_retry_end",
				success: false,
				attempt: this._attempt - 1,
				finalError: message.errorMessage,
			});
			this._attempt = 0;
			this.resolve(); // Resolve so waitForRetry() completes
			return false;
		}

		const delayMs = settings.baseDelayMs * 2 ** (this._attempt - 1);

		this.deps.emit({
			type: "auto_retry_start",
			attempt: this._attempt,
			maxAttempts: settings.maxRetries,
			delayMs,
			errorMessage: message.errorMessage || "Unknown error",
		});

		// Remove error message from agent state (keep in session for history)
		const messages = this.deps.getAgentMessages();
		if (messages.length > 0 && messages[messages.length - 1].role === "assistant") {
			this.deps.setAgentMessages(messages.slice(0, -1));
		}

		// Wait with exponential backoff (abortable)
		this._abortController = new AbortController();
		try {
			await sleep(delayMs, this._abortController.signal);
		} catch {
			// Aborted during sleep - emit end event so UI can clean up
			const attempt = this._attempt;
			this._attempt = 0;
			this._abortController = undefined;
			this.deps.emit({
				type: "auto_retry_end",
				success: false,
				attempt,
				finalError: "Retry cancelled",
			});
			this.resolve();
			return false;
		}
		this._abortController = undefined;

		// Retry via continue() - use setTimeout to break out of event handler chain
		setTimeout(() => {
			this.deps.continueAgent();
		}, 0);

		return true;
	}

	/**
	 * Cancel in-progress retry.
	 */
	abort(): void {
		this._abortController?.abort();
		// Note: _attempt is reset in the catch block of handleRetryableError
		this.resolve();
	}

	/**
	 * Wait for any in-progress retry to complete.
	 * Returns immediately if no retry is in progress.
	 */
	async waitForRetry(): Promise<void> {
		if (!this._promise) {
			return;
		}

		await this._promise;
		await this.deps.waitForAgentIdle();
	}
}
