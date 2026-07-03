/**
 * Tree-navigation controller for AgentSession.
 *
 * Handles navigating to a different node in the session tree (staying in the
 * same session file, unlike fork). When the user opts to summarize the
 * abandoned branch it runs the `session_before_tree` hook, generates a branch
 * summary (from an extension or the default summarizer), attaches it at the new
 * leaf, refreshes agent context, and emits `session_tree`. Owns the branch
 * summarization abort controller. Extracted from agent-session.ts behind a
 * narrow TreeNavigationControllerDeps interface.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { collectEntriesForBranchSummary, generateBranchSummary } from "@kolisachint/hoocode-agent-core";
import type { Model } from "@kolisachint/hoocode-ai";
import { extractUserMessageText } from "./agent-session-stats.js";
import type { ExtensionRunner, SessionBeforeTreeResult, TreePreparation } from "./extensions/index.js";
import type { BranchSummaryEntry, SessionManager } from "./session-manager.js";
import type { SettingsManager } from "./settings-manager.js";

/** Options for navigating the session tree. */
export interface NavigateTreeOptions {
	/** Whether the user wants to summarize the abandoned branch */
	summarize?: boolean;
	/** Custom instructions for the summarizer */
	customInstructions?: string;
	/** If true, customInstructions replaces the default prompt */
	replaceInstructions?: boolean;
	/** Label to attach to the branch summary entry */
	label?: string;
}

/** Result of navigating the session tree. */
export interface NavigateTreeResult {
	editorText?: string;
	cancelled: boolean;
	aborted?: boolean;
	summaryEntry?: BranchSummaryEntry;
}

/** Narrow dependencies the tree-navigation controller needs from AgentSession. */
export interface TreeNavigationControllerDeps {
	sessionManager: SessionManager;
	settingsManager: SettingsManager;
	getModel(): Model<any> | undefined;
	/** Read at call time; the extension runner is swapped on reload. */
	getExtensionRunner(): ExtensionRunner;
	getRequiredRequestAuth(model: Model<any>): Promise<{ apiKey: string; headers?: Record<string, string> }>;
	setAgentMessages(messages: AgentMessage[]): void;
}

export class TreeNavigationController {
	private _branchSummaryAbortController: AbortController | undefined = undefined;

	constructor(private readonly deps: TreeNavigationControllerDeps) {}

	/** Whether branch summarization is currently running */
	get isSummarizing(): boolean {
		return this._branchSummaryAbortController !== undefined;
	}

	/** Cancel in-progress branch summarization. */
	abortBranchSummary(): void {
		this._branchSummaryAbortController?.abort();
	}

	/**
	 * Navigate to a different node in the session tree.
	 * Unlike fork() which creates a new session file, this stays in the same file.
	 *
	 * @param targetId The entry ID to navigate to
	 * @param options.summarize Whether user wants to summarize abandoned branch
	 * @param options.customInstructions Custom instructions for summarizer
	 * @param options.replaceInstructions If true, customInstructions replaces the default prompt
	 * @param options.label Label to attach to the branch summary entry
	 * @returns Result with editorText (if user message) and cancelled status
	 */
	async navigateTree(targetId: string, options: NavigateTreeOptions = {}): Promise<NavigateTreeResult> {
		const sessionManager = this.deps.sessionManager;
		const extensionRunner = this.deps.getExtensionRunner();
		const oldLeafId = sessionManager.getLeafId();

		// No-op if already at target
		if (targetId === oldLeafId) {
			return { cancelled: false };
		}

		// Model required for summarization
		if (options.summarize && !this.deps.getModel()) {
			throw new Error("No model available for summarization");
		}

		const targetEntry = sessionManager.getEntry(targetId);
		if (!targetEntry) {
			throw new Error(`Entry ${targetId} not found`);
		}

		// Collect entries to summarize (from old leaf to common ancestor)
		const { entries: entriesToSummarize, commonAncestorId } = await collectEntriesForBranchSummary(
			sessionManager,
			oldLeafId,
			targetId,
		);

		// Prepare event data - mutable so extensions can override
		let customInstructions = options.customInstructions;
		let replaceInstructions = options.replaceInstructions;
		let label = options.label;

		const preparation: TreePreparation = {
			targetId,
			oldLeafId,
			commonAncestorId,
			entriesToSummarize,
			userWantsSummary: options.summarize ?? false,
			customInstructions,
			replaceInstructions,
			label,
		};

		// Set up abort controller for summarization
		this._branchSummaryAbortController = new AbortController();

		try {
			let extensionSummary: { summary: string; details?: unknown } | undefined;
			let fromExtension = false;

			// Emit session_before_tree event
			if (extensionRunner.hasHandlers("session_before_tree")) {
				const result = (await extensionRunner.emit({
					type: "session_before_tree",
					preparation,
					signal: this._branchSummaryAbortController.signal,
				})) as SessionBeforeTreeResult | undefined;

				if (result?.cancel) {
					return { cancelled: true };
				}

				if (result?.summary && options.summarize) {
					extensionSummary = result.summary;
					fromExtension = true;
				}

				// Allow extensions to override instructions and label
				if (result?.customInstructions !== undefined) {
					customInstructions = result.customInstructions;
				}
				if (result?.replaceInstructions !== undefined) {
					replaceInstructions = result.replaceInstructions;
				}
				if (result?.label !== undefined) {
					label = result.label;
				}
			}

			// Run default summarizer if needed
			let summaryText: string | undefined;
			let summaryDetails: unknown;
			if (options.summarize && entriesToSummarize.length > 0 && !extensionSummary) {
				const model = this.deps.getModel()!;
				const { apiKey, headers } = await this.deps.getRequiredRequestAuth(model);
				const branchSummarySettings = this.deps.settingsManager.getBranchSummarySettings();
				const result = await generateBranchSummary(entriesToSummarize, {
					model,
					apiKey,
					headers,
					signal: this._branchSummaryAbortController.signal,
					customInstructions,
					replaceInstructions,
					reserveTokens: branchSummarySettings.reserveTokens,
				});
				if (result.aborted) {
					return { cancelled: true, aborted: true };
				}
				if (result.error) {
					throw new Error(result.error);
				}
				summaryText = result.summary;
				summaryDetails = {
					readFiles: result.readFiles || [],
					modifiedFiles: result.modifiedFiles || [],
				};
			} else if (extensionSummary) {
				summaryText = extensionSummary.summary;
				summaryDetails = extensionSummary.details;
			}

			// Determine the new leaf position based on target type
			let newLeafId: string | null;
			let editorText: string | undefined;

			if (targetEntry.type === "message" && targetEntry.message.role === "user") {
				// User message: leaf = parent (null if root), text goes to editor
				newLeafId = targetEntry.parentId;
				editorText = extractUserMessageText(targetEntry.message.content);
			} else if (targetEntry.type === "custom_message") {
				// Custom message: leaf = parent (null if root), text goes to editor
				newLeafId = targetEntry.parentId;
				editorText =
					typeof targetEntry.content === "string"
						? targetEntry.content
						: targetEntry.content
								.filter((c): c is { type: "text"; text: string } => c.type === "text")
								.map((c) => c.text)
								.join("");
			} else {
				// Non-user message: leaf = selected node
				newLeafId = targetId;
			}

			// Switch leaf (with or without summary)
			// Summary is attached at the navigation target position (newLeafId), not the old branch
			let summaryEntry: BranchSummaryEntry | undefined;
			if (summaryText) {
				// Create summary at target position (can be null for root)
				const summaryId = sessionManager.branchWithSummary(newLeafId, summaryText, summaryDetails, fromExtension);
				summaryEntry = sessionManager.getEntry(summaryId) as BranchSummaryEntry;

				// Attach label to the summary entry
				if (label) {
					sessionManager.appendLabelChange(summaryId, label);
				}
			} else if (newLeafId === null) {
				// No summary, navigating to root - reset leaf
				sessionManager.resetLeaf();
			} else {
				// No summary, navigating to non-root
				sessionManager.branch(newLeafId);
			}

			// Attach label to target entry when not summarizing (no summary entry to label)
			if (label && !summaryText) {
				sessionManager.appendLabelChange(targetId, label);
			}

			// Update agent state
			const sessionContext = sessionManager.buildSessionContext();
			this.deps.setAgentMessages(sessionContext.messages);

			// Emit session_tree event
			await extensionRunner.emit({
				type: "session_tree",
				newLeafId: sessionManager.getLeafId(),
				oldLeafId,
				summaryEntry,
				fromExtension: summaryText ? fromExtension : undefined,
			});

			return { editorText, cancelled: false, summaryEntry };
		} finally {
			this._branchSummaryAbortController = undefined;
		}
	}
}
