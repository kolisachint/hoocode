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
import { chunkFile } from "../embsearch/chunker.js";
import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { scanRepo } from "../embsearch/repo-scan.js";
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
	/** Files removed from the corpus before indexing — see
	 *  {@link CORPUS_EXCLUSIONS}. A score against a different exclusion list is
	 *  a score against a different corpus, so it is recorded, not assumed. */
	corpusExcluded: string[];
	/**
	 * Set only when the run scored a deliberately shrunk corpus.
	 *
	 * A smaller distractor pool makes every query easier, so these metrics are
	 * higher than a full-corpus run's and are **not** comparable to one. They
	 * are comparable to another subsampled run with the same target and seed,
	 * which is what makes this useful for screening model arms.
	 */
	corpusSubsample?: CorpusSubsampleInfo;
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
		/** Binary that served the embeddings, and its self-reported version. */
		binaryPath?: string;
		binaryVersion?: string;
		/**
		 * Model id the daemon reported — the thing that actually identifies
		 * which model produced these scores.
		 *
		 * This used to be inferred from `binaryVersion`, on the reasoning that
		 * the model was baked into the binary at build time. `--model <dir>`
		 * ends that: one binary now serves any number of models, so two arms of
		 * a model comparison would have carried identical provenance and been
		 * indistinguishable in the record. The id is a hash over the model's
		 * whole spec (pooling, token limit, prefixes), so a change to any of
		 * them shows up here.
		 */
		modelId?: string;
		/** Model directory passed as `--model`, when the run overrode the
		 *  bundled model. Absent means the binary's own model was used. */
		modelDir?: string;
	};
	/** Daemon-side BM25 hybrid store, when the run included one. Absent means
	 *  the record has no `daemon-hybrid` rows. */
	daemonHybrid?: { available: boolean; phase: string };
	/**
	 * Wall time, split at the seam between building the index and scoring the
	 * gold set.
	 *
	 * Recorded because the cost side of a model comparison is almost entirely
	 * indexing, and a single total cannot show it: the first such comparison
	 * could only report "17 -> 60 min" for whole runs and had to note that the
	 * figure was "not isolated from query work", which left the headline cost
	 * of the change unmeasured. These two numbers are machine- and
	 * load-dependent and say nothing about retrieval quality; they are a budget,
	 * not a metric.
	 */
	/**
	 * Chunker character cap, when an arm overrode it. Absent means the shipped
	 * `CHUNK_MAX_CHARS`. Records differing here are not comparable: the chunks
	 * are different text, so every id, span and vector differs.
	 */
	chunkMaxChars?: number;
	timing?: {
		/** Seconds spent bringing the index(es) to `ready`, model load included. */
		indexSeconds: number;
		/** Seconds spent running every config over every gold query. */
		querySeconds: number;
		/** True when one hybrid store served both the dense and BM25 roles
		 *  rather than the corpus being embedded twice. Runs with this false
		 *  paid roughly double the indexing time. */
		sharedStore: boolean;
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
	/** Files removed from the corpus before indexing. Empty when the corpus is
	 *  the live working tree, which is never mutated. */
	excluded: string[];
	/** Present only on a subsampled run. Its presence is what marks a record as
	 *  incomparable to a full-corpus one. */
	subsample?: CorpusSubsampleInfo;
	/** Removes the worktree, if one was created. */
	dispose: () => void;
}

/** Request to shrink the corpus to a chunk budget. See {@link pinCorpus}. */
export interface CorpusSubsampleRequest {
	/** Approximate chunk budget. Gold-bearing files are kept past it. */
	targetChunks: number;
	/** Files that must survive regardless of budget — the gold-bearing ones. */
	keepRelPaths: readonly string[];
	/** Seed for the distractor draw, so a budget reproduces exactly. */
	seed: number;
}

/** What a subsampled run did, recorded so it can never be read as a full one. */
export interface CorpusSubsampleInfo {
	targetChunks: number;
	/** Chunks actually kept. Exceeds the target when gold files alone do. */
	chunkCount: number;
	filesKept: number;
	filesDropped: number;
	/** Gold-bearing files, all of which are kept unconditionally. */
	goldFilesKept: number;
	seed: number;
}

/**
 * Cut `dir` down to a chunk budget, in place.
 *
 * Counts chunks with the indexer's own chunker rather than estimating from
 * file size, because the budget is meant to predict indexing time and
 * indexing time is per chunk. Gold-bearing files are never candidates for
 * removal: dropping one would make its queries unanswerable and score the
 * arm on a corpus that cannot contain the answer.
 *
 * Deletion is what makes this apply to every leg at once. Filtering the
 * indexer's file list instead would shrink the dense and BM25 legs while grep
 * still walked the full tree, and the legs would then be answering about
 * different corpora.
 */
function applySubsample(dir: string, request: CorpusSubsampleRequest): CorpusSubsampleInfo {
	const gold = new Set(request.keepRelPaths);
	const counts = new Map<string, number>();
	for (const file of scanRepo(dir).files) {
		// The scanner skips `.git` as a *directory*, but a linked worktree's
		// `.git` is a file holding the path to the real gitdir — so it comes back
		// as an ordinary indexable file. Deleting it detaches the worktree from
		// the repo, and `worktree remove` then fails on a tree git can no longer
		// validate. Never a candidate.
		if (file.rel === ".git" || file.rel.startsWith(`.git${path.sep}`) || file.rel.startsWith(".git/")) {
			continue;
		}
		let content: string;
		try {
			content = readFileSync(path.join(dir, file.rel), "utf8");
		} catch {
			continue;
		}
		// Deliberately the *default* cap, never the arm's.
		//
		// The budget picks which files survive, and a chunk-cap sweep must
		// compare caps over identical source text. Counting at the arm's cap
		// would let a cap-2000 arm — whose chunks are bigger, so fewer fit the
		// budget — keep far more of the repo than a cap-1000 arm, and the two
		// would then differ by corpus as well as by cap. Sizing is a secondary
		// concern to that: bigger caps produce fewer chunks and index faster
		// anyway, so the budget only ever overestimates their cost.
		const n = chunkFile(file.rel, content).length;
		if (n > 0) counts.set(file.rel, n);
	}

	const keep = new Set<string>();
	let chunks = 0;
	let goldFilesKept = 0;
	for (const rel of counts.keys()) {
		if (!gold.has(rel)) continue;
		keep.add(rel);
		chunks += counts.get(rel) ?? 0;
		goldFilesKept++;
	}

	// Shuffle the distractors rather than taking the scan's order, which is
	// directory order — that would keep a few whole subtrees and drop the rest,
	// making the sample a slice of the repo instead of a sample of it.
	const rng = seededRandom(request.seed);
	const others = [...counts.keys()].filter((rel) => !gold.has(rel));
	for (let i = others.length - 1; i > 0; i--) {
		const j = Math.floor(rng() * (i + 1));
		[others[i], others[j]] = [others[j], others[i]];
	}
	for (const rel of others) {
		if (chunks >= request.targetChunks) break;
		keep.add(rel);
		chunks += counts.get(rel) ?? 0;
	}

	let filesDropped = 0;
	for (const rel of counts.keys()) {
		if (keep.has(rel)) continue;
		try {
			rmSync(path.join(dir, rel), { force: true });
			filesDropped++;
		} catch {
			// A file the scanner listed but cannot be removed stays in the
			// corpus; it inflates the sample slightly and is not worth failing
			// the run over.
		}
	}

	return {
		targetChunks: request.targetChunks,
		chunkCount: chunks,
		filesKept: keep.size,
		filesDropped,
		goldFilesKept,
		seed: request.seed,
	};
}

/**
 * Deterministic PRNG (mulberry32).
 *
 * `Math.random()` would make a "reproducible" subsample a different corpus on
 * every run, which is the one property this must not have.
 */
function seededRandom(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/**
 * Files that describe this eval rather than being searched by it.
 *
 * The fixtures hold all 62 query strings verbatim, so every query is a perfect
 * lexical match against its own entry, and the design note quotes the same
 * queries while discussing the classes they belong to. Measured before this
 * exclusion existed: **56 of 62 queries had one of these files in the top 10,
 * 27 of 62 had one as the #1 result, and they consumed 133 of the 620
 * top-10 slots** — a fifth of the window, spent on the eval reading itself.
 *
 * That is not a ranking artifact a reranker can fix: it displaces real answers
 * out of the window entirely, which is why two boundary-class queries were
 * absent from the top *50* rather than merely buried. Retrieving your own
 * question is not retrieval, so the corpus is scored without them.
 *
 * Removed from the pinned worktree before indexing, never from the repo — and
 * recorded in the run's provenance so a score is never silently taken against
 * a different corpus than it claims.
 */
export const CORPUS_EXCLUSIONS: readonly string[] = [
	"packages/coding-agent/test/fixtures/search-eval.json",
	"packages/coding-agent/test/fixtures/search-eval-live.json",
	"packages/coding-agent/test/fixtures/search-eval-baseline.json",
	"docs/hybrid-retrieval-design.md",
];

/**
 * Materialize the corpus to evaluate.
 *
 * With a `ref`, checks out a detached worktree at that commit so the corpus is
 * byte-identical on every rerun. Without one, falls back to the live working
 * tree and reports `dirty` so the record shows the run was not reproducible.
 *
 * `subsample` shrinks the corpus to a chunk budget, for screening runs where a
 * full arm costs too much to iterate on. It keeps every gold-bearing file and
 * draws distractors deterministically. This is a real change to what is being
 * measured — a smaller distractor pool makes retrieval easier and inflates
 * every metric — so it is recorded in the record and folded into the worktree
 * path, and it needs a `ref`.
 */
export function pinCorpus(
	repoRoot: string,
	ref: string | undefined,
	subsample?: CorpusSubsampleRequest,
): PinnedCorpus {
	const dirty = git(repoRoot, ["status", "--porcelain"]).length > 0;
	if (!ref) {
		if (subsample) {
			// Subsampling deletes files. Against the live checkout that is the
			// user's source tree, so this refuses rather than asks.
			throw new Error("subsampling requires --corpus-ref: it deletes files, and the working tree is not ours to cut");
		}
		// The live working tree is the user's checkout; deleting files from it to
		// tidy a measurement would be an unforgivable trade. Working-tree runs
		// are already stamped non-reproducible, so they carry the contamination.
		return {
			cwd: repoRoot,
			sha: git(repoRoot, ["rev-parse", "HEAD"]),
			fromWorkingTree: true,
			dirty,
			excluded: [],
			dispose: () => {},
		};
	}

	const sha = git(repoRoot, ["rev-parse", ref]);
	// Deterministic path, not mkdtemp: the embedding store is keyed by a hash of
	// the corpus directory, so a fresh temp path every run would re-embed all
	// ~17k chunks (minutes) instead of reusing the store built for this exact
	// SHA. The worktree is still removed afterwards; only the store persists.
	//
	// The subsample is part of the key. Without it a screening run and a full
	// run at the same SHA would share this path *and* the store derived from it,
	// so the second would silently score the first's index — a wrong number that
	// looks entirely normal.
	// No chunk cap in the key: the file set is cap-independent by construction
	// (see `applySubsample`), so cap arms share one worktree. The *store* key
	// does carry the cap, because the vectors differ.
	const subsampleKey = subsample ? `-fast${subsample.targetChunks}s${subsample.seed}` : "";
	const dir = path.join(tmpdir(), `hoocode-search-eval-${sha.slice(0, 12)}${subsampleKey}`);
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

	const excluded: string[] = [];
	for (const rel of CORPUS_EXCLUSIONS) {
		const target = path.join(dir, rel);
		if (existsSync(target)) {
			rmSync(target, { force: true });
			excluded.push(rel);
		}
	}

	const subsampleInfo = subsample ? applySubsample(dir, subsample) : undefined;

	return {
		cwd: dir,
		sha,
		fromWorkingTree: false,
		dirty: false,
		excluded,
		subsample: subsampleInfo,
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
	hybridService?: EmbsearchService,
	modelDir?: string,
	timing?: EvalProvenance["timing"],
	chunkMaxChars?: number,
): EvalProvenance {
	const state = service?.getState();
	const phase = state?.phase ?? "absent";
	return {
		timestampMs: Date.now(),
		corpusSha: corpus.sha,
		corpusRef,
		corpusFromWorkingTree: corpus.fromWorkingTree,
		corpusDirty: corpus.dirty,
		corpusExcluded: corpus.excluded,
		corpusSubsample: corpus.subsample,
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
			modelId: service?.modelId(),
			modelDir,
		},
		daemonHybrid: hybridService
			? { available: hybridService.isAvailable(), phase: hybridService.getState().phase }
			: undefined,
		timing,
		chunkMaxChars,
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
	/** Second service backed by a daemon-side BM25 hybrid store, for the
	 *  `daemon-hybrid` configs. Absent means those rows are omitted. */
	hybridService?: EmbsearchService;
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
		const results = await evaluateQuery(cwd, evalQuery, configs, service, options.hybridService);
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
