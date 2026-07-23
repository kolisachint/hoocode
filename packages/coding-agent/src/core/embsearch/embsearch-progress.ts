/**
 * Routes semantic-index startup progress (`EmbsearchState`) to the right user
 * surface: in interactive mode a transient footer line via the shared
 * `startupProgress` store; otherwise a dim stderr log line.
 *
 * Extracted from main.ts so the mapping — which phase shows what, and which
 * phases stay silent — is unit-testable without booting the whole CLI.
 */

import chalk from "chalk";
import { startupProgress } from "../startup-progress.js";
import type { EmbsearchState } from "./embsearch-service.js";

/**
 * Stable footer key for the semantic index. The binary download and the index
 * build are one logical piece of startup work, so they share one evolving line
 * (Downloading… → Building…) that disappears once the index is ready/skipped.
 */
export const SEMANTIC_INDEX_PROGRESS_KEY = "semantic-index";

export interface EmbsearchProgressSink {
	/** True in the interactive TUI (route to the footer store); false logs to stderr. */
	interactive: boolean;
	/** Non-interactive log sink; defaults to a dim `console.error`. Injectable for tests. */
	log?: (message: string) => void;
}

/**
 * Apply one `EmbsearchState` update to the chosen surface.
 *
 * Interactive: downloading/indexing show a live footer line, unavailable shows a
 * transient error notice, and ready/skipped drop the line. It is deliberately
 * NOT a task-panel row — index progress is startup status, not agent work.
 *
 * Non-interactive: logs indexing/ready/skipped/unavailable, but stays silent on
 * the per-chunk `downloading` ticks (they would spam the log) and on `idle`.
 */
export function reportEmbsearchProgress(state: EmbsearchState, sink: EmbsearchProgressSink): void {
	if (sink.interactive) {
		switch (state.phase) {
			case "downloading":
				startupProgress.set({
					key: SEMANTIC_INDEX_PROGRESS_KEY,
					kind: "download",
					label: "Semantic search index",
					receivedBytes: state.receivedBytes,
					totalBytes: state.totalBytes,
				});
				return;
			case "indexing":
				startupProgress.set({
					key: SEMANTIC_INDEX_PROGRESS_KEY,
					kind: "work",
					label: "Building semantic search index",
					done: state.done,
					total: state.total,
					unit: "files",
				});
				return;
			case "ready":
			case "skipped":
				startupProgress.remove(SEMANTIC_INDEX_PROGRESS_KEY);
				return;
			case "unavailable":
				startupProgress.set({
					key: SEMANTIC_INDEX_PROGRESS_KEY,
					kind: "error",
					label: "Semantic search index unavailable",
					message: state.reason,
				});
				return;
			case "idle":
				return;
		}
	}

	const log = sink.log ?? ((message: string) => console.error(chalk.dim(message)));
	switch (state.phase) {
		case "indexing": {
			const pct = Math.round((state.done / state.total) * 100);
			log(`Building semantic search index – ${state.done}/${state.total} files (${pct}%)`);
			return;
		}
		case "ready":
			log(`Semantic search index ready (${state.chunkCount} chunks)`);
			return;
		case "skipped":
			log(`Semantic search index skipped (${state.reason})`);
			return;
		case "unavailable":
			log(`Semantic search index unavailable (${state.reason})`);
			return;
		default:
			// downloading (no per-chunk spam), idle: nothing to log.
			return;
	}
}
