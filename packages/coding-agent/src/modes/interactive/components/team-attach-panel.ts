/**
 * Attach side-panel for a hooteams role (`a` in team focus).
 *
 * Renders one role's live TeamEvents in the style of hooteams' StreamRenderer
 * (`hooteams attach <role>`): ◉ lifecycle lines, dim italic thinking, inline
 * streaming text, ✓/✗ tool results, dim per-turn usage stamps. Differences
 * from the CLI renderer are dictated by the host: colors go through hoocode's
 * theme helpers (no raw ANSI) and output lands in a bounded ring buffer
 * instead of an unbounded stdout stream.
 *
 * The panel filters the team connection's single shared /events subscription —
 * it never opens its own SSE connection — and unsubscribes on dispose(), so
 * attach/detach cycles leave no leaked subscribers.
 */

import type { Component, Focusable, TUI } from "@kolisachint/hoocode-tui";
import { getKeybindings, matchesKey, truncateToWidth, visibleWidth, wrapTextWithAnsi } from "@kolisachint/hoocode-tui";
import type { TeamViewConnection, TeamViewEvent } from "../../../core/team-view.js";
import { theme } from "../theme/theme.js";

/** Fixed-capacity FIFO over a circular array: push evicts the oldest entry. */
export class RingBuffer<T> {
	private readonly slots: (T | undefined)[];
	private start = 0;
	private count = 0;

	constructor(readonly capacity: number) {
		if (!Number.isInteger(capacity) || capacity <= 0) throw new Error(`invalid ring buffer capacity ${capacity}`);
		this.slots = new Array<T | undefined>(capacity);
	}

	push(item: T): void {
		const end = (this.start + this.count) % this.capacity;
		this.slots[end] = item;
		if (this.count < this.capacity) this.count++;
		else this.start = (this.start + 1) % this.capacity;
	}

	get length(): number {
		return this.count;
	}

	toArray(): T[] {
		const out: T[] = [];
		for (let i = 0; i < this.count; i++) {
			out.push(this.slots[(this.start + i) % this.capacity] as T);
		}
		return out;
	}
}

function argsPreview(args: unknown, max = 60): string {
	const text = JSON.stringify(args) ?? "";
	return text.length > max ? `${text.slice(0, max)}…` : text;
}

export interface TeamAttachPanelCallbacks {
	/** `q`/esc: close the panel; the role keeps running. */
	onDetach: () => void;
	/** `n`: nudge the attached role. */
	onNudge: (role: string) => void;
}

/** Logical event lines kept in the buffer (wrapped to width at render time). */
const DEFAULT_BUFFER_LINES = 200;

export class TeamAttachPanelComponent implements Component, Focusable {
	focused = false;

	private readonly lines: RingBuffer<string>;
	/** Streaming tail (text/thinking deltas) not yet terminated by a line break. */
	private partial = "";
	private partialKind: "text" | "thinking" = "text";
	private unsubscribe: (() => void) | undefined;

	constructor(
		readonly role: string,
		connection: TeamViewConnection,
		private readonly callbacks: TeamAttachPanelCallbacks,
		private readonly ui?: TUI,
		bufferLines = DEFAULT_BUFFER_LINES,
	) {
		this.lines = new RingBuffer(bufferLines);
		// Filter the shared stream down to this role; no second SSE connection.
		this.unsubscribe = connection.subscribe((event) => {
			if (event.role !== this.role) return;
			this.applyEvent(event);
			this.ui?.requestRender();
		});
	}

	/** Detach from the shared event stream. Idempotent. */
	dispose(): void {
		this.unsubscribe?.();
		this.unsubscribe = undefined;
	}

	invalidate(): void {
		// No cached rendering state.
	}

	handleInput(data: string): void {
		if (matchesKey(data, "q") || getKeybindings().matches(data, "tui.select.cancel")) {
			this.callbacks.onDetach();
			return;
		}
		if (matchesKey(data, "n")) {
			this.callbacks.onNudge(this.role);
		}
	}

	private styledPartial(): string {
		return this.partialKind === "thinking" ? theme.italic(theme.fg("dim", this.partial)) : this.partial;
	}

	/** Flush the streaming tail into the buffer as a finished line. */
	private breakLine(): void {
		if (this.partial.length === 0) return;
		this.lines.push(this.styledPartial());
		this.partial = "";
	}

	/** Append streaming delta text, completing buffer lines at embedded newlines. */
	private appendDelta(delta: string, kind: "text" | "thinking"): void {
		if (this.partialKind !== kind) this.breakLine();
		this.partialKind = kind;
		const parts = delta.split("\n");
		for (let i = 0; i < parts.length - 1; i++) {
			this.partial += parts[i];
			this.breakLine();
			this.partialKind = kind;
		}
		this.partial += parts[parts.length - 1];
	}

	/** Mirrors hooteams' StreamRenderer event handling, themed and buffered. */
	private applyEvent(event: TeamViewEvent): void {
		switch (event.type) {
			case "agent_start":
				this.breakLine();
				this.lines.push(theme.fg("accent", `◉ ${event.role} started`));
				break;
			case "message_update": {
				const delta = event.assistantMessageEvent;
				if (!delta) break;
				if (delta.type === "thinking_start") {
					this.breakLine();
					this.lines.push(theme.fg("dim", "◉ thinking…"));
				} else if (delta.type === "thinking_delta") {
					this.appendDelta(delta.delta ?? "", "thinking");
				} else if (delta.type === "thinking_end") {
					this.breakLine();
				} else if (delta.type === "text_delta") {
					this.appendDelta(delta.delta ?? "", "text");
				} else if (delta.type === "text_end") {
					this.breakLine();
				}
				break;
			}
			case "tool_execution_start":
				this.breakLine();
				this.lines.push(
					theme.fg("accent", "◉ tool: ") +
						theme.bold(theme.fg("accent", event.toolName ?? "?")) +
						theme.fg("accent", `(${argsPreview(event.args)})`) +
						" " +
						theme.fg("warning", "running…"),
				);
				break;
			case "tool_execution_end":
				this.breakLine();
				this.lines.push(
					event.isError
						? theme.fg("error", `  ✗ ${event.toolName ?? "?"} failed`)
						: theme.fg("success", `  ✓ ${event.toolName ?? "?"} done`),
				);
				break;
			case "turn_end": {
				this.breakLine();
				const usage = event.message?.usage;
				if (usage) {
					const cost = usage.cost?.total ? ` $${usage.cost.total.toFixed(4)}` : "";
					this.lines.push(
						theme.fg("dim", `— turn: ${usage.input ?? 0} in / ${usage.output ?? 0} out tokens${cost}`),
					);
				}
				if (event.message?.errorMessage) {
					this.lines.push(theme.fg("error", `error: ${event.message.errorMessage}`));
				}
				break;
			}
			case "agent_end":
				this.breakLine();
				this.lines.push(theme.fg("accent", `◉ ${event.role} idle`));
				break;
		}
	}

	/** Buffered logical line count (completed lines only). Exposed for tests. */
	bufferedLineCount(): number {
		return this.lines.length;
	}

	render(width: number): string[] {
		const inner = Math.max(1, width);

		// Header: role identity left, key hints right.
		const titlePlain = `◉ ${this.role} — attached`;
		const title =
			theme.fg("accent", "◉ ") + theme.bold(theme.fg("accent", this.role)) + theme.fg("muted", " — attached");
		const hintsPlain = "n nudge · q detach";
		const hints = theme.fg("dim", hintsPlain);
		let header: string;
		if (visibleWidth(titlePlain) + 2 + visibleWidth(hintsPlain) <= inner) {
			header = title + " ".repeat(inner - visibleWidth(titlePlain) - visibleWidth(hintsPlain)) + hints;
		} else {
			header = truncateToWidth(title, inner, "…");
		}
		const rule = theme.fg("borderMuted", "─".repeat(inner));

		// Body: wrap each buffered line, then keep the newest rows that fit the
		// panel's height budget (the freshest output hugs the bottom, like a
		// terminal tail). Budget derives from the live terminal height so the
		// panel never asks the overlay compositor to clip it (clipping drops the
		// bottom — exactly the rows we care about).
		const rows = this.ui?.terminal.rows ?? 24;
		const bodyBudget = Math.max(5, Math.floor(rows * 0.6) - 3);
		const wrapped: string[] = [];
		for (const line of this.lines.toArray()) {
			if (line.length === 0) {
				wrapped.push("");
				continue;
			}
			wrapped.push(...wrapTextWithAnsi(line, inner));
		}
		if (this.partial.length > 0) {
			wrapped.push(...wrapTextWithAnsi(this.styledPartial(), inner));
		}
		const body = wrapped.slice(-bodyBudget);
		if (body.length === 0) {
			body.push(theme.fg("dim", "waiting for events…"));
		}

		return [header, rule, ...body, rule];
	}
}
