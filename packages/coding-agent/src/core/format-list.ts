/**
 * One row shape for every capability listing.
 *
 * Plugins, marketplaces, agents and skills are listed in at least five places —
 * the `/plugin` commands, the startup resource summary, and the model-facing
 * `SearchPlugins`/`ListPlugins` tools. Each had grown its own `name [a, b] — desc`
 * variant, so the same plugin printed three different ways depending on which
 * surface you asked. This module owns the layout so they cannot drift again.
 *
 * Two constraints shape the API:
 *
 * 1. **Styling is injected, never assumed.** The same rows are rendered as plain
 *    text (tool results the model reads, RPC payloads a client formats itself)
 *    and as themed terminal output. Callers pass a {@link ListStyle}; the default
 *    is identity, so the plain path costs nothing and can never emit an escape
 *    code into a protocol message.
 * 2. **Width is optional.** Only the interactive surface knows the terminal
 *    width. With `columns` set, detail text is wrapped and indented under its
 *    row; without it, detail is emitted unwrapped and the consumer wraps. The
 *    TUI's own wrapper has no hanging indent, which is what made the old
 *    listings unreadable — a wrapped description restarted at column 0 and read
 *    as a new entry.
 */

import { visibleWidth } from "@kolisachint/hoocode-tui";
import { SEGMENT_SEP } from "./brand.js";

/** `1 plugin` / `2 plugins`, so listings stop printing `plugin(s)`. */
export function plural(count: number, word: string, pluralForm?: string): string {
	return `${count} ${count === 1 ? word : (pluralForm ?? `${word}s`)}`;
}

/** Pad to `width` display columns, measuring ANSI-aware. */
export function padCell(text: string, width: number): string {
	return text + " ".repeat(Math.max(0, width - visibleWidth(text)));
}

/**
 * Truncate to `maxWidth` display columns.
 *
 * Deliberately not the TUI's `truncateToWidth`, which wraps its result in
 * `\x1b[0m` even for plain input. That would put escape codes into text the
 * model and RPC clients read, and a *full* reset at that — dropping any style
 * the caller had applied around it, mid-line.
 */
export function truncateVisible(text: string, maxWidth: number, ellipsis = "…"): string {
	if (maxWidth <= 0) return "";
	if (visibleWidth(text) <= maxWidth) return text;

	const budget = maxWidth - visibleWidth(ellipsis);
	if (budget <= 0) return ellipsis;

	let out = "";
	let width = 0;
	for (const char of text) {
		const charWidth = visibleWidth(char);
		if (width + charWidth > budget) break;
		out += char;
		width += charWidth;
	}
	return `${out}${ellipsis}`;
}

/**
 * Wrap `text` to `width`, indenting every line by `indent` spaces — including
 * the continuations, which is the whole point (see the module note).
 */
export function wrapIndented(text: string, indent: number, width?: number): string[] {
	const prefix = " ".repeat(indent);
	if (width === undefined) return [`${prefix}${text}`];

	const budget = Math.max(8, width - indent);
	const words = text.split(/\s+/).filter((w) => w.length > 0);
	if (words.length === 0) return [];

	const lines: string[] = [];
	let current = "";
	for (const word of words) {
		if (current === "") {
			current = word;
		} else if (visibleWidth(current) + 1 + visibleWidth(word) <= budget) {
			current += ` ${word}`;
		} else {
			lines.push(`${prefix}${current}`);
			current = word;
		}
	}
	if (current) lines.push(`${prefix}${current}`);
	return lines;
}

/** A single listed capability. */
export interface ListRow {
	/** Primary identifier. Every row in the listing aligns on this column. */
	name: string;
	/** Marker before the name — e.g. an installed tick. Counted in the column width. */
	marker?: string;
	/** Short facts beside the name (platforms, capabilities, source kind). */
	facts?: string[];
	/** Free text on its own wrapped, indented line(s) below the row. */
	detail?: string;
	/** Extra indented line below the detail — provenance, a path, a URL. */
	trailer?: string;
}

/** Rows under an optional heading (a marketplace, a scope). */
export interface ListGroup {
	title?: string;
	rows: ListRow[];
}

/**
 * Styling hooks. Every one defaults to identity so the plain-text path emits no
 * escape codes at all — required for tool results and RPC payloads.
 */
export interface ListStyle {
	name?(text: string): string;
	marker?(text: string): string;
	facts?(text: string): string;
	detail?(text: string): string;
	trailer?(text: string): string;
	groupTitle?(text: string): string;
}

export interface RenderListOptions {
	/** Terminal width, when the surface knows it. Enables wrapping and truncation. */
	columns?: number;
	/** Columns of indent applied to group titles. Rows sit two further in. */
	indent?: number;
	style?: ListStyle;
	/** Separator between facts. Defaults to the shared segment dot. */
	factSeparator?: string;
}

const identity = (text: string) => text;

/**
 * Render grouped rows as aligned text.
 *
 * The name column is measured across *every* group, not per group, so the
 * listing reads as one table rather than a stack of independently ragged ones.
 */
export function renderList(groups: ListGroup[], options: RenderListOptions = {}): string {
	const style = options.style ?? {};
	const styleName = style.name ?? identity;
	const styleMarker = style.marker ?? identity;
	const styleFacts = style.facts ?? identity;
	const styleDetail = style.detail ?? identity;
	const styleTrailer = style.trailer ?? identity;
	const styleGroupTitle = style.groupTitle ?? identity;
	const factSeparator = options.factSeparator ?? ` ${SEGMENT_SEP} `;

	const baseIndent = options.indent ?? 0;
	const hasTitles = groups.some((g) => g.title !== undefined);
	const rowIndent = baseIndent + (hasTitles ? 2 : 0);
	const detailIndent = rowIndent + 4;

	const allRows = groups.flatMap((g) => g.rows);
	if (allRows.length === 0) return "";

	const anyMarker = allRows.some((r) => r.marker);
	const markerWidth = anyMarker ? Math.max(...allRows.map((r) => visibleWidth(r.marker ?? ""))) + 1 : 0;
	const nameWidth = Math.max(...allRows.map((r) => visibleWidth(r.name))) + markerWidth;

	const lines: string[] = [];
	for (const group of groups) {
		if (group.title !== undefined) {
			lines.push(`${" ".repeat(baseIndent)}${styleGroupTitle(group.title)}`);
		}
		for (const row of group.rows) {
			// Widths are measured on the plain text and the padding is emitted
			// separately, so styling can never be counted as visible cells.
			const markerPlain = row.marker ?? "";
			const markerCell = anyMarker
				? (row.marker ? styleMarker(row.marker) : "") + " ".repeat(markerWidth - visibleWidth(markerPlain))
				: "";
			const namePad = " ".repeat(Math.max(0, nameWidth - markerWidth - visibleWidth(row.name)));
			const nameCell = `${markerCell}${styleName(row.name)}${namePad}`;
			const facts = row.facts?.filter((f) => f.length > 0) ?? [];
			const factsText = facts.length > 0 ? `  ${styleFacts(facts.join(factSeparator))}` : "";
			lines.push(`${" ".repeat(rowIndent)}${nameCell}${factsText}`.trimEnd());

			if (row.detail) {
				for (const line of wrapIndented(row.detail, detailIndent, options.columns)) {
					lines.push(styleDetail(line));
				}
			}
			if (row.trailer) {
				const trailer =
					options.columns === undefined
						? row.trailer
						: truncateVisible(row.trailer, Math.max(8, options.columns - detailIndent));
				lines.push(styleTrailer(`${" ".repeat(detailIndent)}${trailer}`));
			}
		}
	}
	return lines.join("\n");
}

/**
 * Single-line variant: name column plus one description, truncated to width
 * rather than wrapped. Used where a listing must stay one row per item (the
 * startup summary, where every extra line pushes the prompt down a page).
 */
export function renderCompactRows(
	rows: Array<{ name: string; detail: string }>,
	options: { columns?: number; indent?: number; style?: Pick<ListStyle, "name" | "detail"> } = {},
): string {
	if (rows.length === 0) return "";
	const indent = options.indent ?? 2;
	const styleName = options.style?.name ?? identity;
	const styleDetail = options.style?.detail ?? identity;
	const nameWidth = Math.max(...rows.map((r) => visibleWidth(r.name)));

	return rows
		.map((row) => {
			const gap = " ".repeat(Math.max(0, nameWidth - visibleWidth(row.name)) + 2);
			const detail =
				options.columns === undefined
					? row.detail
					: truncateVisible(row.detail, Math.max(8, options.columns - indent - nameWidth - 2));
			return `${" ".repeat(indent)}${styleName(row.name)}${gap}${styleDetail(detail)}`.trimEnd();
		})
		.join("\n");
}
