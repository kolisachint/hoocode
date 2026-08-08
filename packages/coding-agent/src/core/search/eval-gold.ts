/**
 * Gold-set loading and anchor resolution.
 *
 * A gold span recorded as bare line numbers rots on the next refactor, and a
 * rotted gold set fails *silently* — it just scores lower. So each span also
 * carries an `anchor`: a literal snippet that must sit inside the range. The
 * anchor is the source of truth; the line numbers are a cache of where it was
 * when the fixture was last regenerated.
 *
 * That gives three properties the first gold set lacked:
 *  - `resolveGoldSet` recomputes ranges from anchors, so `--fix` re-pins the
 *    whole set after a refactor instead of someone hand-editing 60 numbers;
 *  - `validateGoldSet` fails loudly when an anchor has moved out of its
 *    recorded range, drifted, or become ambiguous;
 *  - anchors are never used for scoring, so this cannot flatter retrieval —
 *    `spanMatchesGold` still sees only a path and a line range.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import type { EvalGoldSpan, EvalQuery } from "./eval.js";

/** Lines of context kept either side of a non-block anchor (an error literal,
 *  a single statement) — enough to be a real target, tight enough that a
 *  60-line chunk elsewhere in the file does not count as a hit. */
const LINE_ANCHOR_PAD = 2;
/** Upper bound on a resolved block, so an anchor that fails to find its
 *  closing brace cannot silently swallow an entire file. */
const MAX_BLOCK_LINES = 120;
/** Fallback extent when a block anchor never finds its closing brace. */
const UNCLOSED_BLOCK_LINES = 15;

export interface GoldValidationIssue {
	queryId: string;
	path: string;
	problem: string;
}

function readLines(corpusRoot: string, rel: string): string[] | undefined {
	try {
		return readFileSync(path.resolve(corpusRoot, rel), "utf-8").split("\n");
	} catch {
		return undefined;
	}
}

/** Indices (0-based) of every line containing `anchor`. */
function findAnchorLines(lines: readonly string[], anchor: string): number[] {
	const found: number[] = [];
	for (let i = 0; i < lines.length; i++) {
		if (lines[i].includes(anchor)) found.push(i);
	}
	return found;
}

/** Leading-whitespace width of a line, for matching a block's closer to its
 *  opener. Tabs count as one; this only ever compares lines in one file, which
 *  is consistently indented. */
function indentWidth(line: string): number {
	return /^[\t ]*/.exec(line)?.[0].length ?? 0;
}

/**
 * Extent of the declaration an anchor names.
 *
 * A line ending in an opener (`{`, `(`, `[`) starts a block, which runs to the
 * first closing bracket indented no deeper than the opener. Anything else is a
 * statement, scored with a small pad.
 *
 * The indentation test is load-bearing. This used to close only on a bracket
 * at *column 0*, which is right for a top-level declaration and wrong for
 * every class member: a method's own `}` is indented, so the scan walked past
 * it to the end of the enclosing class. Three of the four boundary-class gold
 * spans were resolving to 80, 34 and 28 lines for methods that are 7, 4 and 4
 * lines long, all ending on the same line — the class's closing brace. Gold
 * that wide scores a hit for retrieving a neighbouring method, which is not
 * what the query asked for.
 */
function resolveExtent(lines: readonly string[], anchorIndex: number): { startLine: number; endLine: number } {
	const head = lines[anchorIndex];
	const opensBlock = /[{([]\s*$/.test(head);
	if (!opensBlock) {
		return {
			startLine: Math.max(1, anchorIndex + 1 - LINE_ANCHOR_PAD),
			endLine: Math.min(lines.length, anchorIndex + 1 + LINE_ANCHOR_PAD),
		};
	}
	const anchorIndent = indentWidth(head);
	const limit = Math.min(lines.length, anchorIndex + 1 + MAX_BLOCK_LINES);
	for (let i = anchorIndex + 1; i < limit; i++) {
		// `}`, `};`, `});`, `];` at or outside the opener's indentation.
		if (/^[\t ]*[}\])]/.test(lines[i]) && indentWidth(lines[i]) <= anchorIndent) {
			return { startLine: anchorIndex + 1, endLine: i + 1 };
		}
	}
	return { startLine: anchorIndex + 1, endLine: Math.min(lines.length, anchorIndex + 1 + UNCLOSED_BLOCK_LINES) };
}

/** Re-pin one gold span's line range from its anchor. Returns the span
 *  unchanged when it is file-scoped or has no anchor to resolve. */
export function resolveGoldSpan(corpusRoot: string, span: EvalGoldSpan): { span: EvalGoldSpan; problem?: string } {
	const lines = readLines(corpusRoot, span.path);
	if (!lines) return { span, problem: `file not found: ${span.path}` };

	if (span.scope === "file") {
		return { span: { ...span, startLine: 1, endLine: lines.length } };
	}
	if (!span.anchor) return { span, problem: `span-scoped gold needs an anchor: ${span.path}` };

	const matches = findAnchorLines(lines, span.anchor);
	if (matches.length === 0) return { span, problem: `anchor not found in ${span.path}: ${span.anchor}` };
	if (matches.length > 1) {
		return { span, problem: `anchor is ambiguous (${matches.length} matches) in ${span.path}: ${span.anchor}` };
	}

	const { startLine, endLine } = resolveExtent(lines, matches[0]);
	return { span: { ...span, startLine, endLine } };
}

/** Re-pin every gold span in the set. Problems are collected, not thrown, so
 *  a regeneration run reports all drift at once. */
export function resolveGoldSet(
	corpusRoot: string,
	dataset: readonly EvalQuery[],
): { dataset: EvalQuery[]; issues: GoldValidationIssue[] } {
	const issues: GoldValidationIssue[] = [];
	const resolved = dataset.map((query) => ({
		...query,
		gold: query.gold.map((span) => {
			const result = resolveGoldSpan(corpusRoot, span);
			if (result.problem) issues.push({ queryId: query.id, path: span.path, problem: result.problem });
			return result.span;
		}),
	}));
	return { dataset: resolved, issues };
}

/**
 * Check the committed fixture against the corpus without rewriting it.
 *
 * Catches the two ways a gold set goes wrong: the anchor no longer exists (the
 * code was deleted or renamed), or it exists but has moved outside the
 * recorded range (the fixture is stale and every score computed from it is
 * wrong).
 */
export function validateGoldSet(corpusRoot: string, dataset: readonly EvalQuery[]): GoldValidationIssue[] {
	const issues: GoldValidationIssue[] = [];
	const seenIds = new Set<string>();

	for (const query of dataset) {
		if (seenIds.has(query.id)) issues.push({ queryId: query.id, path: "", problem: "duplicate query id" });
		seenIds.add(query.id);
		if (query.gold.length === 0) issues.push({ queryId: query.id, path: "", problem: "query has no gold spans" });

		for (const span of query.gold) {
			const lines = readLines(corpusRoot, span.path);
			if (!lines) {
				issues.push({ queryId: query.id, path: span.path, problem: "file not found" });
				continue;
			}
			if (span.startLine === undefined || span.endLine === undefined) {
				issues.push({ queryId: query.id, path: span.path, problem: "gold span is missing line numbers" });
				continue;
			}
			if (span.startLine < 1 || span.endLine < span.startLine || span.endLine > lines.length) {
				issues.push({
					queryId: query.id,
					path: span.path,
					problem: `line range ${span.startLine}-${span.endLine} out of bounds (file has ${lines.length} lines)`,
				});
				continue;
			}
			if (span.scope === "file") continue;
			if (!span.anchor) {
				issues.push({ queryId: query.id, path: span.path, problem: "span-scoped gold needs an anchor" });
				continue;
			}
			const matches = findAnchorLines(lines, span.anchor);
			if (matches.length === 0) {
				issues.push({ queryId: query.id, path: span.path, problem: `anchor not found: ${span.anchor}` });
			} else if (matches.length > 1) {
				issues.push({
					queryId: query.id,
					path: span.path,
					problem: `anchor is ambiguous (${matches.length} matches): ${span.anchor}`,
				});
			} else if (matches[0] + 1 < span.startLine || matches[0] + 1 > span.endLine) {
				issues.push({
					queryId: query.id,
					path: span.path,
					problem: `anchor moved to line ${matches[0] + 1}, outside recorded range ${span.startLine}-${span.endLine}`,
				});
			}
		}
	}
	return issues;
}

/** Load the gold set from a fixture file. */
export function loadGoldSet(fixturePath: string): EvalQuery[] {
	return JSON.parse(readFileSync(fixturePath, "utf-8")) as EvalQuery[];
}
