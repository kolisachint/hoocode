/**
 * Eval harness: corpus pinning, provenance capture, and run records.
 *
 * The scoring math lives in `eval.ts`; this module is everything around it
 * that makes a number *comparable to a later number*. Three problems it
 * exists to solve, all of which bit the first eval round
 * (docs/hybrid-retrieval-design.md, "Eval results"):
 *
 *  1. **The corpus is the repo.** Retrieval is measured over hoocode itself,
 *     so every commit moves the thing being measured. A baseline taken today
 *     and a rerun taken after a retrieval change differ by both the change
 *     and the intervening commits, and nothing in the output says so. Fix:
 *     run against a detached git worktree pinned to an explicit SHA, and put
 *     that SHA in the record.
 *  2. **Nothing was recorded.** Results were printed to a terminal and
 *     hand-copied into a markdown table with no repo SHA, no embedder
 *     identity, and no index state. Fix: emit a machine-readable run record.
 *  3. **A degraded run looks like a real one.** With no embsearch binary the
 *     semantic and hybrid rows silently degrade to lexical, producing a table
 *     that is all-lexical but reads like a full sweep. Fix: `embedder` in the
 *     record, plus a per-row degraded count that the writer refuses to hide.
 */

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { execFileSync } from "child_process";
import { rmSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { type EvalConfig, type EvalQuery, type EvalQueryResult, evaluateQuery } from "./eval.js";

/** Metrics aggregated per config across the whole gold set. */
export interface EvalAggregate {
	label: string;
	recallAt1: number;
	recallAt5: number;
	recallAt10: number;
	recallAt50: number;
	mrr: number;
	/** Queries scored under this config. */
	n: number;
	/** How many of them ran degraded (requested retriever unavailable). */
	degraded: number;
}

/** Everything needed to decide whether two run records may be compared. */
export interface EvalProvenance {
	timestampMs: number;
	/** SHA of the corpus actually indexed and searched. */
	corpusSha: string;
	/** Ref the caller asked for, before resolution (e.g. "HEAD"). */
	corpusRef: string;
	/** True when the corpus came from the live working tree rather than a
	 *  pinned worktree — results are then not reproducible. */
	corpusFromWorkingTree: boolean;
	/** Uncommitted changes present at run time. Only meaningful (and only
	 *  possible) when `corpusFromWorkingTree` is true. */
	corpusDirty: boolean;
	/** SHA of the tree whose retrieval code ran. Usually equals `corpusSha`,
	 *  but differs when pinning an old corpus with today's code. */
	harnessSha: string;
	/**
	 * Content hash of `src/core/search` + the chunker. Every tuning constant
	 * that shapes a result — the fusion cap, top-k depths, rerank weights,
	 * chunk sizing — lives in those files, so a changed hash means the
	 * numbers are not comparable, without this module having to maintain a
	 * hand-copied (and inevitably stale) list of constants.
	 */
	retrievalSourceHash: string;
	/** Embedding backend state. `available: false` means every semantic and
	 *  hybrid row in this record degraded to lexical. */
	embedder: {
		available: boolean;
		reason?: string;
		/** Indexed chunk count when the index reached `ready`. */
		chunkCount?: number;
		phase: string;
		/** Binary that served the embeddings, and its self-reported version.
		 *  The embedding model is baked into the binary at build time, so this
		 *  is the only thing that identifies which model produced a score. */
		binaryPath?: string;
		binaryVersion?: string;
	};
	runtime: { node: string; platform: string; arch: string };
}

export interface EvalRunRecord {
	provenance: EvalProvenance;
	goldSet: { queryCount: number; byClass: Record<string, number>; goldSpanCount: number };
	configs: readonly EvalConfig[];
	aggregates: EvalAggregate[];
	perQuery: Array<{ id: string; class: string; results: EvalQueryResult[] }>;
}

function git(repoRoot: string, args: string[]): string {
	return execFileSync("git", ["-C", repoRoot, ...args], { encoding: "utf-8" }).trim();
}

/** Hash every retrieval-shaping source file, so a tuning change is visible as
 *  a changed provenance field rather than an unexplained metric shift. */
export function hashRetrievalSource(repoRoot: string): string {
	const roots = [
		path.join(repoRoot, "packages/coding-agent/src/core/search"),
		path.join(repoRoot, "packages/coding-agent/src/core/embsearch/chunker.ts"),
	];
	const files: string[] = [];
	const walk = (target: string): void => {
		let stat: ReturnType<typeof statSync>;
		try {
			stat = statSync(target);
		} catch {
			return;
		}
		if (stat.isDirectory()) {
			for (const entry of readdirSync(target).sort()) walk(path.join(target, entry));
		} else if (target.endsWith(".ts")) {
			files.push(target);
		}
	};
	for (const root of roots) walk(root);

	const hash = createHash("sha256");
	for (const file of files) {
		// Eval-only modules are excluded: changing how we measure must not look
		// like changing what we measure.
		const base = path.basename(file);
		if (base.startsWith("eval")) continue;
		hash.update(path.relative(repoRoot, file).replace(/\\/g, "/"));
		hash.update(readFileSync(file));
	}
	return hash.digest("hex").slice(0, 16);
}

export interface PinnedCorpus {
	/** Directory to index and search. */
	cwd: string;
	sha: string;
	fromWorkingTree: boolean;
	dirty: boolean;
	/** Removes the worktree, if one was created. */
	dispose: () => void;
}

/**
 * Materialize the corpus to evaluate.
 *
 * With a `ref`, checks out a detached worktree at that commit so the corpus is
 * byte-identical on every rerun. Without one, falls back to the live working
 * tree and reports `dirty` so the record shows the run was not reproducible.
 */
export function pinCorpus(repoRoot: string, ref: string | undefined): PinnedCorpus {
	const dirty = git(repoRoot, ["status", "--porcelain"]).length > 0;
	if (!ref) {
		return {
			cwd: repoRoot,
			sha: git(repoRoot, ["rev-parse", "HEAD"]),
			fromWorkingTree: true,
			dirty,
			dispose: () => {},
		};
	}

	const sha = git(repoRoot, ["rev-parse", ref]);
	// Deterministic path, not mkdtemp: the embedding store is keyed by a hash of
	// the corpus directory, so a fresh temp path every run would re-embed all
	// ~17k chunks (minutes) instead of reusing the store built for this exact
	// SHA. The worktree is still removed afterwards; only the store persists.
	const dir = path.join(tmpdir(), `hoocode-search-eval-${sha.slice(0, 12)}`);
	if (existsSync(dir)) {
		// Left behind by an interrupted run — drop it so `worktree add` succeeds.
		try {
			git(repoRoot, ["worktree", "remove", "--force", dir]);
		} catch {
			rmSync(dir, { recursive: true, force: true });
			git(repoRoot, ["worktree", "prune"]);
		}
	}
	git(repoRoot, ["worktree", "add", "--detach", dir, sha]);
	return {
		cwd: dir,
		sha,
		fromWorkingTree: false,
		dirty: false,
		dispose: () => {
			try {
				git(repoRoot, ["worktree", "remove", "--force", dir]);
			} catch {
				rmSync(dir, { recursive: true, force: true });
			}
		},
	};
}

/** `<binary> --version`, or undefined when it cannot be run. */
function probeBinaryVersion(binaryPath: string | undefined): string | undefined {
	if (!binaryPath) return undefined;
	try {
		return execFileSync(binaryPath, ["--version"], { encoding: "utf-8" }).trim();
	} catch {
		return undefined;
	}
}

export function collectProvenance(
	repoRoot: string,
	corpus: PinnedCorpus,
	corpusRef: string,
	service: EmbsearchService | undefined,
	embsearchBinary?: string,
): EvalProvenance {
	const state = service?.getState();
	const phase = state?.phase ?? "absent";
	return {
		timestampMs: Date.now(),
		corpusSha: corpus.sha,
		corpusRef,
		corpusFromWorkingTree: corpus.fromWorkingTree,
		corpusDirty: corpus.dirty,
		harnessSha: git(repoRoot, ["rev-parse", "HEAD"]),
		retrievalSourceHash: hashRetrievalSource(repoRoot),
		embedder: {
			// `ready` is the only phase the service reaches with a real embedder:
			// it rejects the mock backend at startup, so availability here also
			// certifies the numbers came from a genuine ONNX build.
			available: service?.isAvailable() ?? false,
			reason: state && "reason" in state ? state.reason : undefined,
			chunkCount: state?.phase === "ready" ? state.chunkCount : undefined,
			phase,
			binaryPath: embsearchBinary,
			binaryVersion: probeBinaryVersion(embsearchBinary),
		},
		runtime: { node: process.version, platform: process.platform, arch: process.arch },
	};
}

export function summarizeGoldSet(dataset: readonly EvalQuery[]): EvalRunRecord["goldSet"] {
	const byClass: Record<string, number> = {};
	let goldSpanCount = 0;
	for (const query of dataset) {
		byClass[query.class] = (byClass[query.class] ?? 0) + 1;
		goldSpanCount += query.gold.length;
	}
	return { queryCount: dataset.length, byClass, goldSpanCount };
}

export interface RunEvalSuiteOptions {
	cwd: string;
	dataset: readonly EvalQuery[];
	configs: readonly EvalConfig[];
	service?: EmbsearchService;
	onQuery?: (index: number, query: EvalQuery) => void;
}

export async function runEvalSuite(options: RunEvalSuiteOptions): Promise<{
	aggregates: EvalAggregate[];
	perQuery: EvalRunRecord["perQuery"];
}> {
	const { cwd, dataset, configs, service } = options;
	const totals = new Map<string, EvalAggregate>();
	const perQuery: EvalRunRecord["perQuery"] = [];

	for (const [index, evalQuery] of dataset.entries()) {
		options.onQuery?.(index, evalQuery);
		const results = await evaluateQuery(cwd, evalQuery, configs, service);
		perQuery.push({ id: evalQuery.id, class: evalQuery.class, results });
		for (const result of results) {
			const total = totals.get(result.label) ?? {
				label: result.label,
				recallAt1: 0,
				recallAt5: 0,
				recallAt10: 0,
				recallAt50: 0,
				mrr: 0,
				n: 0,
				degraded: 0,
			};
			total.recallAt1 += result.recallAt1;
			total.recallAt5 += result.recallAt5;
			total.recallAt10 += result.recallAt10;
			total.recallAt50 += result.recallAt50;
			total.mrr += result.mrr;
			total.n++;
			if (result.degraded) total.degraded++;
			totals.set(result.label, total);
		}
	}

	const aggregates = configs
		.map((config) => totals.get(config.label))
		.filter((total): total is EvalAggregate => total !== undefined)
		.map((total) => ({
			...total,
			recallAt1: total.recallAt1 / total.n,
			recallAt5: total.recallAt5 / total.n,
			recallAt10: total.recallAt10 / total.n,
			recallAt50: total.recallAt50 / total.n,
			mrr: total.mrr / total.n,
		}));

	return { aggregates, perQuery };
}

export function formatAggregateTable(aggregates: readonly EvalAggregate[]): string {
	const pct = (x: number) => `${Math.round(x * 100)}%`.padStart(5);
	const lines = [
		"config           |  R@1  |  R@5  | R@10  | R@50  |  MRR  | notes",
		"-----------------|-------|-------|-------|-------|-------|------",
	];
	for (const a of aggregates) {
		const notes = a.degraded === a.n ? "degraded to lexical" : a.degraded > 0 ? `${a.degraded}/${a.n} degraded` : "";
		lines.push(
			`${a.label.padEnd(16)} | ${pct(a.recallAt1)} | ${pct(a.recallAt5)} | ${pct(a.recallAt10)} | ` +
				`${pct(a.recallAt50)} | ${a.mrr.toFixed(3)} | ${notes}`,
		);
	}
	return lines.join("\n");
}
