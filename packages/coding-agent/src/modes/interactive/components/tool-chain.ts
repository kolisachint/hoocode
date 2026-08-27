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
	/** The newest run in the transcript; radar marks it. */
	private latest = false;
	private memo?: { width: number; out: string[] };
	/** Lead-in memo, keyed on the rows it wraps, so the array stays stable. */
	private leadIn?: { src: string[]; out: string[] };

	constructor(view: ToolOutputView) {
		super();
		this.view = view;
	}

	/** Every cache this chain keeps. The two go stale for the same reasons. */
	private forget(): void {
		this.memo = undefined;
		this.leadIn = undefined;
	}

	add(block: ToolExecutionComponent): void {
		this.blocks.push(block);
		this.addChild(block);
		this.forget();
	}

	/** True until the agent speaks or the turn settles. */
	get isOpen(): boolean {
		return this.state === "running";
	}

	get isOpened(): boolean {
		return this.opened;
	}

	/** Whether a summary line is standing in for this chain's calls right now. */
	get isSummarised(): boolean {
		return this.isCollapsed();
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
		this.forget();
	}

	setView(view: ToolOutputView): void {
		this.view = view;
		this.forget();
		for (const block of this.blocks) block.setView(view);
	}

	/** The global expand key opens every block, which also opens the chain. */
	setExpanded(expanded: boolean): void {
		this.opened = expanded;
		this.forget();
		for (const block of this.blocks) block.setExpanded(expanded);
	}

	/** `alt+u` in radar: open this chain into its per-call rows, or fold it back. */
	setOpened(opened: boolean): void {
		this.opened = opened;
		this.forget();
	}

	/**
	 * Mark this run as the newest in the transcript, or no longer it.
	 *
	 * The per-call rows carry the same mark, but they are not what radar usually
	 * shows: a run of more than one call folds to this single line, so without
	 * marking the line too the stroke would be invisible in the view it was
	 * built for.
	 */
	setLatest(latest: boolean): void {
		if (this.latest === latest) return;
		this.latest = latest;
		this.forget();
	}

	override invalidate(): void {
		super.invalidate();
		this.forget();
	}

	/**
	 * Whether this chain is currently drawn as a single summary line.
	 *
	 * A chain of one is not summarised. Its phrase would be `Ran npm run check`,
	 * which is strictly less than the radar row it replaced: the row names the
	 * tool and how much came back, and the phrase drops both to say the same
	 * thing in prose. Summarising is only worth a lossy rewrite when there is
	 * more than one call to fold — and by measurement most chains have exactly
	 * one, so this is the common case, not an edge.
	 */
	private isCollapsed(): boolean {
		return this.view === "radar" && !this.opened && this.blocks.length > 1;
	}

	/**
	 * The blank row that holds a run off whatever came before it.
	 *
	 * Radar's rows stack without gaps on purpose, and that is right *between*
	 * rows — but it left the first row of a run pressed against the prose that
	 * introduced it, with the turn above and the run below reading as one
	 * paragraph. The gap belongs to the run rather than to its rows: one blank
	 * line at the top of the chain, and the rows go on stacking underneath it.
	 *
	 * Only radar needs it. Every other view gives each block a leading spacer of
	 * its own, and so does a radar block that has been opened, so asking the
	 * first block whether it draws one keeps the two from doubling up.
	 */
	private needsLeadIn(): boolean {
		if (this.view !== "radar") return false;
		if (this.isCollapsed()) return true;
		return this.blocks[0]?.drawsLeadingGap() === false;
	}

	override render(width: number): string[] {
		const rows = this.rows(width);
		if (rows.length === 0 || !this.needsLeadIn()) return rows;
		// Keep the wrapped array reference-stable: the TUI diffs whole subtrees
		// by identity, and a fresh array every frame would defeat that.
		if (this.leadIn?.src === rows) return this.leadIn.out;
		const out = ["", ...rows];
		this.leadIn = { src: rows, out };
		return out;
	}

	private rows(width: number): string[] {
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
		// The marker stroke runs over what you read and stops before the stats,
		// the same shape it takes on a per-call row.
		const stroke = this.latest && theme.hasBg("activeToolBg");
		const mark = (text: string) => (stroke ? theme.bg("activeToolBg", text) : text);
		// " ● " + left, then stats flush right when there is room for both.
		const prefix = `${glyph} `;
		const budget = Math.max(0, width - 1 - visibleWidth(prefix));
		let line: string;
		if (visibleWidth(leftPlain) + 2 + visibleWidth(stats) <= budget) {
			const pad = " ".repeat(budget - visibleWidth(leftPlain) - visibleWidth(stats));
			line = ` ${theme.fg(glyphTone, prefix)}${mark(leftStyled)}${pad}${theme.fg("muted", stats)}`;
		} else {
			line = ` ${theme.fg(glyphTone, prefix)}${mark(truncateToWidth(leftStyled, budget))}`;
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
