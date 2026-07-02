/**
 * Extension dialog primitives: the selector, options pane, confirm, text
 * input, multi-line editor, and custom-component dialogs that swap into the
 * editor slot (or an overlay) and restore the prompt editor when done.
 * Extracted from interactive-mode.ts; consumed by the ExtensionUIContext.
 */

import type { Component, Container, OverlayHandle, OverlayOptions, TUI } from "@kolisachint/hoocode-tui";
import type { AskQuestion, ExtensionUIDialogOptions } from "../../core/extensions/index.js";
import type { KeybindingsManager } from "../../core/keybindings.js";
import { AskOptionsComponent } from "./components/ask-options.js";
import { ExtensionEditorComponent } from "./components/extension-editor.js";
import { ExtensionInputComponent } from "./components/extension-input.js";
import { ExtensionSelectorComponent } from "./components/extension-selector.js";
import { type Theme, theme } from "./theme/theme.js";

/** The slice of the interactive mode the dialogs render into. */
export interface ExtensionDialogsDeps {
	ui: TUI;
	editorContainer: Container;
	keybindings: KeybindingsManager;
	/** The prompt editor (read at call time; the editor can be swapped). */
	getEditor(): Component & { getText(): string; setText(text: string): void };
}

export class ExtensionDialogs {
	private selector: ExtensionSelectorComponent | undefined = undefined;
	private input: ExtensionInputComponent | undefined = undefined;
	private askOptionsPane: AskOptionsComponent | undefined = undefined;
	private editorDialog: ExtensionEditorComponent | undefined = undefined;

	constructor(private readonly deps: ExtensionDialogsDeps) {}

	/** True while the options pane is on screen (used to queue team approvals). */
	isAskOptionsOpen(): boolean {
		return this.askOptionsPane !== undefined;
	}

	/** Hide whatever dialog is open and restore the prompt editor. */
	reset(): void {
		if (this.selector) {
			this.hideSelector();
		}
		if (this.input) {
			this.hideInput();
		}
		if (this.askOptionsPane) {
			this.hideAskOptions();
		}
		if (this.editorDialog) {
			this.hideEditor();
		}
	}

	private restoreEditor(): void {
		const editor = this.deps.getEditor();
		this.deps.editorContainer.clear();
		this.deps.editorContainer.addChild(editor);
		this.deps.ui.setFocus(editor);
		this.deps.ui.requestRender();
	}

	/**
	 * Show a selector for extensions.
	 */
	showSelector(title: string, options: string[], opts?: ExtensionUIDialogOptions): Promise<string | undefined> {
		return new Promise((resolve) => {
			if (opts?.signal?.aborted) {
				resolve(undefined);
				return;
			}

			const onAbort = () => {
				this.hideSelector();
				resolve(undefined);
			};
			opts?.signal?.addEventListener("abort", onAbort, { once: true });

			this.selector = new ExtensionSelectorComponent(
				title,
				options,
				(option) => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideSelector();
					resolve(option);
				},
				() => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideSelector();
					resolve(undefined);
				},
				{ tui: this.deps.ui, timeout: opts?.timeout },
			);

			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(this.selector);
			this.deps.ui.setFocus(this.selector);
			this.deps.ui.requestRender();
		});
	}

	/**
	 * Hide the extension selector.
	 */
	hideSelector(): void {
		this.selector?.dispose();
		this.selector = undefined;
		this.restoreEditor();
	}

	/**
	 * Show the options pane — the agent asking the user one or more questions.
	 * Resolves with one answer per question, or undefined if skipped/aborted.
	 */
	showAskOptions(questions: AskQuestion[], opts?: ExtensionUIDialogOptions): Promise<string[] | undefined> {
		return new Promise((resolve) => {
			if (!questions.length || opts?.signal?.aborted) {
				resolve(undefined);
				return;
			}

			const onAbort = () => {
				this.hideAskOptions();
				resolve(undefined);
			};
			opts?.signal?.addEventListener("abort", onAbort, { once: true });

			this.askOptionsPane = new AskOptionsComponent(
				questions,
				(answers) => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideAskOptions();
					resolve(answers);
				},
				() => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideAskOptions();
					resolve(undefined);
				},
			);

			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(this.askOptionsPane);
			this.deps.ui.setFocus(this.askOptionsPane);
			this.deps.ui.requestRender();
		});
	}

	/**
	 * Hide the options pane and restore the editor.
	 */
	hideAskOptions(): void {
		if (!this.askOptionsPane) return;
		this.askOptionsPane = undefined;
		this.restoreEditor();
	}

	/**
	 * Show a confirmation dialog for extensions.
	 */
	async confirm(title: string, message: string, opts?: ExtensionUIDialogOptions): Promise<boolean> {
		const result = await this.showSelector(`${title}\n${message}`, ["Yes", "No"], opts);
		return result === "Yes";
	}

	/**
	 * Show a text input for extensions.
	 */
	showInput(title: string, placeholder?: string, opts?: ExtensionUIDialogOptions): Promise<string | undefined> {
		return new Promise((resolve) => {
			if (opts?.signal?.aborted) {
				resolve(undefined);
				return;
			}

			const onAbort = () => {
				this.hideInput();
				resolve(undefined);
			};
			opts?.signal?.addEventListener("abort", onAbort, { once: true });

			this.input = new ExtensionInputComponent(
				title,
				placeholder,
				(value) => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideInput();
					resolve(value);
				},
				() => {
					opts?.signal?.removeEventListener("abort", onAbort);
					this.hideInput();
					resolve(undefined);
				},
				{ tui: this.deps.ui, timeout: opts?.timeout },
			);

			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(this.input);
			this.deps.ui.setFocus(this.input);
			this.deps.ui.requestRender();
		});
	}

	/**
	 * Hide the extension input.
	 */
	hideInput(): void {
		this.input?.dispose();
		this.input = undefined;
		this.restoreEditor();
	}

	/**
	 * Show a multi-line editor for extensions (with Ctrl+G support).
	 */
	showEditor(title: string, prefill?: string): Promise<string | undefined> {
		return new Promise((resolve) => {
			this.editorDialog = new ExtensionEditorComponent(
				this.deps.ui,
				this.deps.keybindings,
				title,
				prefill,
				(value) => {
					this.hideEditor();
					resolve(value);
				},
				() => {
					this.hideEditor();
					resolve(undefined);
				},
			);

			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(this.editorDialog);
			this.deps.ui.setFocus(this.editorDialog);
			this.deps.ui.requestRender();
		});
	}

	/**
	 * Hide the extension editor.
	 */
	hideEditor(): void {
		this.editorDialog = undefined;
		this.restoreEditor();
	}

	/** Show a custom component with keyboard focus. Overlay mode renders on top of existing content. */
	async showCustom<T>(
		factory: (
			tui: TUI,
			theme: Theme,
			keybindings: KeybindingsManager,
			done: (result: T) => void,
		) => (Component & { dispose?(): void }) | Promise<Component & { dispose?(): void }>,
		options?: {
			overlay?: boolean;
			overlayOptions?: OverlayOptions | (() => OverlayOptions);
			onHandle?: (handle: OverlayHandle) => void;
		},
	): Promise<T> {
		const savedText = this.deps.getEditor().getText();
		const isOverlay = options?.overlay ?? false;

		const restoreEditorWithText = () => {
			this.restoreEditor();
			this.deps.getEditor().setText(savedText);
		};

		return new Promise((resolve, reject) => {
			let component: Component & { dispose?(): void };
			let closed = false;

			const close = (result: T) => {
				if (closed) return;
				closed = true;
				if (isOverlay) this.deps.ui.hideOverlay();
				else restoreEditorWithText();
				// Note: both branches above already call requestRender
				resolve(result);
				try {
					component?.dispose?.();
				} catch {
					/* ignore dispose errors */
				}
			};

			Promise.resolve(factory(this.deps.ui, theme, this.deps.keybindings, close))
				.then((c) => {
					if (closed) return;
					component = c;
					if (isOverlay) {
						// Resolve overlay options - can be static or dynamic function
						const resolveOptions = (): OverlayOptions | undefined => {
							if (options?.overlayOptions) {
								const opts =
									typeof options.overlayOptions === "function"
										? options.overlayOptions()
										: options.overlayOptions;
								return opts;
							}
							// Fallback: use component's width property if available
							const w = (component as { width?: number }).width;
							return w ? { width: w } : undefined;
						};
						const handle = this.deps.ui.showOverlay(component, resolveOptions());
						// Expose handle to caller for visibility control
						options?.onHandle?.(handle);
					} else {
						this.deps.editorContainer.clear();
						this.deps.editorContainer.addChild(component);
						this.deps.ui.setFocus(component);
						this.deps.ui.requestRender();
					}
				})
				.catch((err) => {
					if (closed) return;
					if (!isOverlay) restoreEditorWithText();
					reject(err);
				});
		});
	}
}
