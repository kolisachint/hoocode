# Design Note: Hybrid retrieval — one `search` tool, RRF fusion

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

## Baseline (2026-08-08, corpus `8a7743b`, embsearch 0.1.7 / MiniLM int8)

```
bun run search-eval -- --corpus-ref 8a7743b \
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
| semantic | 11% | 43% | 49% | 69% | 0.245 |
| hybrid k=0 | 8% | 48% | 57% | 78% | 0.224 |
| hybrid k=2 | 10% | 48% | 57% | 78% | 0.246 |
| hybrid k=10 | 8% | 46% | 57% | 78% | 0.239 |
| hybrid k=60 | 10% | 46% | 59% | 78% | 0.246 |
| auto | 10% | 46% | 59% | 78% | 0.246 |
| lexical +rr | 16% | 52% | 52% | 52% | 0.316 |
| semantic +rr | 37% | 51% | 59% | 69% | 0.439 |
| hybrid k=2 +rr | 24% | 60% | 65% | 78% | 0.414 |
| hybrid k=60 +rr | 34% | 58% | 67% | 78% | 0.464 |
| auto +rr | 34% | 58% | 67% | 78% | 0.464 |
| daemon-hybrid | 6% | 40% | 56% | 87% | 0.228 |
| **daemon-hybrid +rr** | **34%** | **62%** | **74%** | **87%** | **0.472** |

R@10 by query class, the part the aggregate hides:

| class | n | lexical | semantic | hybrid k=60 | semantic +rr | daemon-hybrid +rr |
|---|---|---|---|---|---|---|
| exact-symbol | 22 | 73% | 68% | 86% | 82% | 100% |
| error-fragment | 10 | 100% | 50% | 100% | 50% | 100% |
| path | 6 | 0% | 50% | 33% | 67% | 83% |
| conceptual | 14 | 0% | 43% | 36% | 43% | 36% |
| cross-file | 6 | 0% | 25% | 8% | 42% | 33% |
| boundary | 4 | 0% | 0% | 0% | 25% | 50% |

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
| semantic +rr | 37% | 51% | 59% | 69% | 0.439 |
| hybrid k=60 +rr (ripgrep) | 34% | 58% | 67% | 78% | 0.464 |
| **daemon-hybrid +rr (BM25)** | **34%** | **62%** | **74%** | **87%** | **0.472** |

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

| class | n | semantic +rr | hybrid k=60 +rr | daemon-hybrid +rr |
|---|---|---|---|---|
| exact-symbol | 22 | 82% | 95% | 100% |
| error-fragment | 10 | 50% | 100% | 100% |
| path | 6 | 67% | 67% | 83% |
| conceptual | 14 | 79% | 64% | 79% |
| cross-file | 6 | 58% | 58% | 67% |
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

- Folding `grep` into `search` — revisit only if evals show `search
  --mode lexical` covers actual grep usage.
- Cross-encoder reranking (step 7 gate).
- Convex-combination fusion (needs labeled retrieval data).
- Any Rust-side changes (belong in `kolisachint/embeddingsearchtools`).
