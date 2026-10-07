# Web tools: `webfetch` and `websearch`

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger task 10.2e.
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

## Problem

hoocode-ts ships two opt-in tools (`--enable-webtools`):

- `webfetch(url, maxTokens?, output?: text|markdown, offset?, outline?, grep?)`
- `websearch(query, maxResults?, safeSearch?: on|off)`

Both shell out to the `webtools` binary (`kolisachint/webtools`, Rust, MIT). The TS
tools-manager downloads it from GitHub releases into the hoocode bin directory, and
the tools parse its `--json` output. The TS tool layer (`webtools-shared.ts`, 651
lines) adds:

- clamping of `maxTokens` and timeout (default 15s, 1–120s);
- a 15-minute in-process result cache;
- the `.webtoolsignore` host policy (gitignore semantics, from
  `<agentDir>/webtoolsignore`, `~/.webtoolsignore` and `<cwd>/.webtoolsignore`),
  checked before a fetch and when filtering search result links;
- content status notes (`ok | empty | needs_js | too_complex`;
  `ok | empty | blocked` for search);
- continuation notes when a page was cut, and the "no keyed search backend" warning.

`cortex` has none of this. `webfetch` and `websearch` are already in the permission
gate's `GATED_TOOLS`, so the gate is ready for them.

The 10.2e block (2026-09-26) listed three options and stalled because the webtools
source was not readable then. It is now: `kolisachint/webtools` is public, MIT, and a
Cargo workspace whose library crates hold all the logic:

| Crate (lib name) | What it does |
|---|---|
| `webtools-core` (`webfetch_core`) | HTTP client (reqwest 0.12 + rustls + OS cert store), encoding, token estimate, truncation marker |
| `webtools-fetch` (`webfetch`) | `fetch_and_convert(FetchOptions) -> FetchResult`. `FetchOptions` has `offset`, `outline`, `grep`, `max_tokens`, `timeout_secs` and `tls`, which is every TS parameter. |
| `webtools-search` (`websearch`) | `run_search(SearchOptions) -> SearchOutput`. Providers: DuckDuckGo (keyless default), Brave, Tavily, SearXNG; a `fallback` provider. Credentials are passed in; the library never reads config. |

The binary adds only CLI parsing, a disk cache, config-file credential lookup and an
MCP stdio server (`src/mcp.rs`, exposing `fetch` and `search`).

## Decision W1: how cortex gets the web tools

| | **(C) Library dependency** | (A) Spawn the `webtools` binary (TS parity) | (D) webtools as an MCP server |
|---|---|---|---|
| Output | Identical, because it's the same code | Identical | Identical, but under MCP tool names `mcp_webtools_fetch` |
| Install | Nothing to download | Download per platform, keep a bin dir, version skew between cortex and binary | User configures it in `mcp.json` |
| Startup cost per call | None | Process spawn | Long-lived child |
| Binary size | + scraper/html5ever (~1–2 MB) | 0 | 0 |
| Couples releases | Yes: a webtools fix needs a cortex release | No | No |
| Prompt and gate | Same tool names and texts as TS; gated | Same | Different names; gated as MCP |

**Recommendation: (C).** Depend on `webtools-fetch` and `webtools-search` as git
dependencies pinned to a webtools tag. Wrap them in a new crate,
`cortexcode-code-tool-web`, that ports the TS tool layer (names, parameters,
descriptions, clamps, notes, `.webtoolsignore`, cache) on top of the library calls.

Why not the TS route (A): a downloaded binary is a second artifact to version,
checksum and keep in step, and `cortex` is itself a single Rust binary. The thing (A)
buys, decoupled releases, is available in (C) by bumping a git tag.

(D) stays available for free once MCP lands, for users who want the binary's own
config file.

Conditions for (C):

- **Workspace compatibility.** webtools uses reqwest 0.12 with
  `rustls-tls, json, gzip, brotli`. That matches the workspace's reqwest line
  exactly, so no second TLS stack is pulled in. It also needs `scraper` (html5ever)
  and `encoding_rs`. Check with `cargo tree -d` when adding it.
- **Publishing.** crates.io rejects git dependencies. crates.io publishing is
  deferred anyway ([distribution.md](distribution.md)). When it comes back, either
  publish the three webtools crates first (the user owns them) or keep
  `code-tool-web` unpublished. Releases build from the workspace and are unaffected.
- **Firewall.** Only `code-tool-web` may depend on webtools crates.

## Design

- **Crate** `cortexcode-code-tool-web`: `webfetch` and `websearch` `AgentTool`s
  built on the existing `code-tool-api` traits, plus `WebtoolsPolicy` (the
  `.webtoolsignore` matcher on the `ignore` crate, which three crates already use)
  and `WebCache`.
- **Opt-in:** `--enable-webtools` / settings `enableWebtools`, as in TS. The tools
  aren't registered otherwise, so their schemas cost nothing.
- **Texts:** descriptions and notes are copied from the pin (MIT) so the model sees
  the same tool. When TS changes them, `pin_drift.py delta` lists `webfetch.ts`,
  `websearch.ts` and `webtools-shared.ts`.
- **Credentials:** search backend keys come from the environment, using the names the
  webtools CLI reads (`WEBTOOLS_BRAVE_API_KEY` or `BRAVE_API_KEY`,
  `WEBTOOLS_TAVILY_API_KEY` or `TAVILY_API_KEY`, `WEBTOOLS_SEARXNG_URL`) and from settings, through
  the same `resolve-config-value` mechanism (`!cmd`, env var, literal) the models
  registry uses. Nothing is passed on a command line.
- **TLS:** cortex's TLS settings (custom CA) map onto `FetchOptions.tls`. The
  `--insecure` option is not exposed to the model.
- **Policy check:** `.webtoolsignore` is checked before the permission prompt, so a
  blocked host never asks. Search results drop links the policy blocks and add a
  note saying how many were dropped. The 10.6 ledger log noted that the
  `.webtoolsignore` host rule in the permission gate was deferred here; it lands with
  this crate.
- **Cache:** in memory, per session, 15 minutes, keyed on the normalized options.
  The webtools binary's disk cache is not used.
- **Cancellation:** the tool future is dropped on abort. reqwest cancels the request
  when its future is dropped.
- **Subagents:** read-only agent types (explore) may be granted `webfetch` and
  `websearch`. That follows the agent registry's existing allowlists.

## Tests

- Port the behaviour of `coding-agent/test/webtools.test.ts` and
  `websearch-api-key-warning.test.ts`: parameter clamps, notes per content status,
  continuation note, policy blocking (fetch and search), cache hit/expiry, missing-key
  warning.
- **Network-free:** a local test HTTP server (axum as a dev-dependency) serves fixture HTML, and search is tested
  through `websearch::build_output_from_ddg` on a recorded page.
- Permission gate: both tools prompt; a blocked host fails before the prompt.

Done = the above green plus a manual fetch and search through the TUI.

## Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| W1 | Library (C), binary (A) or MCP (D)? | (C), pinned to a webtools tag |
| W2 | Keep `--enable-webtools` opt-in (TS default), or turn on by default? | Opt-in, as in TS |
| W3 | Should webtools publish its library crates to crates.io (needed if `cortexcode` crates are ever published)? | Later, with distribution's crates.io item |
