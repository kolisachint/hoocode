/**
 * Generic selector component for extensions.
 * Displays a list of string options with keyboard navigation.
 */

import { Container, getKeybindings, type TUI } from "@kolisachint/hoocode-tui";
import { SELECT_CURSOR, SELECT_GUTTER, theme } from "../theme/theme.js";
import { CountdownTimer } from "./countdown-timer.js";
import { InputFrame } from "./input-frame.js";
import { keyHint, rawKeyHint } from "./keybinding-hints.js";
import { type SelectableRow, SelectedRowList } from "./selected-row-list.js";

export interface ExtensionSelectorOptions {
	tui?: TUI;
	timeout?: number;
}

export class ExtensionSelectorComponent extends InputFrame {
	private options: string[];
	private selectedIndex = 0;
	private listContainer: Container;
	private onSelectCallback: (option: string) => void;
	private onCancelCallback: () => void;
	private baseTitle: string;
	private countdown: CountdownTimer | undefined;

	constructor(
		title: string,
		options: string[],
		onSelect: (option: string) => void,
		onCancel: () => void,
		opts?: ExtensionSelectorOptions,
	) {
		super({ title });

		this.options = options;
		this.onSelectCallback = onSelect;
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

		this.listContainer = new Container();
		this.addChild(this.listContainer);
		this.setHint(
			rawKeyHint("↑↓", "navigate") +
				"  " +
				keyHint("tui.select.confirm", "select") +
				"  " +
				keyHint("tui.select.cancel", "cancel"),
		);

		this.updateList();
	}

	private updateList(): void {
		this.listContainer.clear();
		const rows: SelectableRow[] = this.options.map((option, i) => {
			const isSelected = i === this.selectedIndex;
			return {
				text: isSelected
					? theme.fg("accent", SELECT_CURSOR) + theme.fg("accent", option)
					: SELECT_GUTTER + theme.fg("text", option),
				selected: isSelected,
			};
		});
		this.listContainer.addChild(new SelectedRowList(rows, 1));
	}

	handleInput(keyData: string): void {
		const kb = getKeybindings();
		if (kb.matches(keyData, "tui.select.up") || keyData === "k") {
			this.selectedIndex = Math.max(0, this.selectedIndex - 1);
			this.updateList();
		} else if (kb.matches(keyData, "tui.select.down") || keyData === "j") {
			this.selectedIndex = Math.min(this.options.length - 1, this.selectedIndex + 1);
			this.updateList();
		} else if (kb.matches(keyData, "tui.select.confirm") || keyData === "\n") {
			const selected = this.options[this.selectedIndex];
			if (selected) this.onSelectCallback(selected);
		} else if (kb.matches(keyData, "tui.select.cancel")) {
			this.onCancelCallback();
		}
	}

	dispose(): void {
		this.countdown?.dispose();
	}
}
