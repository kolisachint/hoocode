# Design Note: Hybrid retrieval — one `search` tool, RRF fusion

> **Naming note (superseded):** this note is the historical record of the
> decision and uses the tool's original name, `search`. That tool has since
> been renamed `SearchCodebase` (clean break, no alias), and the gating flag
> `--enable-search-tool` / `--enable-embsearchtools` is now
> `--enable-semantic-index`. Read every `search` below as `SearchCodebase`.


**Status:** All seven steps of the shipping order implemented in
`packages/coding-agent/src/core/search/` (rrf.ts, adapter.ts, mode.ts,
lexical-retriever.ts, context-assembler.ts, hybrid-search.ts, rerank.ts,
trace.ts, eval.ts) and `src/core/tools/search.ts`; `semantic_search` is
replaced by the unified `search` tool behind `--enable-search-tool` (legacy
alias `--enable-embsearchtools`). The eval gate ran with a full index —
see "Eval results" below for the measured numbers and the defaults they
set (`k = 2`, lexical fusion cap 20, rerank on). Step 7 shipped as a
deterministic lexical-statistical reranker; a cross-encoder can replace
its scoring function later. All work is TypeScript-side in hoocode. The
Rust daemon (`kolisachint/embeddingsearchtools`) needs **no changes** —
its protocol (`query` with `k`, ids + scores back) already supports
fetching a deeper top-k per retriever for fusion. Any future Rust work
(e.g. a cross-encoder op) happens in that repo, separately.

**Motivation:** hoocode has grep and a flag-gated `semantic_search`
(`packages/coding-agent/src/core/tools/semantic-search.ts`) as separate agent
tools, so the LLM is the only router between lexical and semantic retrieval —
and agents demonstrably underuse a separately-named semantic tool. Hybrid
retrieval (run both, fuse by rank) recovers the recall each retriever misses
alone. This note fixes the design: one unified tool, Reciprocal Rank Fusion
(RRF) as a transparent rank-only recall layer, explicit mode resolution,
diagnostics off the token path, and an eval gate before any reranker.

## TL;DR

- **One `search` tool** with `mode: auto | lexical | semantic | hybrid`
  (default `auto`) and an optional `glob` filter (e.g. `".ts"`,
  `"src/**/*.ts"` literal), registered only behind the existing opt-in flag.
  `semantic_search` does not survive as a separate tool — it becomes
  `mode: "semantic"`. The per-cwd service registry
  (`embsearch-service.ts:283-306`) already provides the wiring; this is a
  rename-and-widen, not new plumbing.
- **`grep` stays separate** (v1). It has a genuinely different contract —
  line matches with regex, globs, context lines — that doesn't map onto a
  ranked-retrieval interface. Lexical retrieval *inside* `search` uses grep
  as a backend anyway. Revisit full unification only with eval data.
- **RRF with `k = 2` default** (eval-gated), rank-only, deterministic. Raw
  BM25/cosine scores are retriever-local diagnostics, never fused.
- **The real correctness risk is the grep→chunk adapter**, not the fusion
  math: map, collapse, re-rank, *then* fuse — and never silently drop hits
  from files the embedding index doesn't cover.
- **Eval before rerank.** A cross-encoder only reranks the fused top-50 once
  Recall@K is demonstrably stable.

## Background: what exists today

- `semantic_search` tool (flag-gated via `--enable-embsearchtools`) returns
  `path:start-end (score)` lines from a local embedding index
  (`semantic-search.ts:119`).
- `EmbsearchService` orchestrates indexing/search over a stdio daemon; chunk
  ids are positional `${rel}#${chunkIndex}`, remapped to line ranges through
  a sidecar (`embsearch-service.ts:202,211`, `index-meta.ts`).
- The daemon client spawns once and stays hot; queries after startup are
  cheap (`embsearch/client.ts:5-8`).
- The service is frequently *unavailable by design*: flag off, repo under
  byte threshold, binary missing, mock backend, or still indexing
  (`embsearch-service.ts:41-46`). The current tool throws in that case.

## Decision 1 — Tool surface: one `search` tool, grep untouched

> **Superseded.** `grep`, `find` and `ls` have since been removed outright:
> `search` is the only dedicated discovery tool, and exact matching lines,
> counts, and directory listings are a shell job through `bash`. The rest of
> this section records the decision as it stood, and the `"grep"` retriever
> source below still names the lexical leg *inside* `search`, which is
> unchanged.

Three overlapping search tools would mean two stacked routers: the LLM
choosing a tool, then a heuristic choosing retrievers. Since the tool
description is hoocode's primary steering mechanism, collapse to:

| Tool | Job | Returns |
|------|-----|---------|
| `grep` | Exact line-level mechanics: "show every call site", counts, context lines | line matches |
| `search` | Ranked discovery in any mode: "find where X lives" | ranked `path:start-end` chunks |

Tool descriptions state the split explicitly ("use grep when you want
matching lines; use search when you want to find where something is").

Parameters for `search`:
  - `query` — identifier, error text, or natural-language description.
  - `mode` — `auto | lexical | semantic | hybrid` (default `auto`).
  - `glob` — optional path filter (e.g. `".ts"`, `"src/**/*.ts"`). Applied to
    ripgrep via `--glob` and to embedding hits via post-filter; the `glob`
    scopes results, it does not route mode selection.
  - `limit` — max results (default 10, max 30).

Gating: when the opt-in flag is off, no `search` tool is registered (current
`semantic_search` behavior). When on, a single `search` tool appears. The
flag should be renamed (e.g. `--enable-search-tool`) since it no longer just
gates embeddings.

## Decision 2 — Fusion: RRF, rank-only, deterministic

```typescript
export type RetrieverSource = "grep" | "embed";

export interface RankedHit {
  id: string;
  rank: number; // positive, 1-indexed
  score?: number;
  source: RetrieverSource;
}

export interface FusedHit {
  id: string;
  rrfScore: number;
  ranks: Partial<Record<RetrieverSource, number>>;
  rawScores: Partial<Record<RetrieverSource, number>>;
}

export function rrfFuse(lists: readonly RankedHit[][], k = DEFAULT_RRF_K /* = 2 */): FusedHit[];
```

Requirements (each caught in review of the first sketch):

- **Validate `k`**: finite and non-negative, else throw. `k = 0` is valid
  but intentionally top-heavy; negative `k` changes the algorithm.
- **Validate ranks**: positive integers only.
- **Per-list duplicate guard** (`source:id` seen-set): a retriever emitting
  the same chunk twice must not double its vote. This is a safety net — the
  adapter (Decision 3) dedupes upstream, so the guard should never fire in
  practice.
- **Best-rank retention** for diagnostics: keep the *best* rank per source,
  not the last-seen duplicate. Nit: `rawScores` follows the best rank, so a
  best-ranked duplicate without a score can shadow a worse-ranked one that
  had a score — harmless for diagnostics, worth a comment.
- **Deterministic ordering**: sort by `rrfScore` desc, then by number of
  agreeing retrievers desc, then `id.localeCompare`. Determinism matters in
  an agent harness — it removes "same query, different context" noise from
  debugging, compaction, and evals.
- **Never fuse raw scores.** BM25 and cosine similarity are not comparable;
  that is the whole reason to start rank-based. Raw scores are logged as
  retriever-local diagnostics only. A learned convex-combination path
  (`fusion: "rrf" | "cc"`) is a future extension behind the same contract,
  relevant only once labeled retrieval data exists.

## Decision 3 — The grep→chunk adapter (where the correctness risk lives)

Fusion needs a shared identity. The embedding side already has one:
`${rel}#${chunkIndex}` with line ranges in the sidecar
(`meta.files[rel].chunks`). Grep hits are line numbers; the sidecar makes
mapping a grep line into its enclosing chunk a binary search — nothing
needed from the Rust side.

Adapter pipeline, in order:

1. **Map** each grep line-hit to its enclosing indexed chunk id.
2. **Fallback identity** for the coverage hole: grep reaches files the index
   doesn't — excluded by the repo scan, repos under the byte threshold,
   chunks not yet embedded while `phase === "indexing"`. Synthesize
   `rel#L<line>` pseudo-ids for those so they enter fusion as single-source
   candidates. Without this, hybrid mode silently drops lexical-only files —
   the exact failure hybrid exists to prevent.
3. **Collapse** multiple line-hits in the same chunk to one candidate.
4. **Re-rank 1..N after collapsing.** If RRF is fed raw line-hit ranks with
   gaps, `1/(k + rank)` no longer means what the algorithm assumes and files
   with many matches get quietly penalized.
5. **Fuse.**

After fusion, token-budgeted context assembly (`assembleContext` in
`context-assembler.ts`) expands winning chunks
into exact line windows; only final compressed spans enter model context.
Retrieval identity and span expansion stay separate so an exact grep match
at line 411 and an embedding chunk at 390–440 share their evidence.

### Chunk-id stability caveat

`${rel}#${chunkIndex}` is stable **per index build**, not durable: an edit
early in a file shifts every subsequent chunk's index, and chunker/model
changes trigger a clean rebuild (`embsearch-service.ts:139-140`). This is
fine for fusion and expansion (both consult the same sidecar snapshot
within a query) but wrong for evals — see Decision 5.

## Decision 4 — Mode resolution: availability-first, minimal heuristics

The original proposal included a regex-heuristic query router
(identifier-like → lexical, conceptual wording → semantic). Rejected: the
heuristics are overfit (`\.\w+` matches any query containing a filename),
and the cost asymmetry is wrong — with a hot local daemon, running both
retrievers costs one extra local embedding query, i.e. nearly nothing,
while misrouting costs recall.

Resolution rules:

| Requested | Embed available | Resolved |
|-----------|-----------------|----------|
| `auto` | yes | `hybrid` — unless strong lexical signals (regex metacharacters or quoted strings) → `lexical` |
| `auto` | no | `lexical` |
| `hybrid` / `semantic` | no | degrade to `lexical`, reason recorded in trace — **never throw** (unlike today's `semantic_search`) |
| `lexical` | — | `lexical` |

The explicit `mode` param preserves the agent override: an agent
investigating "why does token overflow happen" may want semantic retrieval
even after learning the symbol name.

When `phase === "indexing"` the embedding list is partial, which biases
RRF's consensus toward whatever happens to be embedded. Still run hybrid,
but record the index phase in the trace so eval numbers from a half-built
index aren't trusted.

## Decision 5 — Diagnostics and evaluation

**Trace, off the token path.** The model sees only compact
`path:start-end` results. A full per-call trace goes to a jsonl sidecar in
the embsearch store dir (`getEmbsearchStoreDir`), not session events —
keeps session files lean per the token-efficiency goal:

```typescript
export interface SearchTrace {
  query: string;
  requestedMode: "auto" | "lexical" | "semantic" | "hybrid";
  resolvedMode: "lexical" | "semantic" | "hybrid";
  degradedReason?: string;
  indexPhase: "ready" | "indexing" | "unavailable";
  rrfK?: number;
  retrievers: Partial<Record<RetrieverSource, { latencyMs: number; hitCount: number }>>;
  fused: Array<Pick<FusedHit, "id" | "rrfScore" | "ranks" | "rawScores">>;
  rerank?: { applied: boolean; candidateCount: number; latencyMs?: number };
}
```

**Eval gate before any reranker.** A compact eval set with query classes
that reflect actual agent work:

- exact symbol (`parseTokenStream`), import/path, error-message fragment,
  conceptual behavior ("how does compaction preserve call chains?"),
  cross-file architecture, Rust/TS boundary questions.

Measure Recall@5/10 for lexical, semantic, hybrid at `k ∈ {0, 2, 10, 60}`,
plus resolved-auto. If the gold span never reaches the fused top-50, no
reranker can rescue it. RRF's `k` is parameter-sensitive in the literature
(Bruch et al., [An Analysis of Fusion Functions for Hybrid
Retrieval](https://arxiv.org/abs/2210.11934)) — sweep it rather than trust
the universal-robustness claim.

**Gold answers are `path` + line range matched by span overlap, never
chunkId equality.** A chunkId-keyed gold set rots the first time the repo
or the chunker changes (see stability caveat above).

## Eval methodology (rebuilt 2026-08-08)

The first eval round produced the table below and then stopped working. Four
problems, all fixed in the harness rather than in retrieval:

1. **The gate did not run.** `EVAL_CONFIGS` lost its `export` to a knip
   dead-export sweep (`02efaab`) — the only importer was a `.mjs` script
   reading `dist/`, which static analysis cannot see. The harness is now
   `scripts/search-eval.ts` importing from `src/`, `test/search-eval.test.ts`
   imports the same surface, so the same sweep now breaks a test instead of
   silently disabling the gate.

   The remaining protection — proving the harness still *executes* — wants a
   CI job. It is not in `ci.yml` because the session that wrote this could
   not push workflow changes (the OAuth token lacks `workflow` scope); add
   it with a token that has one:

   ```yaml
     search-eval:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - name: Setup bun
           uses: oven-sh/setup-bun@v2
           with:
             bun-version: '1.3.13'
         - name: Install dependencies
           run: bun install --frozen-lockfile
         - name: Check gold set is pinned to the current tree
           run: cd packages/coding-agent && bun run search-eval:gold
         - name: Run eval harness (lexical only)
           run: cd packages/coding-agent && bun run search-eval -- --no-embed
   ```

   Lexical-only by design: the semantic rows need the embsearch binary,
   whose release download would make CI depend on a third-party host. What
   CI protects is the harness and the gold set; a scored baseline is
   produced deliberately, not per-PR.
2. **The corpus was unpinned.** Retrieval is measured over this repo, so
   every commit moved the corpus and no two runs were comparable. Runs now
   take `--corpus-ref <sha>` and execute in a detached worktree; a run
   without one is stamped `corpusFromWorkingTree` in the record.
3. **The metric was not what the doc claimed.** No gold entry carried line
   numbers, so `spanMatchesGold` short-circuited to a path comparison: the
   table below is *file-level* recall, not span overlap. All 68 spans now
   carry ranges plus an `anchor` (a literal that must sit inside the range,
   never used for scoring) so the set re-pins after a refactor via
   `bun run search-eval:gold -- --fix` and fails loudly when it drifts.
   Recall@1 and MRR are recorded because an agent reads the top result or
   two — a distinction Recall@5 cannot make.
4. **Nothing was recorded.** Numbers were hand-copied here with no corpus
   SHA, embedder identity, or index state. Runs now emit a run record
   (`test/fixtures/search-eval-baseline.json`) carrying all of it, including
   a content hash of the retrieval sources so a tuning change invalidates
   old numbers automatically.

The gold set grew from 12 queries to **62 (68 spans, 28 files, 16
subsystems)**, weighted toward exact-symbol (2 → 22) as the class that most
separates a lexical retriever from an embedding one, and no longer
concentrated in the `core/search` subsystem it is measuring (5 of 28 files).

**What this means for the table below:** it was measured on an unpinned
corpus, with a file-level metric, over 12 near-binary queries. At n=12 the
binomial standard error is ~14pp, which is wider than every gap in it —
including the ones that set `k = 2`, `LEXICAL_FUSION_CAP = 20`, and
rerank-on. Those defaults are unchanged here (this work touches no retrieval
behaviour), but they should be treated as unvalidated until re-measured on
the current harness, not as settled results. The baseline immediately below
is that re-measurement, and it reverses the `k` decision.

## Baseline (2026-08-08, corpus `ffeaad9`, embsearch 0.1.7 / MiniLM int8)

> **Every absolute rate in this section is depressed.** The corpus included
> the eval's own gold fixture, which contains all 62 query strings verbatim,
> so a fifth of the top-10 window was spent on the eval reading itself. See
> [The eval was reading its own fixture](#the-eval-was-reading-its-own-fixture)
> for what it cost and the corrected numbers. The *comparisons* below stand:
> both arms of each faced the same corpus.

```
bun run search-eval -- --corpus-ref ffeaad9 \
  --embsearch-binary ./embsearch --daemon-hybrid \
  --out test/fixtures/search-eval-baseline.json
```

62 queries, 68 gold spans, 17,158 chunks indexed, `all-MiniLM-L6-v2-int8`,
flat index. Re-running the same command reproduces the record exactly:
determinism was verified byte-for-byte on an earlier corpus, and the
pipeline has no nondeterministic step (ripgrep runs `--sort path`, fusion
and reranking break ties deterministically).

| config | R@1 | R@5 | R@10 | R@50 | MRR |
|---|---|---|---|---|---|
| lexical | 2% | 34% | 42% | 52% | 0.160 |
| semantic | 10% | 40% | 48% | 65% | 0.234 |
| hybrid k=0 | 6% | 48% | 56% | 77% | 0.214 |
| hybrid k=2 | 8% | 48% | 56% | 77% | 0.237 |
| hybrid k=10 | 8% | 46% | 56% | 77% | 0.235 |
| hybrid k=60 | 8% | 46% | 57% | 77% | 0.234 |
| auto | 8% | 46% | 57% | 77% | 0.234 |
| lexical +rr | 16% | 52% | 52% | 52% | 0.316 |
| semantic +rr | 37% | 53% | 56% | 65% | 0.436 |
| hybrid k=2 +rr | 24% | 60% | 66% | 77% | 0.413 |
| hybrid k=60 +rr | 34% | 60% | 66% | 77% | 0.464 |
| auto +rr | 34% | 60% | 66% | 77% | 0.464 |
| daemon-hybrid | 8% | 41% | 57% | 85% | 0.232 |
| daemon-hybrid +rr | 35% | 62% | 72% | 85% | 0.475 |
| **bm25+dense +rr** | **34%** | **60%** | **72%** | **85%** | **0.468** |
| 3-way +rr | 26% | 56% | 64% | 77% | 0.404 |

R@10 by query class, the part the aggregate hides:

| class | n | lexical | semantic | hybrid k=60 | semantic +rr | bm25+dense +rr |
|---|---|---|---|---|---|---|
| exact-symbol | 22 | 73% | 68% | 86% | 82% | 100% |
| error-fragment | 10 | 100% | 50% | 100% | 50% | 100% |
| path | 6 | 0% | 50% | 33% | 67% | 83% |
| conceptual | 14 | 0% | 36% | 29% | 36% | 36% |
| cross-file | 6 | 0% | 25% | 8% | 33% | 25% |
| boundary | 4 | 0% | 0% | 0% | 25% | 25% |

### What the larger gold set changes, and what it shipped

Aggregate means mislead here, because every config scores the *same* 62
queries. `bun run search-eval:compare -- "<A>" "<B>"` runs a paired sign
test over them, and `--against <record>` compares one config across two
runs. Three retrieval changes came out of it, each measured on a pinned
corpus with everything else held fixed.

**1. `auto` no longer routes quoted queries to lexical** (`mode.ts`). All
ten error-fragment queries are quoted, and quoting sent them to the weakest
leg: R@1 0.000 and MRR 0.483 under lexical against 0.600 and 0.800 for the
same queries in hybrid. Metacharacters *inside* a quoted span no longer
count either — `buildLexicalQueryPlan` escapes a quoted segment and matches
it verbatim, so the parentheses in `"initTheme() first."` are literal text,
not a signal about intent. Measured on `auto +rr`: R@1 0.145 -> 0.242, 6
better, 0 worse (p<=0.05), R@10 completely unchanged. A pure ranking gain.

**2. The reranker learned to tell a declaration from a call site**
(`rerank.ts`). The largest gap in the eval was that on the 22 exact-symbol
queries the definition sat in the top 10 about 85% of the time but ranked
first only about 20% of the time. Call sites outnumber definitions and
carry the identical identifier, so term coverage saturates at 1.0 for both.
A window that *declares* a query term now gets an additive bonus. Measured
on `daemon-hybrid +rr`: R@1 0.177 -> 0.323, 9 better, 0 worse (p<=0.05).

Two smaller fixes ship alongside and earn less: the fused prior normalizes
the RRF score instead of using a uniform `1 - index/length` ramp that threw
retriever agreement away, and term coverage is IDF-weighted over the
candidate pool rather than counting every term equally. An ablation with the
declaration bonus at zero returns scores to roughly the old reranker, so
that signal is doing the work; the other two move R@10 without moving R@1
or MRR.

**3. `k` rose from 2 to 60 — but only after (2) landed.** The 12-query set
picked k = 2 on a measurement that was never reproducible. The 62-query
re-sweep found k = 2 and k = 60 *indistinguishable*: a 3pp R@10 gap carried
by two queries (p = 0.50), MRR worse on more queries than better. The
default was deliberately left alone rather than churned on noise. Fixing the
reranker is what made k decidable — once it could exploit the tail, the
deeper, flatter mix k = 60 produces became worth having: MRR 0.403 -> 0.464,
20 better against 7 worse (p<=0.05), R@10 unchanged. **`k` was never really
a question about fusion; it was a question about what the reranker could
use.**

Cumulative effect on `auto +rr`, the shipped default path: R@1 15% -> 34%,
R@5 51% -> 58%, R@10 60% -> 67%, MRR 0.315 -> 0.464.

### The BM25 leg, fused here (embsearch 0.2.0)

embsearch 0.2.0 serves `retriever: "lexical"`, so BM25 arrives as its own
ranked list instead of pre-fused. Fusing it here rather than in the daemon
means our `k`, n-way fusion, and per-leg ranks surviving into the trace.

Two things had to be true for that to be worth doing, and both were measured:

- **Parity.** TS-side `bm25+dense +rr` against daemon-side
  `daemon-hybrid +rr` ties exactly on R@10 (0 better, 0 worse, 62 tied) and
  is indistinguishable on MRR (5 better, 7 worse, p = 0.77). Getting there
  needed the pool depth fix — fetching 200 per leg rather than 50, matching
  the daemon's internal `4·k`. Before that, TS-side fusion lost 12pp R@10
  and 8pp R@50 purely to candidates falling outside a too-shallow pool.
- **The grep leg is now a significant regression.** Three-way
  dense + BM25 + grep against dense + BM25 is 7 better and 23 worse on MRR
  (p <= 0.05), 0 better / 5 worse on R@10. Grep earned its place as a
  stand-in for a real lexical index; with one present it contributes noise.

Defaults are unchanged. Dropping grep would be premature: the eval corpus is
a clean checkout, so it structurally cannot test the one thing grep uniquely
does — seeing files the index has not indexed, including edits made during
the session. Measuring that needs a corpus that diverges from the index,
which the harness cannot currently build.

### Live edits: what the clean-checkout sweep cannot see

`bun run search-eval -- --live-edits` writes files into the corpus **after**
indexing and scores queries only that content can answer — standing in for
the files an agent creates and edits while it works. The daemon has no file
watcher, so anything written during a session is invisible to dense and BM25
until the next index build.

| config | R@1 | R@10 | MRR |
|---|---|---|---|
| lexical (grep) | 100% | 100% | 1.000 |
| semantic | 0% | 0% | 0.000 |
| daemon-hybrid +rr | 0% | 0% | 0.000 |
| bm25+dense +rr | 0% | 0% | 0.000 |
| hybrid k=60 +rr (grep + dense) | 100% | 100% | 1.000 |
| 3-way +rr (grep + dense + BM25) | 100% | 100% | 1.000 |

**Every index-backed configuration scores zero, by construction.** Any
configuration containing the grep leg scores perfectly. This is the
comparison the main sweep structurally could not make, and it settles the
question the BM25 work opened.

### Why the default stays grep + dense

Putting both halves together, for the three candidate defaults:

| configuration | clean corpus (MRR) | live edits (MRR) |
|---|---|---|
| **grep + dense — current default** | **0.464** | **1.000** |
| bm25 + dense | 0.468 | 0.000 |
| 3-way (grep + dense + BM25) | 0.404 | 1.000 |

BM25's advantage over grep on indexed content is **not significant** (R@10
7 better / 3 worse, p = 0.34; R@50 7 / 1, p = 0.07; MRR 12 / 16, p = 0.57),
and switching to it would make the agent blind to its own edits. Adding BM25
*alongside* grep costs 0.060 MRR on the clean corpus and buys nothing on
live edits, since grep already covers them.

So the shipped default is already the best available configuration, and the
earlier reading — "the grep leg is a significant regression once BM25 is
present" — was true only of the half of reality the eval could see. Grep is
not a stand-in for a real lexical index; it is the only retriever that sees
the working tree.

`bm25Leg` stays available for callers who want the depth-50 recall (85% vs
77%) and can tolerate an index-lagged view.

### Cross-encoder reranking: measured, and rejected as a replacement

embsearch 0.3.0 bundles `ms-marco-MiniLM-L-6-v2` and serves a `rerank` op, so
the fused shortlist can be reordered by a model that reads query and candidate
together. Scored against the same retrieval each deterministic `+rr` row uses,
so the delta is the reranker alone.

| config | R@1 | R@10 | MRR |
|---|---|---|---|
| semantic +rr | **39%** | 56% | **0.444** |
| semantic +ce | 21% | 48% | 0.295 |
| auto +rr | **34%** | **66%** | **0.466** |
| auto +ce | 18% | 60% | 0.316 |
| bm25+dense +rr | **34%** | **70%** | **0.468** |
| bm25+dense +ce | 16% | 56% | 0.304 |

**The cross-encoder loses, significantly.** R@1 is worse on all three
(2/13, 4/14, 4/15 better/worse, p<=0.05) and MRR on two of three. It still
beats *no* reranking (semantic 0.228 -> 0.295), so it is doing something —
just far less than a small amount of code-aware structure.

The per-class MRR says why, and it is not a close call:

| class | n | semantic +rr | semantic +ce |
|---|---|---|---|
| exact-symbol | 22 | **0.742** | 0.555 |
| error-fragment | 10 | **0.500** | 0.267 |
| path | 6 | **0.667** | 0.080 |
| conceptual | 14 | 0.106 | **0.179** |
| cross-file | 6 | **0.087** | 0.052 |
| boundary | 4 | **0.042** | 0.025 |

`ms-marco-MiniLM` is trained to rank web passages against natural-language
questions. Conceptual queries are exactly that, and it wins there. Everything
else in this gold set is an *exact-match* problem — an identifier, a quoted
error string, a filename — which is out of distribution for a passage ranker
and precisely what the deterministic signals were built for. On path queries
it collapses (0.667 -> 0.080): the reranker has an exact-path bonus, and the
model has no notion that the query *is* a filename.

### The more interesting finding: deterministic reranking hurts conceptual queries

Reading down the conceptual column: plain `semantic` scores MRR 0.230,
`semantic +rr` scores **0.106**. Reranking makes conceptual queries
*materially worse than not reranking at all*, and `auto` is the same story
(0.239 -> 0.072). The declaration bonus and path affinity are identifier
signals; applied to a natural-language query they promote whatever happens to
contain a matching token.

That is a live regression in the shipped default, worth more than the
cross-encoder question that surfaced it.

Nothing adopts the cross-encoder as a default on this evidence. It stays
opt-in (`crossEncoder`), and the ~23 MB it adds to the binary is not yet
earned.

#### Fix: gate the declaration bonus on query shape

Two candidate fixes were on the table — suppress the identifier signals on
non-identifier queries, or route conceptual queries to the cross-encoder. The
first was tried, because it costs nothing at run time and does not depend on
a model that has not earned its place in the binary.

A query counts as prose when it contains two or more function words
(`queryIsProse` in `rerank.ts`). On this gold set that classifier is exact:
all 24 exact-symbol / error-fragment / path queries are names, all 24
conceptual / cross-file / boundary queries are prose.

Two variants were measured against the same corpus, index and gold set
(62 queries, corpus `974bf58d`, embsearch 0.3.0), differing only in which
signals the gate suppresses. Per-class MRR, `A` = today's shipped scoring:

| class | n | A | B: gate declaration | C: gate declaration + path |
|---|---|---|---|---|
| exact-symbol | 22 | 0.742 | 0.742 | 0.742 |
| error-fragment | 10 | 0.500 | 0.500 | 0.500 |
| path | 6 | 0.667 | 0.667 | 0.667 |
| conceptual | 14 | 0.124 | **0.170** | 0.170 |
| cross-file | 6 | 0.089 | 0.057 | 0.032 |
| boundary | 4 | 0.104 | **0.188** | 0.175 |

(`semantic +rr`; `auto +rr` and `bm25+dense +rr` show the same shape, with
conceptual +0.040 and +0.056 respectively.)

**B ships.** The three name-shaped classes are bit-identical under both
variants — by construction, since the gate never fires on them — so the
comparison is confined to the classes it targets. B takes conceptual up
+0.046 and boundary +0.083 for a cross-file cost of −0.032; C buys no extra
conceptual gain and doubles the cross-file loss. Path affinity is a *topic*
signal, not a name signal: "how does a grep line number become an embedding
chunk id" genuinely wants files with `grep` and `chunk` in the path, so
gating it throws away evidence that prose queries can use.

Overall MRR moves +0.007 to +0.013 depending on config, which the paired sign
test does **not** call significant (best p = 0.109, 8 better / 2 worse on
`auto +rr`). The justification is not the aggregate — it is that the change
is free for three of six classes and directionally right for the other three.

**This halves the regression rather than removing it.** Reranked conceptual
is still below un-reranked conceptual (0.170 against 0.246). The remaining
gap is the open question: for prose queries the deterministic reranker has
nothing left but term coverage, and term coverage is not what orders a
behavioural question correctly. This is the one class where the cross-encoder
won (0.106 -> 0.179), so a query-shape router between the two rerankers is
the obvious next experiment.

### What is still open

- **Hybrid and semantic are now indistinguishable on rank.** Reranked
  semantic against reranked hybrid is 10 better / 21 worse on MRR but with a
  higher mean (p = 0.071) — hybrid wins big on a few queries and loses
  slightly on many. The earlier finding that hybrid was *significantly*
  worse on rank no longer holds: the declaration bonus fixed the mechanism
  causing it, which was precisely the demotion of definitions by call sites.
- **BM25's recall lead is the one unambiguous result** and is unchanged by
  any of this — see below.
- **Ranking is still the binding constraint.** `daemon-hybrid +rr` reaches
  R@50 87% against R@10 74%, so roughly an eighth of the gold set is
  retrieved but not surfaced. That is what a cross-encoder would attack.

### Daemon BM25 as the lexical leg

The `daemon-hybrid` configs replace the ripgrep leg entirely: the Rust
daemon fuses its own Okapi BM25 index with the vectors and returns one
ranking. Scored on the same 62 queries, same corpus, same run
(`bun run search-eval -- --daemon-hybrid ...`).

| config | R@1 | R@5 | R@10 | R@50 | MRR |
|---|---|---|---|---|---|
| semantic +rr | 37% | 53% | 56% | 65% | 0.436 |
| hybrid k=60 +rr (ripgrep) | 34% | 60% | 66% | 77% | 0.464 |
| **bm25+dense +rr (BM25)** | **34%** | **60%** | **72%** | **85%** | **0.468** |

**BM25 is a significantly better lexical leg, and the entire win is in the
candidate pool.** R@50 goes 79% -> 90% against the ripgrep leg (7 queries
better, 0 worse, p<=0.05) and 69% -> 90% against semantic alone (14 better,
1 worse, p<=0.05). R@10 (+2.4pp, p=0.75) and MRR (-0.001, p=1.00) are
statistically tied.

That split is the actionable part: BM25 puts far more gold spans within
reach, and the deterministic reranker cannot convert the extra reach into
better ordering. The step-7 gate said "a gold span that never reaches the
fused top-50 cannot be rescued by any reranker" — 90% of them now do, which
is precisely the precondition that makes a real cross-encoder worth
building. Ranking is now the binding constraint, not retrieval.

It also fixes the dilution that ripgrep fusion causes. R@50 by class:

| class | n | semantic +rr | hybrid k=60 +rr | bm25+dense +rr |
|---|---|---|---|---|
| exact-symbol | 22 | 82% | 95% | 100% |
| error-fragment | 10 | 50% | 100% | 100% |
| path | 6 | 67% | 67% | 83% |
| conceptual | 14 | 71% | 64% | 71% |
| cross-file | 6 | 42% | 42% | 58% |
| boundary | 4 | 25% | 25% | 50% |

The ripgrep leg drags conceptual recall from 79% down to 64%; BM25 keeps it
level with semantic while adding the exact-term classes. Boundary — 0% in
every config of the previous baseline — reaches 75% at depth 50 under BM25.
Unweighted substring matching contributes noise where it has nothing to
say; IDF-weighted term matching mostly does not.

The one place ripgrep still wins is exact-symbol at R@10 (91% vs 82%):
regex substring matching finds an identifier written in a different case or
convention, which BM25's tokenizer (split on non-alphanumeric, lowercase)
cannot. `parse_command_args` tokenizes to `["parse","command","args"]`
while a `parseCommandArgs` query tokenizes to one token that matches
neither.

**Caveat on what was measured.** v0.1.7 exposes no BM25-only query.
`query_hybrid` fuses BM25 with the vectors inside Rust using its own
hardcoded RRF constant of 60, so this compares *systems* — BM25+dense fused
in Rust against ripgrep+dense fused in TypeScript — not two lexical legs
under identical fusion. Isolating the BM25 leg so TypeScript can fuse it
n-way and keep per-source diagnostics needs a new daemon op. That op is the
natural next request on the Rust side.

### Where hybrid actively hurts

Fusion is not a free win, and the per-class table is where that shows:

- **conceptual**: semantic +rr 36% against hybrid k=60 +rr 29%.
- **cross-file**: semantic +rr 42% against hybrid k=60 +rr 8%.

On queries with no exact term to anchor on, the lexical leg contributes
noise that displaces correct embedding hits — the dilution the design note
already identified and capped at 20 candidates, still present at the classes
where lexical has nothing useful to say. Hybrid wins the exact-symbol and
error-fragment classes outright (86% and 100% R@10), so per-class routing,
or a lexical leg with IDF instead of ripgrep's unweighted substring matching,
is where the remaining headroom is.

The IDF half of that has now been measured — see "Daemon BM25 as the lexical
leg" above. Swapping ripgrep for the daemon's BM25 index removes the
conceptual and cross-file dilution and lifts R@50 from 79% to 90%, without
yet improving R@10 or MRR.

**boundary is 0% everywhere, in every config.** Four queries that no
retriever reaches — worth treating as a separate problem from tuning.

## The eval was reading its own fixture

Two boundary-class queries sat at 0% Recall@10 in every configuration ever
measured. Probing them showed the gold span was absent from the top **50**, not
merely buried — a recall failure no reranker can reach. The top result for
"which protocol operation reports the model id and the vector count" was
`test/fixtures/search-eval.json` itself.

The gold fixture lives in the repository the eval searches, and it contains all
62 query strings verbatim, so every query is a perfect lexical match against
its own entry. The design note you are reading has the same problem: it quotes
the queries while discussing the classes they belong to.

Measured across the 62 queries, before any exclusion existed:

| | |
|---|---|
| queries with a self-referential file in the top 10 | **56 / 62** |
| queries whose #1 result is self-referential | **27 / 62** |
| top-10 slots consumed by self-referential files | **133 / 620** |

`pinCorpus` now removes those files from the pinned worktree before indexing
(`CORPUS_EXCLUSIONS`), records the list in the run's provenance, and never
touches the live working tree — a measurement is not worth deleting a file
from someone's checkout. `search-eval-compare` warns when two records carry
different exclusion lists, because that delta is not a retrieval delta.

### What it was costing: ranking, not recall

Same corpus SHA (`974bf58d`), same index, same gold, embsearch 0.3.0 — the
only difference is the four excluded files:

| config | R@1 | R@10 | MRR |
|---|---|---|---|
| semantic +rr | 40% → **44%** | 60% → 63% | 0.465 → **0.497** |
| auto +rr | 35% → **53%** | 68% → 69% | 0.478 → **0.583** |
| bm25+dense +rr | 35% → **56%** | 69% → 69% | 0.479 → **0.608** |

Recall@10 barely moves — the right answers were already being retrieved. What
the fixture was taking is the *top of the window*. On `bm25+dense +rr`, MRR
improves on 30 of 62 queries and worsens on 1 (p < 0.0001) while Recall@10
changes on **zero**; Recall@1 gains 13 queries and loses none (p = 0.0002).

Per class on `auto +rr` (MRR): exact-symbol 0.701 → 0.888, error-fragment
0.783 → 0.950, boundary 0.146 → 0.250, conceptual 0.118 → 0.133, cross-file
0.029 → 0.041, path unchanged at 0.667.

The comparative conclusions in this document survive — both arms of every
comparison faced the same contaminated corpus, so the paired tests measured
what they claimed. The headline rates were not the system's true rates.

### A gold-set bug found on the way, which changed nothing

`resolveExtent` closed a block at the first bracket in *column 0*, which is the
shape of a top-level declaration and wrong for every class member: a method's
own indented `}` was skipped, so the span ran to the end of the enclosing
class. Three boundary gold spans resolved to 80, 34 and 28 lines for methods
that are 7, 4 and 4 lines long, all ending on the class's closing brace.

Re-pinned against a resolver that matches the closer to the opener's
indentation, five spans narrowed — and **every score was identical to three
decimal places**. Retrieval chunks are coarse enough (~48 lines) that a chunk
overlapping the narrow span overlapped the wide one too. The bug was real, the
distortion was not. Fixed anyway, with a regression test; the next gold set may
not be so lucky.

### What is left in the boundary class

Decontamination did not rescue the two stuck queries. `boundary-daemon-info`
and `boundary-mock-embedder` are still 0% at Recall@10 in every config, and
still absent from the top 50 rather than buried in it. Their gold spans are
four- and three-line method bodies whose surrounding text paraphrases the query
almost exactly — `/** Model id, dimensionality, and live vector count of the
daemon. */` sits one line above the `async info` gold span, and the query is
"which protocol operation reports the model id and the vector count".

A span that well described and still unreachable points at chunking or
embedding, not at fusion or ranking. That is the next thing to look at, and the
first retrieval question in this document that the reranker work cannot answer.

## Shipped: BM25 is the default lexical leg

The store now carries a BM25 index by default, and search fuses BM25 + dense +
grep-scoped-to-stale-files. Measured at HEAD, `--daemon-hybrid --live-edits`:

| config | R@1 | R@10 | R@50 | MRR | live edits |
|---|---|---|---|---|---|
| `auto +rr` (grep + dense, the old default) | 53% | 63% | 77% | 0.566 | 1.000 |
| `bm25+dense +rr` (no grep at all) | 55% | 69% | 83% | 0.599 | **0.000** |
| `3-way +rr` (**the new default**) | **55%** | **69%** | **83%** | **0.599** | **1.000** |

The new default matches BM25-only on the indexed corpus and grep-only on files
written after indexing. Neither is traded, because the legs no longer overlap:
grep narrows to what `staleFiles()` reports the index has not read.

One of the two stuck boundary queries fell out of this. `boundary-daemon-info`
— "which protocol operation reports the model id and the vector count" — goes
from 0% to 100%, because its answer sits under a doc comment that paraphrases
the query almost word for word. That is a lexical match BM25 catches and the
bi-encoder never placed. It is a useful reminder of what the two legs are for:
the remaining failure, `boundary-mock-embedder`, has no such literal overlap.

### The migration

Hybrid-ness is fixed when a store is created, and passing `--hybrid` at an
existing plain store only warns — so an index built before this would silently
stay dense-only while every BM25 query against it failed. `EmbsearchService`
therefore asks the store itself (`info.hybrid`) rather than trusting the
sidecar, and rebuilds once when it disagrees. Verified end to end: a dense-only
store reopens, rebuilds, serves BM25, and stays stable on the next open.

Users pay one full re-index. The chunker line-range fix rides along with it,
since it needed a `CHUNKER_VERSION` bump it could not justify on its own.

## Chunking: two hypotheses, both rejected

The two boundary queries that survived decontamination are absent from the top
50, not buried in it. The covering chunk for `boundary-daemon-info` holds
`async info()` *and* its doc comment — "Model id, dimensionality, and live
vector count of the daemon", which paraphrases the query almost word for word
— alongside `bulk`, `remove` and their comments. One vector for six methods.
Dilution was the obvious suspect.

**Halving the chunk (30 lines / 500 chars, overlap 5) made it worse.** Measured
at the same corpus with a full re-index, `auto +rr`:

| | R@1 | R@10 | R@50 | MRR |
|---|---|---|---|---|
| 60 lines / 1000 chars | 53% | 66% | **77%** | 0.577 |
| 30 lines / 500 chars | **55%** | 62% | 69% | 0.577 |

Recall@1 ticks up — that is the sharpening working — and reach falls off a
cliff. The boundary class went from 2 of 4 findable to **0 of 4**. The reason
is mechanical: a fixed top-k over smaller chunks retrieves less *content*. 50
chunks at ~460 characters sees half the corpus that 50 at ~930 does. Sharper
chunks need a deeper k to break even, and that is a different experiment.

Overlap has to scale with chunk size, incidentally: leaving it at 10 lines
against 30-line chunks advanced the window three lines at a time and grew the
index 5x, to 85k chunks.

### A real chunker bug worth exactly nothing

`chunkFile` trimmed the *joined* chunk text but recorded the *untrimmed* line
range, so a window with blank lines at either end claimed lines whose text was
never embedded. It affected **31% of chunks** in this repo, by up to 3 lines
each — 5,848 lines claimed but not indexed.

Fixing it (trim whole blank lines off both ends, narrow the range to match) is
unambiguously correct and measured **0 better, 0 worse on all four metrics**,
every query tied. Blank lines carry no embedding signal, and 1–3 lines against
a 48-line chunk rarely changes an overlap decision.

That leaves it a fix that costs every user a full re-index — `CHUNKER_VERSION`
must bump — for no measured gain. It should ride along the next time something
else earns the rebuild, which the hybrid BM25 store will.

### What this leaves

Neither chunk size nor chunk metadata explains the boundary failures. The
answer text is present, well described, and in the index; the bi-encoder simply
does not place it near this query. That points at the embedder — a 384-dim
MiniLM asked to relate "which protocol operation reports…" to a method
signature — rather than at anything the retrieval pipeline controls. Testing it
means swapping the embedding model, which is the first open question here that
neither fusion, reranking, nor chunking can reach.

**Answered, 2026-09-05, and the answer was "probably, but not on this
evidence."** bge-small-en-v1.5 was measured against MiniLM over the arms
pre-registered in embeddingsearchtools' `docs/embedder-strategy.md`. It beat
MiniLM on every endpoint — semantic-subgroup MRR 0.173→0.273, R@10
0.333→0.479 — and cleared none of them at p≤0.05. Records:
`packages/coding-agent/runs/a{0,1,2}-*.json`. The embedder hypothesis above
survives its own falsification test; it is not proven.

The blocker moved. It is no longer the model but **this gold set**: 42 of 62
queries tied, so the eval cannot resolve a difference of that size. Growing the
semantic classes — conceptual, cross-file, boundary, 24 queries between them
and where the entire effect lives — is now worth more than another model.

### Parked: the index-recovery path has no CI coverage

`EmbsearchService.start` recovers from a store the daemon refuses to open, by
confirming through `embsearch store-info` and rebuilding. It is exercised only
by `scripts/verify-store-recovery.ts`, which needs a real embsearch binary and
so cannot run in CI — the binary is downloaded on demand and the default build
is a mock the service rejects outright.

That is how the original bug shipped: nothing on any PR touches this path. The
fix is a stub daemon speaking enough of the NDJSON protocol to drive `start()`,
including a `store-info` that reports a foreign `model_id`. Not built here
because a half-finished fake daemon is worse than an honest gap.

## Eval results (2026-07-18, full index: 16.5k chunks over this repo)

Superseded — kept for the decision history, not as evidence. Measured with
the since-removed `scripts/search-eval.mjs` over the 12-query gold set with
the embsearch binary auto-downloaded and the repo fully indexed, on an
unpinned corpus with the file-level metric described above. `+rr` = reranked
(step 7). The tool's default path is `auto +rr`:

| config | R@5 | R@10 | R@50 |
|---|---|---|---|
| lexical | 33% | 42% | 50% |
| semantic | 67% | 83% | 92% |
| hybrid k=0 | 54% | 67% | 92% |
| hybrid k=2 | 54% | 67% | 92% |
| hybrid k=10 | 46% | 67% | 92% |
| hybrid k=60 | 46% | 58% | 92% |
| **auto (k=2) +rr** | **63%** | **83%** | **92%** |
| hybrid k=2 +rr | 63% | 83% | 92% |
| hybrid k=60 +rr | 63% | 75% | 92% |
| semantic +rr | 67% | 83% | 92% |

Decisions taken from these numbers (each reversible via the same harness):

- **Uncapped lexical lists diluted hybrid below plain semantic** (first
  measurement: hybrid R@5 42–54% vs semantic 71%). RRF weighs a rank-30
  lexical candidate like a rank-30 embedding hit, but lexical precision is
  front-loaded. Fix: only the top 20 adapted lexical candidates vote in
  hybrid fusion (`LEXICAL_FUSION_CAP`).
- **`k = 2` replaced `k = 60` as the default**: small k beat 60 on every
  differing query, twice, with and without reranking — consistent with the
  parameter-sensitivity finding in Bruch et al. rather than the k=60
  folklore. Small sample (12 queries); re-sweep when the gold set grows.
- **Reranking is on by default**: it never hurt any config and lifted
  hybrid R@10 from 67% to 83%.
- **`auto` no longer routes path-like queries to lexical**: they scored 0%
  lexically (content grep cannot find a file by its own name) and 100% in
  hybrid, where the reranker's exact-path bonus carries them. Strong
  lexical signals are now only quotes and regex metacharacters.
- **Determinism bug caught by the harness:** ripgrep's parallel walk
  returned a different hit subset per run once the 200-match cap truncated
  the stream, so identical configs scored differently. Fixed with
  `--sort path`; degraded rows must equal the lexical row exactly — a
  standing invariant check.

Known misses (recorded, not hidden): `concept-token-budget-expansion` is
found by semantic alone but lost in hybrid fusion at every `k`; and
`boundary-daemon-protocol` reaches no config's top-50 — a genuinely hard
query that only a better embedder or a real cross-encoder could recover.

## Step 7 as shipped: deterministic reranker

`core/search/rerank.ts` re-orders the fused top-50 (on by default,
`rerank: false` to disable) using evidence that is only cheap to compute
after fusion: distinct-term coverage of the candidate's actual expanded
window read from disk, path affinity (query terms in the file path) with an
exact-path bonus, and the fused RRF ordering as prior. Weights live in one
place and are tuned only through the eval harness. No model, no network,
fully deterministic; a cross-encoder can later replace the scoring function
behind the same signature — that model work belongs to
`kolisachint/embeddingsearchtools`.

## Shipping order (v1 boundary)

1. Stable per-build chunk ids + parallel grep/embed retrieval behind the
   opt-in flag.
2. Grep→chunk adapter: map, fallback ids, collapse, re-rank (the
   correctness-critical step).
3. `rrfFuse(..., k)` with validation, dedupe guard, deterministic
    tie-break, trace logging; default `k = 2` from the eval gate.
4. Unified `search` tool with availability-aware mode resolution replacing
   `semantic_search`; flag renamed.
5. Token-budgeted context assembly (`assembleContext`) reading line windows
   only after fusion.
6. Recall@K evals + `k` sweep (span-overlap gold set).
7. Only after recall is stable: rerank the fused top-50; `fusion: "cc"` as
   a labeled-data follow-up.

## Explicitly deferred

- ~~Folding `grep` into `search`~~ — done, and further: `grep`/`find`/`ls`
  were removed rather than folded in. `search --mode lexical` plus `bash`
  covers what they did.
- Cross-encoder reranking (step 7 gate).
- Convex-combination fusion (needs labeled retrieval data).
- Any Rust-side changes (belong in `kolisachint/embeddingsearchtools`).
