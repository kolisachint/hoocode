/**
 * The external (Rust) binaries hoocode can use, and what each one is worth.
 *
 * hoocode is self-sufficient without any of them: `SearchCodebase` and @-file
 * autocomplete fall back to pure-JS implementations, and the features that have
 * no fallback (web, voice) are off or inert rather than broken. These binaries are an
 * *expansion* layer, which is exactly why they were invisible - nothing failed
 * loudly enough to tell anyone they existed.
 *
 * This module is the single description of that layer. The `/settings` pane
 * renders it; the same table says which settings rows are gated on a binary, so
 * a row that cannot do anything today can say so instead of lying.
 */

import { getToolStatus, isOfflineMode, type ManagedTool, type ManagedToolStatus } from "../utils/tools-manager.js";

/** How hoocode gets a binary it does not already have. */
export type Acquisition =
	/** Fetched in the background at startup. */
	| "startup"
	/** Fetched the first time the feature is actually used. */
	| "on-demand"
	/** Never fetched implicitly; install it yourself or point the env var at it. */
	| "manual";

export interface ExternalToolDoc {
	tool: ManagedTool;
	/** Row label in the pane. */
	label: string;
	/** What the binary does for hoocode, in one line. */
	summary: string;
	/** What it turns on, feature by feature. */
	enables: string[];
	/** What hoocode does instead when it is missing. Never "nothing works". */
	fallback: string;
	acquisition: Acquisition;
	/** Env vars that change how this binary is resolved or driven. */
	env: string[];
	/**
	 * `/settings` row ids whose effect depends on this binary. A row listed here
	 * is annotated (not hidden) while the binary is missing: hiding it would
	 * recreate the discoverability hole this whole surface exists to close.
	 */
	dependentRows: string[];
	/** settings.json keys this binary gates, for docs and search. */
	settingsKeys: string[];
}

/**
 * Ordered so the two that silently make everything faster come first, then the
 * three that add capability hoocode does not otherwise have.
 */
export const EXTERNAL_TOOLS: readonly ExternalToolDoc[] = [
	{
		tool: "rg",
		label: "ripgrep (rg)",
		summary: "Fast path for content search.",
		enables: ["the lexical half of search runs rg instead of the JS scanner"],
		fallback:
			"A pure-JS scanner produces the same match shape, so results are identical - it is materially slower on large trees and respects fewer ignore-file edge cases.",
		acquisition: "startup",
		env: ["HOOCODE_RG_BINARY", "HOOCODE_NATIVE_SEARCH=1 forces the JS path even when rg is present"],
		dependentRows: [],
		settingsKeys: [],
	},
	{
		tool: "fd",
		label: "fd",
		summary: "Fast path for filename search.",
		enables: ["@-file autocomplete lists paths with fd instead of the JS directory walker"],
		fallback:
			"A JS walker produces the same result shape - slower on large trees, and glob/ignore handling is the JS approximation rather than fd's.",
		acquisition: "startup",
		env: ["HOOCODE_FD_BINARY", "HOOCODE_NATIVE_SEARCH=1 forces the JS path even when fd is present"],
		dependentRows: [],
		settingsKeys: [],
	},
	{
		tool: "embsearch",
		label: "embsearch (semantic index)",
		summary: "Local embedding index. The only source of semantic ranking in hoocode.",
		enables: [
			"search fuses semantic hits with its lexical hits",
			"MCP/capability deferral ranks tools by meaning rather than keyword",
		],
		fallback:
			"search is lexical-only and capability lookup ranks lexically. Nothing errors; queries phrased by intent rather than by token simply rank worse. Requires the ONNX build - the mock build is rejected on purpose, because it would rank at random while looking healthy.",
		acquisition: "on-demand",
		env: ["HOOCODE_EMBSEARCH_BINARY"],
		dependentRows: ["group:embsearch"],
		settingsKeys: ["enableSemanticIndex", "embsearchBinaryPath", "embsearchThresholdBytes"],
	},
	{
		tool: "webtools",
		label: "webtools (webfetch/websearch)",
		summary: "The network layer. Without it hoocode has no way to reach the internet.",
		enables: ["the webfetch tool", "the websearch tool"],
		fallback:
			"Both tools return an error when called. The web tool group is off by default, so a missing binary is invisible until you turn the group on.",
		acquisition: "on-demand",
		env: ["HOOCODE_WEBTOOLS_BINARY", "HOOCODE_WEBTOOLS_TIMEOUT"],
		dependentRows: ["group:web", "webtools-timeout-secs"],
		settingsKeys: ["enableWebTools", "webtools.timeoutSecs"],
	},
	{
		tool: "voicetools",
		label: "voicetools (voice input)",
		summary: "Microphone capture and transcription for the TUI.",
		enables: ["push-to-talk voice input in the editor"],
		fallback: "Voice capture reports an error and never starts. Typing is unaffected.",
		acquisition: "on-demand",
		env: ["VOICETOOLS_BIN", "HOOCODE_VOICETOOLS_BINARY", "VOICETOOLS_SILENCE_MS"],
		dependentRows: ["voice-silence-ms"],
		settingsKeys: ["voice.silenceMs"],
	},
];

export interface ExternalToolStatus extends ExternalToolDoc, ManagedToolStatus {
	installed: boolean;
	/**
	 * Whether hoocode would fetch it if the feature were used right now. False in
	 * offline mode, and on Android where the published Linux builds do not run.
	 */
	downloadable: boolean;
}

/** Short, fixed-width-ish status word for the pane's value column. */
export function statusLabel(status: ExternalToolStatus): string {
	if (!status.installed) return status.downloadable ? "not installed" : "unavailable";
	switch (status.source) {
		case "override":
			return "env override";
		case "managed":
			return "installed";
		case "path":
			return "system";
		default:
			return "installed";
	}
}

/**
 * Resolve every external tool. Never downloads: this is what the pane shows
 * *before* the user opts into anything.
 */
export function describeExternalTools(): ExternalToolStatus[] {
	const offline = isOfflineMode();
	const android = process.platform === "android";
	return EXTERNAL_TOOLS.map((doc) => {
		const status = getToolStatus(doc.tool);
		return {
			...doc,
			...status,
			installed: status.path !== null,
			downloadable: !offline && !android && doc.acquisition !== "manual",
		};
	});
}

/** Index of pane-row id -> the tool it needs, for annotating gated rows. */
export function buildRowGates(statuses: readonly ExternalToolStatus[]): Map<string, ExternalToolStatus> {
	const gates = new Map<string, ExternalToolStatus>();
	for (const status of statuses) {
		for (const rowId of status.dependentRows) gates.set(rowId, status);
	}
	return gates;
}
