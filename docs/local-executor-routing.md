# Local Executor Routing — Findings and Design

Status: investigation complete, design proposed, not yet implemented.
Goal: reduce Opus token spend by offloading suitable work to a local MLX model
on Apple Silicon, without degrading output quality.

## TL;DR

- Buildable win: route **tool-result compression / summarization** to a local
  model. Measured 88% critical-fact retention with Qwen3-4B vs 94% frontier.
- Do **not** route **tool-call generation** to a local model. Measured 50%
  correctness with destructive/silent failures (hallucinated paths, broken
  edits, unsafe flags, inconsistent JSON schemas).
- Keep Opus for all decisions, planning, edits, and tool-call synthesis.
- Feature must be opt-in behind a flag, default `primary-only` (current behavior).

## Test environment

- Machine: MacBook Air M1, 8 GB unified memory, macOS 26.4.1.
- Runtime: `mlx-lm` 0.29.1 (already installed).
- All models 4-bit MLX quantizations.
- Frontier baseline: Gemini 2.5 Flash via the `google` API key in
  `~/.hoocode/agent/auth.json` (Anthropic entry is a subscription OAuth token,
  not an API key, so it was intentionally not used for scripted calls).

## Methodology

Two distinct roles were tested separately because they require different skills:

1. **Compression (reproductive):** summarize a ~6,860-token conversation/tool
   history into a faithful summary another model can rely on. Graded by
   critical-fact retention against 18 planted facts distributed at the head,
   middle, and tail of the input (file paths, line numbers, function names,
   error constants, numeric config values, a buried signing key, a deprecation
   date, a hard constraint, and a decision value).

2. **Tool-call generation (productive):** given context plus a decided task,
   emit the correct tool call (read/bash/edit/grep) with correct arguments.
   Graded on correctness, safety (no destructive/unsafe commands), and JSON
   schema consistency across 6 realistic scenarios.

Grader and fixtures are throwaway scripts that lived in `/tmp` during the
investigation; recreate them from the scenario tables below if re-running.

## Results — Compression role (critical-fact retention)

| Model | Retention | HEAD | MIDDLE | TAIL | Notes |
|---|---|---|---|---|---|
| Gemini 2.5 Flash (frontier baseline) | 94% (17/18) | 5/5 | 4/4 | 8/9 | lost only a trivial test timing |
| Qwen3-4B-4bit (`/no_think`) | 88% (16/18) | 5/5 | 4/4 | 7/9 | lost only trivial test timing + counts |
| Qwen2.5-3B-Instruct-4bit | 72% (13/18) | 3/5 | 4/4 | 6/9 | dropped error constant + decision value |
| Qwen2.5-Coder-3B-Instruct-4bit | 61% (11/18) | 1/5 | 4/4 | 6/9 | dropped the hard constraint (worst loss) |
| Gemma-2-2B-it-4bit | 0% (0/18) | 0/5 | 0/4 | 0/9 | emitted empty code fences; unusable in this build |

Key observations:

- **Qwen3-4B is the recommended compression model.** Its only losses
  (`4.317s`, test pass/fail counts) are non-load-bearing — the same trivial
  class the frontier model also dropped. It preserved every critical item:
  full paths, line numbers, `EXPIRED_REFRESH`, the buried `kid-7f3a9c` signing
  key, `id.example.org`, all config values, the `MUST NOT change signature`
  constraint, and the `backoff=2000ms` decision.
- **Thinking must be disabled.** With reasoning on, Qwen3-4B scored 55% because
  the `<think>` block consumed the token budget before the summary was written.
  Forcing `/no_think` raised it to 88%. This is a required setting, not a tweak.
- **Instruct beats Coder.** The Coder variant is biased toward generating code
  and drifted into emitting code blocks instead of summarizing; the non-Coder
  Instruct/Qwen3 models are better for this reproductive task.
- **Gemma-2-2B (this MLX 4-bit build) is broken for this use.** A different
  build/chat-template might behave; not pursued.

### Speed / memory (compression, 6,860-token input)

| Model | Prompt tok/s | Gen tok/s | Peak mem | Wall (gen) |
|---|---|---|---|---|
| Qwen2.5-Coder-3B | 257 | 25 | 2.6 GB | ~16 s / 400 tok |
| Qwen2.5-3B-Instruct | 259 | 25 | 2.6 GB | ~3 min total |
| Qwen3-4B (`/no_think`) | 112 | 16 | 3.9 GB | ~1.5 min total |

On 8 GB, Qwen3-4B runs without catastrophic swap but sits near the memory
edge (3.9 GB peak). A long compaction summary takes roughly 30 s. Acceptable
for a background compaction step; not acceptable inline on every turn.

## Results — Tool-call generation role

Correctness: **3/6**. Schema consistency: **5 different JSON shapes across 6
calls** (mixed `tool` / `action` / `cmd` / `args` wrappers).

| Scenario | Verdict | Failure mode |
|---|---|---|
| S1 read around line 48 | Correct | — |
| S2 run failing test | Wrong | invented non-existent `npm --testFile` flag |
| S3 grep usages | Correct-ish | right call, but inconsistent schema (`args` wrapper) |
| S4 edit with constraint | Broken | hallucinated path `path/to/your/file.js`; try/catch leaves `res` out of scope → non-compiling edit |
| S5 clean reinstall | Wrong + unsafe | `npm install --force --no-save` (not a clean reinstall; `--force` masks errors) |
| S6 precise line read | Correct | exact offset/limit |

Why this role is a no:

- Faithfulness does not transfer to correctness. The same model is 88% at
  reproducing facts and 50% at constructing actions.
- Failures are the silent, expensive kind: S4 produces valid-looking JSON that
  targets the wrong file and writes broken code. If trusted, Opus applies a
  destructive edit.
- Schema drift alone makes the output unreliable to parse.
- Errors compound across a multi-step loop. At 50% per step, a 5-step task is
  ~3% end-to-end success, and each failure requires Opus to detect and unwind.
- Net effect is higher cost (verification + cleanup) than the tokens saved.

## Per-content-type validation (tool-result compression)

This tested whether individual tool results can be compressed by a local model
*before* Opus sees them (the v2 idea), per content type, with Qwen3-4B-4bit.

### Critical finding: faithfulness and compression are in direct tension

A first pass scored 100% retention across all four content types over 3 runs
each. That result was an artifact: the model achieved it by **not compressing**
(output was 99-115% of input length, i.e. near-verbatim reproduction). Perfect
faithfulness with zero compression saves zero tokens, which defeats the purpose.

When forced to actually compress (~50% target), faithfulness collapsed, and it
collapsed hardest on exactly the high-density content where tool-result
compression would be most valuable:

| Content type | Density | Compression ratio | Critical-fact retention |
|---|---|---|---|
| file_read | medium | 42% | 33% (3/9) |
| grep | high | 65% | 62% (5/8) |
| test_log | medium-high | 50% | 12% (1/8) |
| error_list | very-high | 71% | 0% (0/11) |

Cross-family check (Gemma-3-4b-it-4bit, same forced ~50% compression):

| Content type | Density | Qwen3-4b | Gemma-3-4b |
|---|---|---|---|
| file_read | medium | 33% | 44% |
| grep | high | 62% | 0% |
| test_log | medium-high | 12% | 37% |
| error_list | very-high | 0% | 45% |

Gemma is no better overall — it just relocates the failures (better on
error_list/test_log, catastrophic on grep, dropping every file path by writing
prose narrative instead of retaining identifiers). A different model family hits
the same wall, ruling out "wrong model" as the cause. Note: there is no
Gemma-3 2B; the line is 1b/4b/12b/27b, so the 4b is the comparable class.

Interpretation:

- **There is no useful operating point.** Either it reproduces input verbatim
  (no saving) or it compresses and drops critical facts. Neither Qwen3-4b nor
  Gemma-3-4b can do faithful *and* compact on identifier-dense tool output.
- **Worst on the densest, most valuable content.** error_list and test_log
  (TypeScript error codes, line numbers, stack traces) lost 88-100% of critical
  facts when compressed. These are precisely the outputs Opus most needs intact.
- **High density resists compression.** The model could only "compress"
  error_list to 71% of original even when asked for 50%, because dense output
  has little redundancy to remove without dropping facts.

### Consequence for v2 (tool-result compression)

**Tool-result compression on the critical path is not viable with a local
3-4B model on this hardware.** Unlike compaction (where the raw history existed
and could be re-read), pre-compression means Opus never sees the original, so a
dropped fact is unrecoverable. The data shows dropped facts are the norm, not
the exception, once real compression is applied.

This does not depend on prompt tuning: the tension is structural. Dense tool
output is mostly load-bearing tokens, so any real size reduction removes
load-bearing information. A larger/frontier model would push the curve out but
the same tension applies.

### Per-tool extractive prompts (breakthrough for retention)

The collapse above came from *abstractive* prompts ("rewrite in prose at half
length"), which force the model to describe instead of retain. Switching to
*extractive* per-tool prompts (keep identifiers, drop only redundant filler)
fixed retention for dense types:

| Type | Compression ratio | Retention (extractive) | vs abstractive |
|---|---|---|---|
| grep | 94% | 100% (8/8) | was 62% |
| error_list | 93% | 100% (11/11) | was 0% |
| test_log | 117% | 87% (7/8) | was 12% |
| file_read | 53% | 55% (5/9) | was 33% |

The real rule this reveals: **compression only exists where there is
redundancy.** Dense output (grep, errors, test failures) is all load-bearing,
so extractive prompts retain 100% but compress ~0% (ratio ~93-94%). Low-density
output (file bodies, verbose logs) compresses, but facts can hide in the filler
(file_read dropped 401/429/EXPIRED_REFRESH because they were in function bodies
the prompt discarded).

Working per-tool extractive prompts (Qwen3-4b, `/no_think`):

- grep: "Output ONE line per match as `path:line` + matched symbol. Keep EVERY
  path and line number exactly. Drop surrounding code snippet text. No prose."
- error_list: "Output ONE line per error as `file:line:col TSxxxx message`. Keep
  EVERY path, line:col, and error code. Keep total count. No prose."
- test_log: "Extract pass/fail counts, total time, and for EACH failing test:
  file, test name, expected vs received, error location `file:line:col`. Drop
  passing-test lines. Keep every number and path exactly."
- file_read: "Extract the path and each declaration line as `LINE: signature`
  (functions, classes, constants WITH values, status-check lines, TODOs). Drop
  only plain function-body statements. Keep every line number and value."
  (Note: must explicitly include status-check/throw lines or constants in the
  body are lost.)

### Per-tool validation against real hoocode tools

Tested one extractive prompt per actual built-in tool (read, bash, grep, find,
ls, edit) with realistic LARGE outputs (so real compression is forced, not
verbatim echo). Qwen3-4b, `/no_think`, temp 0.

Small fixtures gave misleading 84-100% retention because there was nothing to
compress (output ratio 84-100% = near-verbatim). The honest test uses large
outputs with noise + planted facts:

| Tool | Output size | Compression ratio | Retention | Verdict |
|---|---|---|---|---|
| read (large) | ~7.5 KB | 5% (95% smaller) | 100% (8/8) | Excellent |
| bash (large) | ~5 KB | 6% (94% smaller) | 100% (8/8) | Excellent |
| grep (large) | ~3.3 KB | 32% | 20% (1/5) | Fails |

Why the split is clean and structural:

- **read and bash are compressible AND safe.** Their large outputs are mostly
  low-value noise (function-body statements, passing-test lines, progress bars)
  around a few load-bearing facts. The extractive prompt drops the noise and
  keeps every declaration / every failure + counts. read dropped 168 noise
  lines and kept all 5 declarations; bash dropped 107 passing-test lines and
  kept both failures with `file:line:col`, the totals, the time, and exit code.
  This is real 94-95% compression with zero critical-fact loss.
- **grep (and find, ls) are NOT compressible.** Every line is a distinct fact
  (path:line). There is no noise to remove, so any size reduction = dropping
  matches. Under forced compression grep retained only 20%. These must be
  pass-through (keep raw).

The earlier abstractive failures and this per-tool result agree on one rule:
**compressibility tracks redundancy, by tool.** read/bash outputs have
redundancy (bodies, passing lines, progress) -> safe to compress hard.
grep/find/ls outputs are pure fact lists -> not compressible without loss.

Prompt caveat found: the read prompt abstracted the path to `/path/to/...`. The
production prompt must pin the literal path explicitly ("Output the path EXACTLY
as given").

Working per-tool prompts (validated, Qwen3-4b `/no_think`):

- read: "Output the path EXACTLY as given, then ONLY declaration lines as
  `LINE: code` for imports, class/function/const declarations WITH values,
  throw/status-check lines, and TODOs. Drop plain body statements. Keep every
  line number, identifier, string, and number exactly. No prose."
- bash: "Keep ONLY: the command, every error/warning with file:line:col and
  code, and final counts/timings/exit code. Drop progress, info, and
  passing/OK lines. Keep numbers and paths exactly. No prose."
- grep / find / ls: PASS-THROUGH. Do not compress (every line is a fact; the
  model drops matches under compression). Keep raw.
- edit / write / TodoWrite: already small; no compression needed.

### v2 design: per-tool, explicit-gated

Viable as an **opt-in, explicitly commanded** feature, not always-on. Routing
is decided per tool by the validated table:

| Tool | Action | Rationale |
|---|---|---|
| read | Compress (extractive prompt) | 95% smaller, 100% retention |
| bash | Compress (extractive prompt) | 94% smaller, 100% retention |
| grep | Pass-through (raw) | every line a fact; 20% retention if compressed |
| find | Pass-through (raw) | pure path list, not compressible |
| ls | Pass-through (raw) | pure entry list, not compressible |
| edit / write / TodoWrite | No-op | already small |

Rules:

- Only read and bash outputs are compressed, and only above a size threshold
  (e.g. > ~2 KB) where there is noise to remove. Small outputs pass through.
- grep/find/ls always pass through. Never compress fact-list outputs.
- Always keep the raw result retrievable so a dropped fact is recoverable.
- Gate behind an explicit command/flag, default OFF. Nothing routes silently.

Honest expectation: real savings come from large read and bash outputs (the
two verbose tools), which is also where Opus spends the most tool-result
tokens. grep/find/ls save nothing but are also the cheapest. With the explicit
gate and retrievable raw, risk is user-controlled and bounded. This makes v2
buildable as a deliberate, gated capability targeting exactly the two tools
where it both works and matters.

## Architectural constraints (from codebase mapping)

- hoocode resolves a **single model per session** (`model-resolver.ts`,
  `findInitialModel()`); `agent.state.model` is used for every turn. There is
  no per-turn router today.
- Compaction/summarization already happens at an isolated call site
  (`packages/coding-agent/src/core/compaction/compaction.ts:compact()`), using
  the active session model. This is the natural, low-blast-radius seam to route.
- Deciding a tool call IS the planning turn — tool calls are parsed from the
  assistant response. There is no separate "tool turn" to hand off; this is why
  the tool-call role cannot be cleanly offloaded.
- Local OpenAI-compatible endpoints are already supported via
  `~/.hoocode/agent/models.json` provider `baseUrl` overrides
  (`model-registry.ts`, `ModelsConfigSchema`) and the `openai-completions`
  provider. `executor-only` (run everything locally) needs no new code.
- Existing env overrides: `HOOCODE_CODING_AGENT_DIR`, `HOOCODE_CODING_AGENT_SESSION_DIR`
  (`config.ts`). There is no model-selection env var yet; `HOOCODE_ROUTING_MODE`
  would be a new pattern.

## Proposed design

### Scope (v1)

Route **only** compaction/tool-result summarization to a local executor model.
Everything else (planning, reasoning, tool-call synthesis, edits) stays on the
primary model. No tool-call routing.

### Routing modes

Both v1 and v2 are exposed as **explicit, opt-in** modes (user requirement),
default OFF. Nothing routes to the executor silently.

- `primary-only` (default) — current behavior, untouched.
- `executor-for-summarization` (v1) — compaction/summarization runs on the
  executor; all else on primary. Lower risk: raw history existed and is
  re-readable.
- `executor-for-tool-results` (v2) — per-tool extractive compression of tool
  results before Opus sees them, using the per-tool prompts above. Higher risk
  (no original seen by Opus), so: raw result kept retrievable, dense types are
  pass-through only, and the mode must be explicitly enabled per session/command.
- `shadow-executor` (later) — primary remains the live path; the same
  workload is mirrored to the executor for measurement only, never affecting
  output. Note: doubles memory pressure during the mirrored call on 8 GB; run
  only in deliberate measurement sessions.

Explicit gating: modes are selected per session via config
(`routing.mode`) or env (`HOOCODE_ROUTING_MODE`), and v2 additionally requires
an explicit enable so it never activates by config drift alone.

Deliberately dropped from the original proposal:

- `executor-for-tools` / `toolCall` executor — fails the correctness test
  (50%, destructive failures). Do not implement.
- `executor-only` — already achievable via `models.json`; no flag needed.
- **Tool-result pre-compression (v2)** — killed by the per-content-type
  validation above. Faithfulness and compression are in direct tension; real
  compression drops 38-100% of critical facts on dense outputs, with no
  recovery path. Do not implement on 3-4B local hardware.

### Configuration

Extend `models.json` (`ModelRegistry` / `ModelsConfigSchema`) rather than
inventing a new `.agents/providers.json`. Routing is a model concern and that
file already handles provider/model config and layered merge.

Sketch:

```json
{
  "routing": {
    "mode": "executor-for-summarization",
    "executor": {
      "provider": "mlx",
      "model": "mlx-community/Qwen3-4B-4bit",
      "baseUrl": "http://localhost:8080/v1",
      "noThink": true
    }
  },
  "providers": {
    "mlx": {
      "baseUrl": "http://localhost:8080/v1",
      "apiKey": "not-needed",
      "api": "openai-completions"
    }
  }
}
```

Optional env override (new pattern): `HOOCODE_ROUTING_MODE=executor-for-summarization`.
Absent config defaults to `primary-only` so existing behavior is unchanged.

### Implementation seam

- Add a `selectModel(turnKind, config)` decision used where the summarization
  call is made (`compaction.ts:compact()`), and/or at
  `packages/agent/src/agent-loop.ts` `streamAssistantResponse()` where both
  `context` and `config` are available per turn.
- Use a deterministic rule for routing (`turnKind === "compaction"`), not a
  model-based classifier — do not add intelligence where an `if` suffices.
- Executor calls must force `/no_think` (or the provider equivalent) for Qwen3.
- Apply a size gate: skip offload for very small inputs (Opus handles cheaply)
  and for inputs beyond the executor's reliable range. Sweet spot observed
  ~1K–6K tokens; re-measure for the chosen model.

### Metrics to log per routed turn

Provider used, input tokens, output tokens, latency, success/failure, and
whether fallback to primary was triggered. The agent already has token/usage
plumbing per response to tap; latency and provider-used are the new additions.

## Cost analysis (expectations before building)

Pricing basis: Opus 4.x ~ $15/M output, $3/M input. Compaction is input-heavy
(reads a large history) with small output (a summary), ~$0.06 per call.

| Slice offloaded | Daily saving (metered API) | Annual |
|---|---|---|
| Compaction only (v1) | $0.10-0.50 | $35-180 |
| + tool-result compression (v2, REJECTED) | would-be $0.40-1.30 | $150-475 |

Important caveats:

- **Subscription users save $0 cash.** The Anthropic credential here is a
  subscription OAuth token, not metered API. On a flat-rate plan, offloaded
  tokens have zero marginal cost; the only benefit is plan-limit headroom.
- **v2's higher savings are unreachable** — the per-content-type validation
  shows tool-result compression destroys critical facts. The realistic ceiling
  is the v1 row: ~$0.10-0.50/day on metered API.
- **Local is not free**: ~30s latency per compaction + ~3.9 GB memory load on an
  8 GB machine. For many sessions this is a poor wall-clock trade.
- **Risk cost**: at 88% faithful, an occasional dropped fact triggers an Opus
  re-read/replan that can exceed the $0.06 saved.

Conclusion: do not build this to save money. The cost ceiling is low (~$180/yr
metered, $0 on subscription) and below the engineering + maintenance cost.
Build it only as a capability play (privacy, offline, foundation for bigger
hardware), with savings as a rounding-error bonus.

## Locked decisions

1. **Scope: v1 + v2 together.** Build compaction routing and per-tool
   tool-result compression in the same effort (shared plumbing).
2. **Master gate: `--enable-local-inference` flag.** All routing is inert
   unless this flag is explicitly set. No config-only or silent activation.
   On any executor failure/timeout: fall back to primary, log silently, never
   hard-fail. v2 fallback = use the raw uncompressed tool result.
3. **MLX server: harness-managed**, but only when `--enable-local-inference`
   is set. hoocode spawns/health-checks/stops the local server; absence or
   crash degrades to primary via the fallback path.
4. **Config home: `models.json`.** Must verify the `openai-completions`
   provider can carry the Qwen3 `/no_think` control (prompt suffix or compat
   option). Report back if this requires breaking changes.
5. **2 KB guard.** v2 only compresses read/bash outputs above ~2 KB
   (configurable, default 2 KB). Smaller outputs pass through.

## Seam investigation results (verified in code)

Decision 4 (`/no_think` plumbing) - NOT breaking, but needs one additive core
change:

- `OpenAICompletionsCompat` (`packages/ai/src/types.ts:666`) has no
  prompt-suffix or custom-body-param passthrough. Qwen thinking is driven by
  `reasoningEffort` at runtime (`enable_thinking` / `chat_template_kwargs` in
  `openai-completions.ts:652-660`), not by per-model config.
- Qwen already has first-class support: `thinkingFormat: "qwen"` and
  `"qwen-chat-template"` exist. No-think may be achievable by setting
  `reasoningEffort` off via compat, or by adding one optional field
  (`modelParams?: Record<string, unknown>` or `promptSuffix?: string`).
- Either way the change is **additive / non-breaking**: existing configs
  unaffected. Confirm the cleaner of the two during implementation.

v1 seam (compaction): `generateSummary()` (`compaction.ts:481`) calls
`completeSimple(model, ...)` with `model` as a plain parameter. One chokepoint
(invoked up to 2x in parallel for turn-splitting). Route by swapping `model`
when gated + turn is summarization.

v2 seam (tool results): `createToolResultMessage()` (`agent-loop.ts:763`) is
the single chokepoint for ALL tool results (sequential, parallel, background).
The `afterToolCall()` hook (`agent-loop.ts:729`) lets us rewrite
`result.content` before serialization - the ideal per-tool compression point.

models.json: a custom provider with `api: "openai-completions"`, custom
`baseUrl`, and a model entry (only `id` required) works today
(`model-registry.ts`, `ModelsConfigSchema`).

## Build plan (v1 + v2, all gated behind `--enable-local-inference`)

| # | File | Change | Breaking |
|---|---|---|---|
| 1 | `packages/ai/src/types.ts` | optional no-think field on `OpenAICompletionsCompat` | No |
| 2 | `packages/ai/src/providers/openai-completions.ts` | apply that field in request builder | No |
| 3 | `model-resolver.ts` / new `routing.ts` | flag + executor resolution + `selectModel(turnKind)` | No |
| 4 | `cli/args.ts` | register `--enable-local-inference` + `HOOCODE_ROUTING_MODE` | No |
| 5 | `compaction.ts` (`generateSummary`) | route to executor when gated; fallback to primary | No |
| 6 | `agent-loop.ts` (`afterToolCall`) | per-tool compression (read/bash, >2KB), keep raw, fallback to raw | No |
| 7 | new prompts + routing table | validated per-tool prompts | New |
| 8 | new MLX server lifecycle manager | spawn/health-check/stop, gated, degrade to primary | New |
| 9 | metrics logging | provider, tokens in/out, latency, fallback-triggered | New |

All changes are additive and inert when the flag is off (default).

## Open items before/while building

1. Confirm `compact()` is the only summarization call site, or enumerate all of
   them (tool-result compression vs full-history compaction may differ).
2. Decide fallback policy when the executor errors or times out: fall back to
   primary for that summary (recommended) vs hard-fail.
3. Re-run the per-content-type faithfulness test on the chosen model with
   multiple samples (n>=5) to get a retention *rate*, not a single sample,
   especially for identifier-dense outputs (grep/error lists).
4. Decide how the local MLX server lifecycle is managed (user-started vs
   harness-started) and how its absence degrades gracefully to `primary-only`.
5. Verify `openai-completions` provider handles the Qwen3 `/no_think` control
   cleanly via config, or add a `noThink` option mapping.

## Hardware note

Findings are for an 8 GB M1, where Qwen3-4B is at the memory edge. On 24–32 GB
machines a 7B/14B-Instruct executor would likely close most of the remaining
gap to the frontier baseline and widen the safe operating range, including
larger compaction inputs. The compression-only scope and flag design carry over
unchanged.

## Build order

1. `executor-for-summarization` with Qwen3-4B (`/no_think`), behind the flag,
   default `primary-only`, fallback to primary on executor error.
2. Per-turn metrics logging.
3. `shadow-executor` for honest measurement on real traffic.
4. Extend to other low-risk reproductive offloads only if data supports it
   (e.g. session titles, commit-message drafts).
