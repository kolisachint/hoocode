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
 *
 * Approval gates: task_* lifecycle events render as stream lines, and when the
 * attached role pauses, presentApproval() embeds the AskOptions pane right
 * where the stream stopped — pick an option (or type a free-form answer) and
 * the caller answers the server; the stream then carries on under a
 * "✓ answered: …" stamp.
 */

import type { Component, Focusable, TUI } from "@kolisachint/hoocode-tui";
import { getKeybindings, matchesKey, truncateToWidth, visibleWidth, wrapTextWithAnsi } from "@kolisachint/hoocode-tui";
import type { AskQuestion } from "../../../core/extensions/types.js";
import type { TeamApproval } from "../../../core/team-approvals.js";
import type { TeamViewConnection, TeamViewEvent } from "../../../core/team-view.js";
import { theme } from "../theme/theme.js";
import { AskOptionsComponent } from "./ask-options.js";
import { appKeyLabel, matchesAppKey, rawKeyHint } from "./keybinding-hints.js";

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
	/** Gate currently embedded in the panel; input is delegated to it. */
	private approval: { component: AskOptionsComponent; settle: (answer: string | undefined) => void } | undefined;

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

	/** Detach from the shared event stream; settles any open gate as skipped. Idempotent. */
	dispose(): void {
		this.unsubscribe?.();
		this.unsubscribe = undefined;
		this.approval?.settle(undefined);
	}

	invalidate(): void {
		// No cached rendering state.
	}

	/**
	 * Embed one approval gate in the panel (the attached role paused). Resolves
	 * with the chosen or free-form answer; undefined when skipped (esc), when
	 * the signal aborts (answered elsewhere), or when the panel is disposed.
	 * Answering the server is the caller's job — on an answer the panel just
	 * stamps "✓ answered: …" into the stream.
	 */
	presentApproval(approval: TeamApproval, signal: AbortSignal): Promise<string | undefined> {
		return new Promise((resolve) => {
			if (signal.aborted || this.unsubscribe === undefined) {
				resolve(undefined);
				return;
			}
			// The coordinator shows one gate at a time; a stray second call
			// settles the first as skipped instead of stacking panes.
			this.approval?.settle(undefined);

			let settled = false;
			const settle = (answer: string | undefined): void => {
				if (settled) return;
				settled = true;
				signal.removeEventListener("abort", onAbort);
				this.approval = undefined;
				if (answer !== undefined) {
					this.breakLine();
					this.lines.push(theme.fg("success", `  ✓ answered: ${answer}`));
				}
				this.ui?.requestRender();
				resolve(answer);
			};
			const onAbort = (): void => settle(undefined);
			signal.addEventListener("abort", onAbort, { once: true });

			const question: AskQuestion = {
				question: approval.question,
				short: approval.taskId,
				detail: `team task "${approval.taskId}" is paused until answered`,
				options: approval.options.map((label) => ({ label })),
				allowCustom: true,
			};
			const component = new AskOptionsComponent(
				[question],
				(answers) => settle(answers[0]),
				() => settle(undefined),
			);
			component.focused = this.focused;
			this.approval = { component, settle };
			this.ui?.requestRender();
		});
	}

	handleInput(data: string): void {
		// An open gate owns the keyboard: q/n must type into the custom row, not
		// detach or nudge. esc skips the gate (AskOptions cancel), not the panel.
		if (this.approval) {
			this.approval.component.handleInput(data);
			return;
		}
		if (matchesKey(data, "q") || getKeybindings().matches(data, "tui.select.cancel")) {
			this.callbacks.onDetach();
			return;
		}
		if (matchesAppKey(data, "app.team.nudge")) {
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
			case "task_started":
				this.breakLine();
				this.lines.push(theme.fg("accent", `◉ task ${event.taskId ?? "?"} started`));
				break;
			case "task_paused":
				this.breakLine();
				// VS15 (U+FE0E) forces text presentation: bare ⏸/▶ carry the Unicode
				// Emoji property and emoji-font fallback renders them double-width,
				// breaking the width math that counts one cell.
				this.lines.push(theme.fg("warning", `⏸︎ awaiting approval: ${event.question ?? "?"}`));
				break;
			case "task_resumed":
				this.breakLine();
				this.lines.push(
					theme.fg(
						"accent",
						`▶︎ task ${event.taskId ?? "?"} resumed${event.chosenOption ? `: ${event.chosenOption}` : ""}`,
					),
				);
				break;
			case "task_finished":
				this.breakLine();
				this.lines.push(
					event.status === "error"
						? theme.fg("error", `✗ task ${event.taskId ?? "?"} failed`)
						: theme.fg("success", `✓ task ${event.taskId ?? "?"} done`),
				);
				break;
		}
	}

	/** Buffered logical line count (completed lines only). Exposed for tests. */
	bufferedLineCount(): number {
		return this.lines.length;
	}

	render(width: number): string[] {
		const inner = Math.max(1, width);

		// Header: role identity left, key hints right. An open gate owns the
		// keyboard and renders its own hints, so the panel's would lie.
		const titlePlain = `◉ ${this.role} — attached`;
		const title =
			theme.fg("accent", "◉ ") + theme.bold(theme.fg("accent", this.role)) + theme.fg("muted", " — attached");
		// House hint style: dim key + muted description, muted · separator. The
		// nudge key resolves from the live keybinding config (same binding the
		// task panel's team focus uses); q stays a literal by convention.
		const nudgeKey = appKeyLabel("app.team.nudge");
		const hintsPlain = this.approval ? "" : `${nudgeKey} nudge · q detach`;
		const hints = this.approval
			? ""
			: rawKeyHint(nudgeKey, "nudge") + theme.fg("muted", " · ") + rawKeyHint("q", "detach");
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

		// An open gate takes its rows out of the stream's budget: the freshest
		// output stays visible above the question for context.
		let approvalLines: string[] = [];
		if (this.approval) {
			this.approval.component.focused = this.focused;
			approvalLines = this.approval.component.render(inner);
		}

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
		const keep = Math.max(0, bodyBudget - approvalLines.length);
		const body = keep > 0 ? wrapped.slice(-keep) : [];
		if (body.length === 0 && approvalLines.length === 0) {
			body.push(theme.fg("dim", "waiting for events…"));
		}

		// The gate brings its own accent rules, so it closes the panel itself.
		if (approvalLines.length > 0) return [header, rule, ...body, ...approvalLines];
		return [header, rule, ...body, rule];
	}
}
