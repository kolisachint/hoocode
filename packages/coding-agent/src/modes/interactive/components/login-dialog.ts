import { getOAuthProviders } from "@kolisachint/hoocode-ai/oauth";
import { Container, type Focusable, getKeybindings, Input, Spacer, Text, type TUI } from "@kolisachint/hoocode-tui";
import { exec } from "child_process";
import { styleInput, theme } from "../theme/theme.js";
import { InputFrame } from "./input-frame.js";
import { keyHint } from "./keybinding-hints.js";

/**
 * Login dialog component - replaces editor during OAuth login flow
 */
export class LoginDialogComponent extends InputFrame implements Focusable {
	private contentContainer: Container;
	private input: Input;
	private tui: TUI;
	private abortController = new AbortController();
	private inputResolver?: (value: string) => void;
	private inputRejecter?: (error: Error) => void;
	/** Holds the frame open until the flow has something to show. */
	private placeholder?: Text;

	// Focusable implementation - propagate to input for IME cursor positioning
	private _focused = false;
	get focused(): boolean {
		return this._focused;
	}
	set focused(value: boolean) {
		this._focused = value;
		this.input.focused = value;
	}

	constructor(
		tui: TUI,
		providerId: string,
		private onComplete: (success: boolean, message?: string) => void,
		providerNameOverride?: string,
		titleOverride?: string,
	) {
		super();
		this.tui = tui;

		const providerInfo = getOAuthProviders().find((p) => p.id === providerId);
		const providerName = providerNameOverride || providerInfo?.name || providerId;
		this.setTitle(titleOverride ?? `Login to ${providerName}`);

		// Dynamic content area
		this.contentContainer = new Container();
		this.addChild(this.contentContainer);
		// A frame with nothing in it draws nothing, and the first thing this
		// dialog has to say arrives over the network — so it says it is starting
		// until then, rather than leaving a gap where the prompt used to be.
		this.placeholder = new Text(theme.fg("dim", "Starting…"), 0, 0);
		this.contentContainer.addChild(this.placeholder);

		// Input (always present, used when needed)
		this.input = new Input();
		styleInput(this.input);
		this.input.onSubmit = () => {
			if (this.inputResolver) {
				this.inputResolver(this.input.getValue());
				this.inputResolver = undefined;
				this.inputRejecter = undefined;
			}
		};
		this.input.onEscape = () => {
			this.cancel();
		};
	}

	/**
	 * A blank row between what is already shown and what is about to be — and
	 * nothing at all when there is nothing above it. The flow's steps append to
	 * each other, so each one asked for a leading `Spacer(1)`; with the title
	 * moved into the border the first step's blank row sits flush under the top
	 * rule, which is the wasted row the vertical rhythm rules name.
	 */
	private separate(): void {
		if (this.contentContainer.children.length > 0) this.contentContainer.addChild(new Spacer(1));
	}

	/** Remove the "Starting…" row the first time the flow has something to say. */
	private dropPlaceholder(): void {
		if (!this.placeholder) return;
		this.contentContainer.removeChild(this.placeholder);
		this.placeholder = undefined;
	}

	get signal(): AbortSignal {
		return this.abortController.signal;
	}

	private cancel(): void {
		this.abortController.abort();
		if (this.inputRejecter) {
			this.inputRejecter(new Error("Login cancelled"));
			this.inputResolver = undefined;
			this.inputRejecter = undefined;
		}
		this.onComplete(false, "Login cancelled");
	}

	/**
	 * Called by onAuth callback - show URL and optional instructions
	 */
	showAuth(url: string, instructions?: string): void {
		this.dropPlaceholder();
		this.contentContainer.clear();
		const linkedUrl = `\x1b]8;;${url}\x07${url}\x1b]8;;\x07`;
		this.contentContainer.addChild(new Text(theme.fg("accent", linkedUrl), 0, 0));

		const clickHint = process.platform === "darwin" ? "Cmd+click to open" : "Ctrl+click to open";
		const hyperlink = `\x1b]8;;${url}\x07${clickHint}\x1b]8;;\x07`;
		this.contentContainer.addChild(new Text(theme.fg("dim", hyperlink), 0, 0));

		if (instructions) {
			this.contentContainer.addChild(new Spacer(1));
			this.contentContainer.addChild(new Text(theme.fg("warning", instructions), 0, 0));
		}

		// Try to open browser
		const openCmd = process.platform === "darwin" ? "open" : process.platform === "win32" ? "start" : "xdg-open";
		exec(`${openCmd} "${url}"`);

		this.tui.requestRender();
	}

	/**
	 * Show input for manual code/URL entry (for callback server providers)
	 */
	showManualInput(prompt: string): Promise<string> {
		this.dropPlaceholder();
		this.separate();
		this.contentContainer.addChild(new Text(theme.fg("dim", prompt), 0, 0));
		this.contentContainer.addChild(this.input);
		this.contentContainer.addChild(new Text(`(${keyHint("tui.select.cancel", "to cancel")})`, 0, 0));
		this.tui.requestRender();

		return new Promise((resolve, reject) => {
			this.inputResolver = resolve;
			this.inputRejecter = reject;
		});
	}

	/**
	 * Called by onPrompt callback - show prompt and wait for input
	 * Note: Does NOT clear content, appends to existing (preserves URL from showAuth)
	 */
	showPrompt(message: string, placeholder?: string): Promise<string> {
		this.dropPlaceholder();
		this.separate();
		this.contentContainer.addChild(new Text(theme.fg("text", message), 0, 0));
		if (placeholder) {
			this.contentContainer.addChild(new Text(theme.fg("dim", `e.g., ${placeholder}`), 0, 0));
		}
		this.contentContainer.addChild(this.input);
		this.contentContainer.addChild(
			new Text(
				`(${keyHint("tui.select.cancel", "to cancel,")} ${keyHint("tui.select.confirm", "to submit")})`,
				1,
				0,
			),
		);

		this.input.setValue("");
		this.tui.requestRender();

		return new Promise((resolve, reject) => {
			this.inputResolver = resolve;
			this.inputRejecter = reject;
		});
	}

	/**
	 * Show informational text without prompting for input.
	 */
	showInfo(lines: string[]): void {
		this.dropPlaceholder();
		this.contentContainer.clear();
		for (const line of lines) {
			this.contentContainer.addChild(new Text(line, 0, 0));
		}
		this.contentContainer.addChild(new Spacer(1));
		this.contentContainer.addChild(new Text(`(${keyHint("tui.select.cancel", "to close")})`, 0, 0));
		this.tui.requestRender();
	}

	/**
	 * Show waiting message (for polling flows like GitHub Copilot)
	 */
	showWaiting(message: string): void {
		this.dropPlaceholder();
		this.separate();
		this.contentContainer.addChild(new Text(theme.fg("dim", message), 0, 0));
		this.contentContainer.addChild(new Text(`(${keyHint("tui.select.cancel", "to cancel")})`, 0, 0));
		this.tui.requestRender();
	}

	/**
	 * Called by onProgress callback
	 */
	showProgress(message: string): void {
		this.dropPlaceholder();
		this.contentContainer.addChild(new Text(theme.fg("dim", message), 0, 0));
		this.tui.requestRender();
	}

	handleInput(data: string): void {
		const kb = getKeybindings();

		if (kb.matches(data, "tui.select.cancel")) {
			this.cancel();
			return;
		}

		// Pass to input
		this.input.handleInput(data);
	}
}
