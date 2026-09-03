/**
 * Shared diff computation utilities for the edit tool.
 * Used by both edit.ts (for execution) and tool-execution.ts (for preview rendering).
 */

import * as Diff from "diff";
import { constants } from "fs";
import { access, readFile } from "fs/promises";
import { resolveToCwd } from "./path-utils.js";

/** Cache for normalized text to avoid redundant processing. Max 100 entries. */
const normalizeCache = new Map<string, string>();
const MAX_CACHE_SIZE = 100;

export function detectLineEnding(content: string): "\r\n" | "\n" {
	const crlfIdx = content.indexOf("\r\n");
	const lfIdx = content.indexOf("\n");
	if (lfIdx === -1) return "\n";
	if (crlfIdx === -1) return "\n";
	return crlfIdx < lfIdx ? "\r\n" : "\n";
}

export function normalizeToLF(text: string): string {
	return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

export function restoreLineEndings(text: string, ending: "\r\n" | "\n"): string {
	return ending === "\r\n" ? text.replace(/\n/g, "\r\n") : text;
}

/**
 * A normalized string plus, for every one of its indices, the index in the
 * source text that produced it. `map` has one extra trailing entry so a
 * half-open normalized span `[a, b)` maps to the source span `[map[a], map[b])`.
 *
 * This is what lets a fuzzy match be written back over exactly the bytes it
 * matched. Without it the only way to place a normalized match in the original
 * was to widen it to whole lines and re-emit the untouched remainder in
 * normalized form - which silently converted tabs, smart quotes and NFKC
 * lookalikes on any line an edit happened to land on.
 */
interface NormalizedWithMap {
	text: string;
	map: number[];
}

/** Combining marks attach to the preceding base character and normalize with it. */
const COMBINING_MARK = /\p{Mn}/u;

/**
 * NFKC, applied per grapheme cluster so each output character can be traced to
 * the cluster that produced it. Clustering keeps `e` + U+0301 composing into
 * `é` the way whole-string NFKC would, while `ﬁ` still expands to `fi` with both
 * characters pointing at the single source ligature.
 */
function nfkcWithMap(text: string): NormalizedWithMap {
	let out = "";
	const map: number[] = [];
	let i = 0;
	while (i < text.length) {
		const start = i;
		const base = String.fromCodePoint(text.codePointAt(i) as number);
		i += base.length;
		let cluster = base;
		while (i < text.length) {
			const next = String.fromCodePoint(text.codePointAt(i) as number);
			if (!COMBINING_MARK.test(next)) break;
			cluster += next;
			i += next.length;
		}
		const composed = cluster.normalize("NFKC");
		for (let k = 0; k < composed.length; k++) map.push(start);
		out += composed;
	}
	map.push(text.length);
	return { text: out, map };
}

/** CRLF and lone CR collapse to LF. */
function toLfWithMap(text: string): NormalizedWithMap {
	let out = "";
	const map: number[] = [];
	for (let i = 0; i < text.length; i++) {
		const ch = text[i];
		if (ch === "\r") {
			map.push(i);
			out += "\n";
			if (text[i + 1] === "\n") i++;
			continue;
		}
		map.push(i);
		out += ch;
	}
	map.push(text.length);
	return { text: out, map };
}

/**
 * The per-line whitespace pass: tabs widen to two spaces, interior runs of two
 * or more spaces collapse to one (leading indentation is left alone), and
 * trailing whitespace is dropped.
 */
function normalizeLineWhitespaceWithMap(text: string): NormalizedWithMap {
	let out = "";
	const map: number[] = [];
	let lineStart = 0;
	while (lineStart <= text.length) {
		let lineEnd = text.indexOf("\n", lineStart);
		const hasNewline = lineEnd !== -1;
		if (!hasNewline) lineEnd = text.length;

		// Tabs first, so indentation is measured the way the old chain measured it.
		let expanded = "";
		const expandedMap: number[] = [];
		for (let i = lineStart; i < lineEnd; i++) {
			if (text[i] === "\t") {
				expanded += "  ";
				expandedMap.push(i, i);
			} else {
				expanded += text[i];
				expandedMap.push(i);
			}
		}

		const leadingLength = (expanded.match(/^\s*/)?.[0] ?? "").length;
		let emitted = "";
		const emittedMap: number[] = [];
		for (let i = 0; i < expanded.length; i++) {
			// Collapse only runs that start past the indentation.
			if (i >= leadingLength && expanded[i] === " " && emitted.endsWith(" ") && emitted.length > leadingLength) {
				continue;
			}
			emitted += expanded[i];
			emittedMap.push(expandedMap[i]);
		}
		// trimEnd
		let end = emitted.length;
		while (end > 0 && /\s/.test(emitted[end - 1])) end--;

		out += emitted.slice(0, end);
		for (let i = 0; i < end; i++) map.push(emittedMap[i]);

		if (hasNewline) {
			out += "\n";
			map.push(lineEnd);
			lineStart = lineEnd + 1;
		} else {
			break;
		}
	}
	map.push(text.length);
	return { text: out, map };
}

/** Compose `outer` (indices into `inner.text`) onto `inner`'s own source indices. */
function composeMaps(outer: number[], inner: number[]): number[] {
	return outer.map((i) => inner[i] ?? inner[inner.length - 1]);
}

/**
 * Normalize text for fuzzy matching, keeping an index back to the source for
 * every character produced. Same output text as `normalizeForFuzzyMatch`.
 */
function normalizeForFuzzyMatchWithMap(text: string): NormalizedWithMap {
	const nfkc = nfkcWithMap(text);
	const lf = toLfWithMap(nfkc.text);
	const lines = normalizeLineWhitespaceWithMap(lf.text);
	// The final substitutions are one-for-one, so they leave the map untouched.
	const substituted = applyCharSubstitutions(lines.text);
	return { text: substituted, map: composeMaps(composeMaps(lines.map, lf.map), nfkc.map) };
}

/** The one-for-one Unicode substitutions: quotes, dashes and exotic spaces. */
function applyCharSubstitutions(text: string): string {
	return text
		.replace(/[\u2018\u2019\u201A\u201B]/g, "'")
		.replace(/[\u201C\u201D\u201E\u201F]/g, '"')
		.replace(/[\u2010\u2011\u2012\u2013\u2014\u2015\u2212]/g, "-")
		.replace(/[\u00A0\u2002-\u200A\u202F\u205F\u3000]/g, " ");
}

/**
 * Normalize text for fuzzy matching. Applies progressive transformations:
 * - Normalize line endings to LF
 * - Strip trailing whitespace from each line
 * - Normalize tabs to spaces (2 spaces per tab)
 * - Collapse multiple spaces to single space
 * - Normalize smart quotes to ASCII equivalents
 * - Normalize Unicode dashes/hyphens to ASCII hyphen
 * - Normalize special Unicode spaces to regular space
 */
function normalizeForFuzzyMatch(text: string): string {
	// Check cache first
	const cached = normalizeCache.get(text);
	if (cached !== undefined) return cached;

	const normalized = text
		.normalize("NFKC")
		// Normalize line endings to LF
		.replace(/\r\n/g, "\n")
		.replace(/\r/g, "\n")
		// Strip trailing whitespace per line
		.split("\n")
		.map((line) => {
			// Normalize tabs to 2 spaces
			let normalized = line.replace(/\t/g, "  ");
			// Collapse multiple spaces to single space (but preserve leading indentation pattern)
			// Only collapse spaces that are NOT at the start of the line (indentation)
			const leadingSpaces = normalized.match(/^(\s*)/)?.[1] ?? "";
			const rest = normalized.slice(leadingSpaces.length);
			normalized = leadingSpaces + rest.replace(/ {2,}/g, " ");
			return normalized.trimEnd();
		})
		.join("\n")
		// Smart single quotes → '
		.replace(/[\u2018\u2019\u201A\u201B]/g, "'")
		// Smart double quotes → "
		.replace(/[\u201C\u201D\u201E\u201F]/g, '"')
		// Various dashes/hyphens → -
		// U+2010 hyphen, U+2011 non-breaking hyphen, U+2012 figure dash,
		// U+2013 en-dash, U+2014 em-dash, U+2015 horizontal bar, U+2212 minus
		.replace(/[\u2010\u2011\u2012\u2013\u2014\u2015\u2212]/g, "-")
		// Special spaces → regular space
		// U+00A0 NBSP, U+2002-U+200A various spaces, U+202F narrow NBSP,
		// U+205F medium math space, U+3000 ideographic space
		.replace(/[\u00A0\u2002-\u200A\u202F\u205F\u3000]/g, " ");

	// Cache the result (with size limit)
	if (normalizeCache.size >= MAX_CACHE_SIZE) {
		// Remove oldest entry (first key)
		const firstKey = normalizeCache.keys().next().value;
		if (firstKey !== undefined) {
			normalizeCache.delete(firstKey);
		}
	}
	normalizeCache.set(text, normalized);

	return normalized;
}

/** Whether `index` in `text` is the first character of a line. */
function isAtLineStart(text: string, index: number): boolean {
	return index === 0 || text[index - 1] === "\n";
}

/**
 * Indentation of `line`, with tabs widened the way `normalizeForFuzzyMatch`
 * widens them, so a tab-indented file and a two-space rendering of it compare
 * equal while genuinely different nesting levels do not.
 */
function normalizedIndent(line: string): string {
	return (line.match(/^[ \t]*/)?.[0] ?? "").replace(/\t/g, "  ");
}

export interface Edit {
	oldText: string;
	newText: string;
	/**
	 * When true, replace every occurrence of oldText instead of requiring it to
	 * be unique. Default (false/undefined) keeps the uniqueness guardrail.
	 */
	replaceAll?: boolean;
}

interface MatchedEdit {
	editIndex: number;
	matchIndex: number;
	matchLength: number;
	/**
	 * Text written over the span. Usually the edit's `newText` verbatim; for a
	 * fuzzy match that covered only part of a line, the untouched remainder of
	 * the first/last line rides along in normalized form (see `resolveFuzzySpan`).
	 */
	replacement: string;
}

/** A match located in, and expressed in coordinates of, the original content. */
interface ResolvedSpan {
	matchIndex: number;
	matchLength: number;
	replacement: string;
}

export interface AppliedEditsResult {
	baseContent: string;
	newContent: string;
}

/**
 * Locate one edit's `oldText` in `content`, always returning spans in
 * `content`'s own coordinates.
 *
 * Three tiers, tried in order: exact, fuzzy-normalized, then the
 * indentation-tolerant line-block fallback. `occurrences` counts the matches the
 * winning tier found, which is what the uniqueness guardrail tests; `spans` can
 * be shorter, because two fuzzy matches sharing a line are emitted as one
 * rewrite of that line.
 */
function findEditSpans(
	content: string,
	edit: { oldText: string; newText: string },
	fuzzyIndex: () => NormalizedWithMap | null,
): { spans: ResolvedSpan[]; occurrences: number; noopFuzzySpan?: ResolvedSpan } {
	// An oldText that opens with indentation is a statement about a whole line, so
	// it must not match mid-line inside a more deeply indented one - that lands the
	// edit in a different block entirely. An oldText that opens with a non-blank
	// character claims nothing about indentation, so every tier stays tolerant.
	const anchored = /^[ \t]/.test(edit.oldText);

	// Tier 1: exact. Already in original coordinates.
	const exact = collectMatchIndices(content, edit.oldText).filter((i) => !anchored || isAtLineStart(content, i));
	if (exact.length > 0) {
		return {
			spans: exact.map((matchIndex) => ({
				matchIndex,
				matchLength: edit.oldText.length,
				replacement: edit.newText,
			})),
			occurrences: exact.length,
		};
	}

	// Tier 2: fuzzy. Located in normalized space, then mapped straight back onto
	// the bytes it matched - nothing outside the match is rewritten.
	const normalized = fuzzyIndex();
	if (normalized) {
		const fuzzyOldText = normalizeForFuzzyMatch(edit.oldText);
		const normalizedText = normalized.text;
		const fuzzy = collectMatchIndices(normalizedText, fuzzyOldText).filter(
			(i) => !anchored || isAtLineStart(normalizedText, i),
		);
		const spanAt = (index: number, replacement: string): ResolvedSpan => {
			const start = normalized.map[index];
			const end = normalized.map[index + fuzzyOldText.length];
			return { matchIndex: start, matchLength: end - start, replacement };
		};

		/**
		 * Build the replacement for a fuzzy match.
		 *
		 * newText is written as the model wrote it, with one exception: indentation.
		 * A fuzzy match means oldText was not on disk byte-for-byte, so the whitespace
		 * in it is the model's rendering of the line rather than a statement about the
		 * file - and newText inherits that rendering. Writing it back re-indents lines
		 * the edit never meant to touch, which in a tab-indented file means every
		 * fuzzy edit silently converts tabs to spaces.
		 *
		 * So when newText's indentation says the same thing oldText's did, the file's
		 * own indentation is kept. An edit that means to re-indent says so by giving
		 * newText a different indentation from oldText, and that still applies.
		 */
		const replacementFor = (index: number): string => {
			const start = normalized.map[index];
			const end = normalized.map[index + fuzzyOldText.length];
			if (!isAtLineStart(content, start)) return edit.newText;
			const originalLines = content.slice(start, end).split("\n");
			const newLines = edit.newText.split("\n");
			const oldLines = edit.oldText.split("\n");
			if (originalLines.length !== newLines.length || oldLines.length !== newLines.length) {
				return edit.newText;
			}
			return newLines
				.map((line, i) => {
					const newIndent = line.match(/^[ \t]*/)?.[0] ?? "";
					const oldIndent = oldLines[i].match(/^[ \t]*/)?.[0] ?? "";
					if (normalizedIndent(newIndent) !== normalizedIndent(oldIndent)) return line;
					return (originalLines[i].match(/^[ \t]*/)?.[0] ?? "") + line.slice(newIndent.length);
				})
				.join("\n");
		};

		if (fuzzy.length > 0) {
			// oldText did not match the file byte-for-byte, so the whitespace the model
			// used is its own rendering rather than a statement about the file. If
			// newText normalizes to the same thing, it asked for no change: leave the
			// span exactly as it is so the no-change error fires, instead of rewriting
			// the line's indentation to match the model's rendering of it.
			if (normalizeForFuzzyMatch(edit.newText) === fuzzyOldText) {
				// oldText did not match byte-for-byte, so its whitespace is the model's
				// rendering rather than a statement about the file, and newText asks for
				// nothing the matcher can see. Leave the bytes alone and report the span
				// so the caller can name the character the model failed to reproduce.
				const untouched = fuzzy.map((index) => {
					const span = spanAt(index, "");
					return { ...span, replacement: content.slice(span.matchIndex, span.matchIndex + span.matchLength) };
				});
				return { spans: untouched, occurrences: fuzzy.length, noopFuzzySpan: untouched[0] };
			}
			return {
				spans: fuzzy.map((index) => spanAt(index, replacementFor(index))),
				occurrences: fuzzy.length,
			};
		}
	}

	// Tier 3: indentation-tolerant line blocks. Already in original coordinates.
	const blocks = findLineBlockMatches(content, edit.oldText, anchored).map((span) => ({
		matchIndex: span.matchIndex,
		matchLength: span.matchLength,
		replacement: edit.newText,
	}));
	return { spans: blocks, occurrences: blocks.length };
}

/** Strip UTF-8 BOM if present, return both the BOM (if any) and the text without it */
export function stripBom(content: string): { bom: string; text: string } {
	return content.startsWith("\uFEFF") ? { bom: "\uFEFF", text: content.slice(1) } : { bom: "", text: content };
}

/**
 * Collect the start index of every non-overlapping occurrence of needle in
 * haystack. The needle must already be in the same space as haystack (raw for
 * exact matches, fuzzy-normalized when haystack is fuzzy-normalized).
 */
function collectMatchIndices(haystack: string, needle: string): number[] {
	const indices: number[] = [];
	if (needle.length === 0) return indices;
	let from = 0;
	while (true) {
		const idx = haystack.indexOf(needle, from);
		if (idx === -1) break;
		indices.push(idx);
		from = idx + needle.length; // non-overlapping
	}
	return indices;
}

/**
 * Per-line normalization for indentation-tolerant block matching. Applies the
 * same Unicode/space normalization as fuzzy matching, then strips all leading
 * and trailing whitespace so indentation differences are ignored entirely.
 */
function blockNormalizeLine(line: string): string {
	return normalizeForFuzzyMatch(line).trim();
}

interface LineBlockMatch {
	matchIndex: number;
	matchLength: number;
}

/**
 * Indentation-tolerant fallback matcher. Compares oldText against content line
 * by line, ignoring each line's leading/trailing whitespace (and Unicode
 * formatting). Returns character spans in `content` for every block whose
 * trimmed lines equal the trimmed oldText lines. Replacement still happens in
 * the original content space, so surrounding formatting is preserved.
 */
function findLineBlockMatches(content: string, oldText: string, anchored = false): LineBlockMatch[] {
	const hadTrailingNewline = oldText.endsWith("\n");
	const oldLines = oldText.split("\n");
	if (hadTrailingNewline) oldLines.pop();
	if (oldLines.length === 0) return [];
	const trimmedOld = oldLines.map(blockNormalizeLine);

	const contentLines = content.split("\n");
	const k = trimmedOld.length;
	if (k > contentLines.length) return [];

	// Char offset of each line start within content.
	const offsets = new Array<number>(contentLines.length);
	let acc = 0;
	for (let i = 0; i < contentLines.length; i++) {
		offsets[i] = acc;
		acc += contentLines[i].length + 1; // + newline
	}

	const matches: LineBlockMatch[] = [];
	for (let i = 0; i + k <= contentLines.length; i++) {
		let ok = true;
		for (let j = 0; j < k; j++) {
			if (blockNormalizeLine(contentLines[i + j]) !== trimmedOld[j]) {
				ok = false;
				break;
			}
			// Tier 3 ignores indentation by design. When oldText stated its own
			// indentation, honour that statement rather than matching any nesting level.
			if (anchored && normalizedIndent(contentLines[i + j]) !== normalizedIndent(oldLines[j])) {
				ok = false;
				break;
			}
		}
		if (!ok) continue;
		const matchIndex = offsets[i];
		let matchLength = 0;
		for (let j = 0; j < k; j++) matchLength += contentLines[i + j].length + (j < k - 1 ? 1 : 0);
		// Include the trailing newline when oldText carried one and a line follows the block.
		if (hadTrailingNewline && i + k < contentLines.length) matchLength += 1;
		matches.push({ matchIndex, matchLength });
	}
	return matches;
}

function getNotFoundError(path: string, editIndex: number, totalEdits: number): Error {
	if (totalEdits === 1) {
		return new Error(
			`Could not find the exact text in ${path}. The old text must match exactly including all whitespace and newlines.`,
		);
	}
	return new Error(
		`Could not find edits[${editIndex}] in ${path}. The oldText must match exactly including all whitespace and newlines.`,
	);
}

function getDuplicateError(path: string, editIndex: number, totalEdits: number, occurrences: number): Error {
	if (totalEdits === 1) {
		return new Error(
			`Found ${occurrences} occurrences of the text in ${path}. The text must be unique. Please provide more context to make it unique.`,
		);
	}
	return new Error(
		`Found ${occurrences} occurrences of edits[${editIndex}] in ${path}. Each oldText must be unique. Please provide more context to make it unique.`,
	);
}

function getEmptyOldTextError(path: string, editIndex: number, totalEdits: number): Error {
	if (totalEdits === 1) {
		return new Error(`oldText must not be empty in ${path}.`);
	}
	return new Error(`edits[${editIndex}].oldText must not be empty in ${path}.`);
}

/**
 * Characters the fuzzy matcher erases. When an edit matched only fuzzily and its
 * newText normalizes to the same text, one of these is why: the file holds a
 * character the model reproduced as its plain-ASCII lookalike, so the change it
 * asked for is invisible to the matcher and can never be applied by retrying the
 * same text. Naming the character is the whole recovery - resend oldText with it.
 */
const NORMALIZED_AWAY_NAMES = new Map<number, string>([
	[0x0009, "TAB"],
	[0x00a0, "NO-BREAK SPACE"],
	[0x2002, "EN SPACE"],
	[0x2003, "EM SPACE"],
	[0x2009, "THIN SPACE"],
	[0x200a, "HAIR SPACE"],
	[0x2010, "HYPHEN"],
	[0x2011, "NON-BREAKING HYPHEN"],
	[0x2012, "FIGURE DASH"],
	[0x2013, "EN DASH"],
	[0x2014, "EM DASH"],
	[0x2015, "HORIZONTAL BAR"],
	[0x2018, "LEFT SINGLE QUOTATION MARK"],
	[0x2019, "RIGHT SINGLE QUOTATION MARK"],
	[0x201a, "SINGLE LOW-9 QUOTATION MARK"],
	[0x201b, "SINGLE HIGH-REVERSED-9 QUOTATION MARK"],
	[0x201c, "LEFT DOUBLE QUOTATION MARK"],
	[0x201d, "RIGHT DOUBLE QUOTATION MARK"],
	[0x201e, "DOUBLE LOW-9 QUOTATION MARK"],
	[0x201f, "DOUBLE HIGH-REVERSED-9 QUOTATION MARK"],
	[0x202f, "NARROW NO-BREAK SPACE"],
	[0x205f, "MEDIUM MATHEMATICAL SPACE"],
	[0x2212, "MINUS SIGN"],
	[0x3000, "IDEOGRAPHIC SPACE"],
]);

function formatCodePoint(ch: string): string {
	const cp = ch.codePointAt(0) ?? 0;
	const hex = `U+${cp.toString(16).toUpperCase().padStart(4, "0")}`;
	const name = NORMALIZED_AWAY_NAMES.get(cp);
	if (name) return `${hex} ${name}`;
	// NFKC-only difference (ligature, full-width form, superscript, ...).
	return `${hex} (normalizes to ${JSON.stringify(ch.normalize("NFKC"))})`;
}

/** Cap on how many offending characters one error names before summarising. */
const MAX_REPORTED_CHARS = 5;

/**
 * Every character in `text` that fuzzy normalization would not leave alone.
 *
 * All of them are listed rather than just the first: the match span is widened to
 * whole lines, so the first offender is often a leading tab the model never put
 * in its oldText, while the character it actually needs sits further along the
 * line. Naming one of them and guessing wrong is worse than naming them all.
 */
function findNormalizedAwayChars(text: string): Array<{ ch: string; index: number }> {
	const found: Array<{ ch: string; index: number }> = [];
	let index = 0;
	for (const ch of text) {
		const cp = ch.codePointAt(0) ?? 0;
		if (NORMALIZED_AWAY_NAMES.has(cp) || ch.normalize("NFKC") !== ch) {
			found.push({ ch, index });
		}
		index += ch.length;
	}
	return found;
}

/** 1-indexed line and column of `offset` within `content`. */
function lineAndColumn(content: string, offset: number): { line: number; column: number } {
	const before = content.slice(0, offset);
	const line = before.split("\n").length;
	const column = offset - (before.lastIndexOf("\n") + 1) + 1;
	return { line, column };
}

/**
 * The edit matched, but only after normalization erased the very difference it
 * asked for. Retrying the same oldText can never succeed, so say which character
 * is actually on disk and where.
 */
function getFuzzyNoopError(
	path: string,
	editIndex: number,
	totalEdits: number,
	content: string,
	span: ResolvedSpan,
): Error {
	const matched = content.slice(span.matchIndex, span.matchIndex + span.matchLength);
	const which = totalEdits === 1 ? "The edit" : `edits[${editIndex}]`;
	const offenders = findNormalizedAwayChars(matched);
	let detail: string;
	if (offenders.length > 0) {
		const shown = offenders.slice(0, MAX_REPORTED_CHARS).map((o) => {
			const { line, column } = lineAndColumn(content, span.matchIndex + o.index);
			return `${formatCodePoint(o.ch)} at line ${line}, column ${column}`;
		});
		const more = offenders.length - shown.length;
		detail =
			`the text it matched in ${path} contains ${shown.join("; ")}` +
			`${more > 0 ? `; and ${more} more` : ""}, which your oldText spelled as plain-ASCII lookalikes. ` +
			`Send oldText containing those exact characters and the replacement will apply.`;
	} else {
		detail =
			`oldText matched ${path} only after whitespace normalization, and newText normalizes to the same text, ` +
			`so nothing would change. Send oldText exactly as the file spells it.`;
	}
	return new Error(`No changes made to ${path}. ${which} asked for a change that is invisible to matching: ${detail}`);
}

function getNoChangeError(path: string, totalEdits: number): Error {
	if (totalEdits === 1) {
		return new Error(
			`No changes made to ${path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected.`,
		);
	}
	return new Error(`No changes made to ${path}. The replacements produced identical content.`);
}

/**
 * Apply one or more exact-text replacements to LF-normalized content.
 *
 * All edits are matched against the same original content. Replacements are
 * then applied in reverse order so offsets remain stable.
 *
 * Every match, however loosely it was found, is resolved back to a span of the
 * *original* content before anything is written. A fuzzy or indentation-tolerant
 * match therefore rewrites only the lines it landed on: the rest of the file
 * keeps its exact bytes, and the diff callers render from `baseContent` is the
 * real change on disk rather than a normalized-vs-normalized view of it.
 */
export function applyEditsToNormalizedContent(
	normalizedContent: string,
	edits: Edit[],
	path: string,
): AppliedEditsResult {
	const normalizedEdits = edits.map((edit) => ({
		oldText: normalizeToLF(edit.oldText),
		newText: normalizeToLF(edit.newText),
		replaceAll: edit.replaceAll === true,
	}));

	for (let i = 0; i < normalizedEdits.length; i++) {
		if (normalizedEdits[i].oldText.length === 0) {
			throw getEmptyOldTextError(path, i, normalizedEdits.length);
		}
	}

	const baseContent = normalizedContent;

	// Needed only if some edit reaches the fuzzy tier, and identical for every
	// edit, so build it at most once. A map that does not line up with its own
	// text would put spans in the wrong place, so the tier is skipped rather than
	// trusted if that ever fails to hold.
	let normalizedCache: NormalizedWithMap | null | undefined;
	const fuzzyIndex = () => {
		if (normalizedCache === undefined) {
			const built = normalizeForFuzzyMatchWithMap(baseContent);
			normalizedCache = built.map.length === built.text.length + 1 ? built : null;
		}
		return normalizedCache;
	};

	const matchedEdits: MatchedEdit[] = [];
	for (let i = 0; i < normalizedEdits.length; i++) {
		const edit = normalizedEdits[i];
		const { spans, occurrences, noopFuzzySpan } = findEditSpans(baseContent, edit, fuzzyIndex);
		if (spans.length === 0) {
			throw getNotFoundError(path, i, normalizedEdits.length);
		}
		// Raised per edit, not once for the whole call: a no-op hidden among edits
		// that do change bytes used to be swallowed and reported as a success.
		if (noopFuzzySpan) {
			throw getFuzzyNoopError(path, i, normalizedEdits.length, baseContent, noopFuzzySpan);
		}

		if (edit.replaceAll) {
			// Replace every occurrence so the shared reverse-order applier rewrites them all.
			for (const span of spans) {
				matchedEdits.push({
					editIndex: i,
					matchIndex: span.matchIndex,
					matchLength: span.matchLength,
					replacement: span.replacement,
				});
			}
			continue;
		}

		if (occurrences > 1) {
			throw getDuplicateError(path, i, normalizedEdits.length, occurrences);
		}

		matchedEdits.push({
			editIndex: i,
			matchIndex: spans[0].matchIndex,
			matchLength: spans[0].matchLength,
			replacement: spans[0].replacement,
		});
	}

	matchedEdits.sort((a, b) => a.matchIndex - b.matchIndex);
	for (let i = 1; i < matchedEdits.length; i++) {
		const previous = matchedEdits[i - 1];
		const current = matchedEdits[i];
		if (previous.matchIndex + previous.matchLength > current.matchIndex) {
			throw new Error(
				`edits[${previous.editIndex}] and edits[${current.editIndex}] overlap in ${path}. Merge them into one edit or target disjoint regions.`,
			);
		}
	}

	let newContent = baseContent;
	for (let i = matchedEdits.length - 1; i >= 0; i--) {
		const edit = matchedEdits[i];
		newContent =
			newContent.substring(0, edit.matchIndex) +
			edit.replacement +
			newContent.substring(edit.matchIndex + edit.matchLength);
	}

	if (baseContent === newContent) {
		throw getNoChangeError(path, normalizedEdits.length);
	}

	return { baseContent, newContent };
}

/**
 * Generate a unified diff string with line numbers and context.
 * Returns both the diff string and the first changed line number (in the new file).
 */
export function generateDiffString(
	oldContent: string,
	newContent: string,
	contextLines = 4,
): { diff: string; firstChangedLine: number | undefined } {
	const parts = Diff.diffLines(oldContent, newContent);
	const output: string[] = [];

	const oldLines = oldContent.split("\n");
	const newLines = newContent.split("\n");
	const maxLineNum = Math.max(oldLines.length, newLines.length);
	const lineNumWidth = String(maxLineNum).length;

	let oldLineNum = 1;
	let newLineNum = 1;
	let lastWasChange = false;
	let firstChangedLine: number | undefined;

	for (let i = 0; i < parts.length; i++) {
		const part = parts[i];
		const raw = part.value.split("\n");
		if (raw[raw.length - 1] === "") {
			raw.pop();
		}

		if (part.added || part.removed) {
			// Capture the first changed line (in the new file)
			if (firstChangedLine === undefined) {
				firstChangedLine = newLineNum;
			}

			// Show the change
			for (const line of raw) {
				if (part.added) {
					const lineNum = String(newLineNum).padStart(lineNumWidth, " ");
					output.push(`+${lineNum} ${line}`);
					newLineNum++;
				} else {
					// removed
					const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
					output.push(`-${lineNum} ${line}`);
					oldLineNum++;
				}
			}
			lastWasChange = true;
		} else {
			// Context lines - only show a few before/after changes
			const nextPartIsChange = i < parts.length - 1 && (parts[i + 1].added || parts[i + 1].removed);
			const hasLeadingChange = lastWasChange;
			const hasTrailingChange = nextPartIsChange;

			if (hasLeadingChange && hasTrailingChange) {
				if (raw.length <= contextLines * 2) {
					for (const line of raw) {
						const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
						output.push(` ${lineNum} ${line}`);
						oldLineNum++;
						newLineNum++;
					}
				} else {
					const leadingLines = raw.slice(0, contextLines);
					const trailingLines = raw.slice(raw.length - contextLines);
					const skippedLines = raw.length - leadingLines.length - trailingLines.length;

					for (const line of leadingLines) {
						const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
						output.push(` ${lineNum} ${line}`);
						oldLineNum++;
						newLineNum++;
					}

					output.push(` ${"".padStart(lineNumWidth, " ")} ...`);
					oldLineNum += skippedLines;
					newLineNum += skippedLines;

					for (const line of trailingLines) {
						const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
						output.push(` ${lineNum} ${line}`);
						oldLineNum++;
						newLineNum++;
					}
				}
			} else if (hasLeadingChange) {
				const shownLines = raw.slice(0, contextLines);
				const skippedLines = raw.length - shownLines.length;

				for (const line of shownLines) {
					const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
					output.push(` ${lineNum} ${line}`);
					oldLineNum++;
					newLineNum++;
				}

				if (skippedLines > 0) {
					output.push(` ${"".padStart(lineNumWidth, " ")} ...`);
					oldLineNum += skippedLines;
					newLineNum += skippedLines;
				}
			} else if (hasTrailingChange) {
				const skippedLines = Math.max(0, raw.length - contextLines);
				if (skippedLines > 0) {
					output.push(` ${"".padStart(lineNumWidth, " ")} ...`);
					oldLineNum += skippedLines;
					newLineNum += skippedLines;
				}

				for (const line of raw.slice(skippedLines)) {
					const lineNum = String(oldLineNum).padStart(lineNumWidth, " ");
					output.push(` ${lineNum} ${line}`);
					oldLineNum++;
					newLineNum++;
				}
			} else {
				// Skip these context lines entirely
				oldLineNum += raw.length;
				newLineNum += raw.length;
			}

			lastWasChange = false;
		}
	}

	return { diff: output.join("\n"), firstChangedLine };
}

export interface EditDiffResult {
	diff: string;
	firstChangedLine: number | undefined;
}

export interface EditDiffError {
	error: string;
}

/**
 * Compute the diff for one or more edit operations without applying them.
 * Used for preview rendering in the TUI before the tool executes.
 */
export async function computeEditsDiff(
	path: string,
	edits: Edit[],
	cwd: string,
): Promise<EditDiffResult | EditDiffError> {
	const absolutePath = resolveToCwd(path, cwd);

	try {
		// Check if file exists and is readable
		try {
			await access(absolutePath, constants.R_OK);
		} catch (error: unknown) {
			const errorMessage = error instanceof Error && "code" in error ? `Error code: ${error.code}` : String(error);
			return { error: `Could not edit file: ${path}. ${errorMessage}.` };
		}

		// Read the file
		const rawContent = await readFile(absolutePath, "utf-8");

		// Strip BOM before matching (LLM won't include invisible BOM in oldText)
		const { text: content } = stripBom(rawContent);
		const normalizedContent = normalizeToLF(content);
		const { baseContent, newContent } = applyEditsToNormalizedContent(normalizedContent, edits, path);

		// Generate the diff
		return generateDiffString(baseContent, newContent);
	} catch (err) {
		return { error: err instanceof Error ? err.message : String(err) };
	}
}
