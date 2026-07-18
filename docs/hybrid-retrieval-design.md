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
  (default `auto`), registered only behind the existing opt-in flag.
  `semantic_search` does not survive as a separate tool — it becomes
  `mode: "semantic"`. The per-cwd service registry
  (`embsearch-service.ts:283-306`) already provides the wiring; this is a
  rename-and-widen, not new plumbing.
- **`grep` stays separate** (v1). It has a genuinely different contract —
  line matches with regex, globs, context lines — that doesn't map onto a
  ranked-retrieval interface. Lexical retrieval *inside* `search` uses grep
  as a backend anyway. Revisit full unification only with eval data.
- **RRF with explicit `k = 60`**, rank-only, deterministic. Raw BM25/cosine
  scores are retriever-local diagnostics, never fused.
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

export function rrfFuse(lists: readonly RankedHit[][], k = 60): FusedHit[];
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

After fusion, a token-budgeted `ContextAssembler` expands winning chunks
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
| `auto` | yes | `hybrid` — unless strong lexical signals (regex metacharacters, quoted strings, explicit paths) → `lexical` |
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

## Eval results (2026-07-18, full index: 16.5k chunks over this repo)

`node packages/coding-agent/scripts/search-eval.mjs` over the 12-query gold
set with the embsearch binary auto-downloaded and the repo fully indexed.
`+rr` = reranked (step 7). The tool's default path is `auto +rr`:

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
3. `rrfFuse(..., 60)` with validation, dedupe guard, deterministic
   tie-break, trace logging.
4. Unified `search` tool with availability-aware mode resolution replacing
   `semantic_search`; flag renamed.
5. Token-budgeted `ContextAssembler` reading line windows only after fusion.
6. Recall@K evals + `k` sweep (span-overlap gold set).
7. Only after recall is stable: rerank the fused top-50; `fusion: "cc"` as
   a labeled-data follow-up.

## Explicitly deferred

- Folding `grep` into `search` — revisit only if evals show `search
  --mode lexical` covers actual grep usage.
- Cross-encoder reranking (step 7 gate).
- Convex-combination fusion (needs labeled retrieval data).
- Any Rust-side changes (belong in `kolisachint/embeddingsearchtools`).
