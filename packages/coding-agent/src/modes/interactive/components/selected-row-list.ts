import { type Component, truncateToWidth } from "@kolisachint/hoocode-tui";
import { paintSelectedRow } from "../theme/theme.js";

/**
 * The cursor every picker marks its selected row with. Pickers used to pick
 * their own — `→`, `›` and `>` were all in use — so which glyph you saw
 * depended on which list you had opened.
 */
export const SELECT_CURSOR = "→";

export interface SelectableRow {
	/** The row's already-styled content, without any left margin. */
	text: string;
	selected?: boolean;
}

/**
 * A block of picker rows rendered at terminal width, so the selected one can be
 * filled edge to edge.
 *
 * Pickers used to add one `Text` child per row from an update method that never
 * sees the width — width only arrives later, when the container renders each
 * child — which left no point at which a row could be padded before being
 * painted. Holding the rows and rendering them lazily puts the width back in
 * reach.
 *
 * The left margin is part of the row rather than component padding, so the
 * highlight starts at column 0 like the session picker's does.
 */
export class SelectedRowList implements Component {
	private rows: SelectableRow[];
	private readonly marginX: number;

	constructor(rows: SelectableRow[] = [], marginX: number = 0) {
		this.rows = rows;
		this.marginX = marginX;
	}

	setRows(rows: SelectableRow[]): void {
		this.rows = rows;
	}

	invalidate(): void {
		// No cached state to invalidate currently
	}

	render(width: number): string[] {
		const margin = " ".repeat(this.marginX);
		return this.rows.map((row) => {
			const line = truncateToWidth(margin + row.text, width);
			return row.selected ? paintSelectedRow(line, width) : line;
		});
	}
}
