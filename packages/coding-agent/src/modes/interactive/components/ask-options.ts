/**
 * Options pane — the agent asking the USER for a decision before it acts.
 *
 * Rendered inline in the transcript (never a modal), in the same boxless
 * language as the other selectors: blue rules top & bottom, a cyan
 * "INPUT NEEDED" label, and the active row marked with the accent cursor.
 *
 * One decision per step:
 *   - tui.select.up / down  move between options (wraps)
 *   - tui.select.next (→)   confirm the highlighted option and advance; on the
 *                           last step it submits every answer
 *   - tui.select.back (←)   step back to the previous decision
 *   - 1-9                   quick-pick an option
 *   - drop onto the custom row and type a free-form answer when none fit
 *   - tui.select.cancel (esc) skips the whole sequence
 *
 * Answered steps stay on screen as a deterministic breadcrumb so you always
 * see what you've already committed to.
 */

import {
	type Component,
	type Focusable,
	getKeybindings,
	Input,
	type Keybinding,
	truncateToWidth,
	visibleWidth,
} from "@kolisachint/hoocode-tui";
import type { AskQuestion } from "../../../core/extensions/types.js";
import { theme } from "../theme/theme.js";

export class AskOptionsComponent implements Component, Focusable {
	private questions: AskQuestion[];
	private step = 0;
	private index = 0;
	private answers: string[] = [];
	private customInput = new Input();
	private onSubmitCallback: (answers: string[]) => void;
	private onCancelCallback: () => void;

	private _focused = false;
	get focused(): boolean {
		return this._focused;
	}
	set focused(value: boolean) {
		this._focused = value;
		this.customInput.focused = value;
	}

	constructor(questions: AskQuestion[], onSubmit: (answers: string[]) => void, onCancel: () => void) {
		this.questions = questions;
		this.onSubmitCallback = onSubmit;
		this.onCancelCallback = onCancel;
	}

	invalidate(): void {}

	private rowCount(q: AskQuestion): number {
		return q.options.length + (q.allowCustom ? 1 : 0);
	}

	private isOnCustomRow(q: AskQuestion): boolean {
		return !!q.allowCustom && this.index === q.options.length;
	}

	private spread(left: string, right: string, width: number): string {
		const gap = Math.max(1, width - visibleWidth(left) - visibleWidth(right));
		return truncateToWidth(`${left}${" ".repeat(gap)}${right}`, width, "");
	}

	private renderHints(last: boolean): string {
		const kb = getKeybindings();
		const sep = theme.fg("muted", " · ");
		const hint = (keys: string, desc: string) => theme.fg("dim", keys) + theme.fg("muted", ` ${desc}`);
		const upDown = `${kb.getKeys("tui.select.up").join("/")}/${kb.getKeys("tui.select.down").join("/")}`;
		const parts = [
			hint(upDown, "move"),
			hint(kb.getKeys("app.options.next" as Keybinding).join("/"), last ? "submit" : "next"),
		];
		if (this.step > 0) {
			parts.push(hint(kb.getKeys("app.options.back" as Keybinding).join("/"), "back"));
		}
		parts.push(hint(kb.getKeys("tui.select.cancel").join("/"), "skip"));
		return parts.join(sep);
	}

	render(width: number): string[] {
		const lines: string[] = [];
		const rule = theme.fg("borderAccent", "─".repeat(Math.max(1, width)));
		const cur = this.questions[this.step];
		const last = this.step === this.questions.length - 1;

		lines.push(rule);

		// Header: "INPUT NEEDED" on the left, key hints on the right.
		const title = theme.bold(theme.fg("borderAccent", "INPUT NEEDED"));
		lines.push(this.spread(title, this.renderHints(last), width));

		// Answered-step breadcrumb.
		for (let i = 0; i < this.step; i++) {
			const q = this.questions[i];
			const check = theme.fg("success", "✓");
			const label = theme.fg("dim", q.short ?? q.question);
			const arrow = theme.fg("dim", "→ ");
			const answer = theme.fg("accent", this.answers[i] ?? "");
			lines.push(truncateToWidth(`${check} ${label}   ${arrow}${answer}`, width, "..."));
		}
		if (this.step > 0) lines.push("");

		// Current question.
		const stepLabel = theme.fg("dim", `${this.step + 1}/${this.questions.length}`);
		lines.push(truncateToWidth(`${stepLabel} ${theme.bold(cur.question)}`, width, "..."));
		if (cur.detail) lines.push(truncateToWidth(theme.fg("muted", `    ${cur.detail}`), width, "..."));
		lines.push("");

		// Options.
		for (let i = 0; i < cur.options.length; i++) {
			const o = cur.options[i];
			const active = i === this.index;
			const cursor = theme.fg("accent", active ? ">" : " ");
			const num = theme.fg(active ? "accent" : "dim", String(i + 1));
			const label = active ? theme.bold(o.label) : o.label;
			let line = `${cursor} ${num} ${label}`;
			if (o.recommended) line += theme.fg("success", " (recommended)");
			if (o.description) line += theme.fg("muted", `   ${o.description}`);
			lines.push(truncateToWidth(line, width, "..."));
		}

		// Custom row.
		if (cur.allowCustom) {
			const customIndex = cur.options.length;
			const active = this.index === customIndex;
			const cursor = theme.fg("accent", active ? ">" : " ");
			const plus = theme.fg(active ? "accent" : "dim", "+");
			if (active) {
				const value = this.customInput.getValue();
				const prompt = theme.fg("accent", ">");
				const caret = this._focused ? theme.fg("accent", "▏") : "";
				const body = value
					? `${theme.fg("text", value)}${caret}`
					: `${caret}${theme.fg("dim", "type your own answer")}`;
				lines.push(truncateToWidth(`${cursor} ${plus} ${prompt} ${body}`, width, "..."));
			} else {
				const label = theme.fg("muted", "custom answer");
				const desc = theme.fg("dim", "type your own");
				lines.push(truncateToWidth(`${cursor} ${plus} ${label}   ${desc}`, width, "..."));
			}
		}

		// Count.
		lines.push(theme.fg("dim", `(${this.index + 1}/${this.rowCount(cur)})`));
		lines.push(rule);
		return lines;
	}

	private confirm(): void {
		const cur = this.questions[this.step];
		let value: string;
		if (this.isOnCustomRow(cur)) {
			value = this.customInput.getValue().trim();
			if (!value) return; // can't submit an empty custom answer
		} else {
			value = cur.options[this.index].label;
		}
		this.answers[this.step] = value;
		if (this.step < this.questions.length - 1) {
			this.step += 1;
			this.index = 0;
			this.customInput.setValue("");
		} else {
			this.onSubmitCallback(this.answers.slice());
		}
	}

	private back(): void {
		if (this.step === 0) return;
		this.step -= 1;
		this.index = 0;
		this.customInput.setValue("");
	}

	handleInput(data: string): void {
		const kb = getKeybindings();
		const cur = this.questions[this.step];
		const rows = this.rowCount(cur);
		const onCustom = this.isOnCustomRow(cur);

		if (kb.matches(data, "tui.select.up")) {
			this.index = this.index === 0 ? rows - 1 : this.index - 1;
			return;
		}
		if (kb.matches(data, "tui.select.down")) {
			this.index = this.index === rows - 1 ? 0 : this.index + 1;
			return;
		}
		if (kb.matches(data, "app.options.back" as Keybinding)) {
			this.back();
			return;
		}
		if (kb.matches(data, "app.options.next" as Keybinding) || kb.matches(data, "tui.select.confirm")) {
			this.confirm();
			return;
		}
		if (kb.matches(data, "tui.select.cancel")) {
			this.onCancelCallback();
			return;
		}

		if (onCustom) {
			// Delegate free-form typing (insert, backspace, paste, word-delete)
			// to a hidden Input buffer; the value is rendered inline above.
			this.customInput.handleInput(data);
			return;
		}

		// Number quick-pick for the listed options (not the custom row).
		if (/^[1-9]$/.test(data)) {
			const n = Number(data) - 1;
			if (n < cur.options.length) {
				this.index = n;
				this.confirm();
			}
		}
	}
}
