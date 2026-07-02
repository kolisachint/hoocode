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
import { getDebugLogPath, getShareViewerUrl } from "../../config.js";
import { loadAgentRegistry } from "../../core/agent-registry.js";
import type { AgentSession } from "../../core/agent-session.js";
import type { AgentSessionRuntime } from "../../core/agent-session-runtime.js";
import { SessionImportFileNotFoundError } from "../../core/agent-session-runtime.js";
import type { KeybindingsManager } from "../../core/keybindings.js";
import { MissingSessionCwdError } from "../../core/session-cwd.js";
import type { SessionManager } from "../../core/session-manager.js";
import { getSubagentPool } from "../../core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../../core/subagent-result.js";
import { getChangelogPath, parseChangelog } from "../../utils/changelog.js";
import { copyToClipboard } from "../../utils/clipboard.js";
import { BorderedLoader } from "./components/bordered-loader.js";
import { DynamicBorder } from "./components/dynamic-border.js";
import type { FooterComponent } from "./components/footer.js";
import { formatKeyText, keyDisplayText } from "./components/keybinding-hints.js";
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
	showStatus: (message: string) => void;
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
				this.ctx.showStatus(`Model: ${model.id}`);
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
			const pool = getSubagentPool(this.ctx.sessionManager.getCwd());
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
				this.ctx.showStatus(`Session exported to: ${filePath}`);
			} else {
				const filePath = await this.ctx.session.exportToHtml(outputPath);
				this.ctx.showStatus(`Session exported to: ${filePath}`);
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
			this.ctx.showStatus(`Session imported from: ${inputPath}`);
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
				this.ctx.showStatus(`Session imported from: ${inputPath}`);
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

			// Create the preview URL
			const previewUrl = getShareViewerUrl(gistId);
			this.ctx.showStatus(`Share URL: ${previewUrl}\nGist: ${gistUrl}`);
		} catch (error: unknown) {
			if (!loader.signal.aborted) {
				restoreEditor();
				this.ctx.showError(`Failed to create gist: ${error instanceof Error ? error.message : "Unknown error"}`);
			}
		}
	}

	async handleCopy(): Promise<void> {
		const text = this.ctx.session.getLastAssistantText();
		if (!text) {
			this.ctx.showError("No agent messages to copy yet.");
			return;
		}

		try {
			await copyToClipboard(text);
			this.ctx.showStatus("Copied last agent message to clipboard");
		} catch (error) {
			this.ctx.showError(error instanceof Error ? error.message : String(error));
		}
	}

	handleName(text: string): void {
		const name = text.replace(/^\/name\s*/, "").trim();
		if (!name) {
			const currentName = this.ctx.sessionManager.getSessionName();
			if (currentName) {
				this.ctx.chatContainer.addChild(new Spacer(1));
				this.ctx.chatContainer.addChild(new Text(theme.fg("dim", `Session name: ${currentName}`), 1, 0));
			} else {
				this.ctx.showWarning("Usage: /name <name>");
			}
			this.ctx.ui.requestRender();
			return;
		}

		this.ctx.session.setSessionName(name);
		this.ctx.chatContainer.addChild(new Spacer(1));
		this.ctx.chatContainer.addChild(new Text(theme.fg("dim", `Session name set: ${name}`), 1, 0));
		this.ctx.ui.requestRender();
	}

	handleSession(): void {
		const stats = this.ctx.session.getSessionStats();
		const sessionName = this.ctx.sessionManager.getSessionName();

		let info = `${theme.bold("Session Info")}\n\n`;
		if (sessionName) {
			info += `${theme.fg("dim", "Name:")} ${sessionName}\n`;
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
		this.ctx.chatContainer.addChild(new Markdown(changelogMarkdown, 1, 1, this.ctx.getMarkdownThemeWithSettings()));
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
		const pageUp = keyDisplayText("tui.editor.pageUp");
		const pageDown = keyDisplayText("tui.editor.pageDown");

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
		const cycleThinkingLevel = keyDisplayText("app.thinking.cycle");
		const cycleModelForward = keyDisplayText("app.model.cycleForward");
		const selectModel = keyDisplayText("app.model.select");
		const expandTools = keyDisplayText("app.tools.expand");
		const toggleThinking = keyDisplayText("app.thinking.toggle");
		const cycleTaskView = keyDisplayText("app.tasks.cycleView");
		const externalEditor = keyDisplayText("app.editor.external");
		const cycleModelBackward = keyDisplayText("app.model.cycleBackward");
		const followUp = keyDisplayText("app.message.followUp");
		const dequeue = keyDisplayText("app.message.dequeue");
		const pasteImage = keyDisplayText("app.clipboard.pasteImage");

		let hotkeys = `
**Navigation**
| Key | Action |
|-----|--------|
| \`${cursorUp}\` / \`${cursorDown}\` / \`${cursorLeft}\` / \`${cursorRight}\` | Move cursor / browse history (Up when empty) |
| \`${cursorWordLeft}\` / \`${cursorWordRight}\` | Move by word |
| \`${cursorLineStart}\` | Start of line |
| \`${cursorLineEnd}\` | End of line |
| \`${jumpForward}\` | Jump forward to character |
| \`${jumpBackward}\` | Jump backward to character |
| \`${pageUp}\` / \`${pageDown}\` | Scroll by page |

**Editing**
| Key | Action |
|-----|--------|
| \`${submit}\` | Send message |
| \`${newLine}\` | New line${process.platform === "win32" ? " (Ctrl+Enter on Windows Terminal)" : ""} |
| \`${deleteWordBackward}\` | Delete word backwards |
| \`${deleteWordForward}\` | Delete word forwards |
| \`${deleteToLineStart}\` | Delete to start of line |
| \`${deleteToLineEnd}\` | Delete to end of line |
| \`${yank}\` | Paste the most-recently-deleted text |
| \`${yankPop}\` | Cycle through the deleted text after pasting |
| \`${undo}\` | Undo |

**Other**
| Key | Action |
|-----|--------|
| \`${tab}\` | Path completion / accept autocomplete |
| \`${interrupt}\` | Cancel autocomplete / abort streaming |
| \`${clear}\` | Clear editor (first) / exit (second) |
| \`${exit}\` | Exit (when editor is empty) |
| \`${suspend}\` | Suspend to background |
| \`${cycleThinkingLevel}\` | Cycle thinking level |
| \`${cycleModelForward}\` / \`${cycleModelBackward}\` | Cycle models |
| \`${selectModel}\` | Open model selector |
| \`${expandTools}\` | Toggle tool output expansion |
| \`${toggleThinking}\` | Toggle thinking block visibility |
| \`${cycleTaskView}\` | Cycle task panel view (tasks → subagents → teams) |
| \`${externalEditor}\` | Edit message in external editor |
| \`${followUp}\` | Queue follow-up message |
| \`${dequeue}\` | Restore queued messages |
| \`${pasteImage}\` | Paste image from clipboard |
| \`/\` | Slash commands |
| \`!\` | Run bash command |
| \`!!\` | Run bash command (excluded from context) |
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
		this.ctx.chatContainer.addChild(new Markdown(hotkeys.trim(), 1, 1, this.ctx.getMarkdownThemeWithSettings()));
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
			this.ctx.chatContainer.addChild(new Text(`${theme.fg("accent", "✓ New session started")}`, 1, 1));
			this.ctx.ui.requestRender();
		} catch (error: unknown) {
			await this.ctx.handleFatalRuntimeError("Failed to create session", error);
		}
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
			new Text(`${theme.fg("accent", "✓ Debug log written")}\n${theme.fg("muted", debugLogPath)}`, 1, 1),
		);
		this.ctx.ui.requestRender();
	}
}
