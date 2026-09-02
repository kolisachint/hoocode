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

/**
 * Line starts and line texts for one string. Each entry in `lines` includes its
 * own trailing newline, so `starts[i] + lines[i].length` is the start of line
 * `i + 1`. Used to translate a fuzzy match back into original coordinates.
 */
interface LineIndex {
	text: string;
	starts: number[];
	lines: string[];
}

function buildLineIndex(text: string): LineIndex {
	const starts: number[] = [];
	const lines: string[] = [];
	let start = 0;
	for (;;) {
		const newline = text.indexOf("\n", start);
		if (newline === -1) {
			starts.push(start);
			lines.push(text.slice(start));
			return { text, starts, lines };
		}
		starts.push(start);
		lines.push(text.slice(start, newline + 1));
		start = newline + 1;
	}
}

/** Index of the line containing `offset` (greatest `i` with `starts[i] <= offset`). */
function lineIndexAt(starts: number[], offset: number): number {
	let low = 0;
	let high = starts.length - 1;
	while (low < high) {
		const mid = (low + high + 1) >> 1;
		if (starts[mid] <= offset) low = mid;
		else high = mid - 1;
	}
	return low;
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
 * Translate a match found in fuzzy-normalized space back into a span of the
 * original content.
 *
 * Normalization never adds or removes a newline, so normalized line N always
 * corresponds to original line N; only columns within a line can shift (a tab
 * widens to two spaces, a run of spaces collapses, NFKC changes a character's
 * length). Rather than track per-character offsets through those transforms,
 * the span is widened to whole original lines, and whatever of the first and
 * last line the match did not cover is re-attached around `newText` in
 * normalized form.
 *
 * The practical effect: a match that covers whole lines - by far the common
 * case - rewrites exactly those lines and leaves every other byte of the file
 * untouched. Only a partial-line fuzzy match normalizes anything the edit did
 * not ask for, and never beyond the lines it landed on.
 */
function resolveFuzzySpans(
	original: LineIndex,
	normalized: LineIndex,
	normStarts: number[],
	normLength: number,
	newText: string,
): ResolvedSpan[] {
	const spans: ResolvedSpan[] = [];
	const lineOf = (offset: number) => lineIndexAt(normalized.starts, offset);

	let i = 0;
	while (i < normStarts.length) {
		const firstLine = lineOf(normStarts[i]);
		let lastLine = lineOf(normStarts[i] + normLength - 1);

		// Matches are ascending and non-overlapping. Absorb any that land inside
		// the same line block so the block is emitted once with every
		// substitution applied, rather than as colliding same-line spans.
		let end = i + 1;
		while (end < normStarts.length && lineOf(normStarts[end]) <= lastLine) {
			lastLine = Math.max(lastLine, lineOf(normStarts[end] + normLength - 1));
			end++;
		}

		const blockStart = normalized.starts[firstLine];
		const blockEnd = normalized.starts[lastLine] + normalized.lines[lastLine].length;
		let replacement = "";
		let cursor = blockStart;
		for (let k = i; k < end; k++) {
			replacement += normalized.text.slice(cursor, normStarts[k]) + newText;
			cursor = normStarts[k] + normLength;
		}
		replacement += normalized.text.slice(cursor, blockEnd);

		const matchIndex = original.starts[firstLine];
		const matchEnd = original.starts[lastLine] + original.lines[lastLine].length;
		spans.push({ matchIndex, matchLength: matchEnd - matchIndex, replacement });
		i = end;
	}

	return spans;
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
	lineIndexes: () => { original: LineIndex; normalized: LineIndex } | null,
): { spans: ResolvedSpan[]; occurrences: number } {
	// Tier 1: exact. Already in original coordinates.
	const exact = collectMatchIndices(content, edit.oldText);
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

	// Tier 2: fuzzy. Located in normalized space, then translated back.
	const indexes = lineIndexes();
	if (indexes) {
		const fuzzyOldText = normalizeForFuzzyMatch(edit.oldText);
		const fuzzy = collectMatchIndices(indexes.normalized.text, fuzzyOldText);
		if (fuzzy.length > 0) {
			// oldText did not match the file byte-for-byte, so the whitespace the model
			// used is its own rendering rather than a statement about the file. If
			// newText normalizes to the same thing, it asked for no change: leave the
			// span exactly as it is so the no-change error fires, instead of rewriting
			// the line's indentation to match the model's rendering of it.
			if (normalizeForFuzzyMatch(edit.newText) === fuzzyOldText) {
				return {
					spans: resolveFuzzySpans(
						indexes.original,
						indexes.normalized,
						fuzzy,
						fuzzyOldText.length,
						edit.newText,
					).map((span) => ({
						...span,
						replacement: content.slice(span.matchIndex, span.matchIndex + span.matchLength),
					})),
					occurrences: fuzzy.length,
				};
			}
			return {
				spans: resolveFuzzySpans(indexes.original, indexes.normalized, fuzzy, fuzzyOldText.length, edit.newText),
				occurrences: fuzzy.length,
			};
		}
	}

	// Tier 3: indentation-tolerant line blocks. Already in original coordinates.
	const blocks = findLineBlockMatches(content, edit.oldText).map((span) => ({
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
function findLineBlockMatches(content: string, oldText: string): LineBlockMatch[] {
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

	// Both line indexes are needed only if some edit reaches the fuzzy tier, and
	// they are identical for every edit, so build them at most once. Normalization
	// is line-preserving; if that ever fails to hold, the fuzzy tier is skipped
	// rather than risk translating a span against a mismatched line table.
	let lineIndexesCache: { original: LineIndex; normalized: LineIndex } | null | undefined;
	const lineIndexes = () => {
		if (lineIndexesCache === undefined) {
			const original = buildLineIndex(baseContent);
			const normalized = buildLineIndex(normalizeForFuzzyMatch(baseContent));
			lineIndexesCache = original.lines.length === normalized.lines.length ? { original, normalized } : null;
		}
		return lineIndexesCache;
	};

	const matchedEdits: MatchedEdit[] = [];
	for (let i = 0; i < normalizedEdits.length; i++) {
		const edit = normalizedEdits[i];
		const { spans, occurrences } = findEditSpans(baseContent, edit, lineIndexes);
		if (spans.length === 0) {
			throw getNotFoundError(path, i, normalizedEdits.length);
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
