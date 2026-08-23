import { type Component, Container, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import type { ToolOutputView } from "../../../core/tool-output-view.js";
import { theme } from "../theme/theme.js";
import { type ChainState, chainPhrase, chainSegments, chainStats } from "./tool-chain-summary.js";
import type { ToolExecutionComponent } from "./tool-execution.js";

/**
 * A run of consecutive tool calls, rendered as one line in the radar view.
 *
 * The chain owns its blocks rather than sitting beside them, so there is one
 * place that decides whether you are looking at a summary or at the calls. In
 * every view but a collapsed radar it is a plain pass-through container and the
 * blocks render exactly as they always did.
 *
 * A chain closes when the agent next speaks, or when the turn settles. Closing
 * early matters more than it looks: rewriting a line that has scrolled above
 * the viewport forces the TUI into a full redraw, which clears the terminal's
 * scrollback. Chains that close while they are still the bottom of the screen
 * flip for the price of one line.
 */
export class ToolChainComponent extends Container {
	private blocks: ToolExecutionComponent[] = [];
	private view: ToolOutputView;
	private state: ChainState = "running";
	/** Opened into its per-call rows by `app.tools.unfoldOne`. */
	private opened = false;
	private memo?: { width: number; out: string[] };

	constructor(view: ToolOutputView) {
		super();
		this.view = view;
	}

	add(block: ToolExecutionComponent): void {
		this.blocks.push(block);
		this.addChild(block);
		this.memo = undefined;
	}

	/** True until the agent speaks or the turn settles. */
	get isOpen(): boolean {
		return this.state === "running";
	}

	get isOpened(): boolean {
		return this.opened;
	}

	get isEmpty(): boolean {
		return this.blocks.length === 0;
	}

	get toolBlocks(): readonly ToolExecutionComponent[] {
		return this.blocks;
	}

	/**
	 * Settle the chain.
	 *
	 * `interrupted` keeps the running rendering, because the settled phrase is a
	 * claim about what the run amounted to and a run cut off partway through has
	 * no such claim to make — the same reason an aborted turn's plan items settle
	 * to cancelled rather than done.
	 */
	close(outcome: "done" | "interrupted"): void {
		this.state = outcome;
		this.memo = undefined;
	}

	setView(view: ToolOutputView): void {
		this.view = view;
		this.memo = undefined;
		for (const block of this.blocks) block.setView(view);
	}

	/** The global expand key opens every block, which also opens the chain. */
	setExpanded(expanded: boolean): void {
		this.opened = expanded;
		this.memo = undefined;
		for (const block of this.blocks) block.setExpanded(expanded);
	}

	/** `alt+u` in radar: open this chain into its per-call rows, or fold it back. */
	setOpened(opened: boolean): void {
		this.opened = opened;
		this.memo = undefined;
	}

	override invalidate(): void {
		super.invalidate();
		this.memo = undefined;
	}

	/** Whether this chain is currently drawn as a single summary line. */
	private isCollapsed(): boolean {
		return this.view === "radar" && !this.opened && this.blocks.length > 0;
	}

	override render(width: number): string[] {
		if (!this.isCollapsed()) return super.render(width);
		if (this.memo && this.memo.width === width) return this.memo.out;

		const entries = this.blocks.map((block) => block.chainEntry());
		const running = this.state === "running";
		const glyph = running ? "◐" : "●";
		const glyphTone = entries.some((e) => e.isError) ? "error" : running ? "warning" : "success";

		// While it runs the chain shows its shape in order; once it is over, what
		// it amounted to. See tool-chain-summary.ts for why they differ.
		let leftPlain: string;
		let leftStyled: string;
		if (this.state === "done") {
			const phrase = chainPhrase(entries);
			leftPlain = phrase;
			leftStyled = theme.fg("toolTitle", phrase);
		} else {
			const segments = chainSegments(entries);
			const sep = " › ";
			leftPlain = segments.map((s) => s.label).join(sep) + (running ? "…" : "");
			leftStyled =
				segments
					.map((s) =>
						theme.fg(s.tone === "error" ? "error" : s.tone === "running" ? "warning" : "toolTitle", s.label),
					)
					.join(theme.fg("dim", sep)) + (running ? theme.fg("dim", "…") : "");
		}

		const stats = chainStats(entries, this.state);
		// " ● " + left, then stats flush right when there is room for both.
		const prefix = `${glyph} `;
		const budget = Math.max(0, width - 1 - visibleWidth(prefix));
		let line: string;
		if (visibleWidth(leftPlain) + 2 + visibleWidth(stats) <= budget) {
			const pad = " ".repeat(budget - visibleWidth(leftPlain) - visibleWidth(stats));
			line = ` ${theme.fg(glyphTone, prefix)}${leftStyled}${pad}${theme.fg("muted", stats)}`;
		} else {
			line = ` ${theme.fg(glyphTone, prefix)}${truncateToWidth(leftStyled, budget)}`;
		}

		const out = [line, ...this.failureLines(width)];
		this.memo = { width, out };
		return out;
	}

	/**
	 * Each failed call's reason, indented under the chain line.
	 *
	 * A collapsed chain hides its blocks, so without this a failure would be a
	 * count in the stats and nothing else. Whatever else these views fold away,
	 * they never fold away why something broke.
	 */
	private failureLines(width: number): string[] {
		const lines: string[] = [];
		for (const block of this.blocks) {
			const entry = block.chainEntry();
			if (!entry.isError) continue;
			const header = `   ${theme.fg("error", "✗")} ${theme.fg("toolTitle", entry.tool)}  ${theme.fg("toolOutput", entry.subject)}`;
			lines.push(truncateToWidth(header, width));
			for (const raw of block.errorText().split("\n")) {
				if (!raw.trim()) continue;
				lines.push(truncateToWidth(`     ${theme.fg("toolOutput", raw)}`, width));
			}
		}
		return lines;
	}
}

/** Narrow a transcript child to a chain. */
export function isToolChain(child: Component): child is ToolChainComponent {
	return child instanceof ToolChainComponent;
}
