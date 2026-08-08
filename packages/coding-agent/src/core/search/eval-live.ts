/**
 * Live-edit evaluation: the one thing the main gold set structurally cannot
 * measure.
 *
 * The eval corpus is a pinned checkout that exactly matches the embedding
 * index, so every retriever sees the same content and the grep leg looks
 * redundant — measurably so, since it is a significant regression once BM25 is
 * present. But an agent's corpus is *not* a clean checkout: it edits files as
 * it works, and the daemon has no file watcher, so anything written during a
 * session is invisible to both dense and BM25 until the next index build.
 * Grep is the only leg that sees it.
 *
 * This applies edits to the corpus **after** indexing and then asks queries
 * that only those edits can answer. A retriever backed by the index scores 0
 * by construction; grep scores what it can actually find. That is the number
 * the decision to keep or drop the grep leg rests on, and nothing in the main
 * sweep produces it.
 *
 * Gold line ranges are resolved from anchors after the edits land, so the
 * fixture never hardcodes line numbers for content it also defines.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { readFileSync } from "fs";
import type { EvalQuery } from "./eval.js";
import { resolveGoldSet } from "./eval-gold.js";

/** A file written into the corpus after indexing. */
export interface LiveEdit {
	/** Repo-relative POSIX path. Created if absent, overwritten if present. */
	path: string;
	/** Full file contents. Held inline so the edit is reproducible from the
	 *  fixture alone, with no dependency on what the corpus already held. */
	content: string;
	/** Why this edit exists — reporting only. */
	note?: string;
}

export interface LiveEditFixture {
	edits: LiveEdit[];
	/** Queries answerable *only* from the edited content. */
	queries: EvalQuery[];
}

export function loadLiveEditFixture(fixturePath: string): LiveEditFixture {
	return JSON.parse(readFileSync(fixturePath, "utf-8")) as LiveEditFixture;
}

/**
 * Write the edits into `corpusRoot`, then resolve the queries' gold ranges
 * against the resulting tree.
 *
 * Order matters: resolving before the write would look for anchors in content
 * that does not exist yet.
 */
export function applyLiveEdits(
	corpusRoot: string,
	fixture: LiveEditFixture,
): { queries: EvalQuery[]; issues: string[] } {
	for (const edit of fixture.edits) {
		const target = path.resolve(corpusRoot, edit.path);
		mkdirSync(path.dirname(target), { recursive: true });
		writeFileSync(target, edit.content);
	}

	const { dataset, issues } = resolveGoldSet(corpusRoot, fixture.queries);
	return {
		queries: dataset,
		issues: issues.map((i) => `${i.queryId} [${i.path}]: ${i.problem}`),
	};
}
