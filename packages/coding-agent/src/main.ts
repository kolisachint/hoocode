/**
 * Main entry point for the coding agent CLI.
 *
 * This file handles CLI argument parsing and translates them into
 * createAgentSession() options. The SDK does the heavy lifting.
 */

import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { type ImageContent, modelsAreEqual } from "@kolisachint/hoocode-ai";
import { ProcessTerminal, setKeybindings, TUI } from "@kolisachint/hoocode-tui";
import chalk from "chalk";
import { type Args, type Mode, parseArgs, printHelp } from "./cli/args.js";
import { processFileArguments } from "./cli/file-processor.js";
import { buildInitialMessage } from "./cli/initial-message.js";
import { listModels } from "./cli/list-models.js";
import { selectSession } from "./cli/session-picker.js";
import { ENV_SESSION_DIR, expandTildePath, getAgentDir, VERSION } from "./config.js";
import { setAgentCliPaths } from "./core/agent-manifest-paths.js";
import { type CreateAgentSessionRuntimeFactory, createAgentSessionRuntime } from "./core/agent-session-runtime.js";
import {
	type AgentSessionRuntimeDiagnostic,
	createAgentSessionFromServices,
	createAgentSessionServices,
} from "./core/agent-session-services.js";
import { formatNoModelsAvailableMessage } from "./core/auth-guidance.js";
import { AuthStorage } from "./core/auth-storage.js";
import { reportEmbsearchProgress } from "./core/embsearch/embsearch-progress.js";
import {
	EmbsearchService,
	registerEmbsearchService,
	unregisterEmbsearchService,
} from "./core/embsearch/embsearch-service.js";
import { exportFromFile } from "./core/export-html/index.js";
import { parseSupportPlatforms, setSupportPlatforms } from "./core/extensions/plugins/formats/platform-targets.js";
import type { ExtensionAPI, ExtensionFactory } from "./core/extensions/types.js";
import { KeybindingsManager } from "./core/keybindings.js";
import {
	createLightTools,
	LIGHT_MODE_ENV,
	LIGHT_SYSTEM_PROMPT,
	LIGHT_TOOL_NAMES,
	measurePromptSurface,
} from "./core/light.js";
import type { ModelRegistry } from "./core/model-registry.js";
import { resolveCliModel, resolveModelScope, type ScopedModel } from "./core/model-resolver.js";
import { restoreStdout, takeOverStdout } from "./core/output-guard.js";
import type { CreateAgentSessionOptions } from "./core/sdk.js";
import {
	formatMissingSessionCwdPrompt,
	getMissingSessionCwdIssue,
	MissingSessionCwdError,
	type SessionCwdIssue,
} from "./core/session-cwd.js";
import { SessionManager } from "./core/session-manager.js";
import { SettingsManager } from "./core/settings-manager.js";
import {
	canSpawnSubagent,
	DEFER_MCP_SCHEMAS_ENV,
	DELEGATE_ALLOW_ENV,
	NESTED_CONCURRENCY_ENV,
	resolveMaxSubagentDepth,
	resolveNestedConcurrency,
	SUBAGENT_MAX_DEPTH_ENV,
} from "./core/subagent-depth.js";
import { printTimings, resetTimings, time } from "./core/timings.js";
import { createPluginLifecycleToolDefinitions } from "./core/tools/plugins.js";
import { createProposePluginToolDefinitions } from "./core/tools/propose-plugin.js";
import {
	buildTaskMainPrompt,
	createTaskOutputToolDefinition,
	createTaskToolDefinition,
} from "./core/tools/subagent.js";
import { createTodoWriteToolDefinition } from "./core/tools/todo.js";
import { WARM_SUBAGENTS_ENV } from "./core/warm-subagent-pool-instance.js";
// Static import (not dynamic) so `bun build --compile` statically reaches
// hoo-core from the compiled entry chain (src/bun/cli.ts -> src/cli.ts ->
// main.ts) and bundles it into the standalone binary. The node CLI reaches it
// via this same path through DEFAULT_EXTENSION_FACTORIES below.
import hooCore from "./extensions/core/hoo-core.js";
import { runMigrations, showDeprecationWarnings } from "./migrations.js";
import { InteractiveMode, runPrintMode, runRpcMode } from "./modes/index.js";
import { ExtensionSelectorComponent } from "./modes/interactive/components/extension-selector.js";
import { initTheme, stopThemeWatcher } from "./modes/interactive/theme/theme.js";
import { handleConfigCommand, handlePackageCommand } from "./package-manager-cli.js";
import { handleResourcesCommand } from "./resources-cli.js";
import { isLocalPath } from "./utils/paths.js";

/**
 * Read all content from piped stdin.
 * Returns undefined if stdin is a TTY (interactive terminal).
 */
async function readPipedStdin(): Promise<string | undefined> {
	// If stdin is a TTY, we're running interactively - don't read stdin
	if (process.stdin.isTTY) {
		return undefined;
	}

	return new Promise((resolve) => {
		let data = "";
		process.stdin.setEncoding("utf8");
		process.stdin.on("data", (chunk) => {
			data += chunk;
		});
		process.stdin.on("end", () => {
			resolve(data.trim() || undefined);
		});
		process.stdin.resume();
	});
}

function collectSettingsDiagnostics(
	settingsManager: SettingsManager,
	context: string,
): AgentSessionRuntimeDiagnostic[] {
	return settingsManager.drainErrors().map(({ scope, error }) => ({
		type: "warning",
		message: `(${context}, ${scope} settings) ${error.message}`,
	}));
}

function reportDiagnostics(diagnostics: readonly AgentSessionRuntimeDiagnostic[]): void {
	for (const diagnostic of diagnostics) {
		const color = diagnostic.type === "error" ? chalk.red : diagnostic.type === "warning" ? chalk.yellow : chalk.dim;
		const prefix = diagnostic.type === "error" ? "Error: " : diagnostic.type === "warning" ? "Warning: " : "";
		console.error(color(`${prefix}${diagnostic.message}`));
	}
}

function isTruthyEnvFlag(value: string | undefined): boolean {
	if (!value) return false;
	return value === "1" || value.toLowerCase() === "true" || value.toLowerCase() === "yes";
}

type AppMode = "interactive" | "print" | "json" | "rpc";

function resolveAppMode(parsed: Args, stdinIsTTY: boolean): AppMode {
	if (parsed.mode === "rpc") {
		return "rpc";
	}
	if (parsed.mode === "json") {
		return "json";
	}
	if (parsed.print || !stdinIsTTY) {
		return "print";
	}
	return "interactive";
}

function toPrintOutputMode(appMode: AppMode): Exclude<Mode, "rpc"> {
	return appMode === "json" ? "json" : "text";
}

async function prepareInitialMessage(
	parsed: Args,
	autoResizeImages: boolean,
	stdinContent?: string,
): Promise<{
	initialMessage?: string;
	initialImages?: ImageContent[];
}> {
	if (parsed.fileArgs.length === 0) {
		return buildInitialMessage({ parsed, stdinContent });
	}

	const { text, images } = await processFileArguments(parsed.fileArgs, { autoResizeImages });
	return buildInitialMessage({
		parsed,
		fileText: text,
		fileImages: images,
		stdinContent,
	});
}

/** Result from resolving a session argument */
type ResolvedSession =
	| { type: "path"; path: string } // Direct file path
	| { type: "local"; path: string } // Found in current project
	| { type: "global"; path: string; cwd: string } // Found in different project
	| { type: "not_found"; arg: string }; // Not found anywhere

/**
 * Resolve a session argument to a file path.
 * If it looks like a path, use as-is. Otherwise try to match as session ID prefix.
 */
async function resolveSessionPath(sessionArg: string, cwd: string, sessionDir?: string): Promise<ResolvedSession> {
	// If it looks like a file path, use as-is
	if (sessionArg.includes("/") || sessionArg.includes("\\") || sessionArg.endsWith(".jsonl")) {
		return { type: "path", path: sessionArg };
	}

	// Try to match as session ID in current project first
	const localSessions = await SessionManager.list(cwd, sessionDir);
	const localMatches = localSessions.filter((s) => s.id.startsWith(sessionArg));

	if (localMatches.length >= 1) {
		return { type: "local", path: localMatches[0].path };
	}

	// Try global search across all projects
	const allSessions = await SessionManager.listAll();
	const globalMatches = allSessions.filter((s) => s.id.startsWith(sessionArg));

	if (globalMatches.length >= 1) {
		const match = globalMatches[0];
		return { type: "global", path: match.path, cwd: match.cwd };
	}

	// Not found anywhere
	return { type: "not_found", arg: sessionArg };
}

/** Prompt user for yes/no confirmation */
async function promptConfirm(message: string): Promise<boolean> {
	return new Promise((resolve) => {
		const rl = createInterface({
			input: process.stdin,
			output: process.stdout,
		});
		rl.question(`${message} [y/N] `, (answer) => {
			rl.close();
			resolve(answer.toLowerCase() === "y" || answer.toLowerCase() === "yes");
		});
	});
}

function validateForkFlags(parsed: Args): void {
	if (!parsed.fork) return;

	const conflictingFlags = [
		parsed.session ? "--session" : undefined,
		parsed.continue ? "--continue" : undefined,
		parsed.resume ? "--resume" : undefined,
		parsed.noSession ? "--no-session" : undefined,
	].filter((flag): flag is string => flag !== undefined);

	if (conflictingFlags.length > 0) {
		console.error(chalk.red(`Error: --fork cannot be combined with ${conflictingFlags.join(", ")}`));
		process.exit(1);
	}
}

function forkSessionOrExit(sourcePath: string, cwd: string, sessionDir?: string): SessionManager {
	try {
		return SessionManager.forkFrom(sourcePath, cwd, sessionDir);
	} catch (error: unknown) {
		const message = error instanceof Error ? error.message : String(error);
		console.error(chalk.red(`Error: ${message}`));
		process.exit(1);
	}
}

async function createSessionManager(
	parsed: Args,
	cwd: string,
	sessionDir: string | undefined,
	settingsManager: SettingsManager,
): Promise<SessionManager> {
	if (parsed.noSession) {
		return SessionManager.inMemory();
	}

	if (parsed.fork) {
		const resolved = await resolveSessionPath(parsed.fork, cwd, sessionDir);

		switch (resolved.type) {
			case "path":
			case "local":
			case "global":
				return forkSessionOrExit(resolved.path, cwd, sessionDir);

			case "not_found":
				console.error(chalk.red(`No session found matching '${resolved.arg}'`));
				process.exit(1);
		}
	}

	if (parsed.session) {
		const resolved = await resolveSessionPath(parsed.session, cwd, sessionDir);

		switch (resolved.type) {
			case "path":
			case "local":
				return SessionManager.open(resolved.path, sessionDir);

			case "global": {
				console.log(chalk.yellow(`Session found in different project: ${resolved.cwd}`));
				const shouldFork = await promptConfirm("Fork this session into current directory?");
				if (!shouldFork) {
					console.log(chalk.dim("Aborted."));
					process.exit(0);
				}
				return forkSessionOrExit(resolved.path, cwd, sessionDir);
			}

			case "not_found":
				console.error(chalk.red(`No session found matching '${resolved.arg}'`));
				process.exit(1);
		}
	}

	if (parsed.resume) {
		initTheme(settingsManager.getTheme(), true);
		try {
			const selectedPath = await selectSession(
				(onProgress) => SessionManager.list(cwd, sessionDir, onProgress),
				SessionManager.listAll,
			);
			if (!selectedPath) {
				console.log(chalk.dim("No session selected"));
				process.exit(0);
			}
			return SessionManager.open(selectedPath, sessionDir);
		} finally {
			stopThemeWatcher();
		}
	}

	if (parsed.continue) {
		return SessionManager.continueRecent(cwd, sessionDir);
	}

	return SessionManager.create(cwd, sessionDir);
}

function buildSessionOptions(
	parsed: Args,
	scopedModels: ScopedModel[],
	hasExistingSession: boolean,
	modelRegistry: ModelRegistry,
	settingsManager: SettingsManager,
	lightMode: boolean,
): {
	options: CreateAgentSessionOptions;
	cliThinkingFromModel: boolean;
	diagnostics: AgentSessionRuntimeDiagnostic[];
} {
	const options: CreateAgentSessionOptions = {};
	const diagnostics: AgentSessionRuntimeDiagnostic[] = [];
	let cliThinkingFromModel = false;

	// Model from CLI
	// - supports --provider <name> --model <pattern>
	// - supports --model <provider>/<pattern>
	if (parsed.model) {
		const resolved = resolveCliModel({
			cliProvider: parsed.provider,
			cliModel: parsed.model,
			modelRegistry,
		});
		if (resolved.warning) {
			diagnostics.push({ type: "warning", message: resolved.warning });
		}
		if (resolved.error) {
			diagnostics.push({ type: "error", message: resolved.error });
		}
		if (resolved.model) {
			options.model = resolved.model;
			// Allow "--model <pattern>:<thinking>" as a shorthand.
			// Explicit --thinking still takes precedence (applied later).
			if (!parsed.thinking && resolved.thinkingLevel) {
				options.thinkingLevel = resolved.thinkingLevel;
				cliThinkingFromModel = true;
			}
		}
	}

	if (!options.model && scopedModels.length > 0 && !hasExistingSession) {
		// Check if saved default is in scoped models - use it if so, otherwise first scoped model
		const savedProvider = settingsManager.getDefaultProvider();
		const savedModelId = settingsManager.getDefaultModel();
		const savedModel = savedProvider && savedModelId ? modelRegistry.find(savedProvider, savedModelId) : undefined;
		const savedInScope = savedModel ? scopedModels.find((sm) => modelsAreEqual(sm.model, savedModel)) : undefined;

		if (savedInScope) {
			options.model = savedInScope.model;
			// Use thinking level from scoped model config if explicitly set
			if (!parsed.thinking && savedInScope.thinkingLevel) {
				options.thinkingLevel = savedInScope.thinkingLevel;
			}
		} else {
			options.model = scopedModels[0].model;
			// Use thinking level from first scoped model if explicitly set
			if (!parsed.thinking && scopedModels[0].thinkingLevel) {
				options.thinkingLevel = scopedModels[0].thinkingLevel;
			}
		}
	}

	// Thinking level from CLI (takes precedence over scoped model thinking levels set above)
	if (parsed.thinking) {
		options.thinkingLevel = parsed.thinking;
	}

	// Scoped models for Ctrl+P cycling
	// Keep thinking level undefined when not explicitly set in the model pattern.
	// Undefined means "inherit current session thinking level" during cycling.
	if (scopedModels.length > 0) {
		options.scopedModels = scopedModels.map((sm) => ({
			model: sm.model,
			thinkingLevel: sm.thinkingLevel,
		}));
	}

	// API key from CLI - set in authStorage
	// (handled by caller before createAgentSession)

	// Tools
	if (parsed.noTools) {
		options.noTools = "all";
	} else if (parsed.noBuiltinTools) {
		options.noTools = "builtin";
	}
	if (parsed.tools) {
		options.tools = [...parsed.tools];
	}
	if (parsed.disallowedTools) {
		options.disallowedTools = [...parsed.disallowedTools];
	}
	// Persisted per-tool disables from the TUI (/settings → Tools). These compose
	// with any CLI denylist and always subtract from the active tool set: a tool
	// listed here stays off even if it also appears in the --tools allowlist.
	const disabledTools = settingsManager.getDisabledTools();
	if (disabledTools.length > 0) {
		options.disallowedTools = [...new Set([...(options.disallowedTools ?? []), ...disabledTools])];
	}
	// Light preset: restrict to the four core tools unless the user passed an
	// explicit allowlist or suppression flag (explicit flags win over the preset).
	// The allowlist also blocks Task/TodoWrite/plugin tools from becoming active.
	if (lightMode && !parsed.tools && !parsed.noTools && !parsed.noBuiltinTools) {
		options.tools = [...LIGHT_TOOL_NAMES];
	}
	// Artifact platform targeting: --support-platform flag (falls back to the
	// supportPlatform setting) picks which vendor layout(s) hoocode writes when
	// it produces artifacts — authored plugins (ProposePlugin) and the /new-skill
	// //new-agent //new-command scaffolds. Stored as process-wide state next to
	// the format registry; readers of every format stay unaffected.
	const supportPlatformTokens = parsed.supportPlatform ?? settingsManager.getSupportPlatform();
	if (supportPlatformTokens && supportPlatformTokens.length > 0) {
		const { platforms, invalid } = parseSupportPlatforms(supportPlatformTokens);
		for (const token of invalid) {
			diagnostics.push({
				type: "warning",
				message: `Unknown --support-platform "${token}" (valid: claude, copilot|github|gh, agents|native)`,
			});
		}
		if (platforms.length > 0) {
			setSupportPlatforms(platforms);
		} else {
			diagnostics.push({
				type: "warning",
				message: "--support-platform resolved no valid platforms; using the default targets",
			});
		}
	}

	// Web tools (webfetch + websearch): opt-in via --enable-webtools flag or the
	// enableWebTools setting. They are registered as base tools but inactive by
	// default; this adds them to the default active set.
	if (parsed.enableWebTools ?? settingsManager.getEnableWebTools()) {
		options.enableWebTools = true;
	}
	// Ranked code search (search tool, lexical + semantic hybrid). The search tool
	// is active by default. The enableEmbsearchTools setting controls whether the
	// optional semantic index layer starts; when off, search degrades to lexical-only.
	if (parsed.enableEmbsearchTools ?? settingsManager.getEnableEmbsearchTools()) {
		options.enableEmbsearchTools = true;
	}
	// Browser tools (browser_run + browser_continue): opt-in via --enable-browsertools
	// flag or the enableBrowserTools setting. Registered as base tools but inactive by
	// default; this adds them to the default active set.
	if (parsed.enableBrowserTools ?? settingsManager.getEnableBrowserTools()) {
		options.enableBrowserTools = true;
	}
	// Live preview for browser_run: defaults the streamed viewer on and auto-opens
	// it. Applied as a runtime settings override so the session's tool factory reads
	// it via the shared settingsManager (no extra option plumbing).
	if (parsed.enableBrowserLivePreview) {
		settingsManager.applyOverrides({ enableBrowserLivePreview: true });
	}
	// Document tools (DocRead/DocEdit/DocWrite + the DocScan/DocGrep/DocPeek
	// discovery loop): opt-in via --enable-filetools flag or the enableFileTools
	// setting. Registered as base tools but inactive by default; this adds them to
	// the default active set.
	if (parsed.enableFileTools ?? settingsManager.getEnableFileTools()) {
		options.enableFileTools = true;
		// DocRead/DocEdit/DocWrite drive a lossless id-based extract -> patch ->
		// reconstruct flow that depends on precise, well-formed tool calls. Models
		// that are weak at tool calling tend to mangle the id-based patches and
		// corrupt documents, so surface a one-time heads-up when these are enabled.
		diagnostics.push({
			type: "warning",
			message:
				"Document tools (DocRead/DocEdit/DocWrite) are enabled. They require precise, id-based patches; " +
				"use a model that is strong at tool calling, or these edits can corrupt files.",
		});
	}

	// Optional Task (subagent) tool: opt-in via --enable-subagents flag or the enableSubagent setting.
	// Registered as a custom tool; respects --tools/--no-tools allowlists like any other tool.
	//
	// Nesting is bounded by the tree-wide cap (maxSubagentDepth, default 1). The root
	// seeds the cap into the environment so every descendant agrees on one value; the
	// Task tool is registered only while this process's depth is below that cap. At the
	// default cap this reproduces the original guard exactly: subagents (depth >= 1) get
	// no Task tool and cannot recursively dispatch.
	const isSubagentChild = parsed.taskId !== undefined;
	if (process.env[SUBAGENT_MAX_DEPTH_ENV] === undefined) {
		// The root seeds the tree-wide cap; the --max-subagent-depth flag overrides the
		// setting. resolveMaxSubagentDepth clamps it to the supported range so the seeded
		// env (and everything that inherits it) carries a sane value. Descendants inherit
		// it via the environment (env already set => keep it).
		process.env[SUBAGENT_MAX_DEPTH_ENV] = String(
			resolveMaxSubagentDepth(parsed.maxSubagentDepth ?? settingsManager.getMaxSubagentDepth()),
		);
	}
	if (process.env[NESTED_CONCURRENCY_ENV] === undefined) {
		// Seed the nested-pool concurrency from settings so descendants agree on one value.
		process.env[NESTED_CONCURRENCY_ENV] = String(
			resolveNestedConcurrency(settingsManager.getNestedSubagentConcurrency()),
		);
	}
	// Scoped delegation: --delegate-allow is the authoritative restriction for this
	// process. Set it from the flag, or clear any inherited value so a restricted
	// parent's scope never leaks into a child that wasn't given its own.
	if (parsed.delegateAllow && parsed.delegateAllow.length > 0) {
		process.env[DELEGATE_ALLOW_ENV] = parsed.delegateAllow.join(",");
	} else {
		delete process.env[DELEGATE_ALLOW_ENV];
	}
	if (!lightMode && canSpawnSubagent() && (parsed.subagent ?? settingsManager.getEnableSubagent())) {
		options.customTools = [
			...(options.customTools ?? []),
			createTaskToolDefinition(),
			createTaskOutputToolDefinition(),
		];
		// Warm subagents (experimental): dispatch eligible foreground subagents on
		// reused RPC workers to skip the cold-boot. Root-only — the Task tool exists
		// only where canSpawnSubagent holds and never inside a spawned child — and
		// carried via env so the Task tool reads it without threading a setting.
		if (!isSubagentChild && (parsed.warmSubagents ?? settingsManager.getWarmSubagents())) {
			process.env[WARM_SUBAGENTS_ENV] = "1";
		}
	}

	// Optional TodoWrite tool: opt-in via --enable-todowrite flag or the
	// enableTodoWrite setting. Never registered inside a spawned subagent child —
	// its todos would otherwise leak into the parent's "main" task group in the pane.
	if (!isSubagentChild && !lightMode && (parsed.todoWrite ?? settingsManager.getEnableTodoWrite())) {
		options.customTools = [...(options.customTools ?? []), createTodoWriteToolDefinition()];
	}

	// Deferred MCP tool schemas (default on; disable via deferMcpSchemas=false): set
	// for the top-level agent only. Subagent
	// children clear this env (see subagent-pool) so a child that needs MCP resolves
	// its allowlisted tools eagerly at dispatch.
	if (!isSubagentChild && settingsManager.getDeferMcpSchemas()) {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
	}

	// Plugin lifecycle tools (SearchPlugins, InstallPlugin, ...). Top-level agent
	// only: these are capability-acquisition tools and must never be available to
	// a spawned subagent child (privilege-amplification guardrail, spec §3).
	// `enablePluginTools` is the master switch for the whole autonomous plugin
	// system (default off) — it gates both these tools and the runtime reuse
	// nudge (see extensions/core/prompt-reactive), so both flip together.
	if (!isSubagentChild && !lightMode && (parsed.enablePluginTools ?? settingsManager.getEnablePluginTools())) {
		options.customTools = [
			...(options.customTools ?? []),
			...createPluginLifecycleToolDefinitions(),
			...createProposePluginToolDefinitions(),
		];
	}

	return { options, cliThinkingFromModel, diagnostics };
}

function resolveCliPaths(cwd: string, paths: string[] | undefined): string[] | undefined {
	return paths?.map((value) => (isLocalPath(value) ? resolve(cwd, value) : value));
}

async function promptForMissingSessionCwd(
	issue: SessionCwdIssue,
	settingsManager: SettingsManager,
): Promise<string | undefined> {
	initTheme(settingsManager.getTheme());
	setKeybindings(KeybindingsManager.create());

	return new Promise((resolve) => {
		const ui = new TUI(new ProcessTerminal(), settingsManager.getShowHardwareCursor());
		ui.setClearOnShrink(settingsManager.getClearOnShrink());

		let settled = false;
		const finish = (result: string | undefined) => {
			if (settled) {
				return;
			}
			settled = true;
			ui.stop();
			resolve(result);
		};

		const selector = new ExtensionSelectorComponent(
			formatMissingSessionCwdPrompt(issue),
			["Continue", "Cancel"],
			(option) => finish(option === "Continue" ? issue.fallbackCwd : undefined),
			() => finish(undefined),
			{ tui: ui },
		);
		ui.addChild(selector);
		ui.setFocus(selector);
		ui.start();
	});
}

export interface MainOptions {
	extensionFactories?: ExtensionFactory[];
}

/**
 * Built-in extension factories loaded when the caller supplies none. This is
 * the single source of truth for the app's built-ins (hoo-core: /loop, /plugin,
 * /mode, /cost, scaffold commands, the MCP loader + remote-MCP/OAuth flow).
 * Both entry points — the node CLI (bin/hoocode.js) and the compiled binary
 * (src/cli.ts) — call main() without factories and inherit this default.
 * Downstream embedders that pass their own extensionFactories override it.
 */
const DEFAULT_EXTENSION_FACTORIES: ExtensionFactory[] = [hooCore];

export async function main(args: string[], options?: MainOptions) {
	resetTimings();
	const offlineMode = args.includes("--offline") || isTruthyEnvFlag(process.env.HOOCODE_OFFLINE);
	if (offlineMode) {
		process.env.HOOCODE_OFFLINE = "1";
		process.env.HOOCODE_SKIP_VERSION_CHECK = "1";
	}

	if (await handlePackageCommand(args)) {
		return;
	}

	if (await handleConfigCommand(args)) {
		return;
	}

	if (await handleResourcesCommand(args)) {
		return;
	}

	const parsed = parseArgs(args);
	if (parsed.diagnostics.length > 0) {
		for (const d of parsed.diagnostics) {
			const color = d.type === "error" ? chalk.red : chalk.yellow;
			console.error(color(`${d.type === "error" ? "Error" : "Warning"}: ${d.message}`));
		}
		if (parsed.diagnostics.some((d) => d.type === "error")) {
			process.exit(1);
		}
	}
	time("parseArgs");
	let appMode = resolveAppMode(parsed, process.stdin.isTTY);
	const shouldTakeOverStdout = appMode !== "interactive";
	if (shouldTakeOverStdout) {
		takeOverStdout();
	}

	if (parsed.version) {
		console.log(VERSION);
		process.exit(0);
	}

	if (parsed.export) {
		let result: string;
		try {
			const outputPath = parsed.messages.length > 0 ? parsed.messages[0] : undefined;
			result = await exportFromFile(parsed.export, outputPath);
		} catch (error: unknown) {
			const message = error instanceof Error ? error.message : "Failed to export session";
			console.error(chalk.red(`Error: ${message}`));
			process.exit(1);
		}
		console.log(`Exported to: ${result}`);
		process.exit(0);
	}

	if (parsed.mode === "rpc" && parsed.fileArgs.length > 0) {
		console.error(chalk.red("Error: @file arguments are not supported in RPC mode"));
		process.exit(1);
	}

	validateForkFlags(parsed);

	// Run migrations (pass cwd for project-local migrations)
	const { migratedAuthProviders: migratedProviders, deprecationWarnings } = runMigrations(process.cwd());
	time("runMigrations");

	const cwd = process.cwd();
	const agentDir = getAgentDir();
	const startupSettingsManager = SettingsManager.create(cwd, agentDir);
	reportDiagnostics(collectSettingsDiagnostics(startupSettingsManager, "startup session lookup"));

	// Decide the final runtime cwd before creating cwd-bound runtime services.
	// --session and --resume may select a session from another project, so project-local
	// settings, resources, provider registrations, and models must be resolved only after
	// the target session cwd is known. The startup-cwd settings manager is used only for
	// sessionDir lookup during session selection.
	const envSessionDir = process.env[ENV_SESSION_DIR];
	const sessionDir =
		parsed.sessionDir ??
		(envSessionDir ? expandTildePath(envSessionDir) : undefined) ??
		startupSettingsManager.getSessionDir();
	let sessionManager = await createSessionManager(parsed, cwd, sessionDir, startupSettingsManager);
	const missingSessionCwdIssue = getMissingSessionCwdIssue(sessionManager, cwd);
	if (missingSessionCwdIssue) {
		if (appMode === "interactive") {
			const selectedCwd = await promptForMissingSessionCwd(missingSessionCwdIssue, startupSettingsManager);
			if (!selectedCwd) {
				process.exit(0);
			}
			sessionManager = SessionManager.open(missingSessionCwdIssue.sessionFile!, sessionDir, selectedCwd);
		} else {
			console.error(chalk.red(new MissingSessionCwdError(missingSessionCwdIssue).message));
			process.exit(1);
		}
	}
	time("createSessionManager");

	const resolvedExtensionPaths = resolveCliPaths(cwd, parsed.extensions);
	const resolvedSkillPaths = resolveCliPaths(cwd, parsed.skills);
	const resolvedAgentPaths = resolveCliPaths(cwd, parsed.agents);
	const resolvedPromptTemplatePaths = resolveCliPaths(cwd, parsed.promptTemplates);
	const resolvedSlashCommandPaths = resolveCliPaths(cwd, parsed.slashCommands);
	const resolvedThemePaths = resolveCliPaths(cwd, parsed.themes);

	// Populate the module-level CLI agent store before any session is created.
	setAgentCliPaths(resolvedAgentPaths ?? []);

	// Synthetic factory: feed CLI --mode-path values into the extension runtime
	// so hoo-core (and any other extension that reads pi.getModeSearchPaths)
	// sees them alongside extension-registered dirs.
	const cliModePaths = parsed.modePaths ?? [];
	const cliResourcePathFactories: ExtensionFactory[] =
		cliModePaths.length === 0
			? []
			: [
					Object.assign(
						(pi: ExtensionAPI) => {
							for (const p of cliModePaths) pi.addModeSearchPath(p);
						},
						{ internal: true },
					),
				];
	// `??` (not `[...a, ...b]`) so an explicit list — from bin/hoocode.js or a
	// downstream embedder — fully replaces the default and hoo-core is never
	// registered twice.
	const allExtensionFactories: ExtensionFactory[] = [
		...cliResourcePathFactories,
		...(options?.extensionFactories ?? DEFAULT_EXTENSION_FACTORIES),
	];
	const authStorage = AuthStorage.create();
	// A spawned subagent (run with a task id) is a single-shot, non-interactive
	// process: it renders no TUI and its prompt is a plain task, never a slash
	// command. Themes, slash commands, and prompt templates are dead weight in that
	// path, so skip loading them to trim the child's cold-boot cost. Skills, context
	// files, and extensions (which carry the core tools) are kept — they affect the
	// subagent's actual work.
	const isSubagentBoot = parsed.taskId !== undefined;
	const createRuntime: CreateAgentSessionRuntimeFactory = async ({
		cwd,
		agentDir,
		sessionManager,
		sessionStartEvent,
	}) => {
		// Light preset: resolve before services so it can shape resource loading
		// (no skills/context files, terse system prompt). The env flag tells code
		// that cannot see flags or settings — the hoo-core modes extension — to
		// skip its system-prompt appendix.
		const earlySettingsManager = SettingsManager.create(cwd, agentDir);
		const lightMode = parsed.light ?? earlySettingsManager.getLight();
		if (lightMode) {
			process.env[LIGHT_MODE_ENV] = "1";
		} else {
			delete process.env[LIGHT_MODE_ENV];
		}
		const services = await createAgentSessionServices({
			cwd,
			agentDir,
			authStorage,
			settingsManager: earlySettingsManager,
			extensionFlagValues: parsed.unknownFlags,
			resourceLoaderOptions: {
				additionalExtensionPaths: resolvedExtensionPaths,
				additionalSkillPaths: resolvedSkillPaths,
				additionalPromptTemplatePaths: resolvedPromptTemplatePaths,
				additionalSlashCommandPaths: resolvedSlashCommandPaths,
				additionalThemePaths: resolvedThemePaths,
				noExtensions: parsed.noExtensions,
				noSkills: parsed.noSkills || lightMode,
				noPromptTemplates: parsed.noPromptTemplates || isSubagentBoot,
				noSlashCommands: parsed.noSlashCommands || isSubagentBoot,
				noThemes: parsed.noThemes || isSubagentBoot,
				noContextFiles: parsed.noContextFiles || lightMode,
				systemPrompt: parsed.systemPrompt ?? (lightMode ? LIGHT_SYSTEM_PROMPT : undefined),
				extensionFactories: allExtensionFactories,
			},
		});
		const { settingsManager, modelRegistry, resourceLoader } = services;
		const diagnostics: AgentSessionRuntimeDiagnostic[] = [
			...services.diagnostics,
			...collectSettingsDiagnostics(settingsManager, "runtime creation"),
			...resourceLoader.getExtensions().errors.map(({ path, error }) => ({
				type: "error" as const,
				message: `Failed to load extension "${path}": ${error}`,
			})),
		];

		// When subagent tooling is enabled, append the main session subagent instructions.
		if (!lightMode && (parsed.subagent ?? settingsManager.getEnableSubagent())) {
			resourceLoader.addAppendSystemPrompt(buildTaskMainPrompt());
		}

		const modelPatterns = parsed.models ?? settingsManager.getEnabledModels();
		const scopedModels =
			modelPatterns && modelPatterns.length > 0 ? await resolveModelScope(modelPatterns, modelRegistry) : [];
		const {
			options: sessionOptions,
			cliThinkingFromModel,
			diagnostics: sessionOptionDiagnostics,
		} = buildSessionOptions(
			parsed,
			scopedModels,
			sessionManager.buildSessionContext().messages.length > 0,
			modelRegistry,
			settingsManager,
			lightMode,
		);
		diagnostics.push(...sessionOptionDiagnostics);
		// Swap in the short-schema read/write/edit/bash tool variants. Skipped
		// when the user's own --tools allowlist overrode the light tool set.
		if (lightMode && !parsed.tools) {
			sessionOptions.baseToolsOverride = createLightTools(cwd);
		}

		if (parsed.apiKey) {
			if (!sessionOptions.model) {
				diagnostics.push({
					type: "error",
					message: "--api-key requires a model to be specified via --model, --provider/--model, or --models",
				});
			} else {
				authStorage.setRuntimeApiKey(sessionOptions.model.provider, parsed.apiKey);
			}
		}

		const created = await createAgentSessionFromServices({
			services,
			sessionManager,
			sessionStartEvent,
			model: sessionOptions.model,
			thinkingLevel: sessionOptions.thinkingLevel,
			scopedModels: sessionOptions.scopedModels,
			tools: sessionOptions.tools,
			noTools: sessionOptions.noTools,
			disallowedTools: sessionOptions.disallowedTools,
			customTools: sessionOptions.customTools,
			enableWebTools: sessionOptions.enableWebTools,
			enableBrowserTools: sessionOptions.enableBrowserTools,
			enableFileTools: sessionOptions.enableFileTools,
			enableEmbsearchTools: sessionOptions.enableEmbsearchTools,
			baseToolsOverride: sessionOptions.baseToolsOverride,
		});
		const cliThinkingOverride = parsed.thinking !== undefined || cliThinkingFromModel;
		if (created.session.model && cliThinkingOverride) {
			created.session.setThinkingLevel(created.session.thinkingLevel);
		}

		return {
			...created,
			services,
			diagnostics,
		};
	};
	time("createRuntime");
	const runtime = await createAgentSessionRuntime(createRuntime, {
		cwd: sessionManager.getCwd(),
		agentDir,
		sessionManager,
	});
	const { services, session, modelFallbackMessage } = runtime;
	const { settingsManager, modelRegistry, resourceLoader } = services;

	if (parsed.help) {
		const extensionFlags = resourceLoader
			.getExtensions()
			.extensions.flatMap((extension) => Array.from(extension.flags.values()));
		printHelp(extensionFlags);
		process.exit(0);
	}

	if (parsed.listModels !== undefined) {
		const searchPattern = typeof parsed.listModels === "string" ? parsed.listModels : undefined;
		await listModels(modelRegistry, searchPattern);
		process.exit(0);
	}

	if (parsed.printTokenSurface) {
		const surface = measurePromptSurface(session);
		console.log("Fixed per-turn surface (chars/4 token estimate):");
		console.log(`  system prompt: ${surface.systemPromptTokens} tokens`);
		for (const tool of surface.tools) {
			console.log(`  tool ${tool.name}: ${tool.tokens} tokens`);
		}
		console.log(`  tool schemas total: ${surface.toolSchemaTokens} tokens`);
		console.log(`  total: ${surface.totalTokens} tokens`);
		process.exit(0);
	}

	// Read piped stdin content (if any) - skip for RPC mode which uses stdin for JSON-RPC
	let stdinContent: string | undefined;
	if (appMode !== "rpc") {
		stdinContent = await readPipedStdin();
		if (stdinContent !== undefined && appMode === "interactive") {
			appMode = "print";
		}
	}
	time("readPipedStdin");

	const { initialMessage, initialImages } = await prepareInitialMessage(
		parsed,
		settingsManager.getImageAutoResize(),
		stdinContent,
	);
	time("prepareInitialMessage");
	initTheme(settingsManager.getTheme(), appMode === "interactive");
	time("initTheme");

	// Show deprecation warnings in interactive mode
	if (appMode === "interactive" && deprecationWarnings.length > 0) {
		await showDeprecationWarnings(deprecationWarnings);
	}

	const scopedModels = [...session.scopedModels];
	time("resolveModelScope");
	reportDiagnostics(runtime.diagnostics);
	if (runtime.diagnostics.some((diagnostic) => diagnostic.type === "error")) {
		process.exit(1);
	}
	time("createAgentSession");

	if (appMode !== "interactive" && !session.model) {
		console.error(chalk.red(formatNoModelsAvailableMessage()));
		process.exit(1);
	}

	const startupBenchmark = isTruthyEnvFlag(process.env.HOOCODE_STARTUP_BENCHMARK);
	if (startupBenchmark && appMode !== "interactive") {
		console.error(chalk.red("Error: HOOCODE_STARTUP_BENCHMARK only supports interactive mode"));
		process.exit(1);
	}

	// Start the optional local embedding index service in the background. The
	// `search` tool is always available and runs lexical-only by default; this
	// semantic-index layer is completely off unless --enable-search-tool (or the
	// setting) is true. When on, the tool fuses semantic hits once the index
	// reports available.
	let embsearchService: EmbsearchService | undefined;
	if (parsed.enableEmbsearchTools ?? settingsManager.getEnableEmbsearchTools()) {
		const isInteractive = appMode === "interactive";
		embsearchService = new EmbsearchService({
			cwd: sessionManager.getCwd(),
			binaryPath: settingsManager.getEmbsearchBinaryPath(),
			thresholdBytes: settingsManager.getEmbsearchThresholdBytes(),
			// Interactive routes to the footer's startupProgress store; non-interactive
			// logs to stderr. See reportEmbsearchProgress for the per-phase mapping.
			onProgress: (state) => reportEmbsearchProgress(state, { interactive: isInteractive }),
		});
		registerEmbsearchService(sessionManager.getCwd(), embsearchService);
		embsearchService.start().catch(() => {
			// Errors are logged by the onProgress unavailable state; swallow to avoid
			// crashing the session because of an optional background index.
		});
	}

	if (appMode === "rpc") {
		printTimings();
		try {
			await runRpcMode(runtime);
		} finally {
			await embsearchService?.dispose();
			unregisterEmbsearchService(sessionManager.getCwd());
		}
	} else if (appMode === "interactive") {
		if (scopedModels.length > 0 && (parsed.verbose || !settingsManager.getQuietStartup())) {
			const modelList = scopedModels
				.map((sm) => {
					const thinkingStr = sm.thinkingLevel ? `:${sm.thinkingLevel}` : "";
					return `${sm.model.id}${thinkingStr}`;
				})
				.join(", ");
			console.log(chalk.dim(`Model scope: ${modelList} ${chalk.gray("(Ctrl+P to cycle)")}`));
		}

		const interactiveMode = new InteractiveMode(runtime, {
			migratedProviders,
			modelFallbackMessage,
			initialMessage,
			initialImages,
			initialMessages: parsed.messages,
			verbose: parsed.verbose,
		});

		// Optional hooteams bridge. `--team auto` discovers a config, spawns a
		// local hooteams child on a free port, and proceeds as if its URL had
		// been passed; the child is reaped on exit (clean or signal) via a
		// process "exit" hook plus the explicit stop below.
		let autoTeam: { url: string; stop(): Promise<void> } | undefined;
		let teamUrl = parsed.team;
		if (teamUrl === "auto") {
			const { startAutoTeam } = await import("./core/team-auto.js");
			try {
				autoTeam = await startAutoTeam(process.cwd(), { log: (message) => console.log(chalk.dim(message)) });
				teamUrl = autoTeam.url;
			} catch (error) {
				console.error(chalk.red(error instanceof Error ? error.message : String(error)));
				process.exit(1);
			}
		}

		// Team mirror + client. Fire-and-forget: connect failures and drops warn
		// in the background and never block the main agent. Warnings go through
		// the chat, not console.error: a raw stderr write while the TUI owns the
		// screen scribbles over the render and can leave the editor looking
		// frozen. The same connection powers team focus / nudge / attach.
		let teamView: { stop(): void } | undefined;
		if (teamUrl) {
			const { connectTeamView } = await import("./core/team-view.js");
			const teamClient = connectTeamView(teamUrl, {
				warn: (message) => interactiveMode.showWarning(message),
			});
			teamView = teamClient;
			interactiveMode.attachTeamClient(teamClient);
		}
		if (startupBenchmark) {
			await interactiveMode.init();
			time("interactiveMode.init");
			printTimings();
			teamView?.stop();
			await autoTeam?.stop();
			interactiveMode.stop();
			stopThemeWatcher();
			if (process.stdout.writableLength > 0) {
				await new Promise<void>((resolve) => process.stdout.once("drain", resolve));
			}
			if (process.stderr.writableLength > 0) {
				await new Promise<void>((resolve) => process.stderr.once("drain", resolve));
			}
			return;
		}

		printTimings();
		try {
			await interactiveMode.run();
		} finally {
			teamView?.stop();
			await autoTeam?.stop();
			await embsearchService?.dispose();
			unregisterEmbsearchService(sessionManager.getCwd());
		}
	} else {
		printTimings();
		const exitCode = await runPrintMode(runtime, {
			mode: toPrintOutputMode(appMode),
			messages: parsed.messages,
			initialMessage,
			initialImages,
			taskId: parsed.taskId,
			maxTurns: parsed.maxTurns,
		});
		await embsearchService?.dispose();
		unregisterEmbsearchService(sessionManager.getCwd());
		stopThemeWatcher();
		restoreStdout();
		if (exitCode !== 0) {
			process.exitCode = exitCode;
		}
		// Spawned subagents (run with a task id) must exit promptly once their work is
		// done and result.json is written. The child's runtime can leave handles open
		// (e.g. MCP client connections) that keep the event loop alive, so a natural
		// exit may never happen. When that occurs the parent lifeguard SIGKILLs the idle
		// child at the 60s heartbeat threshold and misreports an already-completed task
		// as "stalled". Force a clean exit after draining output to avoid that false stall.
		const ranAsSubagent = typeof parsed.taskId === "string" && parsed.taskId.length > 0;
		if (ranAsSubagent) {
			if (process.stdout.writableLength > 0) {
				await new Promise<void>((resolve) => process.stdout.once("drain", resolve));
			}
			if (process.stderr.writableLength > 0) {
				await new Promise<void>((resolve) => process.stderr.once("drain", resolve));
			}
			process.exit(exitCode);
		}
		return;
	}
}
