# Semantic search, capability index and `SearchHooCode`

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger task 12.4
(semantic/hybrid search and capability retrieval).
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

hoocode-ts references: `docs/hybrid-retrieval-design.md` (decisions and evals),
`docs/plugin-system-architecture.md` §6 (capability index), and sources
`core/embsearch/` (1,323 lines), `core/search/` (2,819), `core/capabilities/` (692),
`extensions/core/self-knowledge.ts` and `core/self-docs.ts`.

## Problem: three things share one ledger task

1. **Semantic `SearchCodebase`.** 10.2d ported the tool, its lexical mode and all the
   fusion code: `code-tool-search` has `hybrid.rs`, `rrf.rs`, `rerank.rs`,
   `adapter.rs`, `mode.rs` and an `EmbsearchService` trait. Nothing implements the
   service, so `auto` always resolves to `lexical`. TS implements it by driving the
   `embsearch` daemon (`kolisachint/embeddingsearchtools`, Rust, MIT) over NDJSON
   stdio, behind `--enable-semantic-index`.
2. **The capability index.** This is one BM25 (plus optional dense) index over
   MCP tools, skills, commands, agents, installed and available plugins, and doc
   sections. It backs `ResolveMcpTools`' `query`, `SearchPlugins`' extra hits and
   `SearchHooCode`.
3. **`SearchHooCode`**, a self-knowledge tool: `query`, plus `limit` (default 8,
   max 25) over doc sections and capabilities. It is in the pin's default-bundle
   system prompt, and that is the one block that keeps the eight `l1_done` tasks
   off Level 2.

They are designed together because (2) is shared by (3), by MCP deferral
([mcp.md](mcp.md) §7) and by `SearchPlugins` ([plugins.md](plugins.md) phase 3).

## 1. Order

Build (2), then (3), then (1).

- (2) is small, has no dependencies and is used by three other docs.
- (3) closes the `l1_done` tasks.
- (1) needs an optional external binary and helps only repositories over the size
  threshold.

## 2. Capability index (crate `cortexcode-code-capabilities`)

- **Document:** `{ id, kind, name, description, source, deferred }`, where
  `kind ∈ { doc, skill, command, agent, mcp-tool, plugin-installed, plugin-available }`
  (TS `registry.ts`).
- **Producers register by kind and replace the whole kind** on change: MCP loader,
  resource loader (skills, commands), agent registry, plugin discovery, marketplace
  lister, docs.
- **Lexical leg:** in-process BM25 with a code-aware tokenizer (split camelCase,
  snake_case and `mcp_server_tool`), with exact-name lookup kept exact. There are no
  dependencies, so this leg always works. That is TS's "lexical leg is the floor".
- **Dense leg (optional):** embeddings from the embsearch daemon (§3) in a separate
  store keyed by a hash of the capability set, never the repository store. When it
  is absent, results say "lexical only" rather than silently returning less.
- **Fusion:** reuse `code-tool-search::rrf` (`k` as in TS).
- **Search API:** `search(query, kinds, limit, deferred_only)`, returning hits with
  the legs that answered.
- **Eager-below-threshold rule** (TS §6.3/§6.4): retrieval never *replaces* a flat
  catalog under 30 entries. Skills, commands and agents stay eager in the prompt.
  The index adds a way to ask; it doesn't remove the list.
- **Token surface report:** `cortex --print-token-surface` prints the system prompt
  and tool-schema token counts per source, plus the deferral break-even from TS
  `deferral.ts`, N = P × (cacheWrite − cacheRead) / (D × cacheRead), using the
  session model's real prices. [mcp.md](mcp.md) §7's `auto` policy reads it.

## 3. Semantic `SearchCodebase` (the `EmbsearchService` impl)

### Decision S1: daemon or library

`embeddingsearchtools` is a Cargo workspace with a `core` library. It is ~1.1 MB
without a model and **~40 MB with the int8 MiniLM** compiled in through ONNX Runtime.

| | **Daemon (TS parity)** | Library in `cortex` |
|---|---|---|
| `cortex` binary | unchanged | +40 MB, plus the ONNX Runtime native dependency on every target (musl static builds are hard) |
| Install | optional `embsearch` binary, fetched on first `--enable-semantic-index` (checksummed release asset) or found on PATH | always present |
| Crash isolation | a daemon crash degrades to `lexical` | a model crash takes the session down |

**Recommendation: daemon.** Semantic search is opt-in and only useful above the
repository byte threshold. Most sessions never use it, and they shouldn't carry
40 MB or ONNX.

### Design

- **Client:** NDJSON over the daemon's stdio (`index`, `query` with `k` and
  `retriever: dense|lexical|hybrid`, `info`, `save`, `close`, `rerank`). It is
  spawned once per cwd and stays hot (TS `client.ts`). Requests are serialized, and
  a query is never stuck behind a full index batch (48 chunks per bulk request,
  yields between batches).
- **Service** (port `embsearch-service.ts`): resolve the binary; scan the repository
  respecting ignore rules (`repo-scan.ts` and `native-search.ts collectEntries`, per
  the ledger note); below the byte threshold, stay dormant; refuse the mock embedder;
  index changed files in the background with progress in the footer; report
  `phase: indexing | ready | unavailable(reason)` through `EmbsearchState`.
- **Store:** `~/.hoocode/rust/embsearch/<repo-hash>/` with the TS sidecar
  `index-meta` format. Rebuild once if the store's `info.hybrid` disagrees (TS
  migration rule). `CHUNKER_VERSION` matches TS so stores are interchangeable.
- **Mode resolution** (already ported in `mode.rs`): `auto` with embeddings becomes
  `hybrid`, unless the query has regex metacharacters or quotes, in which case it
  becomes `lexical`. A missing service degrades with a reason in the trace and never
  throws.
- **Live edits:** grep is scoped to files the index reports stale (TS 3-way default).

## 4. `SearchHooCode`

- **Tool:** the name, description and schema from the pin (`query`, optional
  `limit`), registered in the default bundle whenever `read` is available, as TS
  does. It searches kinds `doc, skill, command, agent, plugin-installed`;
  `mcp-tool` belongs to `ResolveMcpTools`.
- **Docs are indexed at heading level, lazily on first use.** That raises
  Decision S2: *which docs?* The Rust build ships no user docs today.
  `code-paths::docs_path()` points next to the executable, where nothing is
  installed, and [distribution.md](distribution.md) defers site docs. Options:
  - **(a)** Embed a curated Rust docs set (`docs/user/*.md`, to be written, starting
    with README, CHANGELOG and a features page) into the binary with
    `include_str!`, compressed. Truthful about what this build can do.
  - (b) Ship hoocode-ts's `docs/` from the pin. It is complete, but it describes
    features `cortex` lacks (canvas, plugin authoring) and would make the model
    promise them.
  - (c) Docs index off: `SearchHooCode` searches live capabilities only.

  Recommend **(c) now and (a) when user docs exist.** The capability half is always
  truthful because it is built from what the session actually loaded.
- **The eight `l1_done` tasks:** registering `SearchHooCode` with the pin's
  description and schema removes the tool-block difference the ledger records. If
  the harness then shows the self-docs *list* in the system prompt differing (it
  depends on the shipped docs), the harness masks that section like other
  install-specific text. If that isn't acceptable, roadmap Decision R1 applies.

## 5. Phases

| Phase | Scope |
|---|---|
| 1 | `code-capabilities` (registry, BM25, RRF reuse), producers for skills/commands/agents, `SearchHooCode` with docs off, `--print-token-surface`. Close the `l1_done` tasks. |
| 2 | MCP and plugin producers (with [mcp.md](mcp.md) phase 3 and [plugins.md](plugins.md) phase 3) |
| 3 | embsearch client and service, semantic and hybrid `SearchCodebase`, dense leg for the capability index |
| 4 | Embedded user docs (S2 option a), once written |

## 6. Tests

- **BM25** tokenizer and ranking snapshots; exact name beats fuzzy; kind filters;
  replace-by-kind.
- **`SearchHooCode`:** results with no docs and with a fixture docs tree; schema
  matches the pin.
- **embsearch:** a fake daemon (scripted NDJSON) for the client and service state
  machine, including mock-embedder refusal, dormancy, hybrid-store rebuild and crash
  degradation. A real-binary test runs only when `embsearch` is on PATH (CI job
  downloads it).
- **Reading list (TS):** `capability-index`, `deferral-economics`, `self-docs`,
  `self-knowledge`, `embsearch`, `search-eval`, `agent-selection-eval`,
  `startup-progress-tui`, and the eval-scoring cases of `hybrid-search.test.ts`
  (ledger note from 10.2d).

## 7. Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| S1 | embsearch as an optional daemon or linked in? | Daemon |
| S2 | Which docs does `SearchHooCode` index? | None now (capabilities only); embedded Rust user docs later |
| S3 | Who downloads the `embsearch` binary: `cortex` on first use (checksummed), or the user? | `cortex`, from the embeddingsearchtools release, verified against its SHA256SUMS |
