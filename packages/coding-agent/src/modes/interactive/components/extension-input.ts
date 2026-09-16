/**
 * Simple text input component for extensions.
 */

import { type Focusable, getKeybindings, Input, type TUI } from "@kolisachint/hoocode-tui";
import { CountdownTimer } from "./countdown-timer.js";
import { InputFrame } from "./input-frame.js";
import { keyHint } from "./keybinding-hints.js";

export interface ExtensionInputOptions {
	tui?: TUI;
	timeout?: number;
}

export class ExtensionInputComponent extends InputFrame implements Focusable {
	private input: Input;
	private onSubmitCallback: (value: string) => void;
	private onCancelCallback: () => void;
	private baseTitle: string;
	private countdown: CountdownTimer | undefined;

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
		title: string,
		_placeholder: string | undefined,
		onSubmit: (value: string) => void,
		onCancel: () => void,
		opts?: ExtensionInputOptions,
	) {
		super({ title });

		this.onSubmitCallback = onSubmit;
		this.onCancelCallback = onCancel;
		this.baseTitle = title;

		if (opts?.timeout && opts.timeout > 0 && opts.tui) {
			// The countdown rides the title in the border, where the title now is.
			this.countdown = new CountdownTimer(
				opts.timeout,
				opts.tui,
				(s) => this.setTitle(`${this.baseTitle} (${s}s)`),
				() => this.onCancelCallback(),
			);
		}

		this.input = new Input();
		this.addChild(this.input);
		this.setHint(`${keyHint("tui.select.confirm", "submit")}  ${keyHint("tui.select.cancel", "cancel")}`);
	}

	handleInput(keyData: string): void {
		const kb = getKeybindings();
		if (kb.matches(keyData, "tui.select.confirm") || keyData === "\n") {
			this.onSubmitCallback(this.input.getValue());
		} else if (kb.matches(keyData, "tui.select.cancel")) {
			this.onCancelCallback();
		} else {
			this.input.handleInput(keyData);
		}
	}

	dispose(): void {
		this.countdown?.dispose();
	}
}
