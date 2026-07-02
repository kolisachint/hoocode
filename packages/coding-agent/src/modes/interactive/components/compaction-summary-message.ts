import type { CompactionSummaryMessage } from "@kolisachint/hoocode-agent-core";
import { Box, Markdown, type MarkdownTheme, Spacer, Text } from "@kolisachint/hoocode-tui";
import { getMarkdownTheme, theme } from "../theme/theme.js";
import { keyText } from "./keybinding-hints.js";

/**
 * Component that renders a compaction message with collapsed/expanded state.
 * Uses same background color as custom messages for visual consistency.
 */
export class CompactionSummaryMessageComponent extends Box {
	private expanded = false;
	private message: CompactionSummaryMessage;
	private markdownTheme: MarkdownTheme;

	constructor(message: CompactionSummaryMessage, markdownTheme: MarkdownTheme = getMarkdownTheme()) {
		super(1, 1, (t) => theme.bg("customMessageBg", t));
		this.message = message;
		this.markdownTheme = markdownTheme;
		this.updateDisplay();
	}

	setExpanded(expanded: boolean): void {
		this.expanded = expanded;
		this.updateDisplay();
	}

	override invalidate(): void {
		super.invalidate();
		this.updateDisplay();
	}

	private updateDisplay(): void {
		this.clear();

		const beforeStr = this.message.tokensBefore.toLocaleString();
		const after = this.message.tokensAfter;
		const summaryText =
			after !== undefined && this.message.tokensBefore > 0
				? `Compacted ${beforeStr} → ${after.toLocaleString()} tokens (saved ${formatSavings(this.message.tokensBefore, after)})`
				: `Compacted from ${beforeStr} tokens`;
		const label = theme.fg("customMessageLabel", `\x1b[1m[compaction]\x1b[22m`);
		this.addChild(new Text(label, 0, 0));
		this.addChild(new Spacer(1));

		if (this.expanded) {
			const header = `**${summaryText}**\n\n`;
			this.addChild(
				new Markdown(header + this.message.summary, 0, 0, this.markdownTheme, {
					color: (text: string) => theme.fg("customMessageText", text),
				}),
			);
		} else {
			this.addChild(
				new Text(
					theme.fg("customMessageText", `${summaryText} (`) +
						theme.fg("dim", keyText("app.tools.expand")) +
						theme.fg("customMessageText", " to expand)"),
					0,
					0,
				),
			);
		}
	}
}

function formatSavings(before: number, after: number): string {
	const saved = Math.max(0, before - after);
	const pct = Math.round((saved / before) * 100);
	return `${pct}%`;
}
