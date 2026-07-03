/**
 * Compaction controller for AgentSession.
 *
 * Owns manual compaction (the /compact flow) and automatic compaction (overflow
 * recovery and threshold-triggered). It runs the shared apply pipeline — the
 * `session_before_compact` extension hook, summary generation, persistence,
 * agent-context refresh, and the `session_compact` event — and manages the
 * abort controllers and one-shot overflow-recovery guard. Extracted from
 * agent-session.ts behind a narrow CompactionControllerDeps interface.
 */

import type {
	AgentMessage,
	CompactionPreparation,
	CompactionResult,
	ThinkingLevel,
} from "@kolisachint/hoocode-agent-core";
import {
	calculateContextTokens,
	compact,
	estimateContextTokens,
	prepareCompaction,
	shouldCompact,
} from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage, Model } from "@kolisachint/hoocode-ai";
import { isContextOverflow } from "@kolisachint/hoocode-ai";
import type { AgentSessionEvent } from "./agent-session.js";
import { formatNoModelSelectedMessage } from "./auth-guidance.js";
import type { ExtensionRunner, SessionBeforeCompactResult } from "./extensions/index.js";
import type { ModelRegistry } from "./model-registry.js";
import type { CompactionEntry, SessionEntry, SessionManager } from "./session-manager.js";
import { getLatestCompactionEntry } from "./session-manager.js";
import type { SettingsManager } from "./settings-manager.js";

/** Narrow dependencies the compaction controller needs from AgentSession. */
export interface CompactionControllerDeps {
	sessionManager: SessionManager;
	settingsManager: SettingsManager;
	modelRegistry: ModelRegistry;
	getModel(): Model<any> | undefined;
	getThinkingLevel(): ThinkingLevel;
	/** Read at call time; the extension runner is swapped on reload. */
	getExtensionRunner(): ExtensionRunner;
	getAgentMessages(): AgentMessage[];
	setAgentMessages(messages: AgentMessage[]): void;
	getRequiredRequestAuth(model: Model<any>): Promise<{ apiKey: string; headers?: Record<string, string> }>;
	emit(event: AgentSessionEvent): void;
	disconnectFromAgent(): void;
	reconnectToAgent(): void;
	/** Abort the current agent operation and wait for idle (AgentSession.abort). */
	abortSession(): Promise<void>;
	/** Fire-and-forget continue() on the agent. */
	continueAgent(): void;
	hasQueuedMessages(): boolean;
}

export class CompactionController {
	private _compactionAbortController: AbortController | undefined = undefined;
	private _autoCompactionAbortController: AbortController | undefined = undefined;
	private _overflowRecoveryAttempted = false;

	constructor(private readonly deps: CompactionControllerDeps) {}

	/** Whether manual or auto compaction is currently running */
	get isCompacting(): boolean {
		return this._autoCompactionAbortController !== undefined || this._compactionAbortController !== undefined;
	}

	/** Whether auto-compaction is enabled */
	get autoCompactionEnabled(): boolean {
		return this.deps.settingsManager.getCompactionEnabled();
	}

	/** Toggle auto-compaction setting. */
	setAutoCompactionEnabled(enabled: boolean): void {
		this.deps.settingsManager.setCompactionEnabled(enabled);
	}

	/** Clear the one-shot overflow-recovery guard (on new user input / successful response). */
	resetOverflowRecovery(): void {
		this._overflowRecoveryAttempted = false;
	}

	/** Cancel in-progress compaction (manual or auto). */
	abortCompaction(): void {
		this._compactionAbortController?.abort();
		this._autoCompactionAbortController?.abort();
	}

	/**
	 * Shared core for manual and auto compaction.
	 *
	 * Runs the `session_before_compact` extension hook, produces the compaction
	 * (from an extension or by summarizing), persists it, updates agent context,
	 * and emits `session_compact`. Returns `{ status: "cancelled" }` if an
	 * extension cancels or the signal aborts; callers map that to their own
	 * cancel handling (manual throws, auto emits).
	 */
	private async _applyCompaction(params: {
		preparation: CompactionPreparation;
		branchEntries: SessionEntry[];
		model: Model<any>;
		apiKey: string;
		headers?: Record<string, string>;
		customInstructions?: string;
		signal: AbortSignal;
	}): Promise<{ status: "ok"; result: CompactionResult } | { status: "cancelled" }> {
		const { preparation, branchEntries, model, apiKey, headers, customInstructions, signal } = params;
		const extensionRunner = this.deps.getExtensionRunner();

		let extensionCompaction: CompactionResult | undefined;
		let fromExtension = false;

		if (extensionRunner.hasHandlers("session_before_compact")) {
			const result = (await extensionRunner.emit({
				type: "session_before_compact",
				preparation,
				branchEntries,
				customInstructions,
				signal,
			})) as SessionBeforeCompactResult | undefined;

			if (result?.cancel) {
				return { status: "cancelled" };
			}
			if (result?.compaction) {
				extensionCompaction = result.compaction;
				fromExtension = true;
			}
		}

		const generated =
			extensionCompaction ??
			(await compact(preparation, model, apiKey, headers, customInstructions, signal, this.deps.getThinkingLevel()));

		if (signal.aborted) {
			return { status: "cancelled" };
		}

		const { summary, firstKeptEntryId, tokensBefore, tokensAfter, details } = generated;

		this.deps.sessionManager.appendCompaction(
			summary,
			firstKeptEntryId,
			tokensBefore,
			details,
			fromExtension,
			tokensAfter,
		);
		const newEntries = this.deps.sessionManager.getEntries();
		this.deps.setAgentMessages(this.deps.sessionManager.buildSessionContext().messages);

		const savedCompactionEntry = newEntries.find((e) => e.type === "compaction" && e.summary === summary) as
			| CompactionEntry
			| undefined;
		if (savedCompactionEntry) {
			await extensionRunner.emit({
				type: "session_compact",
				compactionEntry: savedCompactionEntry,
				fromExtension,
			});
		}

		return {
			status: "ok",
			result: { summary, firstKeptEntryId, tokensBefore, tokensAfter: tokensAfter ?? tokensBefore, details },
		};
	}

	/**
	 * Manually compact the session context.
	 * Aborts current agent operation first.
	 * @param customInstructions Optional instructions for the compaction summary
	 */
	async compact(customInstructions?: string): Promise<CompactionResult> {
		this.deps.disconnectFromAgent();
		await this.deps.abortSession();
		this._compactionAbortController = new AbortController();
		this.deps.emit({ type: "compaction_start", reason: "manual" });

		try {
			const model = this.deps.getModel();
			if (!model) {
				throw new Error(formatNoModelSelectedMessage());
			}

			const { apiKey, headers } = await this.deps.getRequiredRequestAuth(model);

			const pathEntries = this.deps.sessionManager.getBranch();
			const settings = this.deps.settingsManager.getCompactionSettings();

			const preparation = prepareCompaction(pathEntries, settings);
			if (!preparation) {
				// Check why we can't compact
				const lastEntry = pathEntries[pathEntries.length - 1];
				if (lastEntry?.type === "compaction") {
					throw new Error("Already compacted");
				}
				throw new Error("Nothing to compact (session too small)");
			}

			const applied = await this._applyCompaction({
				preparation,
				branchEntries: pathEntries,
				model,
				apiKey,
				headers,
				customInstructions,
				signal: this._compactionAbortController.signal,
			});

			if (applied.status === "cancelled") {
				throw new Error("Compaction cancelled");
			}

			const compactionResult = applied.result;
			this.deps.emit({
				type: "compaction_end",
				reason: "manual",
				result: compactionResult,
				aborted: false,
				willRetry: false,
			});
			return compactionResult;
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			const aborted = message === "Compaction cancelled" || (error instanceof Error && error.name === "AbortError");
			this.deps.emit({
				type: "compaction_end",
				reason: "manual",
				result: undefined,
				aborted,
				willRetry: false,
				errorMessage: aborted ? undefined : `Compaction failed: ${message}`,
			});
			throw error;
		} finally {
			this._compactionAbortController = undefined;
			this.deps.reconnectToAgent();
		}
	}

	/**
	 * Check if compaction is needed and run it.
	 * Called after agent_end and before prompt submission.
	 *
	 * Two cases:
	 * 1. Overflow: LLM returned context overflow error, remove error message from agent state, compact, auto-retry
	 * 2. Threshold: Context over threshold, compact, NO auto-retry (user continues manually)
	 *
	 * @param assistantMessage The assistant message to check
	 * @param skipAbortedCheck If false, include aborted messages (for pre-prompt check). Default: true
	 */
	async checkCompaction(assistantMessage: AssistantMessage, skipAbortedCheck = true): Promise<void> {
		const settings = this.deps.settingsManager.getCompactionSettings();
		if (!settings.enabled) return;

		// Skip if message was aborted (user cancelled) - unless skipAbortedCheck is false
		if (skipAbortedCheck && assistantMessage.stopReason === "aborted") return;

		const model = this.deps.getModel();
		const contextWindow = model?.contextWindow ?? 0;

		// Skip overflow check if the message came from a different model.
		// This handles the case where user switched from a smaller-context model (e.g. opus)
		// to a larger-context model (e.g. codex) - the overflow error from the old model
		// shouldn't trigger compaction for the new model.
		const sameModel = model && assistantMessage.provider === model.provider && assistantMessage.model === model.id;

		// Skip compaction checks if this assistant message is older than the latest
		// compaction boundary. This prevents a stale pre-compaction usage/error
		// from retriggering compaction on the first prompt after compaction.
		const compactionEntry = getLatestCompactionEntry(this.deps.sessionManager.getBranch());
		const assistantIsFromBeforeCompaction =
			compactionEntry !== null && assistantMessage.timestamp <= new Date(compactionEntry.timestamp).getTime();
		if (assistantIsFromBeforeCompaction) {
			return;
		}

		// Case 1: Overflow - LLM returned context overflow error
		if (sameModel && isContextOverflow(assistantMessage, contextWindow)) {
			if (this._overflowRecoveryAttempted) {
				this.deps.emit({
					type: "compaction_end",
					reason: "overflow",
					result: undefined,
					aborted: false,
					willRetry: false,
					errorMessage:
						"Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model.",
				});
				return;
			}

			this._overflowRecoveryAttempted = true;
			// Remove the error message from agent state (it IS saved to session for history,
			// but we don't want it in context for the retry)
			const messages = this.deps.getAgentMessages();
			if (messages.length > 0 && messages[messages.length - 1].role === "assistant") {
				this.deps.setAgentMessages(messages.slice(0, -1));
			}
			await this._runAutoCompaction("overflow", true);
			return;
		}

		// Case 2: Threshold - context is getting large
		// For error messages (no usage data), estimate from last successful response.
		// This ensures sessions that hit persistent API errors (e.g. 529) can still compact.
		let contextTokens: number;
		if (assistantMessage.stopReason === "error") {
			const messages = this.deps.getAgentMessages();
			const estimate = estimateContextTokens(messages);
			if (estimate.lastUsageIndex === null) return; // No usage data at all
			// Verify the usage source is post-compaction. Kept pre-compaction messages
			// have stale usage reflecting the old (larger) context and would falsely
			// trigger compaction right after one just finished.
			const usageMsg = messages[estimate.lastUsageIndex];
			if (
				compactionEntry &&
				usageMsg.role === "assistant" &&
				(usageMsg as AssistantMessage).timestamp <= new Date(compactionEntry.timestamp).getTime()
			) {
				return;
			}
			contextTokens = estimate.tokens;
		} else {
			contextTokens = calculateContextTokens(assistantMessage.usage);
		}
		if (shouldCompact(contextTokens, contextWindow, settings)) {
			await this._runAutoCompaction("threshold", false);
		}
	}

	/**
	 * Internal: Run auto-compaction with events.
	 */
	private async _runAutoCompaction(reason: "overflow" | "threshold", willRetry: boolean): Promise<void> {
		const settings = this.deps.settingsManager.getCompactionSettings();

		this.deps.emit({ type: "compaction_start", reason });
		this._autoCompactionAbortController = new AbortController();

		try {
			const model = this.deps.getModel();
			if (!model) {
				this.deps.emit({
					type: "compaction_end",
					reason,
					result: undefined,
					aborted: false,
					willRetry: false,
				});
				return;
			}

			const authResult = await this.deps.modelRegistry.getApiKeyAndHeaders(model);
			if (!authResult.ok || !authResult.apiKey) {
				this.deps.emit({
					type: "compaction_end",
					reason,
					result: undefined,
					aborted: false,
					willRetry: false,
				});
				return;
			}
			const { apiKey, headers } = authResult;

			const pathEntries = this.deps.sessionManager.getBranch();

			const preparation = prepareCompaction(pathEntries, settings);
			if (!preparation) {
				this.deps.emit({
					type: "compaction_end",
					reason,
					result: undefined,
					aborted: false,
					willRetry: false,
				});
				return;
			}

			const applied = await this._applyCompaction({
				preparation,
				branchEntries: pathEntries,
				model,
				apiKey,
				headers,
				customInstructions: undefined,
				signal: this._autoCompactionAbortController.signal,
			});

			if (applied.status === "cancelled") {
				this.deps.emit({
					type: "compaction_end",
					reason,
					result: undefined,
					aborted: true,
					willRetry: false,
				});
				return;
			}

			const result = applied.result;
			this.deps.emit({ type: "compaction_end", reason, result, aborted: false, willRetry });

			if (willRetry) {
				const messages = this.deps.getAgentMessages();
				const lastMsg = messages[messages.length - 1];
				if (lastMsg?.role === "assistant" && (lastMsg as AssistantMessage).stopReason === "error") {
					this.deps.setAgentMessages(messages.slice(0, -1));
				}

				setTimeout(() => {
					this.deps.continueAgent();
				}, 100);
			} else if (this.deps.hasQueuedMessages()) {
				// Auto-compaction can complete while follow-up/steering/custom messages are waiting.
				// Kick the loop so queued messages are actually delivered.
				setTimeout(() => {
					this.deps.continueAgent();
				}, 100);
			}
		} catch (error) {
			const errorMessage = error instanceof Error ? error.message : "compaction failed";
			this.deps.emit({
				type: "compaction_end",
				reason,
				result: undefined,
				aborted: false,
				willRetry: false,
				errorMessage:
					reason === "overflow"
						? `Context overflow recovery failed: ${errorMessage}`
						: `Auto-compaction failed: ${errorMessage}`,
			});
		} finally {
			this._autoCompactionAbortController = undefined;
		}
	}
}
