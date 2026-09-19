/**
 * Extracted command handlers from InteractiveMode.
 * All methods receive dependencies via CommandContext rather than
 * reaching back into the parent class.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { Model } from "@kolisachint/hoocode-ai";
import type { EditorComponent, MarkdownTheme, TUI } from "@kolisachint/hoocode-tui";
import { type Container, Markdown, Spacer, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import { spawn, spawnSync } from "child_process";
import { APP_TITLE, getDebugLogPath, getShareViewerUrl } from "../../config.js";
import { loadAgentRegistry } from "../../core/agent-registry.js";
import type { AgentSession } from "../../core/agent-session.js";
import type { AgentSessionRuntime } from "../../core/agent-session-runtime.js";
import { ChangeDirectoryError, SessionImportFileNotFoundError } from "../../core/agent-session-runtime.js";
import type { KeybindingsManager } from "../../core/keybindings.js";
import { MissingSessionCwdError } from "../../core/session-cwd.js";
import {
	parseSessionColorSlot,
	SESSION_COLOR_NAME_LIST,
	SESSION_COLOR_SLOTS,
	sessionColorName,
} from "../../core/session-identity.js";
import type { SessionManager } from "../../core/session-manager.js";
import { getSubagentPool } from "../../core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../../core/subagent-result.js";
import { getChangelogPath, parseChangelog } from "../../utils/changelog.js";
import { markdownToHtml } from "../../utils/markdown-to-html.js";
import { copyRichToClipboard } from "../../utils/rich-clipboard.js";
import { BorderedLoader } from "./components/bordered-loader.js";
import { DynamicBorder } from "./components/dynamic-border.js";
import type { FooterComponent } from "./components/footer.js";
import { formatKeyText, keyDisplayLabel, keyDisplayText } from "./components/keybinding-hints.js";
import { renderSessionChip } from "./components/session-chip.js";
import { theme } from "./theme/theme.js";

export interface CommandContext {
	// Core dependencies
	session: AgentSession;
	sessionManager: SessionManager;
	runtimeHost: AgentSessionRuntime;
	ui: TUI;
	editor: EditorComponent;
	editorContainer: Container;
	chatContainer: Container;
	statusContainer: Container;
	footer: FooterComponent;
	keybindings: KeybindingsManager;

	// UI callbacks
	/** What a command has to say, on the band above the prompt; gone in seconds. */
	showStatus: (message: string) => void;
	/**
	 * A status the transcript keeps, for the handful that carry something the
	 * screen cannot answer for later: a share URL, a path written to.
	 */
	showRecord: (message: string) => void;
	/** A glimpse on the band above the prompt; gone in a few seconds. */
	notify: (message: string, note?: string) => void;
	showError: (message: string) => void;
	showWarning: (message: string) => void;
	updateEditorBorderColor: () => void;
	renderCurrentSessionState: () => void;
	rebuildChatFromMessages: () => void;
	getMarkdownThemeWithSettings: () => MarkdownTheme;
	stopLoadingAnimation: () => void;

	// Auth/model helpers
	findExactModelMatch: (searchTerm: string) => Promise<Model<any> | undefined>;
	maybeWarnAboutAnthropicSubscriptionAuth: (model?: Model<any>) => Promise<void>;

	// Dialog callbacks
	showModelSelector: (searchTerm?: string) => void;
	showExtensionConfirm: (title: string, message: string) => Promise<boolean>;
	promptForMissingSessionCwd: (error: MissingSessionCwdError) => Promise<string | undefined>;

	// Fatal error handler
	handleFatalRuntimeError: (prefix: string, error: unknown) => Promise<never>;
}

export class CommandExecutor {
	/** Where `/cd -` returns to. Set on every successful move, session-local. */
	private previousCwd?: string;

	constructor(private readonly ctx: CommandContext) {}

	// =========================================================================
	// Slash command handlers
	// =========================================================================

	async handleModel(searchTerm?: string): Promise<void> {
		if (!searchTerm) {
			this.ctx.showModelSelector();
			return;
		}

		const model = await this.ctx.findExactModelMatch(searchTerm);
		if (model) {
			try {
				await this.ctx.session.setModel(model);
				this.ctx.footer.invalidate();
				this.ctx.updateEditorBorderColor();
				this.ctx.notify(`Model: ${model.id}`);
				void this.ctx.maybeWarnAboutAnthropicSubscriptionAuth(model);
			} catch (error) {
				this.ctx.showError(error instanceof Error ? error.message : String(error));
			}
			return;
		}

		this.ctx.showModelSelector(searchTerm);
	}

	async handleClone(): Promise<void> {
		const leafId = this.ctx.sessionManager.getLeafId();
		if (!leafId) {
			this.ctx.showStatus("Nothing to clone yet");
			return;
		}

		try {
			const result = await this.ctx.runtimeHost.fork(leafId, { position: "at" });
			if (result.cancelled) {
				this.ctx.ui.requestRender();
				return;
			}

			this.ctx.renderCurrentSessionState();
			this.ctx.editor.setText("");
			this.ctx.showStatus("Cloned to new session");
		} catch (error: unknown) {
			this.ctx.showError(error instanceof Error ? error.message : String(error));
		}
	}

	async handleSubagent(text: string): Promise<void> {
		const prefix = "/subagent ";
		const args = text.startsWith(prefix) ? text.slice(prefix.length).trim() : "";
		if (!args) {
			this.ctx.showStatus("Usage: /subagent <mode> <task>");
			return;
		}

		const firstSpace = args.indexOf(" ");
		if (firstSpace === -1) {
			this.ctx.showStatus("Usage: /subagent <mode> <task>");
			return;
		}

		const mode = args.slice(0, firstSpace).trim();
		const task = args.slice(firstSpace + 1).trim();
		if (!task) {
			this.ctx.showStatus("Usage: /subagent <mode> <task>");
			return;
		}

		const validModes = loadAgentRegistry({ cwd: this.ctx.sessionManager.getCwd() })
			.list()
			.map((a) => a.name);
		if (!validModes.includes(mode)) {
			this.ctx.showStatus(`Unknown subagent_type: ${mode}. Available: ${validModes.join(", ")}`);
			return;
		}

		this.ctx.showStatus(`Spawning ${mode} subagent...`);
		try {
			const pool = getSubagentPool(this.ctx.sessionManager.getCwd(), this.ctx.session.modelRegistry.getAvailable());
			const dispatchResult = await pool.dispatch(task, {
				forceAgent: mode,
				model: this.ctx.session.model?.id,
				provider: this.ctx.session.model?.provider,
			});
			const result = dispatchResult.result;
			const resultData = result?.result_data as SubagentResultFile | undefined;
			if (result?.ok) {
				this.ctx.showStatus(`${mode} subagent completed`);
				// Inject the subagent answer as a custom message so the user can see it in the chat
				this.ctx.sessionManager.appendMessage({
					role: "custom",
					customType: "subagent",
					content: resultData?.summary || "(no output)",
					display: true,
					timestamp: Date.now(),
				});
			} else {
				this.ctx.showError(`Subagent (${mode}) failed: ${result?.error ?? "unknown error"}`);
			}
		} catch (error: unknown) {
			this.ctx.showError(error instanceof Error ? error.message : String(error));
		}
	}

	async handleExport(text: string): Promise<void> {
		const outputPath = this.getPathArgument(text, "/export");

		try {
			if (outputPath?.endsWith(".jsonl")) {
				const filePath = this.ctx.session.exportToJsonl(outputPath);
				this.ctx.showRecord(`Session exported to: ${filePath}`);
			} else {
				const filePath = await this.ctx.session.exportToHtml(outputPath);
				this.ctx.showRecord(`Session exported to: ${filePath}`);
			}
		} catch (error: unknown) {
			this.ctx.showError(`Failed to export session: ${error instanceof Error ? error.message : "Unknown error"}`);
		}
	}

	private getPathArgument(text: string, command: "/export" | "/import"): string | undefined {
		if (text === command) {
			return undefined;
		}
		if (!text.startsWith(`${command} `)) {
			return undefined;
		}

		const argsString = text.slice(command.length + 1).trimStart();
		if (!argsString) {
			return undefined;
		}

		const firstChar = argsString[0];
		if (firstChar === '"' || firstChar === "'") {
			const closingQuoteIndex = argsString.indexOf(firstChar, 1);
			if (closingQuoteIndex < 0) {
				return undefined;
			}
			return argsString.slice(1, closingQuoteIndex);
		}

		const firstWhitespaceIndex = argsString.search(/\s/);
		if (firstWhitespaceIndex < 0) {
			return argsString;
		}
		return argsString.slice(0, firstWhitespaceIndex);
	}

	async handleImport(text: string): Promise<void> {
		const inputPath = this.getPathArgument(text, "/import");
		if (!inputPath) {
			this.ctx.showError("Usage: /import <path.jsonl>");
			return;
		}

		const confirmed = await this.ctx.showExtensionConfirm(
			"Import session",
			`Replace current session with ${inputPath}?`,
		);
		if (!confirmed) {
			this.ctx.showStatus("Import cancelled");
			return;
		}

		try {
			this.ctx.stopLoadingAnimation();
			this.ctx.statusContainer.clear();
			const result = await this.ctx.runtimeHost.importFromJsonl(inputPath);
			if (result.cancelled) {
				this.ctx.showStatus("Import cancelled");
				return;
			}
			this.ctx.renderCurrentSessionState();
			this.ctx.showRecord(`Session imported from: ${inputPath}`);
		} catch (error: unknown) {
			if (error instanceof MissingSessionCwdError) {
				const selectedCwd = await this.ctx.promptForMissingSessionCwd(error);
				if (!selectedCwd) {
					this.ctx.showStatus("Import cancelled");
					return;
				}
				const result = await this.ctx.runtimeHost.importFromJsonl(inputPath, selectedCwd);
				if (result.cancelled) {
					this.ctx.showStatus("Import cancelled");
					return;
				}
				this.ctx.renderCurrentSessionState();
				this.ctx.showRecord(`Session imported from: ${inputPath}`);
				return;
			}
			if (error instanceof SessionImportFileNotFoundError) {
				this.ctx.showError(`Failed to import session: ${error.message}`);
				return;
			}
			await this.ctx.handleFatalRuntimeError("Failed to import session", error);
		}
	}

	async handleShare(): Promise<void> {
		// Check if gh is available and logged in
		try {
			const authResult = spawnSync("gh", ["auth", "status"], { encoding: "utf-8" });
			if (authResult.status !== 0) {
				this.ctx.showError("GitHub CLI is not logged in. Run 'gh auth login' first.");
				return;
			}
		} catch {
			this.ctx.showError("GitHub CLI (gh) is not installed. Install it from https://cli.github.com/");
			return;
		}

		// Export to a temp file
		const tmpFile = path.join(os.tmpdir(), "session.html");
		try {
			await this.ctx.session.exportToHtml(tmpFile);
		} catch (error: unknown) {
			this.ctx.showError(`Failed to export session: ${error instanceof Error ? error.message : "Unknown error"}`);
			return;
		}

		// Show cancellable loader, replacing the editor
		const loader = new BorderedLoader(this.ctx.ui, theme, "Creating gist...");
		this.ctx.editorContainer.clear();
		this.ctx.editorContainer.addChild(loader);
		this.ctx.ui.setFocus(loader);
		this.ctx.ui.requestRender();

		const restoreEditor = () => {
			loader.dispose();
			this.ctx.editorContainer.clear();
			this.ctx.editorContainer.addChild(this.ctx.editor);
			this.ctx.ui.setFocus(this.ctx.editor);
			try {
				fs.unlinkSync(tmpFile);
			} catch {
				// Ignore cleanup errors
			}
		};

		// Create a secret gist asynchronously
		let proc: ReturnType<typeof spawn> | null = null;

		loader.onAbort = () => {
			proc?.kill();
			restoreEditor();
			this.ctx.showStatus("Share cancelled");
		};

		try {
			const result = await new Promise<{ stdout: string; stderr: string; code: number | null }>((resolve) => {
				proc = spawn("gh", ["gist", "create", "--public=false", tmpFile]);
				let stdout = "";
				let stderr = "";
				proc.stdout?.on("data", (data) => {
					stdout += data.toString();
				});
				proc.stderr?.on("data", (data) => {
					stderr += data.toString();
				});
				proc.on("close", (code) => resolve({ stdout, stderr, code }));
			});

			if (loader.signal.aborted) return;

			restoreEditor();

			if (result.code !== 0) {
				const errorMsg = result.stderr?.trim() || "Unknown error";
				this.ctx.showError(`Failed to create gist: ${errorMsg}`);
				return;
			}

			// Extract gist ID from the URL returned by gh
			// gh returns something like: https://gist.github.com/username/GIST_ID
			const gistUrl = result.stdout?.trim();
			const gistId = gistUrl?.split("/").pop();
			if (!gistId) {
				this.ctx.showError("Failed to parse gist ID from gh output");
				return;
			}

			// The gist is the artifact. A viewer URL is an extra only when one is
			// actually configured (see getShareViewerUrl).
			const previewUrl = getShareViewerUrl(gistId);
			this.ctx.showRecord(previewUrl ? `Share URL: ${previewUrl}\nGist: ${gistUrl}` : `Gist: ${gistUrl}`);
		} catch (error: unknown) {
			if (!loader.signal.aborted) {
				restoreEditor();
				this.ctx.showError(`Failed to create gist: ${error instanceof Error ? error.message : "Unknown error"}`);
			}
		}
	}

	/**
	 * `/copy`, `/copy all`, `/copy <n>` — the conversation, not a picture of it.
	 *
	 * What is on screen is markdown already rendered: a table is box drawing, a
	 * code block is a bordered panel, every line is wrapped to whatever width
	 * the window happened to be. Dragging over that and pasting it into a
	 * document pastes the drawing — dotted rules, broken table edges, and
	 * wrapping frozen at eighty columns — because a terminal's clipboard carries
	 * glyphs and nothing else.
	 *
	 * So the copy goes back to the source and puts it on the clipboard twice:
	 * the markdown as text, and HTML for anything that takes a rich paste. Word
	 * and Confluence then paste real headings, lists and tables; a terminal or a
	 * commit message still gets the markdown. Which flavours landed depends on
	 * the platform (see `copyRichToClipboard`), so the status line says.
	 */
	async handleCopy(text = ""): Promise<void> {
		const argument = text
			.replace(/^\/copy\s*/, "")
			.trim()
			.toLowerCase();
		// `/copy 0` is a typo, not a request for nothing: taking it at face value
		// would copy the whole session and report it as zero turns.
		const turns = /^\d+$/.test(argument) ? Math.max(1, Number.parseInt(argument, 10)) : undefined;
		const whole = argument === "all" || argument === "session";
		if (argument && !whole && turns === undefined) {
			this.ctx.showWarning("Usage: /copy [all|<number of turns>]");
			return;
		}

		const markdown =
			whole || turns !== undefined
				? this.ctx.session.getTranscriptMarkdown({ turns, agentLabel: APP_TITLE })
				: this.ctx.session.getLastAssistantText();
		if (!markdown) {
			this.ctx.showError(
				whole || turns !== undefined ? "Nothing in this session to copy yet." : "No agent messages to copy yet.",
			);
			return;
		}

		const subject = whole
			? "session transcript"
			: turns !== undefined
				? `last ${turns} turn${turns > 1 ? "s" : ""}`
				: "last agent message";
		try {
			const flavour = await copyRichToClipboard({ text: markdown, html: markdownToHtml(markdown) });
			// Naming the flavour is not chatter: "copied" meaning markdown on one
			// machine and a formatted paste on another is how this gets reported
			// as broken. The user finds out here rather than in the document.
			const as = flavour === "rich" ? "markdown + formatted text" : "markdown";
			this.ctx.showStatus(`Copied ${subject} as ${as}`);
		} catch (error) {
			// The usual reason is a machine with no clipboard to write to (a bare
			// SSH session, no xclip) or a transcript past what OSC 52 can carry.
			// Both have the same answer, and it is worth naming here rather than
			// leaving "Failed to copy to clipboard" as the whole reply.
			const reason = error instanceof Error ? error.message : String(error);
			this.ctx.showError(`Could not copy the ${subject}: ${reason}. /export writes it to a file instead.`);
		}
	}

	/** The session's chip as it currently renders, for echoing back after a change. */
	private currentChip(): string {
		const chip = renderSessionChip(
			this.ctx.sessionManager.getDisplayName(),
			this.ctx.sessionManager.getSessionColorSlot(),
		);
		return chip?.styled ?? this.ctx.sessionManager.getDisplayName();
	}

	handleName(text: string): void {
		const name = text.replace(/^\/name\s*/, "").trim();
		if (!name) {
			// Every session has a name now — an auto-assigned slug until someone
			// picks one — so there is always something to report, and the reply
			// shows the chip rather than describing it.
			const chosen = this.ctx.sessionManager.getSessionName();
			const label = chosen ? "Session name:" : "Session name (auto):";
			this.ctx.chatContainer.addChild(new Spacer(1));
			this.ctx.chatContainer.addChild(
				new Text(
					`${theme.fg("dim", label)} ${this.currentChip()}  ${theme.fg("dim", "/name <name> to change")}`,
					1,
					0,
				),
			);
			this.ctx.ui.requestRender();
			return;
		}

		this.ctx.session.setSessionName(name);
		// The chip itself rides the prompt's top border, so the confirmation only
		// has to bridge the moment between pressing enter and looking at it.
		this.ctx.notify(`${theme.fg("dim", "Session name set:")} ${this.currentChip()}`);
		this.ctx.ui.requestRender();
	}

	/**
	 * `/color <slot>` sets the session's chip colour directly, where a slot is a
	 * number, a name (`green`), or a name's initial (`g`). Bare `/color` is
	 * handled by the caller, which opens the swatch picker instead — the names
	 * describe a hue family rather than the exact colour a theme fills the slot
	 * with, so which one reads best here is still a thing you look at.
	 *
	 * The confirmation names the slot it landed on, because the aliases fold
	 * neighbouring hues together: `/color red` answering with "magenta" is how
	 * you learn the palette has no red of its own.
	 */
	handleColor(text: string): boolean {
		const arg = text.replace(/^\/color\s*/, "").trim();
		if (!arg) return false;
		const slot = parseSessionColorSlot(arg);
		if (slot === undefined) {
			this.ctx.showWarning(
				`Usage: /color <1-${SESSION_COLOR_SLOTS}> or /color <${SESSION_COLOR_NAME_LIST.join("|")}> ` +
					"(first letter works too), or /color on its own to pick one",
			);
			return true;
		}
		this.ctx.session.setSessionColor(slot);
		const name = sessionColorName(slot);
		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(
			new Text(
				`${theme.fg("dim", "Session color set:")} ${this.currentChip()}${name ? `  ${theme.fg("dim", name)}` : ""}`,
				1,
				0,
			),
		);
		this.ctx.ui.requestRender();
		return true;
	}

	handleSession(): void {
		const stats = this.ctx.session.getSessionStats();
		const sessionName = this.ctx.sessionManager.getSessionName();

		let info = `${theme.bold("Session Info")}\n\n`;
		info += `${theme.fg("dim", sessionName ? "Name:" : "Name (auto):")} ${this.currentChip()}\n`;
		const branch = this.ctx.sessionManager.getSessionBranch();
		if (branch) {
			info += `${theme.fg("dim", "Branch:")} ${branch}\n`;
		}
		info += `${theme.fg("dim", "File:")} ${stats.sessionFile ?? "In-memory"}\n`;
		info += `${theme.fg("dim", "ID:")} ${stats.sessionId}\n\n`;
		info += `${theme.bold("Messages")}\n`;
		info += `${theme.fg("dim", "User:")} ${stats.userMessages}\n`;
		info += `${theme.fg("dim", "Assistant:")} ${stats.assistantMessages}\n`;
		info += `${theme.fg("dim", "Tool Calls:")} ${stats.toolCalls}\n`;
		info += `${theme.fg("dim", "Tool Results:")} ${stats.toolResults}\n`;
		info += `${theme.fg("dim", "Total:")} ${stats.totalMessages}\n\n`;
		info += `${theme.bold("Tokens")}\n`;
		info += `${theme.fg("dim", "Input:")} ${stats.tokens.input.toLocaleString()}\n`;
		info += `${theme.fg("dim", "Output:")} ${stats.tokens.output.toLocaleString()}\n`;
		if (stats.tokens.cacheRead > 0) {
			info += `${theme.fg("dim", "Cache Read:")} ${stats.tokens.cacheRead.toLocaleString()}\n`;
		}
		if (stats.tokens.cacheWrite > 0) {
			info += `${theme.fg("dim", "Cache Write:")} ${stats.tokens.cacheWrite.toLocaleString()}\n`;
		}
		info += `${theme.fg("dim", "Total:")} ${stats.tokens.total.toLocaleString()}\n`;

		if (stats.cost > 0) {
			info += `\n${theme.bold("Cost")}\n`;
			info += `${theme.fg("dim", "Total:")} ${stats.cost.toFixed(4)}`;
		}

		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new Text(info, 1, 0));
		this.ctx.ui.requestRender();
	}

	handleChangelog(): void {
		const changelogPath = getChangelogPath();
		const allEntries = parseChangelog(changelogPath);

		if (allEntries.length === 0) {
			this.ctx.chatContainer.addChild(new Spacer(1));
			this.ctx.chatContainer.addChild(new Text(theme.fg("dim", "No changelog entries found."), 1, 0));
			this.ctx.ui.requestRender();
			return;
		}

		const changelogMarkdown = allEntries
			.slice()
			.reverse()
			.map((e) => e.content)
			.join("\n\n");

		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new DynamicBorder());
		this.ctx.chatContainer.addChild(new Text(theme.bold(theme.fg("accent", "What's New")), 1, 0));
		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new Markdown(changelogMarkdown, 1, 0, this.ctx.getMarkdownThemeWithSettings()));
		this.ctx.chatContainer.addChild(new DynamicBorder());
		this.ctx.ui.requestRender();
	}

	handleHotkeys(): void {
		// Navigation keybindings
		const cursorUp = keyDisplayText("tui.editor.cursorUp");
		const cursorDown = keyDisplayText("tui.editor.cursorDown");
		const cursorLeft = keyDisplayText("tui.editor.cursorLeft");
		const cursorRight = keyDisplayText("tui.editor.cursorRight");
		const cursorWordLeft = keyDisplayText("tui.editor.cursorWordLeft");
		const cursorWordRight = keyDisplayText("tui.editor.cursorWordRight");
		const cursorLineStart = keyDisplayText("tui.editor.cursorLineStart");
		const cursorLineEnd = keyDisplayText("tui.editor.cursorLineEnd");
		const jumpForward = keyDisplayText("tui.editor.jumpForward");
		const jumpBackward = keyDisplayText("tui.editor.jumpBackward");

		// Editing keybindings
		const submit = keyDisplayText("tui.input.submit");
		const newLine = keyDisplayText("tui.input.newLine");
		const deleteWordBackward = keyDisplayText("tui.editor.deleteWordBackward");
		const deleteWordForward = keyDisplayText("tui.editor.deleteWordForward");
		const deleteToLineStart = keyDisplayText("tui.editor.deleteToLineStart");
		const deleteToLineEnd = keyDisplayText("tui.editor.deleteToLineEnd");
		const yank = keyDisplayText("tui.editor.yank");
		const yankPop = keyDisplayText("tui.editor.yankPop");
		const undo = keyDisplayText("tui.editor.undo");
		const tab = keyDisplayText("tui.input.tab");

		// App keybindings
		const interrupt = keyDisplayText("app.interrupt");
		const clear = keyDisplayText("app.clear");
		const exit = keyDisplayText("app.exit");
		const suspend = keyDisplayText("app.suspend");
		const cycleThinkingLevel = keyDisplayLabel("app.thinking.cycleForward");
		const cycleThinkingLevelBack = keyDisplayLabel("app.thinking.cycleBackward");
		const cycleModelForward = keyDisplayLabel("app.model.cycleForward");
		const expandTools = keyDisplayText("app.tools.expand");
		const toggleThinking = keyDisplayText("app.thinking.toggle");
		const cycleTaskView = keyDisplayLabel("app.tasks.cycleForward");
		const cycleTaskViewBack = keyDisplayLabel("app.tasks.cycleBackward");
		const teamFocus = keyDisplayText("app.team.focus");
		const teamNudge = keyDisplayText("app.team.nudge");
		const teamAttach = keyDisplayText("app.team.attach");
		const externalEditor = keyDisplayText("app.editor.external");
		const cycleModelBackward = keyDisplayLabel("app.model.cycleBackward");
		const followUp = keyDisplayText("app.message.followUp");
		const dequeue = keyDisplayText("app.message.dequeue");
		const pasteImage = keyDisplayText("app.clipboard.pasteImage");
		const viewForward = keyDisplayLabel("app.view.cycleForward");
		const viewBackward = keyDisplayLabel("app.view.cycleBackward");
		const voice = keyDisplayText("app.input.voiceTranscribe");
		const changeDirectory = keyDisplayText("app.session.changeDirectory");
		const openSettings = keyDisplayText("app.settings.open");
		const openHotkeys = keyDisplayText("app.hotkeys.open");
		const cycleMode = keyDisplayLabel("app.mode.cycleForward");
		const cycleModeBack = keyDisplayLabel("app.mode.cycleBackward");
		const sessionResume = keyDisplayText("app.session.resume");
		const cycleSessionColor = keyDisplayLabel("app.session.color.cycleForward");
		const cycleSessionColorBackward = keyDisplayLabel("app.session.color.cycleBackward");
		const chromeForward = keyDisplayLabel("app.chrome.cycleForward");
		const chromeBackward = keyDisplayLabel("app.chrome.cycleBackward");
		const scrollPageUp = keyDisplayText("app.scroll.pageUp");
		const scrollPageDown = keyDisplayText("app.scroll.pageDown");
		const scrollTop = keyDisplayText("app.scroll.top");
		const scrollBottom = keyDisplayText("app.scroll.bottom");
		const scrollLineUp = keyDisplayText("app.scroll.lineUp");
		const scrollLineDown = keyDisplayText("app.scroll.lineDown");
		const scrollExit = keyDisplayText("app.scroll.exit");
		const scrollSearch = keyDisplayText("app.scroll.search");
		const scrollSearchInView = keyDisplayText("app.scroll.searchInView");
		const scrollSearchNext = keyDisplayText("app.scroll.searchNext");
		const scrollSearchPrevious = keyDisplayText("app.scroll.searchPrevious");
		const scrollPreviousMessage = keyDisplayText("app.scroll.previousMessage");
		const scrollNextMessage = keyDisplayText("app.scroll.nextMessage");
		const copyMessage = keyDisplayText("app.clipboard.copyMessage");
		const redo = keyDisplayText("tui.editor.redo");

		let hotkeys = `
Grouped by what you are doing, not by what the key is. Seven groups, none of the
learned ones bigger than five — the size a person can actually hold. Three are
free: **Flow** is what every terminal program already taught you, **Scroll** is
what every pager did, and every picker prints its own keys on its own hint line,
so you read those instead of remembering them. **Screen** is one key.

**Compose** — the message in your hands
| Key | Action |
|-----|--------|
| \`${submit}\` | Send message |
| \`${newLine}\` | New line${process.platform === "win32" ? " (Ctrl+Enter on Windows Terminal)" : ""} |
| \`${tab}\` | Path completion / accept autocomplete |
| \`${externalEditor}\` | Edit the message in \`$VISUAL\` / \`$EDITOR\` |
| \`${voice}\` | Speak instead of type |
| \`${pasteImage}\` | Paste image from clipboard |
| \`${copyMessage}\` | Copy the agent's last message (\`/copy\`; \`/copy all\` for the session) |
| \`${followUp}\` | Queue a follow-up while the agent works |
| \`${dequeue}\` | Bring every queued message back to the editor |
| \`/\` \`!\` \`!!\` | Slash commands · run bash · run bash off the record |

**Steer** — what the agent is before it runs
The only three keys that change what happens next, and the only three that cost
anything. Step forward on the first key, back on the second; the footer shows
all three.

| Key | Steps | Through |
|-----|-------|---------|
| \`${cycleMode}\` / \`${cycleModeBack}\` | Agent mode | ask → plan → build → debug (\`/mode\` picks one) |
| \`${cycleModelForward}\` / \`${cycleModelBackward}\` | Model | your enabled models (\`/model\` picks one) |
| \`${cycleThinkingLevel}\` / \`${cycleThinkingLevelBack}\` | Thinking level | off → … → high |

**Read** — what you see of what it did
Free and reversible, every one: nothing here touches the work, only the window
onto it. Press again or add \`Shift\` and you are back where you were.

| Key | Action |
|-----|--------|
| \`${viewForward}\` / \`${viewBackward}\` | Step tool output: radar → peek → full |
| \`${cycleTaskView}\` / \`${cycleTaskViewBack}\` | Step the task panel: tasks → subagents → teams |
| \`${expandTools}\` | Jump to the full view and back, without moving the dial |
| \`${toggleThinking}\` | Show or hide thinking blocks |
| \`${teamFocus}\` | Focus the team roster — \`${teamNudge}\` nudges, \`${teamAttach}\` attaches, \`q\`/\`${interrupt}\` leaves (\`--team\`) |

**Screen** — how much room there is to see it in
One dial for the furniture. Everything below the transcript except the prompt,
which never hides.

| Key | Steps | Through |
|-----|-------|---------|
| \`${chromeForward}\` / \`${chromeBackward}\` | Chrome | full → compact (one-row footer, no task list) → bare (neither) — \`/chrome\` picks one |

It also gets out of the way on its own: the footer lends its rows to the
completion list while that is open, and the task list keeps its counts but drops
its rows while the agent is mid-turn. Both come back by themselves.

**Scroll** — where in the session you are looking
The wheel and these keys move the same view, and once it is scrolled back it
**stays** there: output keeps arriving underneath without moving what you are
reading. The bottom row tells you where you are and how to get back.

| Key | Action |
|-----|--------|
| \`${scrollPageUp}\` / \`${scrollPageDown}\` | Scroll a page — \`${scrollPageUp}\` is also how you start |
| \`${scrollTop}\` / \`${scrollBottom}\` | Jump to the start of the session / back to live |
| \`${scrollLineUp}\` / \`${scrollLineDown}\` | A line at a time (while scrolled back) |
| \`${scrollPreviousMessage}\` / \`${scrollNextMessage}\` | Jump to your previous / next message |
| \`${scrollSearch}\` | Search the transcript — \`${scrollSearchInView}\` once you are already scrolled back |
| \`${scrollSearchNext}\` / \`${scrollSearchPrevious}\` | Step the matches, once the query is committed with \`${submit}\` |
| \`${scrollExit}\` | Back to live output |
| Wheel | Three lines a notch, wherever you turn it |

Search runs backwards, like \`${scrollSearch}\` in a shell: the newest match first,
because what you are looking for in a session is behind you.

Scrolling to the bottom hands the live view back on its own, and so does typing:
any key that is not one of these returns you to live and then does its usual job.

**Go** — sessions and places
Each takes the screen and hands it back on \`${interrupt}\`, and each has a slash
command that does the same thing.

| Key | Action |
|-----|--------|
| \`${sessionResume}\` | Resume a session from history (\`/resume\`) |
| \`${changeDirectory}\` | Change working directory (\`/cd\`) |
| \`${cycleSessionColor}\` / \`${cycleSessionColorBackward}\` | Step the session chip's color (\`/color\` picks one) |
| \`${openSettings}\` | Open settings (\`/settings\`) |
| \`${openHotkeys}\` | Show this list (\`/hotkeys\`) |
| \`/tree\` | Open the session tree |

**Flow** — getting out, getting back
| Key | Action |
|-----|--------|
| \`${interrupt}\` | Cancel autocomplete / abort streaming |
| \`${clear}\` | Clear editor (first) / exit (second) |
| \`${exit}\` | Exit when the editor is empty |
| \`${suspend}\` | Suspend to background |

### What the chord tells you

\`Alt\`+letter **sets a value** and nothing takes the screen — you keep typing.
Seven of those are dials, and \`Shift\` always steps one back: **a**gent mode,
**m**odel, **t**hinking, tool **o**utput, task **l**ist, session **c**olor, and
chrome (**z**, the one letter that names nothing — its stop is the shape of the
screen in front of you).

\`Ctrl\`+letter **acts on what is drawn right now** and shares its letter with the
\`Alt\` key for the same subject. \`${viewForward}\` sets how much tool output there
ever is, \`${expandTools}\` jumps to all of it and back; \`${cycleThinkingLevel}\`
sets how much thinking there ever is, \`${toggleThinking}\` shows or hides what you
have. Two subjects, two letters, four keys.

The thinking level also answers to \`Shift+Tab\` — it is the one dial with no
slash command, so that is the way to it on a terminal that does not send \`Alt\`.

Inside a picker every \`Ctrl\` key still edits the query, so the picker's own verbs
are on \`Alt\` and its hint line names them.

### The editor
Standard readline/emacs bindings — reference, not something to memorise.
\`${scrollPageUp}\` / \`${scrollPageDown}\` are **not** here: they scroll the session,
not the prompt, which is one to three lines almost every time you look at it.

| Key | Action |
|-----|--------|
| \`${cursorUp}\` / \`${cursorDown}\` / \`${cursorLeft}\` / \`${cursorRight}\` | Move cursor / browse history (Up when empty) |
| \`${cursorWordLeft}\` / \`${cursorWordRight}\` | Move by word |
| \`${cursorLineStart}\` / \`${cursorLineEnd}\` | Start / end of line |
| \`${jumpForward}\` / \`${jumpBackward}\` | Jump forward / backward to character |
| \`${deleteWordBackward}\` / \`${deleteWordForward}\` | Delete word backwards / forwards |
| \`${deleteToLineStart}\` / \`${deleteToLineEnd}\` | Delete to start / end of line |
| \`${yank}\` / \`${yankPop}\` | Paste the most-recently-deleted text / cycle older ones |
| \`${undo}\` / \`${redo}\` | Undo / redo |
`;

		// Add extension-registered shortcuts
		const extensionRunner = this.ctx.session.extensionRunner;
		const shortcuts = extensionRunner.getShortcuts(this.ctx.keybindings.getEffectiveConfig());
		if (shortcuts.size > 0) {
			hotkeys += `
**Extensions**
| Key | Action |
|-----|--------|
`;
			for (const [key, shortcut] of shortcuts) {
				const description = shortcut.description ?? shortcut.extensionPath;
				const keyDisplay = formatKeyText(key, { capitalize: true });
				hotkeys += `| \`${keyDisplay}\` | ${description} |\n`;
			}
		}

		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new DynamicBorder());
		this.ctx.chatContainer.addChild(new Text(theme.bold(theme.fg("accent", "Keyboard Shortcuts")), 1, 0));
		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new Markdown(hotkeys.trim(), 1, 0, this.ctx.getMarkdownThemeWithSettings()));
		this.ctx.chatContainer.addChild(new DynamicBorder());
		this.ctx.ui.requestRender();
	}

	async handleClear(): Promise<void> {
		this.ctx.stopLoadingAnimation();
		this.ctx.statusContainer.clear();
		try {
			const result = await this.ctx.runtimeHost.newSession();
			if (result.cancelled) {
				return;
			}
			this.ctx.renderCurrentSessionState();
			this.ctx.chatContainer.addChild(new Spacer(1));
			this.ctx.chatContainer.addChild(new Text(`${theme.fg("accent", "✓ New session started")}`, 1, 0));
			this.ctx.ui.requestRender();
		} catch (error: unknown) {
			await this.ctx.handleFatalRuntimeError("Failed to create session", error);
		}
	}

	/**
	 * `/cd [path]` — move the whole session to another directory.
	 *
	 * Written for the case it exists to serve: you are done in one repo and want
	 * to work in the next one without losing the process, its provider auth, or
	 * its warmed model list. Bare `/cd` goes home, `/cd -` goes back to where you
	 * came from, `~` and relative paths resolve as a shell would.
	 *
	 * The move starts a fresh session rooted in the target — see
	 * `AgentSessionRuntime.changeDirectory` for why the whole runtime is rebuilt
	 * rather than a cwd string reassigned.
	 */
	async handleChangeDirectory(text: string): Promise<void> {
		const prefix = "/cd";
		const rawArg = text.startsWith(prefix) ? text.slice(prefix.length).trim() : text.trim();
		const previousCwd = this.ctx.sessionManager.getCwd();

		let target: string;
		if (!rawArg || rawArg === "~") {
			target = os.homedir();
		} else if (rawArg === "-") {
			if (!this.previousCwd) {
				this.ctx.showWarning("No previous directory to return to");
				return;
			}
			target = this.previousCwd;
		} else {
			const expanded = rawArg.startsWith("~/") ? path.join(os.homedir(), rawArg.slice(2)) : rawArg;
			target = path.resolve(previousCwd, expanded);
		}

		if (path.resolve(target) === path.resolve(previousCwd)) {
			this.ctx.notify(`Already in ${target}`);
			return;
		}

		this.ctx.stopLoadingAnimation();
		this.ctx.statusContainer.clear();
		try {
			const result = await this.ctx.runtimeHost.changeDirectory(target);
			if (result.cancelled) {
				return;
			}
			this.previousCwd = previousCwd;
			this.ctx.renderCurrentSessionState();
			this.ctx.chatContainer.addChild(new Spacer(1));
			this.ctx.chatContainer.addChild(
				new Text(
					`${theme.fg("accent", "✓ Working directory")} ${theme.fg("muted", result.cwd)}\n` +
						theme.fg(
							"dim",
							`New session started here. ${keyDisplayText("app.session.resume")} reopens the session you left in ${previousCwd}.`,
						),
					1,
					0,
				),
			);
			this.ctx.ui.requestRender();
		} catch (error: unknown) {
			if (error instanceof ChangeDirectoryError) {
				this.ctx.showError(error.message);
				return;
			}
			await this.ctx.handleFatalRuntimeError(`Failed to change directory to ${target}`, error);
		}
	}

	/** Directory completions for `/cd <prefix>`, plus the two shell shorthands. */
	getChangeDirectoryCompletions(argumentPrefix: string): Array<{ value: string; label: string }> {
		const cwd = this.ctx.sessionManager.getCwd();
		const expanded = argumentPrefix.startsWith("~/")
			? path.join(os.homedir(), argumentPrefix.slice(2))
			: argumentPrefix;
		// A prefix ending in a separator names the directory to list; otherwise the
		// last segment is a partial name to filter its parent by.
		const endsWithSep = expanded.endsWith("/") || expanded.endsWith(path.sep);
		const base = endsWithSep ? expanded : path.dirname(expanded);
		const partial = endsWithSep ? "" : path.basename(expanded);
		const searchDir = path.isAbsolute(base) ? base : path.resolve(cwd, base || ".");

		const completions: Array<{ value: string; label: string }> = [];
		if (!argumentPrefix) {
			if (this.previousCwd) completions.push({ value: "-", label: `- (${this.previousCwd})` });
			completions.push({ value: "~", label: `~ (${os.homedir()})` });
		}

		let entries: fs.Dirent[];
		try {
			entries = fs.readdirSync(searchDir, { withFileTypes: true });
		} catch {
			return completions;
		}

		for (const entry of entries) {
			if (!entry.isDirectory()) continue;
			if (partial && !entry.name.startsWith(partial)) continue;
			if (!partial && entry.name.startsWith(".")) continue;
			const joined =
				base && !endsWithSep
					? path.join(path.dirname(argumentPrefix), entry.name)
					: `${argumentPrefix}${entry.name}`;
			completions.push({ value: `${joined}/`, label: `${entry.name}/` });
		}
		return completions.slice(0, 50);
	}

	handleDebug(): void {
		const width = this.ctx.ui.terminal.columns;
		const height = this.ctx.ui.terminal.rows;
		const allLines = this.ctx.ui.render(width);

		const debugLogPath = getDebugLogPath();
		const debugData = [
			`Debug output at ${new Date().toISOString()}`,
			`Terminal: ${width}x${height}`,
			`Total lines: ${allLines.length}`,
			"",
			"=== All rendered lines with visible widths ===",
			...allLines.map((line, idx) => {
				const vw = visibleWidth(line);
				const escaped = JSON.stringify(line);
				return `[${idx}] (w=${vw}) ${escaped}`;
			}),
			"",
			"=== Agent messages (JSONL) ===",
			...this.ctx.session.messages.map((msg) => JSON.stringify(msg)),
			"",
		].join("\n");

		fs.mkdirSync(path.dirname(debugLogPath), { recursive: true });
		fs.writeFileSync(debugLogPath, debugData);

		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(
			new Text(`${theme.fg("accent", "✓ Debug log written")}\n${theme.fg("muted", debugLogPath)}`, 1, 0),
		);
		this.ctx.ui.requestRender();
	}
}
