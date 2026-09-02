/**
 * Interactive mode for the coding agent.
 * Handles TUI rendering and user interaction, delegating business logic to AgentSession.
 */

import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { createCompactionSummaryMessage } from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage, ImageContent, Message } from "@kolisachint/hoocode-ai";
import { isLongRetryDelayError } from "@kolisachint/hoocode-ai";
import type {
	AutocompleteItem,
	AutocompleteProvider,
	EditorComponent,
	KeyId,
	MarkdownTheme,
	SlashCommand,
} from "@kolisachint/hoocode-tui";
import {
	Box,
	CombinedAutocompleteProvider,
	type Component,
	Container,
	fuzzyFilter,
	getCapabilities,
	hyperlink,
	Loader,
	type LoaderIndicatorOptions,
	Markdown,
	matchesKey,
	ProcessTerminal,
	Spacer,
	setKeybindings,
	Text,
	TUI,
} from "@kolisachint/hoocode-tui";
import { spawnSync } from "child_process";
import { APP_NAME, APP_TITLE, VERSION } from "../../config.js";
import { setTerminalOwnedByTui } from "../../core/agent-log.js";
import { loadAgentRegistry } from "../../core/agent-registry.js";
import { type AgentSession, type AgentSessionEvent, parseSkillBlock } from "../../core/agent-session.js";
import type { AgentSessionRuntime } from "../../core/agent-session-runtime.js";
import { type AssistantUsageTotals, sumAssistantUsage } from "../../core/agent-session-stats.js";
import { canvasSearchRoots, discoverCanvasExtensions } from "../../core/canvas/discovery.js";
import { pluginCanvasExtensions } from "../../core/canvas/plugin-canvases.js";
import { gateCanvasExtensions } from "../../core/canvas/trust.js";
import type {
	AutocompleteProviderFactory,
	EditorFactory,
	ExtensionCommandContext,
	ExtensionContext,
	ExtensionRunner,
	ExtensionUIContext,
} from "../../core/extensions/index.js";
import { getWorkspacePlatforms, setPlatforms } from "../../core/extensions/plugins/formats/platform-targets.js";
import { describeExternalTools } from "../../core/external-tools.js";
import { FooterDataProvider } from "../../core/footer-data-provider.js";
import { formatDurationSecs } from "../../core/format-duration.js";
import { formatTokens } from "../../core/format-tokens.js";
import { type AppKeybinding, KeybindingsManager } from "../../core/keybindings.js";
import { measurePromptSurface, measureToolSchemaTokens } from "../../core/light.js";
import type { ResourceDiagnostic } from "../../core/resource-loader.js";
import { formatMissingSessionCwdPrompt, MissingSessionCwdError } from "../../core/session-cwd.js";
import { type SessionContext, SessionManager } from "../../core/session-manager.js";
import type { SettingsManager } from "../../core/settings-manager.js";
import { BUILTIN_SLASH_COMMANDS } from "../../core/slash-commands.js";
import type { SourceInfo } from "../../core/source-info.js";
import { startupProgress } from "../../core/startup-progress.js";
import { taskStore } from "../../core/task-store.js";
import type { TeamViewConnection } from "../../core/team-view.js";
import { cycleToolOutputView, type ToolOutputView } from "../../core/tool-output-view.js";
import { settleDanglingMainTasks } from "../../core/tools/todo.js";
import type { TruncationResult } from "../../core/tools/truncate.js";
import { buildCompactWordmark } from "../../core/wordmark.js";
import { extensionForImageMimeType, readClipboardImage } from "../../utils/clipboard-image.js";
import { parseGitUrl } from "../../utils/git.js";
import { killTrackedDetachedChildren } from "../../utils/shell.js";
import { ensureTool } from "../../utils/tools-manager.js";
import { checkForNewHooCodeVersion } from "../../utils/version-check.js";
import { BashExecutionController } from "./bash-execution-controller.js";
import { type CommandContext, CommandExecutor } from "./command-executor.js";
import { BELL, CompletionChime } from "./completion-chime.js";
import { AssistantMessageComponent, type ThinkingDisplay } from "./components/assistant-message.js";
import { BashExecutionComponent } from "./components/bash-execution.js";
import { BranchSummaryMessageComponent } from "./components/branch-summary-message.js";
import { CompactionSummaryMessageComponent } from "./components/compaction-summary-message.js";
import { CountdownTimer } from "./components/countdown-timer.js";
import { CustomEditor } from "./components/custom-editor.js";
import { CustomMessageComponent } from "./components/custom-message.js";
import { DynamicBorder } from "./components/dynamic-border.js";
import { FooterComponent } from "./components/footer.js";
import { keyHint, keyText, rawKeyHint } from "./components/keybinding-hints.js";
import { renderSessionChip } from "./components/session-chip.js";
import { SessionColorSelectorComponent } from "./components/session-color-selector.js";
import { SessionSelectorComponent } from "./components/session-selector.js";
import { SettingsSelectorComponent } from "./components/settings-selector.js";
import { SkillInvocationMessageComponent } from "./components/skill-invocation-message.js";
import { TaskPanelComponent } from "./components/task-panel.js";
import { ToolChainComponent } from "./components/tool-chain.js";
import { ToolExecutionComponent } from "./components/tool-execution.js";
import { TreeSelectorComponent } from "./components/tree-selector.js";
import { UserMessageComponent } from "./components/user-message.js";
import { UserMessageSelectorComponent } from "./components/user-message-selector.js";
import { ExtensionChrome } from "./extension-chrome.js";
import { ExtensionDialogs } from "./extension-dialogs.js";
import { LoginController } from "./login-controller.js";
import { MessageQueueController } from "./message-queue-controller.js";
import { ModelController } from "./model-controller.js";
import {
	ExpandableText,
	formatDisplayPath,
	isExpandable,
	showLoadedResources as renderLoadedResources,
} from "./resource-display.js";
import { checkForPackageUpdates, checkTmuxKeyboardSetup, getChangelogForDisplay } from "./startup-checks.js";
import { TeamFocusController } from "./team-focus.js";
import {
	applyPaperSheet,
	getAvailableThemes,
	getAvailableThemesWithPaths,
	getEditorTheme,
	getMarkdownTheme,
	getThemeByName,
	initTheme,
	onThemeChange,
	setRegisteredThemes,
	setTheme,
	setThemeInstance,
	stopThemeWatcher,
	Theme,
	theme,
} from "./theme/theme.js";
import { VoiceController } from "./voice/voice-controller.js";
import { websearchApiKeyNotice } from "./websearch-warning.js";

/** Interface for components that can be expanded/collapsed */

const DEAD_TERMINAL_ERROR_CODES = new Set(["EIO", "EPIPE", "ENOTCONN"]);

/**
 * Resolve the effective voice trailing-silence window (ms): env
 * `VOICETOOLS_SILENCE_MS` wins (like `VOICETOOLS_BIN`), otherwise the settings
 * value. Both paths are clamped to 300-10000; a malformed env value falls back to
 * the settings value.
 */
function resolveVoiceSilenceMs(settingsManager: SettingsManager): number {
	const envRaw = process.env.VOICETOOLS_SILENCE_MS?.trim();
	if (envRaw) {
		const envValue = Number(envRaw);
		if (Number.isFinite(envValue) && envValue > 0) {
			return Math.min(10000, Math.max(300, Math.floor(envValue)));
		}
	}
	return settingsManager.getVoiceSilenceMs();
}

function isDeadTerminalError(error: unknown): boolean {
	if (!error || typeof error !== "object" || !("code" in error)) {
		return false;
	}
	const code = (error as NodeJS.ErrnoException).code;
	return code !== undefined && DEAD_TERMINAL_ERROR_CODES.has(code);
}

/**
 * Options for InteractiveMode initialization.
 */
export interface InteractiveModeOptions {
	/** Providers that were migrated to auth.json (shows warning) */
	migratedProviders?: string[];
	/** Warning message if session model couldn't be restored */
	modelFallbackMessage?: string;
	/** Initial message to send on startup (can include @file content) */
	initialMessage?: string;
	/** Images to attach to the initial message */
	initialImages?: ImageContent[];
	/** Additional messages to send after the initial message */
	initialMessages?: string[];
	/** Force verbose startup (overrides quietStartup setting) */
	verbose?: boolean;
}

// How often the streaming assistant message re-parses its markdown. Deltas can
// arrive far faster than this; every application re-lexes the growing tail
// block, so applying per-delta made streaming cost O(message²).
const STREAM_RENDER_THROTTLE_MS = 100;
// Task-store mutations (subagent tool progress, usage ticks) arrive in bursts;
// each render they trigger reassembles the whole component tree, so cap them.
const TASK_RENDER_THROTTLE_MS = 50;
// Finished tool blocks beyond this many (oldest first) are frozen to release
// their retained output/image payloads (see ToolExecutionComponent.freeze).
// Generous so nothing near the viewport is ever frozen; the cap bounds the
// view layer's memory across a long session heavy on tool output.
const LIVE_TOOL_WINDOW = 50;

/**
 * Leading+trailing throttle: the first call runs immediately, calls landing
 * inside the window coalesce into one trailing run with the latest state.
 */
function throttled(ms: number, fn: () => void): () => void {
	let timer: ReturnType<typeof setTimeout> | undefined;
	let pending = false;
	const run = () => {
		fn();
		timer = setTimeout(() => {
			timer = undefined;
			if (pending) {
				pending = false;
				run();
			}
		}, ms);
		timer.unref?.();
	};
	return () => {
		if (timer) {
			pending = true;
			return;
		}
		run();
	};
}

/**
 * Style a status/notify message for the chat.
 *
 * Plain messages are dimmed, as they always were. A message that already
 * carries escape codes is passed through untouched: `theme.fg` closes with
 * `\x1b[39m` (reset foreground, not "restore"), so wrapping a pre-styled string
 * in dim would drop the dim at the first inner span — and the TUI's wrapper
 * carries that reset state across newlines, so every following line would lose
 * it too. Passing through is what lets a listing colour its own columns.
 */
function styleStatusMessage(message: string): string {
	return message.includes("\x1b[") ? message : theme.fg("dim", message);
}

export class InteractiveMode {
	private runtimeHost: AgentSessionRuntime;
	private ui: TUI;
	private chatContainer: Container;
	private pendingMessagesContainer: Container;
	private statusContainer: Container;
	private defaultEditor: CustomEditor;
	private editor: EditorComponent;
	private editorComponentFactory: EditorFactory | undefined;
	private voice: VoiceController;
	private autocompleteProvider: AutocompleteProvider | undefined;
	private autocompleteProviderWrappers: AutocompleteProviderFactory[] = [];
	private fdPath: string | undefined;
	private editorContainer: Container;
	private footer: FooterComponent;
	private footerDataProvider: FooterDataProvider;
	// Stored so the same manager can be injected into custom editors, selectors, and extension UI.
	private keybindings: KeybindingsManager;
	private version: string;
	private isInitialized = false;
	private onInputCallback?: (text: string) => void;
	private loadingAnimation: Loader | undefined = undefined;
	private workingMessage: string | undefined = undefined;
	private workingVisible = true;
	private workingIndicatorOptions: LoaderIndicatorOptions | undefined = undefined;
	private readonly defaultWorkingMessage = "Working...";
	private readonly defaultHiddenThinkingLabel = "Thinking...";
	private hiddenThinkingLabel = this.defaultHiddenThinkingLabel;

	private lastSigintTime = 0;
	private lastEscapeTime = 0;
	private changelogMarkdown: string | undefined = undefined;
	private startupNoticesShown = false;

	// Status line tracking (for mutating immediately-sequential status updates)
	private lastStatusSpacer: Spacer | undefined = undefined;
	private lastStatusText: Text | undefined = undefined;

	// Streaming message tracking
	private streamingComponent: AssistantMessageComponent | undefined = undefined;
	private streamingMessage: AssistantMessage | undefined = undefined;
	/** Throttled re-parse/re-render of the in-flight assistant message. The
	 * guard makes a trailing run after message_end/agent_end a no-op; a direct
	 * updateContent on message_end flushes the final state regardless. */
	private readonly scheduleStreamingRender = throttled(STREAM_RENDER_THROTTLE_MS, () => {
		if (!this.streamingComponent || !this.streamingMessage) return;
		// streaming=true: large blocks render segmented so only the tail chunk
		// re-parses; message_end's direct updateContent renders the canonical form.
		this.streamingComponent.updateContent(this.streamingMessage, true);
		this.ui.requestRender();
	});

	// Tool execution tracking: toolCallId -> component
	private pendingTools = new Map<string, ToolExecutionComponent>();

	// Tool output expansion state
	private toolOutputExpanded = false;
	// Persisted view dial (radar / glance / full). See core/tool-output-view.ts.
	private toolOutputView: ToolOutputView = "glance";
	// The chain currently collecting tool calls, if the agent is mid-run.
	private openChain?: ToolChainComponent;
	/** The newest tool block in the transcript; radar marks it. */
	private latestToolBlock?: ToolExecutionComponent;
	/** The run that block belongs to; radar marks its folded line too. */
	private latestChain?: ToolChainComponent;
	// Whether the assistant message being streamed has said anything yet. The
	// first words it speaks close the chain the previous message's calls built.
	private sawTextInCurrentMessage = false;

	// Thinking block visibility state
	private hideThinkingBlock = false;

	// Skill commands: command name -> skill file path
	private skillCommands = new Map<string, string>();

	// Agent subscription unsubscribe function
	private unsubscribe?: () => void;
	// Task store subscription unsubscribe function (task panel)
	private taskStoreUnsubscribe?: () => void;
	// Startup-progress subscription unsubscribe function (footer download/index bars)
	private startupProgressUnsubscribe?: () => void;
	private signalCleanupHandlers: Array<() => void> = [];

	// Track if editor is in bash mode (text starts with !)
	private isBashMode = false;

	// Auto-compaction state
	private autoCompactionLoader: Loader | undefined = undefined;
	private autoCompactionEscapeHandler?: () => void;

	// Auto-retry state
	private retryLoader: Loader | undefined = undefined;
	private retryCountdown: CountdownTimer | undefined = undefined;
	private retryEscapeHandler?: () => void;

	// Completion chime state
	private chime?: CompletionChime;
	// True between auto_retry_start and the next agent_start: the turn is not done,
	// it is about to re-run, so the deferred completion check must not fire the chime.
	private chimePendingRetry = false;
	// Stop reason of the request's latest assistant message, reset when the user
	// starts a new one. Read at settle time by two consumers that ask the same
	// question — did this request end cleanly? An "aborted" turn means the user is
	// present, so no chime; anything other than a clean "stop" means dangling plan
	// items must not be claimed as completed.
	private turnStopReason: AssistantMessage["stopReason"] | undefined = undefined;

	// Cumulative usage + wall clock captured at the first agent_start of a request,
	// diffed at agent_end to print that request's own cost into the transcript.
	// Anchored on the FIRST start so a retried request reports one span, not one
	// per attempt (same reason the chime anchors its duration there).
	private turnCostAnchor: { totals: AssistantUsageTotals; at: number } | undefined;

	// Shutdown state
	private shutdownRequested = false;

	// Extension UI state
	private dialogs: ExtensionDialogs;
	private extensionTerminalInputUnsubscribers = new Set<() => void>();

	// Extension widgets (components rendered above/below the editor)
	private chrome!: ExtensionChrome;
	private widgetContainerAbove!: Container;
	private widgetContainerBelow!: Container;

	// Task panel shown just above the editor (active subagent tasks)
	private taskPanel: TaskPanelComponent;

	// hooteams team client (--team): steering + attach share its single SSE stream
	private teamFocus: TeamFocusController;

	// Bash command execution (the `!cmd` prompt mode)
	private bashExecution: BashExecutionController;

	// Message queueing (compaction queue + pending-messages display)
	private messageQueue: MessageQueueController;

	// Model selection (/model, /models, cycle keys)
	private modelController: ModelController;

	// Auth / login flows (/login, /logout)
	private loginController: LoginController;

	// Header container that holds the built-in or custom header
	private headerContainer: Container;

	// Built-in header (logo + keybinding hints + changelog)
	private builtInHeader: Component | undefined = undefined;

	// Convenience accessors
	private get session(): AgentSession {
		return this.runtimeHost.session;
	}
	private _commandExecutor?: CommandExecutor;
	/**
	 * Lazily-built command executor. The context uses getters for mutable
	 * dependencies (e.g. the active session) so handlers always operate on the
	 * current state even after a session switch.
	 */
	private get commandExecutor(): CommandExecutor {
		if (!this._commandExecutor) {
			const self = this;
			const context: CommandContext = {
				get session() {
					return self.session;
				},
				get sessionManager() {
					return self.sessionManager;
				},
				get runtimeHost() {
					return self.runtimeHost;
				},
				get ui() {
					return self.ui;
				},
				get editor() {
					return self.editor;
				},
				get editorContainer() {
					return self.editorContainer;
				},
				get chatContainer() {
					return self.chatContainer;
				},
				get statusContainer() {
					return self.statusContainer;
				},
				get footer() {
					return self.footer;
				},
				get keybindings() {
					return self.keybindings;
				},
				showStatus: (message) => self.showStatus(message),
				showError: (message) => self.showError(message),
				showWarning: (message) => self.showWarning(message),
				updateEditorBorderColor: () => self.updateEditorBorderColor(),
				renderCurrentSessionState: () => self.renderCurrentSessionState(),
				rebuildChatFromMessages: () => self.rebuildChatFromMessages(),
				getMarkdownThemeWithSettings: () => self.getMarkdownThemeWithSettings(),
				stopLoadingAnimation: () => self.stopLoadingAnimation(),
				findExactModelMatch: (searchTerm) => self.modelController.findExactModelMatch(searchTerm),
				maybeWarnAboutAnthropicSubscriptionAuth: (model) =>
					self.modelController.maybeWarnAboutAnthropicSubscriptionAuth(model),
				showModelSelector: (searchTerm) => self.modelController.showModelSelector(searchTerm),
				showExtensionConfirm: (title, message) => self.dialogs.confirm(title, message),
				promptForMissingSessionCwd: (error) => self.promptForMissingSessionCwd(error),
				handleFatalRuntimeError: (prefix, error) => self.handleFatalRuntimeError(prefix, error),
			};
			this._commandExecutor = new CommandExecutor(context);
		}
		return this._commandExecutor;
	}
	private stopLoadingAnimation(): void {
		if (this.loadingAnimation) {
			this.loadingAnimation.stop();
			this.loadingAnimation = undefined;
		}
	}
	private get agent() {
		return this.session.agent;
	}
	private get sessionManager() {
		return this.session.sessionManager;
	}
	private get settingsManager() {
		return this.session.settingsManager;
	}

	constructor(
		runtimeHost: AgentSessionRuntime,
		private options: InteractiveModeOptions = {},
	) {
		this.runtimeHost = runtimeHost;
		this.runtimeHost.setBeforeSessionInvalidate(() => {
			this.resetExtensionUI();
		});
		this.runtimeHost.setRebindSession(async () => {
			await this.rebindCurrentSession();
		});
		this.version = VERSION;
		this.ui = new TUI(new ProcessTerminal(), this.settingsManager.getShowHardwareCursor());
		this.ui.setClearOnShrink(this.settingsManager.getClearOnShrink());
		this.headerContainer = new Container();
		this.chatContainer = new Container();
		this.pendingMessagesContainer = new Container();
		this.statusContainer = new Container();
		this.widgetContainerAbove = new Container();
		this.widgetContainerBelow = new Container();
		this.keybindings = KeybindingsManager.create();
		setKeybindings(this.keybindings);
		const editorPaddingX = this.settingsManager.getEditorPaddingX();
		const autocompleteMaxVisible = this.settingsManager.getAutocompleteMaxVisible();
		this.defaultEditor = new CustomEditor(this.ui, getEditorTheme(), this.keybindings, {
			border: this.settingsManager.getEditorBorder(),
			paddingX: editorPaddingX,
			autocompleteMaxVisible,
		});
		this.editor = this.defaultEditor;
		this.editor.promptPrefix = "❯";
		this.editorContainer = new Container();
		this.editorContainer.addChild(this.editor as Component);
		this.footerDataProvider = new FooterDataProvider(this.sessionManager.getCwd());
		this.footer = new FooterComponent(this.session, this.footerDataProvider);
		this.footer.setAutoCompactEnabled(this.session.autoCompactionEnabled);
		this.footer.setToolOutputView(this.toolOutputView);
		this.footerDataProvider.setSubagentEnabled(this.session.getActiveToolNames().includes("Task"));
		this.chrome = new ExtensionChrome({
			ui: this.ui,
			widgetContainerAbove: this.widgetContainerAbove,
			widgetContainerBelow: this.widgetContainerBelow,
			headerContainer: this.headerContainer,
			footer: this.footer,
			footerDataProvider: this.footerDataProvider,
			getBuiltInHeader: () => this.builtInHeader,
			isToolOutputExpanded: () => this.toolOutputExpanded,
		});
		this.taskPanel = new TaskPanelComponent(this.ui);
		this.dialogs = new ExtensionDialogs({
			ui: this.ui,
			editorContainer: this.editorContainer,
			keybindings: this.keybindings,
			getEditor: () => this.editor,
		});
		this.teamFocus = new TeamFocusController({
			ui: this.ui,
			taskPanel: this.taskPanel,
			editorContainer: this.editorContainer,
			getEditor: () => this.editor as Component,
			showStatus: (message) => this.showStatus(message),
			showWarning: (message) => this.showWarning(message),
			showAskOptions: (questions, opts) => this.dialogs.showAskOptions(questions, opts),
			isAskOptionsOpen: () => this.dialogs.isAskOptionsOpen(),
		});
		this.voice = new VoiceController({
			ui: this.ui,
			statusContainer: this.statusContainer,
			sendEditorInput: (data) => this.editor.handleInput(data),
			showError: (message) => this.showError(message),
			silenceMs: resolveVoiceSilenceMs(this.settingsManager),
		});
		const self = this;
		this.bashExecution = new BashExecutionController({
			get session() {
				return self.session;
			},
			ui: this.ui,
			pendingMessagesContainer: this.pendingMessagesContainer,
			chatContainer: this.chatContainer,
			showError: (message) => this.showError(message),
		});
		this.messageQueue = new MessageQueueController({
			get session() {
				return self.session;
			},
			getEditor: () => this.editor,
			pendingMessagesContainer: this.pendingMessagesContainer,
			showStatus: (message) => this.showStatus(message),
			showError: (message) => this.showError(message),
		});
		this.modelController = new ModelController({
			ui: this.ui,
			get session() {
				return self.session;
			},
			showSelector: (create) => this.showSelector(create),
			showStatus: (message) => this.showStatus(message),
			showError: (message) => this.showError(message),
			showWarning: (message) => this.showWarning(message),
			showNotice: (title, body) => this.showNotice(title, body),
			updateEditorBorderColor: () => this.updateEditorBorderColor(),
			invalidateFooter: () => this.footer.invalidate(),
			setAvailableProviderCount: (count) => this.footerDataProvider.setAvailableProviderCount(count),
		});
		this.loginController = new LoginController({
			ui: this.ui,
			get session() {
				return self.session;
			},
			getEditor: () => this.editor as Component,
			editorContainer: this.editorContainer,
			showSelector: (create) => this.showSelector(create),
			showStatus: (message) => this.showStatus(message),
			showError: (message) => this.showError(message),
			updateAvailableProviderCount: () => this.modelController.updateAvailableProviderCount(),
			updateEditorBorderColor: () => this.updateEditorBorderColor(),
			invalidateFooter: () => this.footer.invalidate(),
			maybeWarnAboutAnthropicSubscriptionAuth: (model) =>
				this.modelController.maybeWarnAboutAnthropicSubscriptionAuth(model),
		});
		this.taskPanel.onNudge = (role) => this.teamFocus.showNudgeInput(role);
		this.taskPanel.onAttach = (role) => this.teamFocus.showAttach(role);
		this.taskPanel.onExitFocus = () => this.teamFocus.exitFocus();

		// Load hide thinking block setting
		this.hideThinkingBlock = this.settingsManager.getHideThinkingBlock();

		// Load the tool-output view dial
		this.toolOutputView = this.settingsManager.getToolOutputView();

		// Completion chime: rings the terminal bell when a long turn finishes or the
		// agent blocks awaiting input. Enable is read fresh so a live toggle applies.
		this.chime = new CompletionChime({
			isEnabled: () => this.settingsManager.getChimeOnTurnComplete(),
			ring: () => this.ui.terminal.write(BELL),
		});

		// Register themes from resource loader and initialize
		setRegisteredThemes(this.session.resourceLoader.getThemes().themes);
		initTheme(this.settingsManager.getTheme(), true);
	}

	private getAutocompleteSourceTag(sourceInfo?: SourceInfo): string | undefined {
		if (!sourceInfo) {
			return undefined;
		}

		const scopePrefix = sourceInfo.scope === "user" ? "u" : sourceInfo.scope === "project" ? "p" : "t";
		const source = sourceInfo.source.trim();

		if (source === "auto" || source === "local" || source === "cli") {
			return scopePrefix;
		}

		if (source.startsWith("npm:")) {
			return `${scopePrefix}:${source}`;
		}

		const gitSource = parseGitUrl(source);
		if (gitSource) {
			const ref = gitSource.ref ? `@${gitSource.ref}` : "";
			return `${scopePrefix}:git:${gitSource.host}/${gitSource.path}${ref}`;
		}

		return scopePrefix;
	}

	private prefixAutocompleteDescription(description: string | undefined, sourceInfo?: SourceInfo): string | undefined {
		const sourceTag = this.getAutocompleteSourceTag(sourceInfo);
		if (!sourceTag) {
			return description;
		}
		return description ? `[${sourceTag}] ${description}` : `[${sourceTag}]`;
	}

	private getBuiltInCommandConflictDiagnostics(extensionRunner: ExtensionRunner): ResourceDiagnostic[] {
		const builtinNames = new Set(BUILTIN_SLASH_COMMANDS.map((command) => command.name));
		return extensionRunner
			.getRegisteredCommands()
			.filter((command) => builtinNames.has(command.name))
			.map((command) => ({
				type: "warning" as const,
				message:
					command.invocationName === command.name
						? `Extension command '/${command.name}' conflicts with built-in interactive command. Skipping in autocomplete.`
						: `Extension command '/${command.name}' conflicts with built-in interactive command. Available as '/${command.invocationName}'.`,
				path: command.sourceInfo.path,
			}));
	}

	private createBaseAutocompleteProvider(): AutocompleteProvider {
		// Define commands for autocomplete
		const slashCommands: SlashCommand[] = BUILTIN_SLASH_COMMANDS.map((command) => ({
			name: command.name,
			description: command.description,
		}));

		const modelCommand = slashCommands.find((command) => command.name === "model");
		if (modelCommand) {
			modelCommand.getArgumentCompletions = (prefix: string): AutocompleteItem[] | null => {
				// Get available models (scoped or from registry)
				const models =
					this.session.scopedModels.length > 0
						? this.session.scopedModels.map((s) => s.model)
						: this.session.modelRegistry.getAvailable();

				if (models.length === 0) return null;

				// Create items with provider/id format
				const items = models.map((m) => ({
					id: m.id,
					provider: m.provider,
					label: `${m.provider}/${m.id}`,
				}));

				// Fuzzy filter by model ID + provider (allows "opus anthropic" to match)
				const filtered = fuzzyFilter(items, prefix, (item) => `${item.id} ${item.provider}`);

				if (filtered.length === 0) return null;

				return filtered.map((item) => ({
					value: item.label,
					label: item.id,
					description: item.provider,
				}));
			};
		}

		const cdCommand = slashCommands.find((command) => command.name === "cd");
		if (cdCommand) {
			cdCommand.argumentHint = "<path>";
			cdCommand.getArgumentCompletions = (prefix: string): AutocompleteItem[] | null => {
				const completions = this.commandExecutor.getChangeDirectoryCompletions(prefix);
				return completions.length > 0 ? completions : null;
			};
		}

		// Convert prompt templates to SlashCommand format for autocomplete
		const templateCommands: SlashCommand[] = this.session.promptTemplates.map((cmd) => ({
			name: cmd.name,
			description: this.prefixAutocompleteDescription(cmd.description, cmd.sourceInfo),
			...(cmd.argumentHint && { argumentHint: cmd.argumentHint }),
		}));

		// Convert extension commands to SlashCommand format
		const builtinCommandNames = new Set(slashCommands.map((c) => c.name));
		const extensionCommands: SlashCommand[] = this.session.extensionRunner
			.getRegisteredCommands()
			.filter((cmd) => !builtinCommandNames.has(cmd.name))
			.map((cmd) => ({
				name: cmd.invocationName,
				description: this.prefixAutocompleteDescription(cmd.description, cmd.sourceInfo),
				getArgumentCompletions: cmd.getArgumentCompletions,
			}));

		// Build skill commands from session.skills (if enabled)
		this.skillCommands.clear();
		const skillCommandList: SlashCommand[] = [];
		if (this.settingsManager.getEnableSkillCommands()) {
			for (const skill of this.session.resourceLoader.getSkills().skills) {
				const commandName = `skill:${skill.name}`;
				this.skillCommands.set(commandName, skill.filePath);
				skillCommandList.push({
					name: commandName,
					description: this.prefixAutocompleteDescription(skill.description, skill.sourceInfo),
				});
			}
		}

		return new CombinedAutocompleteProvider(
			[...slashCommands, ...templateCommands, ...extensionCommands, ...skillCommandList],
			this.sessionManager.getCwd(),
			this.fdPath,
		);
	}

	private setupAutocompleteProvider(): void {
		let provider = this.createBaseAutocompleteProvider();
		for (const wrapProvider of this.autocompleteProviderWrappers) {
			provider = wrapProvider(provider);
		}

		this.autocompleteProvider = provider;
		this.defaultEditor.setAutocompleteProvider(provider);
		if (this.editor !== this.defaultEditor) {
			this.editor.setAutocompleteProvider?.(provider);
		}
	}

	private showStartupNoticesIfNeeded(): void {
		if (this.startupNoticesShown) {
			return;
		}
		this.startupNoticesShown = true;

		if (!this.changelogMarkdown || !this.changelogMarkdown.trim()) {
			return;
		}

		if (this.chatContainer.children.length > 0) {
			this.chatContainer.addChild(new Spacer(1));
		}
		this.chatContainer.addChild(new DynamicBorder());
		if (this.settingsManager.getCollapseChangelog()) {
			const versionMatch = this.changelogMarkdown.match(/##\s+\[?(\d+\.\d+\.\d+)\]?/);
			const latestVersion = versionMatch ? versionMatch[1] : this.version;
			const condensedText = `Updated to v${latestVersion}. Use ${theme.bold("/changelog")} to view full changelog.`;
			this.chatContainer.addChild(new Text(condensedText, 1, 0));
		} else {
			this.chatContainer.addChild(new Text(theme.bold(theme.fg("accent", "What's New")), 1, 0));
			this.chatContainer.addChild(new Spacer(1));
			this.chatContainer.addChild(
				new Markdown(this.changelogMarkdown.trim(), 1, 0, this.getMarkdownThemeWithSettings()),
			);
			this.chatContainer.addChild(new Spacer(1));
		}
		this.chatContainer.addChild(new DynamicBorder());
	}

	/** Feed a first-run tool-binary download into the footer's startup-progress line. */
	private reportToolDownload(key: string, label: string, receivedBytes: number, totalBytes: number | null): void {
		startupProgress.set({ key, kind: "download", label, receivedBytes, totalBytes });
	}

	async init(): Promise<void> {
		if (this.isInitialized) return;

		this.registerSignalHandlers();

		// Load changelog (only show new entries, skip for resumed sessions)
		this.changelogMarkdown = getChangelogForDisplay({
			hasMessages: this.session.state.messages.length > 0,
			settingsManager: this.settingsManager,
		});

		// Ensure fd and rg are available (downloads if missing, adds to PATH via getBinDir).
		// Both help — fd for autocomplete, rg for the grep tool — but neither is required
		// to start: resolving them can hit a slow or failing network in restricted
		// environments, and awaiting here froze first paint. Resolve in the background
		// (never awaited, so a first-run download never blocks first paint) and wire fd
		// in when ready; the grep/find tools resolve rg/fd on demand with a native
		// fallback regardless. embsearch is intentionally NOT preloaded here: when the
		// semantic index is enabled, EmbsearchService (main.ts) owns its download and
		// build, reporting through the same startupProgress channel — preloading it too
		// would fetch the binary twice and race.
		//
		// First-run download progress streams into the footer's startupProgress line
		// via ensureTool's byte-count callback; the line clears when the download
		// settles (path resolved) or is dropped if it fails.
		void Promise.all([
			ensureTool("fd", true, (received, total) => this.reportToolDownload("fd", "fd", received, total)),
			ensureTool("rg", true, (received, total) => this.reportToolDownload("rg", "ripgrep", received, total)),
		])
			.then(([fdPath]) => {
				this.fdPath = fdPath;
			})
			.catch(() => {
				// Non-fatal: autocomplete simply stays disabled until/unless fd resolves.
			})
			.finally(() => {
				// Downloads settled (resolved or failed): drop any lingering bars.
				startupProgress.remove("fd");
				startupProgress.remove("rg");
			});

		// Add header container as first child
		this.ui.addChild(this.headerContainer);

		// Add header with keybindings from config (unless silenced)
		if (this.options.verbose || !this.settingsManager.getQuietStartup()) {
			const termW = this.ui.terminal.columns;
			// A function, not a value: the banner names the working directory, and
			// `/cd` moves it. `refreshBuiltInHeader` re-reads this after a move.
			const logo = () =>
				termW >= 40
					? buildCompactWordmark({
							appName: APP_NAME,
							version: this.version,
							cwd: formatDisplayPath(this.sessionManager.getCwd()),
							accent: (text) => theme.fg("accent", text),
							dim: (text) => theme.fg("dim", text),
							muted: (text) => theme.fg("muted", text),
							cursor: (text) => theme.blink(theme.fg("accent", text)),
							note: () => theme.fg("dim", `  ${keyText("app.tools.expand")} more`),
						})
					: theme.bold(theme.fg("accent", APP_NAME)) + theme.fg("dim", ` v${this.version}`);

			// Build startup instructions using keybinding hint helpers
			const hint = (keybinding: AppKeybinding, description: string) => keyHint(keybinding, description);

			const expandedInstructions = [
				hint("app.interrupt", "to interrupt"),
				hint("app.clear", "to clear"),
				rawKeyHint(`${keyText("app.clear")} twice`, "to exit"),
				hint("app.exit", "to exit (empty)"),
				hint("app.suspend", "to suspend"),
				keyHint("tui.editor.deleteToLineEnd", "to delete to end"),
				hint("app.thinking.cycle", "to cycle thinking level"),
				rawKeyHint(`${keyText("app.model.cycleForward")}/${keyText("app.model.cycleBackward")}`, "to cycle models"),
				hint("app.model.select", "to select model"),
				hint("app.tools.expand", "to expand all tool output"),
				hint("app.tools.unfoldOne", "to open one chain or block (repeat to peel back)"),
				hint("app.view.cycleForward", "to cycle tool output (radar/glance/full)"),
				hint("app.thinking.toggle", "to expand thinking"),
				hint("app.tasks.cycleView", "to cycle task panel view"),
				...(this.teamFocus.connected ? [hint("app.team.focus", "to focus team roster")] : []),
				hint("app.mode.cycle", "to cycle agent mode"),
				hint("app.session.changeDirectory", "to change working directory"),
				hint("app.settings.open", "for settings"),
				hint("app.hotkeys.open", "for all shortcuts"),
				hint("app.editor.external", "for external editor"),
				rawKeyHint("/", "for commands"),
				rawKeyHint("!", "to run bash"),
				rawKeyHint("!!", "to run bash (no context)"),
				hint("app.message.followUp", "to queue follow-up"),
				hint("app.message.dequeue", "to edit all queued messages"),
				hint("app.clipboard.pasteImage", "to paste image"),
				rawKeyHint("drop files", "to attach"),
			].join("\n");
			const onboarding = theme.fg(
				"dim",
				`${APP_NAME} can explain its own features and look up its docs. Ask it how to use or extend ${APP_NAME}.`,
			);
			this.builtInHeader = new ExpandableText(
				() => logo(),
				() => `${logo()}\n${expandedInstructions}\n\n${onboarding}`,
				this.getStartupExpansionState(),
				1,
				0,
			);

			// Setup UI layout
			this.headerContainer.addChild(this.builtInHeader);
		} else {
			// Minimal header when silenced
			this.builtInHeader = new Text("", 0, 0);
			this.headerContainer.addChild(this.builtInHeader);
		}

		this.ui.addChild(this.chatContainer);
		this.ui.addChild(this.pendingMessagesContainer);
		this.ui.addChild(this.statusContainer);
		this.chrome.renderWidgets(); // Initialize with default spacer
		this.ui.addChild(this.widgetContainerAbove);
		this.ui.addChild(this.taskPanel);
		this.ui.addChild(this.editorContainer);
		this.ui.addChild(this.widgetContainerBelow);
		this.ui.addChild(this.footer);
		this.ui.setFocus(this.editor);

		this.setupKeyHandlers();
		this.setupEditorSubmitHandler();

		// Start the UI before initializing extensions so session_start handlers can use interactive dialogs
		this.ui.start();
		// From here the TUI owns the terminal, so the agent's operational log lines
		// (dispatch, warm fallback, lifeguard) must stop writing to it — a write the
		// renderer did not make desyncs its cursor bookkeeping and duplicates rows.
		setTerminalOwnedByTui(true);
		this.isInitialized = true;

		// Initialize extensions first so resources are shown before messages
		await this.rebindCurrentSession();

		// Render initial messages AFTER showing loaded resources
		this.renderInitialMessages();

		// Set up theme file watcher
		onThemeChange(() => {
			this.ui.invalidate();
			this.updateEditorBorderColor();
			// The chip's fill and ink are baked in at build time, so a theme swap
			// has to rebuild it or it keeps the old theme's colours.
			this.updateSessionChip();
			this.ui.requestRender();
		});

		// Set up git branch watcher (uses provider instead of footer)
		this.footerDataProvider.onBranchChange(() => {
			this.ui.requestRender();
		});

		// Re-render the UI when the task list changes (task panel shows active
		// tasks). Throttled: subagent progress mutates the store in bursts, and
		// each resulting render reassembles the entire component tree.
		this.taskStoreUnsubscribe = taskStore.subscribe(
			throttled(TASK_RENDER_THROTTLE_MS, () => {
				this.ui.requestRender();
			}),
		);

		// Re-render the footer as startup progress updates (tool downloads stream
		// byte counts in bursts, the index build ticks per batch). Throttled like the
		// task panel so a fast download doesn't drive a render per chunk.
		this.startupProgressUnsubscribe = startupProgress.subscribe(
			throttled(TASK_RENDER_THROTTLE_MS, () => {
				this.ui.requestRender();
			}),
		);

		// Initialize available provider count for footer display
		await this.modelController.updateAvailableProviderCount();
	}

	/**
	 * Refresh everything in the chrome that says *which session this is*: the
	 * terminal title and the chip on the input box. Both read from the same
	 * source, so they can never disagree about a rename.
	 */
	private refreshSessionIdentity(): void {
		this.updateTerminalTitle();
		this.updateSessionChip();
	}

	/**
	 * Rebuild the session chip and hand it to the editor, telling the footer to
	 * stand down from showing the name itself. Cheap enough to call on any change
	 * that could move the name, the colour, or the theme they are drawn in.
	 */
	private updateSessionChip(): void {
		const chip = renderSessionChip(this.sessionManager.getDisplayName(), this.sessionManager.getSessionColorSlot());
		this.defaultEditor.topBorderLabel = chip;
		if (this.editor !== this.defaultEditor) {
			this.editor.topBorderLabel = chip;
		}
		this.footer.setSessionChipShown(chip !== undefined);
		this.ui.requestRender();
	}

	/**
	 * Update terminal title with session name and cwd.
	 */
	private updateTerminalTitle(): void {
		const cwdBasename = path.basename(this.sessionManager.getCwd());
		const sessionName = this.sessionManager.getSessionName();
		if (sessionName) {
			this.ui.terminal.setTitle(`${APP_TITLE} - ${sessionName} - ${cwdBasename}`);
		} else {
			this.ui.terminal.setTitle(`${APP_TITLE} - ${cwdBasename}`);
		}
	}

	/**
	 * Run the interactive mode. This is the main entry point.
	 * Initializes the UI, shows warnings, processes initial messages, and starts the interactive loop.
	 */
	async run(): Promise<void> {
		await this.init();

		// Start version check asynchronously
		checkForNewHooCodeVersion(this.version).then((newVersion) => {
			if (newVersion) {
				this.showNewVersionNotification(newVersion);
			}
		});

		// Start package update check asynchronously
		checkForPackageUpdates(this.sessionManager.getCwd(), this.settingsManager).then((updates) => {
			if (updates.length > 0) {
				this.showPackageUpdateNotification(updates);
			}
		});

		// Check tmux keyboard setup asynchronously
		checkTmuxKeyboardSetup().then((warning) => {
			if (warning) {
				this.showWarning(warning);
			}
		});

		// Show startup warnings
		const { migratedProviders, modelFallbackMessage, initialMessage, initialImages, initialMessages } = this.options;

		if (migratedProviders && migratedProviders.length > 0) {
			this.showWarning(`Migrated credentials to auth.json: ${migratedProviders.join(", ")}`);
		}

		const modelsJsonError = this.session.modelRegistry.getError();
		if (modelsJsonError) {
			this.showError(`models.json error: ${modelsJsonError}`);
		}

		if (modelFallbackMessage) {
			this.showWarning(modelFallbackMessage);
		}

		void this.modelController.maybeWarnAboutAnthropicSubscriptionAuth();
		this.maybeWarnAboutMissingWebSearchKey();

		// Process initial messages
		if (initialMessage) {
			try {
				await this.session.prompt(initialMessage, { images: initialImages });
			} catch (error: unknown) {
				const errorMessage = error instanceof Error ? error.message : "Unknown error occurred";
				this.showError(errorMessage);
			}
		}

		if (initialMessages) {
			for (const message of initialMessages) {
				try {
					await this.session.prompt(message);
				} catch (error: unknown) {
					const errorMessage = error instanceof Error ? error.message : "Unknown error occurred";
					this.showError(errorMessage);
				}
			}
		}

		// Main interactive loop
		while (true) {
			const userInput = await this.getUserInput();
			try {
				await this.session.prompt(userInput);
			} catch (error: unknown) {
				const errorMessage = error instanceof Error ? error.message : "Unknown error occurred";
				this.showError(errorMessage);
			}
		}
	}

	private getMarkdownThemeWithSettings(): MarkdownTheme {
		return {
			...getMarkdownTheme(),
			codeBlockIndent: this.settingsManager.getCodeBlockIndent(),
		};
	}

	// =========================================================================
	// Extension System
	// =========================================================================

	private getStartupExpansionState(): boolean {
		return this.options.verbose || this.toolOutputExpanded;
	}

	/** Render the startup/reload resource listing (see resource-display.ts). */
	private showLoadedResources(options?: {
		extensions?: Array<{ path: string; sourceInfo?: SourceInfo }>;
		force?: boolean;
		showDiagnosticsWhenQuiet?: boolean;
	}): void {
		renderLoadedResources(
			{
				chatContainer: this.chatContainer,
				getCwd: () => this.sessionManager.getCwd(),
				getResourceLoader: () => this.session.resourceLoader,
				getPromptTemplates: () => this.session.promptTemplates,
				getExtensionRunner: () => this.session.extensionRunner,
				getActiveMode: () => this.footerDataProvider.getActiveMode(),
				getSubagentEnabled: () => this.footerDataProvider.getSubagentEnabled(),
				getAgentCount: () => this.getDispatchableAgentCount(),
				// Cheap and fork-free: directory entries plus each installed plugin's
				// resolved canvas dirs. Computed inside the listing branch, so a quiet
				// startup pays nothing for it.
				getCanvases: () => {
					try {
						const cwd = this.sessionManager.getCwd();
						const found = discoverCanvasExtensions(
							canvasSearchRoots(cwd, os.homedir()),
							pluginCanvasExtensions(cwd),
						);
						const gated = gateCanvasExtensions(found, cwd);
						return [
							...gated.runnable.map((e) => ({ id: e.id, scope: e.scope as string, withheld: false })),
							...gated.withheld.map((w) => ({
								id: w.extension.id,
								scope: w.extension.scope as string,
								withheld: true,
							})),
						];
					} catch {
						return [];
					}
				},
				getAgents: () => {
					if (!this.footerDataProvider.getSubagentEnabled()) return [];
					try {
						return loadAgentRegistry({ cwd: this.sessionManager.getCwd() }).list();
					} catch {
						return [];
					}
				},
				getColumns: () => this.ui.terminal.columns,
				quietStartup: () => this.settingsManager.getQuietStartup(),
				verbose: this.options.verbose ?? false,
				isExpanded: () => this.getStartupExpansionState(),
				getBuiltInCommandConflictDiagnostics: (runner) => this.getBuiltInCommandConflictDiagnostics(runner),
			},
			options,
		);
	}

	/**
	 * Agents the model can actually dispatch. Zero when the Task tool is off, so
	 * the summary never advertises a capability the session cannot use.
	 */
	private getDispatchableAgentCount(): number {
		if (!this.footerDataProvider.getSubagentEnabled()) return 0;
		try {
			return loadAgentRegistry({ cwd: this.sessionManager.getCwd() }).list().length;
		} catch {
			return 0;
		}
	}

	/**
	 * Initialize the extension system with TUI-based UI context.
	 */
	private async bindCurrentSessionExtensions(): Promise<void> {
		const uiContext = this.createExtensionUIContext();
		await this.session.bindExtensions({
			uiContext,
			commandContextActions: {
				waitForIdle: () => this.session.agent.waitForIdle(),
				newSession: async (options) => {
					if (this.loadingAnimation) {
						this.loadingAnimation.stop();
						this.loadingAnimation = undefined;
					}
					this.statusContainer.clear();
					try {
						const result = await this.runtimeHost.newSession(options);
						if (!result.cancelled) {
							this.renderCurrentSessionState();
							this.ui.requestRender();
						}
						return result;
					} catch (error: unknown) {
						return this.handleFatalRuntimeError("Failed to create session", error);
					}
				},
				fork: async (entryId, options) => {
					try {
						const result = await this.runtimeHost.fork(entryId, options);
						if (!result.cancelled) {
							this.renderCurrentSessionState();
							this.editor.setText(result.selectedText ?? "");
							this.showStatus("Forked to new session");
						}
						return { cancelled: result.cancelled };
					} catch (error: unknown) {
						return this.handleFatalRuntimeError("Failed to fork session", error);
					}
				},
				navigateTree: async (targetId, options) => {
					const result = await this.session.navigateTree(targetId, {
						summarize: options?.summarize,
						customInstructions: options?.customInstructions,
						replaceInstructions: options?.replaceInstructions,
						label: options?.label,
					});
					if (result.cancelled) {
						return { cancelled: true };
					}

					this.chatContainer.clear();
					this.renderInitialMessages();
					if (result.editorText && !this.editor.getText().trim()) {
						this.editor.setText(result.editorText);
					}
					this.showStatus("Navigated to selected point");
					void this.messageQueue.flushCompactionQueue({ willRetry: false });
					return { cancelled: false };
				},
				switchSession: async (sessionPath, options) => {
					return this.handleResumeSession(sessionPath, options);
				},
				reload: async () => {
					await this.handleReloadCommand();
				},
			},
			shutdownHandler: () => {
				this.shutdownRequested = true;
				if (!this.session.isStreaming) {
					void this.shutdown();
				}
			},
			onError: (error) => {
				this.showExtensionError(error.extensionPath, error.error, error.stack);
			},
		});

		setRegisteredThemes(this.session.resourceLoader.getThemes().themes);
		this.setupAutocompleteProvider();

		const extensionRunner = this.session.extensionRunner;
		this.setupExtensionShortcuts(extensionRunner);
		// The startup path draws the listing here, because it renders messages
		// without clearing. A *rebind* (/new, /resume, /fork) clears the transcript
		// straight afterwards, so `renderCurrentSessionState` draws it again — this
		// call is the one startup keeps.
		this.showLoadedResources({ force: false, showDiagnosticsWhenQuiet: true });
		this.showStartupNoticesIfNeeded();
	}

	private applyRuntimeSettings(): void {
		this.footer.setSession(this.session);
		this.footer.setAutoCompactEnabled(this.session.autoCompactionEnabled);
		this.footer.setToolOutputView(this.toolOutputView);
		// The startup banner names the cwd, which `/cd` changes under it.
		if (this.builtInHeader instanceof ExpandableText) {
			this.builtInHeader.refresh();
		}
		this.footerDataProvider.setCwd(this.sessionManager.getCwd());
		this.hideThinkingBlock = this.settingsManager.getHideThinkingBlock();
		this.ui.setShowHardwareCursor(this.settingsManager.getShowHardwareCursor());
		this.ui.setClearOnShrink(this.settingsManager.getClearOnShrink());
		const editorBorder = this.settingsManager.getEditorBorder();
		const editorPaddingX = this.settingsManager.getEditorPaddingX();
		const autocompleteMaxVisible = this.settingsManager.getAutocompleteMaxVisible();
		this.defaultEditor.setBorder(editorBorder);
		this.defaultEditor.setPaddingX(editorPaddingX);
		this.defaultEditor.setAutocompleteMaxVisible(autocompleteMaxVisible);
		if (this.editor !== this.defaultEditor) {
			this.editor.setBorder?.(editorBorder);
			this.editor.setPaddingX?.(editorPaddingX);
			this.editor.setAutocompleteMaxVisible?.(autocompleteMaxVisible);
		}
	}

	private async rebindCurrentSession(): Promise<void> {
		this.unsubscribe?.();
		this.unsubscribe = undefined;
		this.applyRuntimeSettings();
		await this.bindCurrentSessionExtensions();
		this.subscribeToAgent();
		await this.modelController.updateAvailableProviderCount();
		this.updateEditorBorderColor();
		this.refreshSessionIdentity();
	}

	private async handleFatalRuntimeError(prefix: string, error: unknown): Promise<never> {
		const message = error instanceof Error ? error.message : String(error);
		this.showError(`${prefix}: ${message}`);
		stopThemeWatcher();
		this.stop();
		process.exit(1);
	}

	/**
	 * Reset the transcript to whatever the (possibly just-swapped) session holds.
	 *
	 * Every caller — /new, /resume, /fork — reaches here *after* the runtime has
	 * rebound extensions, and rebinding is what renders the loaded-resource
	 * listing. Clearing the chat therefore wiped the listing a moment after it was
	 * drawn, which is why /reload showed skills, agents and plugins and starting a
	 * new session showed nothing. Re-rendering it here is what makes the surface
	 * common: one call site, so a session change of any kind reports the same
	 * capabilities /reload does.
	 */
	private renderCurrentSessionState(): void {
		this.chatContainer.clear();
		this.openChain = undefined;
		this.latestToolBlock = undefined;
		this.latestChain = undefined;
		this.pendingMessagesContainer.clear();
		this.messageQueue.resetCompactionQueue();
		this.streamingComponent = undefined;
		this.streamingMessage = undefined;
		this.pendingTools.clear();
		this.showLoadedResources({ force: false, showDiagnosticsWhenQuiet: true });
		this.renderInitialMessages();
	}

	/**
	 * Bound the view layer's memory: freeze finished tool blocks once more than
	 * LIVE_TOOL_WINDOW of them are live, releasing their retained tool output and
	 * base64 image copies. Runs on tool completion (infrequent) and only ever
	 * freezes blocks far above the viewport. The session data stays intact, so a
	 * later full rebuild (theme toggle / reload) restores full fidelity.
	 */
	private trimTranscriptMemory(): void {
		const freezable = this.transcriptToolBlocks().filter((block) => block.isFreezable());
		const excess = freezable.length - LIVE_TOOL_WINDOW;
		for (let i = 0; i < excess; i++) {
			freezable[i].freeze();
		}
	}

	/**
	 * Get a registered tool definition by name (for custom rendering).
	 */
	private getRegisteredToolDefinition(toolName: string) {
		return this.session.getToolDefinition(toolName);
	}

	/**
	 * Set up keyboard shortcuts registered by extensions.
	 */
	private setupExtensionShortcuts(extensionRunner: ExtensionRunner): void {
		const shortcuts = extensionRunner.getShortcuts(this.keybindings.getEffectiveConfig());
		if (shortcuts.size === 0) return;

		// Create a context for shortcut handlers
		const createContext = (): ExtensionContext => ({
			ui: this.createExtensionUIContext(),
			hasUI: true,
			cwd: this.sessionManager.getCwd(),
			sessionManager: this.sessionManager,
			modelRegistry: this.session.modelRegistry,
			model: this.session.model,
			isIdle: () => !this.session.isStreaming,
			signal: this.session.agent.signal,
			abort: () => this.session.abort(),
			hasPendingMessages: () => this.session.pendingMessageCount > 0,
			shutdown: () => {
				this.shutdownRequested = true;
			},
			getContextUsage: () => this.session.getContextUsage(),
			compact: (options) => {
				void (async () => {
					try {
						const result = await this.session.compact(options?.customInstructions);
						options?.onComplete?.(result);
					} catch (error) {
						const err = error instanceof Error ? error : new Error(String(error));
						options?.onError?.(err);
					}
				})();
			},
			getSystemPrompt: () => this.session.systemPrompt,
			activatePlugin: (pluginDir) => this.session.activatePlugin(pluginDir),
			requestReloadWhenIdle: () => this.session.requestReloadWhenIdle(),
		});

		// Set up the extension shortcut handler on the default editor
		this.defaultEditor.onExtensionShortcut = (data: string) => {
			for (const [shortcutStr, shortcut] of shortcuts) {
				// Cast to KeyId - extension shortcuts use the same format
				if (matchesKey(data, shortcutStr as KeyId)) {
					// Run handler async, don't block input
					Promise.resolve(shortcut.handler(createContext())).catch((err) => {
						this.showError(`Shortcut handler error: ${err instanceof Error ? err.message : String(err)}`);
					});
					return true;
				}
			}
			return false;
		};
	}

	/**
	 * Set extension status text in the footer.
	 */
	private setExtensionStatus(key: string, text: string | undefined): void {
		this.footerDataProvider.setExtensionStatus(key, text);
		this.ui.requestRender();
	}

	private getWorkingLoaderMessage(): string {
		return this.workingMessage ?? this.defaultWorkingMessage;
	}

	private createWorkingLoader(): Loader {
		return new Loader(
			this.ui,
			(spinner) => theme.fg("accent", spinner),
			(text) => theme.fg("muted", text),
			this.getWorkingLoaderMessage(),
			this.workingIndicatorOptions,
		);
	}

	private stopWorkingLoader(): void {
		if (this.loadingAnimation) {
			this.loadingAnimation.stop();
			this.loadingAnimation = undefined;
		}
		this.statusContainer.clear();
	}

	private setWorkingVisible(visible: boolean): void {
		this.workingVisible = visible;
		if (!visible) {
			this.stopWorkingLoader();
			this.ui.requestRender();
			return;
		}
		if (this.session.isStreaming && !this.loadingAnimation) {
			this.statusContainer.clear();
			this.loadingAnimation = this.createWorkingLoader();
			this.statusContainer.addChild(this.loadingAnimation);
		}
		this.ui.requestRender();
	}

	private setWorkingIndicator(options?: LoaderIndicatorOptions): void {
		this.workingIndicatorOptions = options;
		this.loadingAnimation?.setIndicator(options);
		this.ui.requestRender();
	}

	private setHiddenThinkingLabel(label?: string): void {
		this.hiddenThinkingLabel = label ?? this.defaultHiddenThinkingLabel;
		for (const child of this.chatContainer.children) {
			if (child instanceof AssistantMessageComponent) {
				child.setHiddenThinkingLabel(this.hiddenThinkingLabel);
			}
		}
		if (this.streamingComponent) {
			this.streamingComponent.setHiddenThinkingLabel(this.hiddenThinkingLabel);
		}
		this.ui.requestRender();
	}

	private resetExtensionUI(): void {
		this.dialogs.reset();
		this.ui.hideOverlay();
		this.clearExtensionTerminalInputListeners();
		this.chrome.reset();
		this.footerDataProvider.clearExtensionStatuses();
		this.footer.invalidate();
		this.autocompleteProviderWrappers = [];
		this.setCustomEditorComponent(undefined);
		this.setupAutocompleteProvider();
		this.defaultEditor.onExtensionShortcut = undefined;
		this.refreshSessionIdentity();
		this.workingMessage = undefined;
		this.workingVisible = true;
		this.setWorkingIndicator();
		if (this.loadingAnimation) {
			this.loadingAnimation.setMessage(`${this.defaultWorkingMessage} (${keyText("app.interrupt")} to interrupt)`);
		}
		this.setHiddenThinkingLabel();
	}

	private addExtensionTerminalInputListener(
		handler: (data: string) => { consume?: boolean; data?: string } | undefined,
	): () => void {
		const unsubscribe = this.ui.addInputListener(handler);
		this.extensionTerminalInputUnsubscribers.add(unsubscribe);
		return () => {
			unsubscribe();
			this.extensionTerminalInputUnsubscribers.delete(unsubscribe);
		};
	}

	private clearExtensionTerminalInputListeners(): void {
		for (const unsubscribe of this.extensionTerminalInputUnsubscribers) {
			unsubscribe();
		}
		this.extensionTerminalInputUnsubscribers.clear();
	}

	/**
	 * Create the ExtensionUIContext for extensions.
	 */
	private createExtensionUIContext(): ExtensionUIContext {
		// Captured for the `columns` getter: `this` inside a getter on the object
		// literal below would be the literal, not the mode.
		const tui = this.ui;
		return {
			select: (title, options, opts) => this.dialogs.showSelector(title, options, opts),
			confirm: (title, message, opts) => this.dialogs.confirm(title, message, opts),
			input: (title, placeholder, opts) => this.dialogs.showInput(title, placeholder, opts),
			askOptions: (questions, opts) => {
				// The agent is blocking on the user (the ask_options pane): ring now,
				// bypassing the duration threshold — this is the "it needs you" cue.
				this.chime?.onBlockedForInput();
				return this.dialogs.showAskOptions(questions, opts);
			},
			notify: (message, type) => this.showExtensionNotify(message, type),
			get columns() {
				// Live, not captured: the listing must reflow after a resize.
				return tui.terminal.columns;
			},
			onTerminalInput: (handler) => this.addExtensionTerminalInputListener(handler),
			setStatus: (key, text) => this.setExtensionStatus(key, text),
			setMode: (mode) => {
				this.footerDataProvider.setActiveMode(mode);
				this.footer.invalidate();
				this.ui.requestRender();
			},
			setWorkingMessage: (message) => {
				this.workingMessage = message;
				if (this.loadingAnimation) {
					this.loadingAnimation.setMessage(message ?? this.defaultWorkingMessage);
				}
			},
			setWorkingVisible: (visible) => this.setWorkingVisible(visible),
			setWorkingIndicator: (options) => this.setWorkingIndicator(options),
			setHiddenThinkingLabel: (label) => this.setHiddenThinkingLabel(label),
			setWidget: (key, content, options) => this.chrome.setWidget(key, content, options),
			setFooter: (factory) => this.chrome.setFooter(factory),
			setHeader: (factory) => this.chrome.setHeader(factory),
			setTitle: (title) => this.ui.terminal.setTitle(title),
			custom: (factory, options) => this.dialogs.showCustom(factory, options),
			pasteToEditor: (text) => this.editor.handleInput(`\x1b[200~${text}\x1b[201~`),
			setEditorText: (text) => this.editor.setText(text),
			getEditorText: () => this.editor.getExpandedText?.() ?? this.editor.getText(),
			editor: (title, prefill) => this.dialogs.showEditor(title, prefill),
			addAutocompleteProvider: (factory) => {
				this.autocompleteProviderWrappers.push(factory);
				this.setupAutocompleteProvider();
			},
			setEditorComponent: (factory) => this.setCustomEditorComponent(factory),
			getEditorComponent: () => this.editorComponentFactory,
			get theme() {
				return theme;
			},
			getAllThemes: () => getAvailableThemesWithPaths(),
			getTheme: (name) => getThemeByName(name),
			setTheme: (themeOrName) => {
				if (themeOrName instanceof Theme) {
					setThemeInstance(themeOrName);
					this.ui.requestRender();
					return { success: true };
				}
				const result = setTheme(themeOrName, true);
				if (result.success) {
					if (this.settingsManager.getTheme() !== themeOrName) {
						this.settingsManager.setTheme(themeOrName);
					}
					this.ui.requestRender();
				}
				return result;
			},
			getToolsExpanded: () => this.toolOutputExpanded,
			setToolsExpanded: (expanded) => this.setToolsExpanded(expanded),
		};
	}

	private async promptForMissingSessionCwd(error: MissingSessionCwdError): Promise<string | undefined> {
		const confirmed = await this.dialogs.confirm("Session cwd not found", formatMissingSessionCwdPrompt(error.issue));
		return confirmed ? error.issue.fallbackCwd : undefined;
	}

	/**
	 * Set a custom editor component from an extension.
	 * Pass undefined to restore the default editor.
	 */
	private setCustomEditorComponent(factory: EditorFactory | undefined): void {
		this.editorComponentFactory = factory;

		// Save text from current editor before switching
		const currentText = this.editor.getText();

		this.editorContainer.clear();

		if (factory) {
			// Create the custom editor with tui, theme, and keybindings
			const newEditor = factory(this.ui, getEditorTheme(), this.keybindings);

			// Wire up callbacks from the default editor
			newEditor.onSubmit = this.defaultEditor.onSubmit;
			newEditor.onChange = this.defaultEditor.onChange;

			// Copy text from previous editor
			newEditor.setText(currentText);

			// Copy appearance settings if supported
			if (newEditor.borderColor !== undefined) {
				newEditor.borderColor = this.defaultEditor.borderColor;
			}
			if (newEditor.setPaddingX !== undefined) {
				newEditor.setPaddingX(this.defaultEditor.getPaddingX());
			}

			// Set autocomplete if supported
			if (newEditor.setAutocompleteProvider && this.autocompleteProvider) {
				newEditor.setAutocompleteProvider(this.autocompleteProvider);
			}

			// If extending CustomEditor, copy app-level handlers
			// Use duck typing since instanceof fails across jiti module boundaries
			const customEditor = newEditor as unknown as Record<string, unknown>;
			if ("actionHandlers" in customEditor && customEditor.actionHandlers instanceof Map) {
				if (!customEditor.onEscape) {
					customEditor.onEscape = () => this.defaultEditor.onEscape?.();
				}
				if (!customEditor.onCtrlD) {
					customEditor.onCtrlD = () => this.defaultEditor.onCtrlD?.();
				}
				if (!customEditor.onPasteImage) {
					customEditor.onPasteImage = () => this.defaultEditor.onPasteImage?.();
				}
				if (!customEditor.onExtensionShortcut) {
					customEditor.onExtensionShortcut = (data: string) => this.defaultEditor.onExtensionShortcut?.(data);
				}
				// Copy action handlers (clear, suspend, model switching, etc.)
				for (const [action, handler] of this.defaultEditor.actionHandlers) {
					(customEditor.actionHandlers as Map<string, () => void>).set(action, handler);
				}
			}

			this.editor = newEditor;
		} else {
			// Restore default editor with text from custom editor
			this.defaultEditor.setText(currentText);
			this.editor = this.defaultEditor;
		}

		this.editorContainer.addChild(this.editor as Component);
		this.updateSessionChip();
		this.ui.setFocus(this.editor as Component);
		this.ui.requestRender();
	}

	/**
	 * Show a notification for extensions.
	 */
	private showExtensionNotify(message: string, type?: "info" | "warning" | "error"): void {
		if (type === "error") {
			this.showError(message);
		} else if (type === "warning") {
			this.showWarning(message);
		} else {
			this.showStatus(message);
		}
	}

	/**
	 * Show an extension error in the UI.
	 */
	private showExtensionError(extensionPath: string, error: string, stack?: string): void {
		const errorMsg = `Extension "${extensionPath}" error: ${error}`;
		const errorText = new Text(theme.fg("error", errorMsg), 1, 0);
		this.chatContainer.addChild(errorText);
		if (stack) {
			// Show stack trace in dim color, indented
			const stackLines = stack
				.split("\n")
				.slice(1) // Skip first line (duplicates error message)
				.map((line) => theme.fg("dim", `  ${line.trim()}`))
				.join("\n");
			if (stackLines) {
				this.chatContainer.addChild(new Text(stackLines, 1, 0));
			}
		}
		this.ui.requestRender();
	}

	// =========================================================================
	// Key Handlers
	// =========================================================================

	private setupKeyHandlers(): void {
		// Set up handlers on defaultEditor - they use this.editor for text access
		// so they work correctly regardless of which editor is active
		this.defaultEditor.onEscape = () => {
			if (this.session.isStreaming) {
				this.messageQueue.restoreQueuedMessagesToEditor({ abort: true });
			} else if (this.session.isBashRunning) {
				this.session.abortBash();
			} else if (this.isBashMode) {
				this.editor.setText("");
				this.isBashMode = false;
				this.updateEditorBorderColor();
			} else if (!this.editor.getText().trim()) {
				// Double-escape with empty editor triggers /tree, /fork, or nothing based on setting
				const action = this.settingsManager.getDoubleEscapeAction();
				if (action !== "none") {
					const now = Date.now();
					if (now - this.lastEscapeTime < 500) {
						if (action === "tree") {
							this.showTreeSelector();
						} else {
							this.showUserMessageSelector();
						}
						this.lastEscapeTime = 0;
					} else {
						this.lastEscapeTime = now;
					}
				}
			}
		};

		// Register app action handlers
		this.defaultEditor.onAction("app.clear", () => this.handleCtrlC());
		this.defaultEditor.onCtrlD = () => this.handleCtrlD();
		this.defaultEditor.onAction("app.suspend", () => this.handleCtrlZ());
		this.defaultEditor.onAction("app.thinking.cycle", () => this.cycleThinkingLevel());
		this.defaultEditor.onAction("app.model.cycleForward", () => this.modelController.cycleModel("forward"));
		this.defaultEditor.onAction("app.model.cycleBackward", () => this.modelController.cycleModel("backward"));

		// Global debug handler on TUI (works regardless of focus)
		this.ui.onDebug = () => this.commandExecutor.handleDebug();
		this.defaultEditor.onAction("app.model.select", () => this.modelController.showModelSelector());
		this.defaultEditor.onAction("app.tools.expand", () => this.toggleToolOutputExpansion());
		this.defaultEditor.onAction("app.view.cycleForward", () => this.cycleToolOutputView("forward"));
		this.defaultEditor.onAction("app.view.cycleBackward", () => this.cycleToolOutputView("backward"));
		this.defaultEditor.onAction("app.tools.unfoldOne", () => this.stepToolOutput("unfold"));
		this.defaultEditor.onAction("app.tools.foldOne", () => this.stepToolOutput("fold"));
		this.defaultEditor.onAction("app.thinking.toggle", () => this.toggleThinkingBlockVisibility());
		this.defaultEditor.onAction("app.tasks.cycleView", () => {
			this.taskPanel.cycleView();
		});
		this.defaultEditor.onAction("app.team.focus", () => this.teamFocus.enterFocus());
		this.defaultEditor.onAction("app.editor.external", () => this.openExternalEditor());
		this.defaultEditor.onAction("app.input.voiceTranscribe", () => this.voice.toggle());
		this.defaultEditor.onAction("app.message.followUp", () => this.handleFollowUp());
		this.defaultEditor.onAction("app.message.dequeue", () => this.handleDequeue());
		this.defaultEditor.onAction("app.session.new", () => this.commandExecutor.handleClear());
		this.defaultEditor.onAction("app.session.tree", () => this.showTreeSelector());
		this.defaultEditor.onAction("app.session.fork", () => this.showUserMessageSelector());
		this.defaultEditor.onAction("app.session.resume", () => this.showSessionSelector());
		this.defaultEditor.onAction("app.session.changeDirectory", () => {
			// Prefill rather than act: a directory change needs a target, and the
			// editor's own path completion is already the best way to name one.
			this.editor.setText("/cd ");
			this.ui.requestRender();
		});
		this.defaultEditor.onAction("app.settings.open", () => this.showSettingsSelector());
		this.defaultEditor.onAction("app.hotkeys.open", () => this.commandExecutor.handleHotkeys());
		this.defaultEditor.onAction("app.mode.cycle", () => void this.cycleAgentMode());

		this.defaultEditor.onChange = (text: string) => {
			const wasBashMode = this.isBashMode;
			this.isBashMode = text.trimStart().startsWith("!");
			if (wasBashMode !== this.isBashMode) {
				this.updateEditorBorderColor();
				this.updateEditorPromptPrefix();
			}
		};

		// Handle clipboard image paste (triggered on Ctrl+V)
		this.defaultEditor.onPasteImage = () => {
			this.handleClipboardImagePaste();
		};
	}

	private async handleClipboardImagePaste(): Promise<void> {
		try {
			const image = await readClipboardImage();
			if (!image) {
				return;
			}

			// Write to temp file
			const tmpDir = os.tmpdir();
			const ext = extensionForImageMimeType(image.mimeType) ?? "png";
			const fileName = `${APP_NAME}-clipboard-${crypto.randomUUID()}.${ext}`;
			const filePath = path.join(tmpDir, fileName);
			fs.writeFileSync(filePath, Buffer.from(image.bytes));

			// Insert file path directly
			this.editor.insertTextAtCursor?.(filePath);
			this.ui.requestRender();
		} catch {
			// Silently ignore clipboard errors (may not have permission, etc.)
		}
	}

	// =========================================================================
	// hooteams team focus (--team): focus role rows, nudge, attach
	// =========================================================================

	/**
	 * Wire a hooteams connection into the TUI. Called by main.ts when `--team`
	 * is set, before run(). See TeamFocusController for the feature itself.
	 */
	attachTeamClient(client: TeamViewConnection): void {
		this.teamFocus.attachClient(client);
	}

	/**
	 * Built-in slash commands dispatched by the editor submit handler.
	 * `withArgs` commands also match "/name <args>" and receive the full text.
	 * Each handler keeps its own editor-clearing order (before vs after the
	 * await) — some commands must show their UI before the prompt is wiped.
	 */
	private createBuiltInSlashCommands(): Map<string, { withArgs?: boolean; run(text: string): Promise<void> | void }> {
		const clearEditor = () => this.editor.setText("");
		return new Map(
			Object.entries({
				"/settings": {
					run: () => {
						this.showSettingsSelector();
						clearEditor();
					},
				},
				"/scoped-models": {
					run: async () => {
						clearEditor();
						await this.modelController.showModelsSelector();
					},
				},
				"/model": {
					withArgs: true,
					run: async (text: string) => {
						const searchTerm = text.startsWith("/model ") ? text.slice(7).trim() : undefined;
						clearEditor();
						await this.commandExecutor.handleModel(searchTerm);
					},
				},
				"/export": {
					withArgs: true,
					run: async (text: string) => {
						await this.commandExecutor.handleExport(text);
						clearEditor();
					},
				},
				"/import": {
					withArgs: true,
					run: async (text: string) => {
						await this.commandExecutor.handleImport(text);
						clearEditor();
					},
				},
				"/share": {
					run: async () => {
						await this.commandExecutor.handleShare();
						clearEditor();
					},
				},
				"/copy": {
					run: async () => {
						await this.commandExecutor.handleCopy();
						clearEditor();
					},
				},
				"/name": {
					withArgs: true,
					run: (text: string) => {
						this.commandExecutor.handleName(text);
						clearEditor();
					},
				},
				"/color": {
					withArgs: true,
					run: (text: string) => {
						// An argument sets the slot outright; bare `/color` opens the
						// swatches, since a slot number is not something anyone knows
						// by heart.
						if (!this.commandExecutor.handleColor(text)) {
							this.showSessionColorSelector();
						}
						clearEditor();
					},
				},
				"/session": {
					run: () => {
						this.commandExecutor.handleSession();
						clearEditor();
					},
				},
				"/changelog": {
					run: () => {
						this.commandExecutor.handleChangelog();
						clearEditor();
					},
				},
				"/hotkeys": {
					run: () => {
						this.commandExecutor.handleHotkeys();
						clearEditor();
					},
				},
				"/fork": {
					run: () => {
						this.showUserMessageSelector();
						clearEditor();
					},
				},
				"/clone": {
					run: async () => {
						clearEditor();
						await this.commandExecutor.handleClone();
					},
				},
				"/tree": {
					run: () => {
						this.showTreeSelector();
						clearEditor();
					},
				},
				"/login": {
					run: () => {
						this.loginController.showOAuthSelector("login");
						clearEditor();
					},
				},
				"/logout": {
					run: () => {
						this.loginController.showOAuthSelector("logout");
						clearEditor();
					},
				},
				"/new": {
					run: async () => {
						clearEditor();
						await this.commandExecutor.handleClear();
					},
				},
				"/compact": {
					withArgs: true,
					run: async (text: string) => {
						const prefix = "/compact ";
						const customInstructions = text.startsWith(prefix)
							? text.slice(prefix.length).trim() || undefined
							: undefined;
						clearEditor();
						await this.handleCompactCommand(customInstructions);
					},
				},
				"/reload": {
					run: async () => {
						clearEditor();
						await this.handleReloadCommand();
					},
				},
				"/debug": {
					run: () => {
						this.commandExecutor.handleDebug();
						clearEditor();
					},
				},
				"/resume": {
					run: () => {
						this.showSessionSelector();
						clearEditor();
					},
				},
				"/cd": {
					withArgs: true,
					run: async (text: string) => {
						clearEditor();
						await this.commandExecutor.handleChangeDirectory(text);
					},
				},
				"/quit": {
					run: async () => {
						clearEditor();
						await this.shutdown();
					},
				},
				"/subagent": {
					withArgs: true,
					run: async (text: string) => {
						clearEditor();
						await this.commandExecutor.handleSubagent(text);
					},
				},
			}),
		);
	}

	private setupEditorSubmitHandler(): void {
		const slashCommands = this.createBuiltInSlashCommands();
		this.defaultEditor.onSubmit = async (text: string) => {
			text = text.trim();
			if (!text) return;

			// Handle built-in slash commands
			const spaceIdx = text.indexOf(" ");
			const commandName = spaceIdx === -1 ? text : text.slice(0, spaceIdx);
			const command = slashCommands.get(commandName);
			if (command && (spaceIdx === -1 || command.withArgs)) {
				await command.run(text);
				return;
			}

			// Handle bash command (! for normal, !! for excluded from context)
			if (text.startsWith("!")) {
				const isExcluded = text.startsWith("!!");
				const command = isExcluded ? text.slice(2).trim() : text.slice(1).trim();
				if (command) {
					if (this.session.isBashRunning) {
						this.showWarning("A bash command is already running. Press Esc to cancel it first.");
						this.editor.setText(text);
						return;
					}
					this.editor.addToHistory?.(text);
					await this.bashExecution.handleBashCommand(command, isExcluded);
					this.isBashMode = false;
					this.updateEditorBorderColor();
					return;
				}
			}

			// Queue input during compaction (extension commands execute immediately)
			if (this.session.isCompacting) {
				if (this.messageQueue.isExtensionCommand(text)) {
					this.editor.addToHistory?.(text);
					this.editor.setText("");
					await this.session.prompt(text);
				} else {
					this.messageQueue.queueCompactionMessage(text, "steer");
				}
				return;
			}

			// If streaming, use prompt() with steer behavior
			// This handles extension commands (execute immediately), prompt template expansion, and queueing
			if (this.session.isStreaming) {
				this.editor.addToHistory?.(text);
				this.editor.setText("");
				await this.session.prompt(text, { streamingBehavior: "steer" });
				this.messageQueue.updatePendingMessagesDisplay();
				this.ui.requestRender();
				return;
			}

			// Normal message submission
			// First, move any pending bash components to chat
			this.bashExecution.flushPendingBashComponents();

			if (this.onInputCallback) {
				this.onInputCallback(text);
			}
			this.editor.addToHistory?.(text);
		};
	}

	private subscribeToAgent(): void {
		this.unsubscribe = this.session.subscribe(async (event) => {
			await this.handleEvent(event);
		});
	}

	/**
	 * Deferred end-of-request settle. Called from the agent_end handler, it waits
	 * one tick so the streaming flag settles and any retry has had a chance to arm,
	 * then acts only when the session is genuinely idle (not streaming, not
	 * compacting) and no retry is pending — so a retried or auto-continued request
	 * neither rings a premature "it's done" nor prints a partial cost line.
	 *
	 * The completion chime, the transcript cost line, and settling dangling plan
	 * items all hang off this single check because they answer the same question:
	 * has the request actually ended?
	 */
	private settleRequestOnIdle(): void {
		setTimeout(() => {
			if (this.chimePendingRetry || this.session.isStreaming || this.session.isCompacting) {
				return;
			}
			// The last chain of a turn has nothing after it to close it, so it
			// settles here. A turn that did not end cleanly leaves its chain in the
			// running rendering plus a marker rather than claiming an outcome.
			this.closeOpenChain(this.turnStopReason === "stop" ? "done" : "interrupted");
			this.showTurnCost();
			this.settleDanglingPlanItems();
			this.chime?.onTurnComplete({ aborted: this.turnStopReason === "aborted" });
		}, 0);
	}

	/**
	 * Flip main-plan rows the model left at in_progress to a settled status now the
	 * request is over, instead of letting them claim live work until the next user
	 * message wipes the pane.
	 *
	 * The model marks its own TodoWrite items and routinely drops the final call
	 * that completes the last one, so the row outlives the work. A clean "stop" is
	 * taken as the model believing it was finished — it chose to stop talking, and
	 * its closing message is the report — so those items settle to done. An abort,
	 * error, or length cutoff says the opposite, so they settle to cancelled: an
	 * honest "never finished" rather than a fabricated completion.
	 *
	 * Skipped while messages are queued: a follow-up/steer arriving during the run
	 * means the request continues, and the plan with it.
	 */
	private settleDanglingPlanItems(): void {
		if (this.session.pendingMessageCount > 0) return;
		const settled = settleDanglingMainTasks(this.turnStopReason === "stop" ? "done" : "cancelled");
		if (settled > 0) this.ui.requestRender();
	}

	/**
	 * Append this request's own token/time/cost to the transcript.
	 *
	 * This is the honest home for the number. The task panel is a live instrument —
	 * present tense, wiped on the next user message — and the footer carries
	 * cumulative session vitals, so after the fact neither can answer "what did that
	 * request cost". Fired at agent_end rather than turn_end because one request is
	 * commonly tens of turns; per-turn would wedge a number between every tool block.
	 *
	 * Delegated runs are reported separately rather than folded into the totals:
	 * subagents bill against their own sessions, so their tokens never appear in this
	 * session's entries and adding them to ↑/↓ would misreport what the parent spent.
	 */
	private showTurnCost(): void {
		const anchor = this.turnCostAnchor;
		this.turnCostAnchor = undefined;
		if (!anchor) return;

		const now = sumAssistantUsage(this.session.sessionManager.getEntries());
		const input = now.input - anchor.totals.input;
		const output = now.output - anchor.totals.output;
		const cost = now.cost - anchor.totals.cost;
		// Nothing was accounted (e.g. aborted before the first response landed): stay
		// silent rather than print a row of zeroes.
		if (input <= 0 && output <= 0) return;

		const segs: string[] = [
			theme.fg("dim", "↑") +
				theme.fg("muted", formatTokens(input)) +
				theme.fg("dim", " ↓") +
				theme.fg("muted", formatTokens(output)),
			theme.fg("muted", formatDurationSecs((Date.now() - anchor.at) / 1000)),
		];
		if (cost > 0) segs.push(theme.fg("muted", `$${cost.toFixed(3)}`));

		const runs = taskStore.list().filter((task) => task.source === "subagent");
		if (runs.length > 0) {
			const delegatedTokens = runs.reduce((sum, r) => sum + (r.usage ? r.usage.input + r.usage.output : 0), 0);
			const label = runs.length === 1 ? (runs[0]?.subagentMode ?? "subagent") : "subagents";
			const text = `◇${runs.length} ${label}${delegatedTokens > 0 ? ` ${formatTokens(delegatedTokens)}` : ""}`;
			segs.push(theme.fg("dim", text));
		}

		this.chatContainer.addChild(new Spacer(1));
		this.chatContainer.addChild(new Text(segs.join(theme.fg("dim", " · ")), 1, 0));
		this.ui.requestRender();
	}

	private async handleEvent(event: AgentSessionEvent): Promise<void> {
		if (!this.isInitialized) {
			await this.init();
		}

		this.footer.invalidate();

		switch (event.type) {
			case "agent_start":
				this.pendingTools.clear();
				// A (possibly retried) turn is (re)starting: anchor its duration on the
				// first start and clear the pending-retry gate now that it has resumed.
				this.chime?.onTurnStart();
				this.chimePendingRetry = false;
				this.turnCostAnchor ??= {
					totals: sumAssistantUsage(this.session.sessionManager.getEntries()),
					at: Date.now(),
				};
				if (this.settingsManager.getShowTerminalProgress()) {
					this.ui.terminal.setProgress(true);
				}
				// Restore main escape handler if retry handler is still active
				// (retry success event fires later, but we need main handler now)
				if (this.retryEscapeHandler) {
					this.defaultEditor.onEscape = this.retryEscapeHandler;
					this.retryEscapeHandler = undefined;
				}
				if (this.retryCountdown) {
					this.retryCountdown.dispose();
					this.retryCountdown = undefined;
				}
				if (this.retryLoader) {
					this.retryLoader.stop();
					this.retryLoader = undefined;
				}
				this.stopWorkingLoader();
				if (this.workingVisible) {
					this.loadingAnimation = this.createWorkingLoader();
					this.statusContainer.addChild(this.loadingAnimation);
				}
				this.ui.requestRender();
				break;

			case "queue_update":
				this.messageQueue.updatePendingMessagesDisplay();
				this.ui.requestRender();
				break;

			case "session_info_changed":
				this.refreshSessionIdentity();
				this.footer.invalidate();
				this.ui.requestRender();
				break;

			case "thinking_level_changed":
				this.footer.invalidate();
				this.updateEditorBorderColor();
				break;

			case "message_start":
				if (event.message.role === "custom") {
					this.addMessageToChat(event.message);
					this.ui.requestRender();
				} else if (event.message.role === "user") {
					// A new user message starts a new turn: drop finished tasks from the
					// previous turn and restart numbering from #1 once the pane is empty.
					// Finished tasks stay visible (with their final status, tokens, and time)
					// until this point — not the moment they finish — so their outcome remains
					// glanceable for the whole turn. Active tasks are kept: a follow-up/steer
					// message can arrive while a subagent is still running.
					taskStore.reset();
					// Startup progress is transient startup status: once the user starts a
					// turn, drop any lingering bars/notices (e.g. an unavailable-index line)
					// so they don't sit in the footer for the rest of the session.
					startupProgress.clear();
					this.turnStopReason = undefined;
					this.addMessageToChat(event.message);
					this.messageQueue.updatePendingMessagesDisplay();
					this.ui.requestRender();
				} else if (event.message.role === "assistant") {
					this.streamingComponent = new AssistantMessageComponent(
						undefined,
						this.thinkingDisplayForView(),
						this.getMarkdownThemeWithSettings(),
						this.hiddenThinkingLabel,
					);
					this.streamingMessage = event.message;
					this.sawTextInCurrentMessage = false;
					this.chatContainer.addChild(this.streamingComponent);
					this.streamingComponent.updateContent(this.streamingMessage);
					this.ui.requestRender();
				}
				break;

			case "message_update":
				if (this.streamingComponent && event.message.role === "assistant") {
					this.streamingMessage = event.message;
					this.scheduleStreamingRender();

					// The agent speaking ends the run its previous calls formed. Tool
					// calls later in this same message open a fresh chain.
					if (!this.sawTextInCurrentMessage) {
						const spoke = this.streamingMessage.content.some(
							(content) => content.type === "text" && content.text.trim() !== "",
						);
						if (spoke) {
							this.sawTextInCurrentMessage = true;
							this.closeOpenChain("done");
						}
					}

					for (const content of this.streamingMessage.content) {
						if (content.type === "toolCall") {
							if (!this.pendingTools.has(content.id)) {
								const component = new ToolExecutionComponent(
									content.name,
									content.id,
									content.arguments,
									{
										showImages: this.settingsManager.getShowImages(),
										imageWidthCells: this.settingsManager.getImageWidthCells(),
										view: this.toolOutputView,
									},
									this.getRegisteredToolDefinition(content.name),
									this.ui,
									this.sessionManager.getCwd(),
								);
								component.setExpanded(this.toolOutputExpanded);
								this.attachToolBlock(component);
								this.pendingTools.set(content.id, component);
							} else {
								const component = this.pendingTools.get(content.id);
								if (component) {
									component.updateArgs(content.arguments);
								}
							}
						}
					}
					this.ui.requestRender();
				}
				break;

			case "message_end":
				if (event.message.role === "user") break;
				if (event.message.role === "assistant") {
					// Remember how the turn's latest assistant message ended, so the deferred
					// settle can tell a clean finish from an abort/error.
					this.turnStopReason = event.message.stopReason;
				}
				if (this.streamingComponent && event.message.role === "assistant") {
					this.streamingMessage = event.message;
					let errorMessage: string | undefined;
					if (this.streamingMessage.stopReason === "aborted") {
						const retryAttempt = this.session.retryAttempt;
						errorMessage =
							retryAttempt > 0
								? `Aborted after ${retryAttempt} retry attempt${retryAttempt > 1 ? "s" : ""}`
								: "Operation aborted";
						this.streamingMessage.errorMessage = errorMessage;
					}
					this.streamingComponent.updateContent(this.streamingMessage);

					if (this.streamingMessage.stopReason === "aborted" || this.streamingMessage.stopReason === "error") {
						if (!errorMessage) {
							errorMessage = this.streamingMessage.errorMessage || "Error";
						}
						for (const [, component] of this.pendingTools.entries()) {
							component.updateResult({
								content: [{ type: "text", text: errorMessage }],
								isError: true,
							});
						}
						this.pendingTools.clear();
					} else {
						// Args are now complete - trigger diff computation for edit tools
						for (const [, component] of this.pendingTools.entries()) {
							component.setArgsComplete();
						}
					}
					this.streamingComponent = undefined;
					this.streamingMessage = undefined;
					this.footer.invalidate();
				}
				this.ui.requestRender();
				break;

			case "tool_execution_start": {
				let component = this.pendingTools.get(event.toolCallId);
				if (!component) {
					component = new ToolExecutionComponent(
						event.toolName,
						event.toolCallId,
						event.args,
						{
							showImages: this.settingsManager.getShowImages(),
							imageWidthCells: this.settingsManager.getImageWidthCells(),
							view: this.toolOutputView,
						},
						this.getRegisteredToolDefinition(event.toolName),
						this.ui,
						this.sessionManager.getCwd(),
					);
					component.setExpanded(this.toolOutputExpanded);
					this.attachToolBlock(component);
					this.pendingTools.set(event.toolCallId, component);
				}
				component.markExecutionStarted();
				this.ui.requestRender();
				break;
			}

			case "tool_execution_update": {
				const component = this.pendingTools.get(event.toolCallId);
				if (component) {
					component.updateResult({ ...event.partialResult, isError: false }, true);
					this.ui.requestRender();
				}
				break;
			}

			case "tool_execution_end": {
				const component = this.pendingTools.get(event.toolCallId);
				if (component) {
					component.updateResult({ ...event.result, isError: event.isError });
					this.pendingTools.delete(event.toolCallId);
					this.trimTranscriptMemory();
					this.ui.requestRender();
				}
				break;
			}

			case "agent_end":
				if (this.settingsManager.getShowTerminalProgress()) {
					this.ui.terminal.setProgress(false);
				}
				if (this.loadingAnimation) {
					this.loadingAnimation.stop();
					this.loadingAnimation = undefined;
					this.statusContainer.clear();
				}
				if (this.streamingComponent) {
					this.chatContainer.removeChild(this.streamingComponent);
					this.streamingComponent = undefined;
					this.streamingMessage = undefined;
				}
				this.pendingTools.clear();

				await this.checkShutdownRequested();

				// agent_end fires before the streaming flag clears and before the retry
				// decision is made (both happen after this listener returns), so defer a
				// tick and re-check: only settle once the session is genuinely idle and no
				// retry is pending — otherwise a retried or auto-continued turn would ring
				// a premature "it's done" and print a partial cost line.
				this.settleRequestOnIdle();

				this.ui.requestRender();
				break;

			case "compaction_start": {
				if (this.settingsManager.getShowTerminalProgress()) {
					this.ui.terminal.setProgress(true);
				}
				// Keep editor active; submissions are queued during compaction.
				this.autoCompactionEscapeHandler = this.defaultEditor.onEscape;
				this.defaultEditor.onEscape = () => {
					this.session.abortCompaction();
				};
				this.statusContainer.clear();
				const cancelHint = `(${keyText("app.interrupt")} to cancel)`;
				const label =
					event.reason === "manual"
						? `Compacting context... ${cancelHint}`
						: `${event.reason === "overflow" ? "Context overflow detected, " : ""}Auto-compacting... ${cancelHint}`;
				this.autoCompactionLoader = new Loader(
					this.ui,
					(spinner) => theme.fg("accent", spinner),
					(text) => theme.fg("muted", text),
					label,
				);
				this.statusContainer.addChild(this.autoCompactionLoader);
				this.ui.requestRender();
				break;
			}

			case "compaction_end": {
				if (this.settingsManager.getShowTerminalProgress()) {
					this.ui.terminal.setProgress(false);
				}
				if (this.autoCompactionEscapeHandler) {
					this.defaultEditor.onEscape = this.autoCompactionEscapeHandler;
					this.autoCompactionEscapeHandler = undefined;
				}
				if (this.autoCompactionLoader) {
					this.autoCompactionLoader.stop();
					this.autoCompactionLoader = undefined;
					this.statusContainer.clear();
				}
				if (event.aborted) {
					if (event.reason === "manual") {
						this.showError("Compaction cancelled");
					} else {
						this.showStatus("Auto-compaction cancelled");
					}
				} else if (event.result) {
					this.chatContainer.clear();
					this.rebuildChatFromMessages();
					this.addMessageToChat(
						createCompactionSummaryMessage(
							event.result.summary,
							event.result.tokensBefore,
							new Date().toISOString(),
							event.result.tokensAfter,
						),
					);
					this.footer.invalidate();
				} else if (event.errorMessage) {
					if (event.reason === "manual") {
						this.showError(event.errorMessage);
					} else {
						this.chatContainer.addChild(new Spacer(1));
						this.chatContainer.addChild(new Text(theme.fg("error", event.errorMessage), 1, 0));
					}
				}
				void this.messageQueue.flushCompactionQueue({ willRetry: event.willRetry });
				this.ui.requestRender();
				break;
			}

			case "auto_retry_start": {
				// The turn is not done — it will re-run after a backoff (during which the
				// session reads as idle). Gate the deferred completion chime until then.
				this.chimePendingRetry = true;
				// Set up escape to abort retry
				this.retryEscapeHandler = this.defaultEditor.onEscape;
				this.defaultEditor.onEscape = () => {
					this.session.abortRetry();
				};
				// Show retry indicator
				this.statusContainer.clear();
				this.retryCountdown?.dispose();
				const retryMessage = (seconds: number) =>
					`Retrying (${event.attempt}/${event.maxAttempts}) in ${seconds}s... (${keyText("app.interrupt")} to cancel)`;
				this.retryLoader = new Loader(
					this.ui,
					(spinner) => theme.fg("warning", spinner),
					(text) => theme.fg("muted", text),
					retryMessage(Math.ceil(event.delayMs / 1000)),
				);
				this.retryCountdown = new CountdownTimer(
					event.delayMs,
					this.ui,
					(seconds) => {
						this.retryLoader?.setMessage(retryMessage(seconds));
					},
					() => {
						this.retryCountdown = undefined;
					},
				);
				this.statusContainer.addChild(this.retryLoader);
				this.ui.requestRender();
				break;
			}

			case "auto_retry_end": {
				// Restore escape handler
				if (this.retryEscapeHandler) {
					this.defaultEditor.onEscape = this.retryEscapeHandler;
					this.retryEscapeHandler = undefined;
				}
				if (this.retryCountdown) {
					this.retryCountdown.dispose();
					this.retryCountdown = undefined;
				}
				// Stop loader
				if (this.retryLoader) {
					this.retryLoader.stop();
					this.retryLoader = undefined;
					this.statusContainer.clear();
				}
				// Show error only on final failure (success shows normal response)
				if (!event.success) {
					this.showError(`Retry failed after ${event.attempt} attempts: ${event.finalError || "Unknown error"}`);
				}
				this.ui.requestRender();
				break;
			}
		}
	}

	/** Extract text content from a user message */
	private getUserMessageText(message: Message): string {
		if (message.role !== "user") return "";
		const textBlocks =
			typeof message.content === "string"
				? [{ type: "text", text: message.content }]
				: message.content.filter((c: { type: string }) => c.type === "text");
		return textBlocks.map((c) => (c as { text: string }).text).join("");
	}

	/**
	 * Show a status message in the chat.
	 *
	 * If multiple status messages are emitted back-to-back (without anything else being added to the chat),
	 * we update the previous status line instead of appending new ones to avoid log spam.
	 */

	private showStatus(message: string): void {
		const children = this.chatContainer.children;
		const last = children.length > 0 ? children[children.length - 1] : undefined;
		const secondLast = children.length > 1 ? children[children.length - 2] : undefined;

		if (last && secondLast && last === this.lastStatusText && secondLast === this.lastStatusSpacer) {
			this.lastStatusText.setText(styleStatusMessage(message));
			this.ui.requestRender();
			return;
		}

		const spacer = new Spacer(1);
		const text = new Text(styleStatusMessage(message), 1, 0);
		this.chatContainer.addChild(spacer);
		this.chatContainer.addChild(text);
		this.lastStatusSpacer = spacer;
		this.lastStatusText = text;
		this.ui.requestRender();
	}

	private addMessageToChat(message: AgentMessage, options?: { populateHistory?: boolean }): void {
		switch (message.role) {
			case "bashExecution": {
				const component = new BashExecutionComponent(message.command, this.ui, message.excludeFromContext);
				if (message.output) {
					component.appendOutput(message.output);
				}
				component.setComplete(
					message.exitCode,
					message.cancelled,
					message.truncated ? ({ truncated: true } as TruncationResult) : undefined,
					message.fullOutputPath,
				);
				this.chatContainer.addChild(component);
				break;
			}
			case "custom": {
				if (message.display) {
					const renderer = this.session.extensionRunner.getMessageRenderer(message.customType);
					const component = new CustomMessageComponent(message, renderer, this.getMarkdownThemeWithSettings());
					component.setExpanded(this.toolOutputExpanded);
					this.chatContainer.addChild(component);
				}
				break;
			}
			case "compactionSummary": {
				this.chatContainer.addChild(new Spacer(1));
				const component = new CompactionSummaryMessageComponent(message, this.getMarkdownThemeWithSettings());
				component.setExpanded(this.toolOutputExpanded);
				this.chatContainer.addChild(component);
				break;
			}
			case "branchSummary": {
				this.chatContainer.addChild(new Spacer(1));
				const component = new BranchSummaryMessageComponent(message, this.getMarkdownThemeWithSettings());
				component.setExpanded(this.toolOutputExpanded);
				this.chatContainer.addChild(component);
				break;
			}
			case "user": {
				const textContent = this.getUserMessageText(message);
				if (textContent) {
					if (this.chatContainer.children.length > 0) {
						this.chatContainer.addChild(new Spacer(1));
					}
					const skillBlock = parseSkillBlock(textContent);
					if (skillBlock) {
						// Render skill block (collapsible)
						const component = new SkillInvocationMessageComponent(
							skillBlock,
							this.getMarkdownThemeWithSettings(),
						);
						component.setExpanded(this.toolOutputExpanded);
						this.chatContainer.addChild(component);
						// Render user message separately if present
						if (skillBlock.userMessage) {
							const userComponent = new UserMessageComponent(
								skillBlock.userMessage,
								this.getMarkdownThemeWithSettings(),
							);
							this.chatContainer.addChild(userComponent);
						}
					} else {
						const userComponent = new UserMessageComponent(textContent, this.getMarkdownThemeWithSettings());
						this.chatContainer.addChild(userComponent);
					}
					if (options?.populateHistory) {
						this.editor.addToHistory?.(textContent);
					}
				}
				break;
			}
			case "assistant": {
				const assistantComponent = new AssistantMessageComponent(
					message,
					this.thinkingDisplayForView(),
					this.getMarkdownThemeWithSettings(),
					this.hiddenThinkingLabel,
				);
				this.chatContainer.addChild(assistantComponent);
				break;
			}
			case "toolResult": {
				// Tool results are rendered inline with tool calls, handled separately
				break;
			}
			default: {
				const _exhaustive: never = message;
			}
		}
	}

	/**
	 * Render session context to chat. Used for initial load and rebuild after compaction.
	 * @param sessionContext Session context to render
	 * @param options.updateFooter Update footer state
	 * @param options.populateHistory Add user messages to editor history
	 */
	private renderSessionContext(
		sessionContext: SessionContext,
		options: { updateFooter?: boolean; populateHistory?: boolean } = {},
	): void {
		this.pendingTools.clear();
		const renderedPendingTools = new Map<string, ToolExecutionComponent>();

		if (options.updateFooter) {
			this.footer.invalidate();
			this.updateEditorBorderColor();
		}

		let lastAssistantStopReason: AssistantMessage["stopReason"] | undefined;
		for (const message of sessionContext.messages) {
			// Assistant messages need special handling for tool calls
			if (message.role === "assistant") {
				// Same boundary the live path uses: the agent speaking ends the run
				// its previous calls formed. Rebuilding history has to reproduce it,
				// or a resumed session would show one chain where it lived through
				// several.
				const spoke = message.content.some((content) => content.type === "text" && content.text.trim() !== "");
				if (spoke) this.closeOpenChain("done");
				lastAssistantStopReason = message.stopReason;
				this.addMessageToChat(message);
				// Render tool call components
				for (const content of message.content) {
					if (content.type === "toolCall") {
						const component = new ToolExecutionComponent(
							content.name,
							content.id,
							content.arguments,
							{
								showImages: this.settingsManager.getShowImages(),
								imageWidthCells: this.settingsManager.getImageWidthCells(),
								view: this.toolOutputView,
							},
							this.getRegisteredToolDefinition(content.name),
							this.ui,
							this.sessionManager.getCwd(),
						);
						component.setExpanded(this.toolOutputExpanded);
						this.attachToolBlock(component);

						if (message.stopReason === "aborted" || message.stopReason === "error") {
							let errorMessage: string;
							if (message.stopReason === "aborted") {
								const retryAttempt = this.session.retryAttempt;
								errorMessage =
									retryAttempt > 0
										? `Aborted after ${retryAttempt} retry attempt${retryAttempt > 1 ? "s" : ""}`
										: "Operation aborted";
							} else {
								errorMessage = message.errorMessage || "Error";
							}
							component.updateResult({ content: [{ type: "text", text: errorMessage }], isError: true });
						} else {
							renderedPendingTools.set(content.id, component);
						}
					}
				}
			} else if (message.role === "toolResult") {
				// Match tool results to pending tool components
				const component = renderedPendingTools.get(message.toolCallId);
				if (component) {
					component.updateResult(message);
					renderedPendingTools.delete(message.toolCallId);
				}
			} else {
				// All other messages use standard rendering
				this.addMessageToChat(message, options);
			}
		}

		// History has no live state: a chain left open at the end of the rebuild is
		// finished, and how the last assistant message ended says whether it
		// finished cleanly. Tool calls still awaiting results are the exception —
		// this is a resumed run in flight, so its chain stays open for them.
		if (renderedPendingTools.size === 0) {
			this.closeOpenChain(lastAssistantStopReason === "stop" ? "done" : "interrupted");
		}

		for (const [toolCallId, component] of renderedPendingTools) {
			this.pendingTools.set(toolCallId, component);
		}
		this.ui.requestRender();
	}

	renderInitialMessages(): void {
		// Get aligned messages and entries from session context
		const context = this.sessionManager.buildSessionContext();
		this.renderSessionContext(context, {
			updateFooter: true,
			populateHistory: true,
		});

		// Show compaction info if session was compacted
		const allEntries = this.sessionManager.getEntries();
		const compactionCount = allEntries.filter((e) => e.type === "compaction").length;
		if (compactionCount > 0) {
			const times = compactionCount === 1 ? "1 time" : `${compactionCount} times`;
			this.showStatus(`Session compacted ${times}`);
		}
	}

	async getUserInput(): Promise<string> {
		return new Promise((resolve) => {
			this.onInputCallback = (text: string) => {
				this.onInputCallback = undefined;
				resolve(text);
			};
		});
	}

	private rebuildChatFromMessages(): void {
		this.chatContainer.clear();
		const context = this.sessionManager.buildSessionContext();
		this.renderSessionContext(context);
	}

	// =========================================================================
	// Key handlers
	// =========================================================================

	private handleCtrlC(): void {
		const now = Date.now();
		if (now - this.lastSigintTime < 500) {
			void this.shutdown();
		} else {
			this.clearEditor();
			this.lastSigintTime = now;
		}
	}

	private handleCtrlD(): void {
		// Only called when editor is empty (enforced by CustomEditor)
		void this.shutdown();
	}

	/**
	 * Gracefully shutdown the agent.
	 * Stops the TUI before emitting shutdown events so extension UI cleanup cannot
	 * repaint the final frame while the process is exiting.
	 */
	private isShuttingDown = false;

	private async shutdown(): Promise<void> {
		if (this.isShuttingDown) return;
		this.isShuttingDown = true;
		this.unregisterSignalHandlers();

		// Drain any in-flight Kitty key release events before stopping.
		// This prevents escape sequences from leaking to the parent shell over slow SSH.
		await this.ui.terminal.drainInput(1000);

		this.taskStoreUnsubscribe?.();
		this.startupProgressUnsubscribe?.();
		this.teamFocus.closeAttach();
		this.taskPanel.dispose();
		this.stop();
		await this.runtimeHost.dispose();
		process.exit(0);
	}

	private emergencyTerminalExit(): never {
		this.isShuttingDown = true;
		this.unregisterSignalHandlers();
		killTrackedDetachedChildren();
		// The terminal is gone. Do not run normal shutdown because TUI and
		// extension cleanup can write restore sequences and re-trigger EIO.
		process.exit(129);
	}

	/**
	 * Check if shutdown was requested and perform shutdown if so.
	 */
	private async checkShutdownRequested(): Promise<void> {
		if (!this.shutdownRequested) return;
		await this.shutdown();
	}

	private registerSignalHandlers(): void {
		this.unregisterSignalHandlers();

		const signals: NodeJS.Signals[] = ["SIGTERM"];
		if (process.platform !== "win32") {
			signals.push("SIGHUP");
		}

		for (const signal of signals) {
			const handler = () => {
				if (signal === "SIGHUP") {
					this.emergencyTerminalExit();
				}
				killTrackedDetachedChildren();
				void this.shutdown();
			};
			process.prependListener(signal, handler);
			this.signalCleanupHandlers.push(() => process.off(signal, handler));
		}

		const terminalErrorHandler = (error: Error) => {
			if (isDeadTerminalError(error)) {
				this.emergencyTerminalExit();
			}
			throw error;
		};
		process.stdout.on("error", terminalErrorHandler);
		process.stderr.on("error", terminalErrorHandler);
		this.signalCleanupHandlers.push(() => process.stdout.off("error", terminalErrorHandler));
		this.signalCleanupHandlers.push(() => process.stderr.off("error", terminalErrorHandler));
	}

	private unregisterSignalHandlers(): void {
		for (const cleanup of this.signalCleanupHandlers) {
			cleanup();
		}
		this.signalCleanupHandlers = [];
	}

	private handleCtrlZ(): void {
		if (process.platform === "win32") {
			this.showStatus("Suspend to background is not supported on Windows");
			return;
		}

		// Keep the event loop alive while suspended. Without this, stopping the TUI
		// can leave Node with no ref'ed handles, causing the process to exit on fg
		// before the SIGCONT handler gets a chance to restore the terminal.
		const suspendKeepAlive = setInterval(() => {}, 2 ** 30);

		// Ignore SIGINT while suspended so Ctrl+C in the terminal does not
		// kill the backgrounded process. The handler is removed on resume.
		const ignoreSigint = () => {};
		process.on("SIGINT", ignoreSigint);

		// Set up handler to restore TUI when resumed
		process.once("SIGCONT", () => {
			clearInterval(suspendKeepAlive);
			process.removeListener("SIGINT", ignoreSigint);
			this.ui.start();
			this.ui.requestRender(true);
		});

		try {
			// Stop the TUI (restore terminal to normal mode)
			this.ui.stop();

			// Send SIGTSTP to process group (pid=0 means all processes in group)
			process.kill(0, "SIGTSTP");
		} catch (error) {
			clearInterval(suspendKeepAlive);
			process.removeListener("SIGINT", ignoreSigint);
			throw error;
		}
	}

	private async handleFollowUp(): Promise<void> {
		const text = (this.editor.getExpandedText?.() ?? this.editor.getText()).trim();
		if (!text) return;

		// Queue input during compaction (extension commands execute immediately)
		if (this.session.isCompacting) {
			if (this.messageQueue.isExtensionCommand(text)) {
				this.editor.addToHistory?.(text);
				this.editor.setText("");
				await this.session.prompt(text);
			} else {
				this.messageQueue.queueCompactionMessage(text, "followUp");
			}
			return;
		}

		// Alt+Enter queues a follow-up message (waits until agent finishes)
		// This handles extension commands (execute immediately), prompt template expansion, and queueing
		if (this.session.isStreaming) {
			this.editor.addToHistory?.(text);
			this.editor.setText("");
			await this.session.prompt(text, { streamingBehavior: "followUp" });
			this.messageQueue.updatePendingMessagesDisplay();
			this.ui.requestRender();
		}
		// If not streaming, Alt+Enter acts like regular Enter (trigger onSubmit)
		else if (this.editor.onSubmit) {
			this.editor.setText("");
			this.editor.onSubmit(text);
		}
	}

	private handleDequeue(): void {
		const restored = this.messageQueue.restoreQueuedMessagesToEditor();
		if (restored === 0) {
			this.showStatus("No queued messages to restore");
		} else {
			this.showStatus(`Restored ${restored} queued message${restored > 1 ? "s" : ""} to editor`);
		}
	}

	private updateEditorBorderColor(): void {
		if (this.isBashMode) {
			this.editor.borderColor = theme.getBashModeBorderColor();
		} else {
			const level = this.session.thinkingLevel || "off";
			this.editor.borderColor = theme.getThinkingBorderColor(level);
		}
		this.ui.requestRender();
	}

	private updateEditorPromptPrefix(): void {
		if (this.isBashMode) {
			this.editor.promptPrefix = "!";
			this.editor.promptColor = theme.getBashModeBorderColor();
		} else {
			this.editor.promptPrefix = "❯";
			this.editor.promptColor = (s: string) => s;
		}
		this.ui.requestRender();
	}

	private cycleThinkingLevel(): void {
		const newLevel = this.session.cycleThinkingLevel();
		if (newLevel === undefined) {
			this.showStatus("Current model does not support thinking");
		} else {
			this.footer.invalidate();
			this.updateEditorBorderColor();
			this.showStatus(`Thinking level: ${newLevel}`);
		}
	}

	/**
	 * Put a tool block into the transcript, extending the open chain or starting
	 * one. Every path that renders a tool call comes through here — live
	 * streaming, execution start, and rebuilding history — so a chain cannot be
	 * assembled correctly in one place and forgotten in another.
	 */
	private attachToolBlock(block: ToolExecutionComponent): void {
		if (!this.openChain || !this.openChain.isOpen) {
			this.openChain = new ToolChainComponent(this.toolOutputView);
			this.chatContainer.addChild(this.openChain);
		}
		this.openChain.add(block);
		// The newest call carries radar's marker stroke. This is the one place
		// every tool block is attached, so it is the one place that can know
		// which block is newest without two paths disagreeing about it. The run
		// it belongs to is marked as well: radar folds a run of more than one
		// call to a single line, and that line is what the view usually shows.
		this.latestToolBlock?.setLatest(false);
		block.setLatest(true);
		this.latestToolBlock = block;
		if (this.latestChain !== this.openChain) {
			this.latestChain?.setLatest(false);
			this.openChain.setLatest(true);
			this.latestChain = this.openChain;
		}
	}

	/**
	 * Settle the open chain.
	 *
	 * Called when the agent speaks (the run it was doing is over, whatever comes
	 * next is a new one) and when the turn settles. Closing on the agent's next
	 * words rather than only at turn end is what keeps the flip cheap: the line
	 * is still at the bottom of the screen, so the TUI rewrites it in place
	 * instead of taking the full-redraw path that clears terminal scrollback.
	 */
	private closeOpenChain(outcome: "done" | "interrupted"): void {
		if (!this.openChain) return;
		if (this.openChain.isEmpty) {
			this.chatContainer.removeChild(this.openChain);
		} else {
			this.openChain.close(outcome);
		}
		this.openChain = undefined;
		this.ui.requestRender();
	}

	/** Every tool block in the transcript, in order, across all chains. */
	private transcriptToolBlocks(): ToolExecutionComponent[] {
		const blocks: ToolExecutionComponent[] = [];
		for (const child of this.chatContainer.children) {
			if (child instanceof ToolChainComponent) blocks.push(...child.toolBlocks);
			else if (child instanceof ToolExecutionComponent) blocks.push(child);
		}
		return blocks;
	}

	/** Every chain in the transcript, in order. */
	private transcriptChains(): ToolChainComponent[] {
		return this.chatContainer.children.filter(
			(child): child is ToolChainComponent => child instanceof ToolChainComponent,
		);
	}

	private toggleToolOutputExpansion(): void {
		this.setToolsExpanded(!this.toolOutputExpanded);
	}

	/**
	 * Open or close one tool block instead of all of them.
	 *
	 * `ctrl+o` is all-or-nothing, which left the ▸ caret on every glance and
	 * radar row advertising something no key could do. This is the missing half:
	 * unfold walks back from the newest block to the first one still folded,
	 * fold walks back to the most recently opened one.
	 *
	 * Working from the tail is not a shortcut, it is the only thing this view can
	 * honestly offer. The transcript is bottom-anchored and has no app-level
	 * scrolling — anything far enough up is in the terminal's own scrollback,
	 * where this process cannot put a cursor or scroll to a selection. So the
	 * key peels backwards through what is actually on screen, and each block it
	 * opens is its own marker for where you have got to.
	 */
	private stepToolOutput(direction: "unfold" | "fold"): void {
		const wantOpen = direction === "unfold";

		// Radar's unit is the chain: one press turns the newest summary line back
		// into the calls it stands for. Its per-call bodies are a glance/full
		// question, so the step stops there rather than cascading.
		if (this.toolOutputView === "radar") {
			const chains = this.transcriptChains();
			for (let i = chains.length - 1; i >= 0; i--) {
				if (chains[i].isEmpty || chains[i].isOpened === wantOpen) continue;
				// A chain of one is never summarised, so there is nothing for unfold to
				// reveal; skipping it keeps the press moving to a chain that will change.
				if (wantOpen && !chains[i].isSummarised) continue;
				chains[i].setOpened(wantOpen);
				this.ui.requestRender();
				return;
			}
			this.showStatus(wantOpen ? "No collapsed chains below this point" : "No open chains below this point");
			return;
		}

		const blocks = this.transcriptToolBlocks();
		for (let i = blocks.length - 1; i >= 0; i--) {
			const block = blocks[i];
			if (block.isRevealed() === wantOpen) continue;
			block.setExpanded(wantOpen);
			this.ui.requestRender();
			return;
		}
		this.showStatus(wantOpen ? "No folded tool output below this point" : "No unfolded tool output below this point");
	}

	/**
	 * Move the view dial one stop and report where it landed.
	 *
	 * The dial is the persistent decision ("how much do I ever want to see"),
	 * which is why it saves; `app.tools.expand` stays the momentary one ("open
	 * what is in front of me"), which is why it does not.
	 */
	private cycleToolOutputView(direction: "forward" | "backward"): void {
		const next = cycleToolOutputView(this.toolOutputView, direction);
		this.applyToolOutputView(next);
	}

	/**
	 * How much of a thinking trace this view shows.
	 *
	 * Radar drops them outright, regardless of the setting. The dial is one
	 * question asked once — "how much do I ever want to see" — and a view whose
	 * whole job is to fold a run of tool calls down to a row cannot then spend
	 * forty lines on the reasoning that led to it. Folding to the label is not
	 * enough either: a message that only thought and called tools has nothing
	 * else on screen once its calls join the chain, so its label would stand
	 * alone under the summary and one row per chain would become one row plus a
	 * label per call.
	 *
	 * It is also what keeps a running chain on screen. A chain stays open across
	 * a thinking block (thinking is not text, so it is not a chain boundary), and
	 * a trace rendering below pushes the chain's own summary line off the top —
	 * where every subsequent call rewrites a line above the viewport and forces
	 * the full redraw that clears terminal scrollback.
	 */
	private thinkingDisplayForView(): ThinkingDisplay {
		if (this.toolOutputView === "radar") return "omit";
		return this.hideThinkingBlock ? "label" : "full";
	}

	private applyToolOutputView(view: ToolOutputView): void {
		const previousThinking = this.thinkingDisplayForView();
		this.toolOutputView = view;
		this.settingsManager.setToolOutputView(view);
		this.footer.setToolOutputView(view);
		const thinking = this.thinkingDisplayForView();
		for (const child of this.chatContainer.children) {
			if (child instanceof ToolChainComponent) child.setView(view);
			else if (child instanceof ToolExecutionComponent) child.setView(view);
			else if (child instanceof AssistantMessageComponent && thinking !== previousThinking) {
				child.setThinkingDisplay(thinking);
			}
		}
		if (this.streamingComponent && thinking !== previousThinking) {
			this.streamingComponent.setThinkingDisplay(thinking);
		}
		this.ui.requestRender();
	}

	private setToolsExpanded(expanded: boolean): void {
		this.toolOutputExpanded = expanded;
		const activeHeader = this.chrome.customHeader ?? this.builtInHeader;
		if (isExpandable(activeHeader)) {
			activeHeader.setExpanded(expanded);
		}
		for (const child of this.chatContainer.children) {
			if (isExpandable(child)) {
				child.setExpanded(expanded);
			}
		}
		this.ui.requestRender();
	}

	/**
	 * Step the agent mode one along (ask → plan → build → debug → ask).
	 *
	 * Mode is the product's central guardrail and sits first in the footer, yet
	 * it was reachable only by typing `/mode <name>`. The list comes from the
	 * mode command's own argument completions rather than a copy kept here, so
	 * a project that adds a mode gets it in the rotation for free; if the command
	 * is missing (a `--light` run strips the extension), the key says so instead
	 * of guessing.
	 */
	private async cycleAgentMode(): Promise<void> {
		const command = this.session.extensionRunner.getCommand("mode");
		if (!command) {
			this.showWarning("Modes are not available in this session");
			return;
		}

		const completions = (await command.getArgumentCompletions?.("")) ?? [];
		const modes = completions.map((item) => item.value);
		if (modes.length === 0) {
			this.showWarning("No modes are configured");
			return;
		}

		const current = this.footerDataProvider.getActiveMode();
		const index = modes.indexOf(current);
		const next = modes[(index + 1) % modes.length];
		await this.session.prompt(`/mode ${next}`);
	}

	private toggleThinkingBlockVisibility(): void {
		this.hideThinkingBlock = !this.hideThinkingBlock;
		this.settingsManager.setHideThinkingBlock(this.hideThinkingBlock);

		// Rebuild chat from session messages
		this.chatContainer.clear();
		this.rebuildChatFromMessages();

		// If streaming, re-add the streaming component with updated visibility and re-render
		if (this.streamingComponent && this.streamingMessage) {
			this.streamingComponent.setThinkingDisplay(this.thinkingDisplayForView());
			this.streamingComponent.updateContent(this.streamingMessage);
			this.chatContainer.addChild(this.streamingComponent);
		}

		// Radar overrides the setting, so say so rather than claiming a change the
		// screen does not show.
		this.showStatus(
			!this.hideThinkingBlock && this.toolOutputView === "radar"
				? "Thinking blocks: visible (radar hides them)"
				: `Thinking blocks: ${this.hideThinkingBlock ? "hidden" : "visible"}`,
		);
	}

	private openExternalEditor(): void {
		// Determine editor (respect $VISUAL, then $EDITOR)
		const editorCmd = process.env.VISUAL || process.env.EDITOR;
		if (!editorCmd) {
			this.showWarning("No editor configured. Set $VISUAL or $EDITOR environment variable.");
			return;
		}

		const currentText = this.editor.getExpandedText?.() ?? this.editor.getText();
		const tmpFile = path.join(os.tmpdir(), `${APP_NAME}-editor-${Date.now()}.${APP_NAME}.md`);

		try {
			// Write current content to temp file
			fs.writeFileSync(tmpFile, currentText, "utf-8");

			// Stop TUI to release terminal
			this.ui.stop();

			// Split by space to support editor arguments (e.g., "code --wait")
			const [editor, ...editorArgs] = editorCmd.split(" ");

			// Spawn editor synchronously with inherited stdio for interactive editing
			const result = spawnSync(editor, [...editorArgs, tmpFile], {
				stdio: "inherit",
				shell: process.platform === "win32",
			});

			// On successful exit (status 0), replace editor content
			if (result.status === 0) {
				const newContent = fs.readFileSync(tmpFile, "utf-8").replace(/\n$/, "");
				this.editor.setText(newContent);
			}
			// On non-zero exit, keep original text (no action needed)
		} finally {
			// Clean up temp file
			try {
				fs.unlinkSync(tmpFile);
			} catch {
				// Ignore cleanup errors
			}

			// Restart TUI
			this.ui.start();
			// Force full re-render since external editor uses alternate screen
			this.ui.requestRender(true);
		}
	}

	// =========================================================================
	// UI helpers
	// =========================================================================

	clearEditor(): void {
		this.editor.setText("");
		this.ui.requestRender();
	}

	/**
	 * Paint a message as a filled block, the shape errors and warnings share.
	 *
	 * These two used to render differently for no reason anyone could name: an
	 * error got a blank line above it and a column of padding, a warning got
	 * neither, so a warning collided with whatever was printed before it and hung
	 * off the left margin. Both are the same kind of interruption, so both get the
	 * same frame, and the fill is what separates them from ordinary chat output —
	 * a single coloured line is easy to scroll straight past.
	 */
	private showBlock(bg: "warningBg" | "toolErrorBg", fg: "warning" | "error", title: string, body: string[]): void {
		const box = new Box(1, 1, (t) => theme.bg(bg, t));
		applyPaperSheet(box);
		box.addChild(new Text(theme.bold(theme.fg(fg, title)), 0, 0));
		for (const line of body) {
			box.addChild(new Text(theme.fg("muted", line), 0, 0));
		}
		this.chatContainer.addChild(new Spacer(1));
		this.chatContainer.addChild(box);
		this.ui.requestRender();
	}

	/**
	 * Split a message into its headline and the rest.
	 *
	 * Multi-line notifications — `/learn` reporting where it searched, a settings
	 * dump — read as a heading over detail, and rendering them as one undifferen-
	 * tiated coloured block loses that. A single-line message is all headline.
	 */
	private splitBlockMessage(message: string): { title: string; body: string[] } {
		const [first = "", ...rest] = message.split("\n");
		return { title: first, body: rest };
	}

	showError(errorMessage: string): void {
		const { title, body } = this.splitBlockMessage(errorMessage);
		// A wait measured in days is a dead end, not a delay, and the block that
		// reports it is the only place the user finds out. Say what to do about
		// it there, rather than leaving them to reread a 429 and guess.
		if (isLongRetryDelayError(errorMessage)) {
			body.push("Switch to another model with /model, or wait for the provider's quota to reset.");
		}
		this.showBlock("toolErrorBg", "error", `Error: ${title}`, body);
	}

	showWarning(warningMessage: string): void {
		const { title, body } = this.splitBlockMessage(warningMessage);
		this.showBlock("warningBg", "warning", title, body);
	}

	/**
	 * A warning the user pays for if they miss it — same frame as `showWarning`,
	 * with an explicit title over its body.
	 */
	showNotice(title: string, body: string[]): void {
		this.showBlock("warningBg", "warning", title, body);
	}

	/**
	 * Say once, at startup, that `websearch` is running on the keyless backend.
	 * Shown only while the tool is actually active and no keyed provider is
	 * configured; silenced by `warnings.websearchApiKey`.
	 */
	private maybeWarnAboutMissingWebSearchKey(): void {
		const notice = websearchApiKeyNotice({
			activeToolNames: this.session.getActiveToolNames(),
			warnings: this.settingsManager.getWarnings(),
			search: this.settingsManager.getWebtoolsSearch(),
		});
		if (notice) this.showNotice(notice.title, notice.body);
	}

	showNewVersionNotification(newVersion: string): void {
		const action = theme.fg("accent", `${APP_NAME} update`);
		const updateInstruction = theme.fg("muted", `New version ${newVersion} is available. Run `) + action;
		const changelogUrl = "https://github.com/kolisachint/hoocode/blob/main/packages/coding-agent/CHANGELOG.md";
		const changelogLink = getCapabilities().hyperlinks
			? hyperlink(theme.fg("accent", "open changelog"), changelogUrl)
			: theme.fg("accent", changelogUrl);
		const changelogLine = theme.fg("muted", "Changelog: ") + changelogLink;

		this.chatContainer.addChild(new Spacer(1));
		this.chatContainer.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
		this.chatContainer.addChild(
			new Text(
				`${theme.bold(theme.fg("warning", "Update Available"))}\n${updateInstruction}\n${changelogLine}`,
				1,
				0,
			),
		);
		this.chatContainer.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
		this.ui.requestRender();
	}

	showPackageUpdateNotification(packages: string[]): void {
		const action = theme.fg("accent", `${APP_NAME} update`);
		const updateInstruction = theme.fg("muted", "Package updates are available. Run ") + action;
		const packageLines = packages.map((pkg) => `- ${pkg}`).join("\n");

		this.chatContainer.addChild(new Spacer(1));
		this.chatContainer.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
		this.chatContainer.addChild(
			new Text(
				`${theme.bold(theme.fg("warning", "Package Updates Available"))}\n${updateInstruction}\n${theme.fg("muted", "Packages:")}\n${packageLines}`,
				1,
				0,
			),
		);
		this.chatContainer.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
		this.ui.requestRender();
	}

	// =========================================================================
	// Selectors
	// =========================================================================

	/**
	 * Shows a selector component in place of the editor.
	 * @param create Factory that receives a `done` callback and returns the component and focus target
	 */
	private showSelector(create: (done: () => void) => { component: Component; focus: Component }): void {
		const done = () => {
			this.editorContainer.clear();
			this.editorContainer.addChild(this.editor);
			this.ui.setFocus(this.editor);
		};
		const { component, focus } = create(done);
		this.editorContainer.clear();
		this.editorContainer.addChild(component);
		this.ui.setFocus(focus);
		this.ui.requestRender();
	}

	private showSettingsSelector(): void {
		this.showSelector((done) => {
			const disabledToolNames = new Set(this.settingsManager.getDisabledTools());
			const builtinToolNames = this.session
				.getAllTools()
				.filter((t) => t.sourceInfo?.source === "builtin")
				.map((t) => t.name);
			// Union so tools disabled at startup (absent from the live registry) still
			// appear and can be re-enabled for the next session.
			const toolToggleNames = [...new Set([...builtinToolNames, ...disabledToolNames])].sort();
			// Price every tool the pane lists, including the ones that are off: what
			// the row needs to show is what turning it back on will cost per turn. A
			// tool disabled before launch has no schema in this session, so it has no
			// price to show either.
			const schemaTokens = new Map(
				this.session.getAllTools().map((tool) => [tool.name, measureToolSchemaTokens(tool)]),
			);
			const toolToggles = toolToggleNames.map((name) => ({
				name,
				enabled: !disabledToolNames.has(name),
				tokens: schemaTokens.get(name),
			}));

			const toolGroups = [
				{
					id: "web",
					label: "Web tools",
					description: "webfetch + websearch (network access).",
					enabled: this.settingsManager.getEnableWebTools(),
				},
				{
					id: "embsearch",
					label: "Semantic search",
					description: "Semantic index layer fused into the always-on search tool.",
					enabled: this.settingsManager.getEnableEmbsearchTools(),
				},
			];

			const flagDefs = this.session.extensionRunner.getFlags();
			const flagValues = this.session.extensionRunner.getFlagValues();
			const flags = [...flagDefs.values()].map((flag) => ({
				name: flag.name,
				description: flag.description,
				type: flag.type,
				value: flagValues.get(flag.name) ?? flag.default ?? (flag.type === "boolean" ? false : ""),
			}));

			const selector = new SettingsSelectorComponent(
				{
					autoCompact: this.session.autoCompactionEnabled,
					tools: toolToggles,
					toolGroups,
					// Resolved fresh on every open, never downloaded: a binary that
					// arrived since the last open (a startup fetch, a `brew install`)
					// shows as present without a restart.
					externalTools: describeExternalTools(),
					flags,
					toolOutputView: this.toolOutputView,
					toolOutputMaxBytes: this.settingsManager.getToolOutputMaxBytes(),
					toolOutputMaxLines: this.settingsManager.getToolOutputMaxLines(),
					contextGc: this.settingsManager.getContextGcEnabled(),
					showImages: this.settingsManager.getShowImages(),
					imageWidthCells: this.settingsManager.getImageWidthCells(),
					autoResizeImages: this.settingsManager.getImageAutoResize(),
					blockImages: this.settingsManager.getBlockImages(),
					enableSkillCommands: this.settingsManager.getEnableSkillCommands(),
					light: this.settingsManager.getLight(),
					pluginInstallScope: this.settingsManager.getPluginInstallScope(),
					enablePluginTools: this.settingsManager.getEnablePluginTools(),
					// Rows write the user settings file, so a key the project file also
					// sets is merged back over it next session. The pane says which.
					projectPinnedSettings: Object.keys(this.settingsManager.getProjectSettings()),
					// The targets actually in force, so a session launched with --platform
					// shows what it is writing rather than the (possibly unset) setting.
					platform: getWorkspacePlatforms() ?? [],
					steeringMode: this.session.steeringMode,
					followUpMode: this.session.followUpMode,
					transport: this.settingsManager.getTransport(),
					thinkingLevel: this.session.thinkingLevel,
					availableThinkingLevels: this.session.getAvailableThinkingLevels(),
					currentTheme: this.settingsManager.getTheme() || "dark",
					availableThemes: getAvailableThemes(),
					hideThinkingBlock: this.hideThinkingBlock,
					collapseChangelog: this.settingsManager.getCollapseChangelog(),
					enableInstallTelemetry: this.settingsManager.getEnableInstallTelemetry(),
					doubleEscapeAction: this.settingsManager.getDoubleEscapeAction(),
					treeFilterMode: this.settingsManager.getTreeFilterMode(),
					showHardwareCursor: this.settingsManager.getShowHardwareCursor(),
					editorBorder: this.settingsManager.getEditorBorder(),
					editorPaddingX: this.settingsManager.getEditorPaddingX(),
					autocompleteMaxVisible: this.settingsManager.getAutocompleteMaxVisible(),
					quietStartup: this.settingsManager.getQuietStartup(),
					clearOnShrink: this.settingsManager.getClearOnShrink(),
					showTerminalProgress: this.settingsManager.getShowTerminalProgress(),
					warnings: this.settingsManager.getWarnings(),
					// Show the effective value: env VOICETOOLS_SILENCE_MS wins for voice;
					// getWebtoolsTimeoutSecs already folds in HOOCODE_WEBTOOLS_TIMEOUT.
					voiceSilenceMs: resolveVoiceSilenceMs(this.settingsManager),
					webtoolsTimeoutSecs: this.settingsManager.getWebtoolsTimeoutSecs(),
					// The effective window, so a project settings.json narrowing it for
					// this repo shows up here rather than the user-scope value alone.
					// Re-measured on every change so the pane can price what a toggle did:
					// the session rebuilds its system prompt as tools come and go, and
					// this reads that live state rather than a snapshot.
					measureTokenSurface: () => measurePromptSurface(this.session),
					learn: (() => {
						const learn = this.settingsManager.getLearnSettings();
						return {
							learnMaxSessions: learn.maxSessions,
							learnMaxAgeDays: learn.maxAgeDays,
							learnMinRepeats: learn.minRepeats,
							learnMinRequestRepeats: learn.minRequestRepeats,
							learnMaxProposals: learn.maxProposals,
						};
					})(),
				},
				{
					onAutoCompactChange: (enabled) => {
						this.session.setAutoCompactionEnabled(enabled);
						this.footer.setAutoCompactEnabled(enabled);
					},
					onToolEnabledChange: (name, enabled) => {
						// Persist for future sessions (feeds the startup denylist).
						const disabled = new Set(this.settingsManager.getDisabledTools());
						if (enabled) {
							disabled.delete(name);
						} else {
							disabled.add(name);
						}
						this.settingsManager.setDisabledTools([...disabled]);

						// Apply live for the current session. Re-enabling only works for
						// tools still in the registry (i.e. not removed at startup); tools
						// disabled before launch take effect on the next session.
						const active = new Set(this.session.getActiveToolNames());
						if (enabled) {
							if (this.session.getToolDefinition(name)) {
								active.add(name);
							}
						} else {
							active.delete(name);
						}
						this.session.setActiveToolsByName([...active]);
						this.footerDataProvider.setSubagentEnabled(this.session.getActiveToolNames().includes("Task"));
					},
					onToolGroupChange: (id, enabled) => {
						// Master switches for tool availability. These gate tool creation at
						// session build, so they persist and take effect on the next session.
						switch (id) {
							case "web":
								this.settingsManager.setEnableWebTools(enabled);
								break;
							case "embsearch":
								this.settingsManager.setEnableEmbsearchTools(enabled);
								break;
						}
					},
					onToolOutputViewChange: (view) => {
						this.applyToolOutputView(view);
					},
					onToolOutputMaxBytesChange: (bytes) => {
						this.settingsManager.setToolOutputMaxBytes(bytes);
					},
					onToolOutputMaxLinesChange: (lines) => {
						this.settingsManager.setToolOutputMaxLines(lines);
					},
					onContextGcChange: (enabled) => {
						this.settingsManager.setContextGcEnabled(enabled);
					},
					onFlagChange: (name, value) => {
						// Persist for future launches; apply live best-effort (extensions
						// that read a flag only at load time pick it up on next launch).
						this.settingsManager.setFlagOverride(name, value);
						this.session.extensionRunner.setFlagValue(name, value);
					},
					onShowImagesChange: (enabled) => {
						this.settingsManager.setShowImages(enabled);
						for (const child of this.chatContainer.children) {
							if (child instanceof ToolExecutionComponent) {
								child.setShowImages(enabled);
							}
						}
					},
					onImageWidthCellsChange: (width) => {
						this.settingsManager.setImageWidthCells(width);
						for (const child of this.chatContainer.children) {
							if (child instanceof ToolExecutionComponent) {
								child.setImageWidthCells(width);
							}
						}
					},
					onAutoResizeImagesChange: (enabled) => {
						this.settingsManager.setImageAutoResize(enabled);
					},
					onBlockImagesChange: (blocked) => {
						this.settingsManager.setBlockImages(blocked);
					},
					onEnableSkillCommandsChange: (enabled) => {
						this.settingsManager.setEnableSkillCommands(enabled);
						this.setupAutocompleteProvider();
					},
					onLightChange: (enabled) => {
						// Picks the tool set and the system prompt at startup, so it binds
						// on the next session rather than this one.
						this.settingsManager.setLight(enabled);
					},
					onPluginInstallScopeChange: (scope) => {
						// Read per install by InstallPlugin, so it applies immediately with
						// nothing to reload.
						this.settingsManager.setPluginInstallScope(scope);
					},
					onEnablePluginToolsChange: (enabled) => {
						// Master switch for the autonomous plugin system. The lifecycle
						// tools are attached when the session is built, so they arrive on
						// the next session; the reuse nudge re-reads settings.json per
						// check and follows immediately.
						this.settingsManager.setEnablePluginTools(enabled);
					},
					onPlatformChange: (platforms) => {
						// Persist for later sessions, then update the process-wide session
						// state so plugin authoring and the /new-* scaffolds pick the new
						// layout up in this session too.
						this.settingsManager.setPlatform(platforms);
						setPlatforms(platforms);
					},
					onSteeringModeChange: (mode) => {
						this.session.setSteeringMode(mode);
					},
					onFollowUpModeChange: (mode) => {
						this.session.setFollowUpMode(mode);
					},
					onTransportChange: (transport) => {
						this.settingsManager.setTransport(transport);
						this.session.agent.transport = transport;
					},
					onThinkingLevelChange: (level) => {
						this.session.setThinkingLevel(level);
						this.footer.invalidate();
						this.updateEditorBorderColor();
					},
					onThemeChange: (themeName) => {
						const result = setTheme(themeName, true);
						this.settingsManager.setTheme(themeName);
						this.ui.invalidate();
						if (!result.success) {
							this.showError(`Failed to load theme "${themeName}": ${result.error}\nFell back to dark theme.`);
						}
					},
					onThemePreview: (themeName) => {
						const result = setTheme(themeName, true);
						if (result.success) {
							this.ui.invalidate();
							this.ui.requestRender();
						}
					},
					onHideThinkingBlockChange: (hidden) => {
						this.hideThinkingBlock = hidden;
						this.settingsManager.setHideThinkingBlock(hidden);
						const effective = this.thinkingDisplayForView();
						for (const child of this.chatContainer.children) {
							if (child instanceof AssistantMessageComponent) {
								child.setThinkingDisplay(effective);
							}
						}
						this.chatContainer.clear();
						this.rebuildChatFromMessages();
					},
					onCollapseChangelogChange: (collapsed) => {
						this.settingsManager.setCollapseChangelog(collapsed);
					},
					onEnableInstallTelemetryChange: (enabled) => {
						this.settingsManager.setEnableInstallTelemetry(enabled);
					},
					onQuietStartupChange: (enabled) => {
						this.settingsManager.setQuietStartup(enabled);
					},
					onDoubleEscapeActionChange: (action) => {
						this.settingsManager.setDoubleEscapeAction(action);
					},
					onTreeFilterModeChange: (mode) => {
						this.settingsManager.setTreeFilterMode(mode);
					},
					onShowHardwareCursorChange: (enabled) => {
						this.settingsManager.setShowHardwareCursor(enabled);
						this.ui.setShowHardwareCursor(enabled);
					},
					onEditorBorderChange: (border) => {
						this.settingsManager.setEditorBorder(border);
						this.defaultEditor.setBorder(border);
						if (this.editor !== this.defaultEditor && this.editor.setBorder !== undefined) {
							this.editor.setBorder(border);
						}
					},
					onEditorPaddingXChange: (padding) => {
						this.settingsManager.setEditorPaddingX(padding);
						this.defaultEditor.setPaddingX(padding);
						if (this.editor !== this.defaultEditor && this.editor.setPaddingX !== undefined) {
							this.editor.setPaddingX(padding);
						}
					},
					onAutocompleteMaxVisibleChange: (maxVisible) => {
						this.settingsManager.setAutocompleteMaxVisible(maxVisible);
						this.defaultEditor.setAutocompleteMaxVisible(maxVisible);
						if (this.editor !== this.defaultEditor && this.editor.setAutocompleteMaxVisible !== undefined) {
							this.editor.setAutocompleteMaxVisible(maxVisible);
						}
					},
					onClearOnShrinkChange: (enabled) => {
						this.settingsManager.setClearOnShrink(enabled);
						this.ui.setClearOnShrink(enabled);
					},
					onShowTerminalProgressChange: (enabled) => {
						this.settingsManager.setShowTerminalProgress(enabled);
					},
					onWarningsChange: (warnings) => {
						this.settingsManager.setWarnings(warnings);
					},
					onVoiceSilenceMsChange: (ms) => {
						this.settingsManager.setVoiceSilenceMs(ms);
						// Env override still wins; re-resolve so the live value stays consistent.
						this.voice.setSilenceMs(resolveVoiceSilenceMs(this.settingsManager));
					},
					onWebtoolsTimeoutSecsChange: (secs) => {
						// Persisted; webfetch/websearch pick it up when tools rebuild next session.
						this.settingsManager.setWebtoolsTimeoutSecs(secs);
					},
					onLearnSettingChange: (key, value) => {
						// /learn reads settings fresh on every invocation, so the next run
						// picks this up with no restart and nothing to re-wire live.
						this.settingsManager.setLearnSetting(key, value);
					},
					onCancel: () => {
						done();
						this.ui.requestRender();
					},
				},
			);
			return { component: selector, focus: selector.getSettingsList() };
		});
	}

	private showUserMessageSelector(): void {
		const userMessages = this.session.getUserMessagesForForking();

		if (userMessages.length === 0) {
			this.showStatus("No messages to fork from");
			return;
		}

		const initialSelectedId = userMessages[userMessages.length - 1]?.entryId;

		this.showSelector((done) => {
			const selector = new UserMessageSelectorComponent(
				userMessages.map((m) => ({ id: m.entryId, text: m.text })),
				async (entryId) => {
					try {
						const result = await this.runtimeHost.fork(entryId);
						if (result.cancelled) {
							done();
							this.ui.requestRender();
							return;
						}

						this.renderCurrentSessionState();
						this.editor.setText(result.selectedText ?? "");
						done();
						this.showStatus("Forked to new session");
					} catch (error: unknown) {
						done();
						this.showError(error instanceof Error ? error.message : String(error));
					}
				},
				() => {
					done();
					this.ui.requestRender();
				},
				initialSelectedId,
			);
			return { component: selector, focus: selector.getMessageList() };
		});
	}

	private showTreeSelector(initialSelectedId?: string): void {
		const tree = this.sessionManager.getTree();
		const realLeafId = this.sessionManager.getLeafId();
		const initialFilterMode = this.settingsManager.getTreeFilterMode();

		if (tree.length === 0) {
			this.showStatus("No entries in session");
			return;
		}

		this.showSelector((done) => {
			const selector = new TreeSelectorComponent(
				tree,
				realLeafId,
				this.ui.terminal.rows,
				async (entryId) => {
					// Selecting the current leaf is a no-op (already there)
					if (entryId === realLeafId) {
						done();
						this.showStatus("Already at this point");
						return;
					}

					// Ask about summarization
					done(); // Close selector first

					// Loop until user makes a complete choice or cancels to tree
					let wantsSummary = false;
					let customInstructions: string | undefined;

					// Check if we should skip the prompt (user preference to always default to no summary)
					if (!this.settingsManager.getBranchSummarySkipPrompt()) {
						while (true) {
							const summaryChoice = await this.dialogs.showSelector("Summarize branch?", [
								"No summary",
								"Summarize",
								"Summarize with custom prompt",
							]);

							if (summaryChoice === undefined) {
								// User pressed escape - re-show tree selector with same selection
								this.showTreeSelector(entryId);
								return;
							}

							wantsSummary = summaryChoice !== "No summary";

							if (summaryChoice === "Summarize with custom prompt") {
								customInstructions = await this.dialogs.showEditor("Custom summarization instructions");
								if (customInstructions === undefined) {
									// User cancelled - loop back to summary selector
									continue;
								}
							}

							// User made a complete choice
							break;
						}
					}

					// Set up escape handler and loader if summarizing
					let summaryLoader: Loader | undefined;
					const originalOnEscape = this.defaultEditor.onEscape;

					if (wantsSummary) {
						this.defaultEditor.onEscape = () => {
							this.session.abortBranchSummary();
						};
						this.chatContainer.addChild(new Spacer(1));
						summaryLoader = new Loader(
							this.ui,
							(spinner) => theme.fg("accent", spinner),
							(text) => theme.fg("muted", text),
							`Summarizing branch... (${keyText("app.interrupt")} to cancel)`,
						);
						this.statusContainer.addChild(summaryLoader);
						this.ui.requestRender();
					}

					try {
						const result = await this.session.navigateTree(entryId, {
							summarize: wantsSummary,
							customInstructions,
						});

						if (result.aborted) {
							// Summarization aborted - re-show tree selector with same selection
							this.showStatus("Branch summarization cancelled");
							this.showTreeSelector(entryId);
							return;
						}
						if (result.cancelled) {
							this.showStatus("Navigation cancelled");
							return;
						}

						// Update UI
						this.chatContainer.clear();
						this.renderInitialMessages();
						if (result.editorText && !this.editor.getText().trim()) {
							this.editor.setText(result.editorText);
						}
						this.showStatus("Navigated to selected point");
						void this.messageQueue.flushCompactionQueue({ willRetry: false });
					} catch (error) {
						this.showError(error instanceof Error ? error.message : String(error));
					} finally {
						if (summaryLoader) {
							summaryLoader.stop();
							this.statusContainer.clear();
						}
						this.defaultEditor.onEscape = originalOnEscape;
					}
				},
				() => {
					done();
					this.ui.requestRender();
				},
				(entryId, label) => {
					this.sessionManager.appendLabelChange(entryId, label);
					this.ui.requestRender();
				},
				initialSelectedId,
				initialFilterMode,
			);
			return { component: selector, focus: selector };
		});
	}

	/**
	 * Swatch picker for the session chip's colour. Moving through the list
	 * repaints the live chip, so the choice is made by looking at the real thing
	 * in the real theme rather than at a preview of it.
	 */
	private showSessionColorSelector(): void {
		const originalSlot = this.sessionManager.getSessionColorSlot();
		this.showSelector((done) => {
			const previewSlot = (slot: number) => {
				const chip = renderSessionChip(this.sessionManager.getDisplayName(), slot);
				this.defaultEditor.topBorderLabel = chip;
				if (this.editor !== this.defaultEditor) {
					this.editor.topBorderLabel = chip;
				}
				this.ui.requestRender();
			};
			const selector = new SessionColorSelectorComponent(
				this.sessionManager.getDisplayName(),
				originalSlot,
				(slot) => {
					done();
					this.session.setSessionColor(slot);
					this.showStatus(`Session color set to ${slot}`);
				},
				() => {
					done();
					// Cancelling must put back the colour the session actually has,
					// not leave it wearing the last swatch the cursor passed over.
					previewSlot(originalSlot);
					this.ui.requestRender();
				},
				previewSlot,
			);
			return { component: selector, focus: selector.getSelectList() };
		});
	}

	private showSessionSelector(): void {
		this.showSelector((done) => {
			const selector = new SessionSelectorComponent(
				(onProgress) =>
					SessionManager.list(this.sessionManager.getCwd(), this.sessionManager.getSessionDir(), onProgress),
				SessionManager.listAll,
				async (sessionPath) => {
					done();
					await this.handleResumeSession(sessionPath);
				},
				() => {
					done();
					this.ui.requestRender();
				},
				() => {
					void this.shutdown();
				},
				() => this.ui.requestRender(),
				{
					renameSession: async (sessionFilePath: string, nextName: string | undefined) => {
						const next = (nextName ?? "").trim();
						if (!next) return;
						const mgr = SessionManager.open(sessionFilePath);
						mgr.appendSessionInfo({ name: next });
					},
					showRenameHint: true,
					keybindings: this.keybindings,
				},

				this.sessionManager.getSessionFile(),
			);
			return { component: selector, focus: selector };
		});
	}

	private async handleResumeSession(
		sessionPath: string,
		options?: Parameters<ExtensionCommandContext["switchSession"]>[1],
	): Promise<{ cancelled: boolean }> {
		if (this.loadingAnimation) {
			this.loadingAnimation.stop();
			this.loadingAnimation = undefined;
		}
		this.statusContainer.clear();
		try {
			const result = await this.runtimeHost.switchSession(sessionPath, {
				withSession: options?.withSession,
			});
			if (result.cancelled) {
				return result;
			}
			this.renderCurrentSessionState();
			this.showStatus("Resumed session");
			return result;
		} catch (error: unknown) {
			if (error instanceof MissingSessionCwdError) {
				const selectedCwd = await this.promptForMissingSessionCwd(error);
				if (!selectedCwd) {
					this.showStatus("Resume cancelled");
					return { cancelled: true };
				}
				const result = await this.runtimeHost.switchSession(sessionPath, {
					cwdOverride: selectedCwd,
					withSession: options?.withSession,
				});
				if (result.cancelled) {
					return result;
				}
				this.renderCurrentSessionState();
				this.showStatus("Resumed session in current cwd");
				return result;
			}
			return this.handleFatalRuntimeError("Failed to resume session", error);
		}
	}

	// =========================================================================
	// Command handlers
	// =========================================================================

	private async handleReloadCommand(): Promise<void> {
		if (this.session.isStreaming) {
			this.showWarning("Wait for the current response to finish before reloading.");
			return;
		}
		if (this.session.isCompacting) {
			this.showWarning("Wait for compaction to finish before reloading.");
			return;
		}

		this.resetExtensionUI();

		const reloadBox = new Container();
		const borderColor = (s: string) => theme.fg("border", s);
		reloadBox.addChild(new DynamicBorder(borderColor));
		reloadBox.addChild(new Spacer(1));
		reloadBox.addChild(
			new Text(theme.fg("muted", "Reloading keybindings, extensions, skills, prompts, themes..."), 1, 0),
		);
		reloadBox.addChild(new Spacer(1));
		reloadBox.addChild(new DynamicBorder(borderColor));

		const previousEditor = this.editor;
		this.editorContainer.clear();
		this.editorContainer.addChild(reloadBox);
		this.ui.setFocus(reloadBox);
		this.ui.requestRender(true);
		await new Promise((resolve) => process.nextTick(resolve));

		const dismissReloadBox = (editor: Component) => {
			this.editorContainer.clear();
			this.editorContainer.addChild(editor);
			this.ui.setFocus(editor);
			this.ui.requestRender();
		};

		try {
			await this.session.reload();
			this.keybindings.reload();
			const activeHeader = this.chrome.customHeader ?? this.builtInHeader;
			if (isExpandable(activeHeader)) {
				activeHeader.setExpanded(this.toolOutputExpanded);
			}
			setRegisteredThemes(this.session.resourceLoader.getThemes().themes);
			this.hideThinkingBlock = this.settingsManager.getHideThinkingBlock();
			const themeName = this.settingsManager.getTheme();
			const themeResult = themeName ? setTheme(themeName, true) : { success: true };
			if (!themeResult.success) {
				this.showError(`Failed to load theme "${themeName}": ${themeResult.error}\nFell back to dark theme.`);
			}
			const editorBorder = this.settingsManager.getEditorBorder();
			const editorPaddingX = this.settingsManager.getEditorPaddingX();
			const autocompleteMaxVisible = this.settingsManager.getAutocompleteMaxVisible();
			this.defaultEditor.setBorder(editorBorder);
			this.defaultEditor.setPaddingX(editorPaddingX);
			this.defaultEditor.setAutocompleteMaxVisible(autocompleteMaxVisible);
			if (this.editor !== this.defaultEditor) {
				this.editor.setBorder?.(editorBorder);
				this.editor.setPaddingX?.(editorPaddingX);
				this.editor.setAutocompleteMaxVisible?.(autocompleteMaxVisible);
			}
			this.ui.setShowHardwareCursor(this.settingsManager.getShowHardwareCursor());
			this.ui.setClearOnShrink(this.settingsManager.getClearOnShrink());
			this.setupAutocompleteProvider();
			const runner = this.session.extensionRunner;
			this.setupExtensionShortcuts(runner);
			this.rebuildChatFromMessages();
			dismissReloadBox(this.editor as Component);
			this.showLoadedResources({
				force: false,
				showDiagnosticsWhenQuiet: true,
			});
			const modelsJsonError = this.session.modelRegistry.getError();
			if (modelsJsonError) {
				this.showError(`models.json error: ${modelsJsonError}`);
			}
			this.showStatus("Reloaded keybindings, extensions, skills, prompts, themes");
		} catch (error) {
			dismissReloadBox(previousEditor as Component);
			this.showError(`Reload failed: ${error instanceof Error ? error.message : String(error)}`);
		}
	}

	private async handleCompactCommand(customInstructions?: string): Promise<void> {
		// The session is the single source of truth for whether compaction is
		// possible (e.g. "Already compacted", "Nothing to compact (session too
		// small)"). Its specific error is surfaced via the compaction_end event,
		// so we don't pre-check here and risk a divergent message.
		if (this.loadingAnimation) {
			this.loadingAnimation.stop();
			this.loadingAnimation = undefined;
		}
		this.statusContainer.clear();

		try {
			await this.session.compact(customInstructions);
		} catch {
			// Ignore, will be emitted as an event
		}
	}

	stop(): void {
		this.unregisterSignalHandlers();
		this.voice.dispose();
		if (this.settingsManager.getShowTerminalProgress()) {
			this.ui.terminal.setProgress(false);
		}
		if (this.loadingAnimation) {
			this.loadingAnimation.stop();
			this.loadingAnimation = undefined;
		}
		this.clearExtensionTerminalInputListeners();
		this.footer.dispose();
		this.footerDataProvider.dispose();
		if (this.unsubscribe) {
			this.unsubscribe();
		}
		if (this.isInitialized) {
			this.ui.stop();
			// The terminal is ours again: later log lines can print normally.
			setTerminalOwnedByTui(false);
			this.isInitialized = false;
		}
	}
}
