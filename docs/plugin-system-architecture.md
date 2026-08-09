# Plugin System Architecture — production model, drift, automation, eval, scope, retrieval

**Status:** proposal (nothing here is implemented yet)
**Scope:** `packages/coding-agent` plugin + capability subsystem
**Companions:** `docs/plugin-system-spec.md` (what shipped),
`docs/plugin-format-mapping.md` (format tables),
`docs/agent-spec-tree-map.md` (on-disk surfaces)

Follows the `agent-spec-tree-map.md` rule: every "hoocode does X" claim carries a
`file:line` citation. Vendor claims cite the page they came from and the date
fetched, because they are the thing most likely to go stale.

---

## 0. Positions

| # | Question | Position |
|---|---|---|
| 1 | Which formats do we read vs. write? | **Consume from all, produce for one.** A plugin's meaning is membership in a platform ecosystem, so production is platform-specific and never targets `agents`. |
| 2 | What if the vendor plugin standard changes? | Read leniently, write pinned, learn offline. Never let a runtime network fetch decide how to interpret executable content. |
| 3 | Total automation of inheritance + publishing? | Total up to the trust boundary, hard stop at it. Automate everything about publishing except the publish. |
| 4 | Missing eval / green signal | Four gates, cheap→expensive. Conformance is measured against the **target platform**, not against our own parser. |
| 5 | Which surfaces are per-repo? | Repo-scoped iff the content is a fact about *this codebase*. Plugins are not. Today we write them into the user's working tree — that is a bug. |
| 6 | Embed search behind ToolSearch | One capability index, hybrid retrieval, per-capability `deferred` policy with a measured threshold. Retrieval is a scale mechanism, not an upgrade. |
| 7 | Aligned or different from Claude / GitHub? | Aligned on every format detail, and now on the production model too. One real divergence: the agent authors its own capabilities. |

---

## 1. Consumption and production are different problems

### 1.1 The rule

> **Consumption reads every format. Production targets exactly one ecosystem.**

Reading is already right: `parseMarketplaceDir` and `parsePluginWithFormats`
accept native, Claude, and Copilot layouts with a documented precedence
(`marketplace.ts:188-244`, `formats/types.ts:124-152`). Nothing there changes.

Writing is wrong. `DEFAULT_AUTHORING_PLATFORMS = ["agents"]`
(`platform-targets.ts:30`) makes the portable native layout the default
production target for authored plugins, with vendor layouts as opt-in interop.
That inverts the actual relationship.

### 1.2 Why plugins cannot be vendor-neutral

A skill is a file. A plugin is a **distribution unit**, and distribution is
platform-owned. The marketplaces that exist are platform ecosystems —
`anthropics/claude-plugins-official`, `anthropics/claude-plugins-community`,
`github/copilot-plugins`. There is no `.agents` ecosystem beyond our own bundled
default marketplace (`install.ts:52-59`).

So a plugin emitted as `.agents-plugin/plugin.json` belongs to no ecosystem: it
cannot be published, and nobody outside hoocode can install it. It is a
distribution unit that cannot be distributed. The `agents` format is the right
answer for *reading* (a superset we normalize into) and the wrong answer for
*producing*.

### 1.3 Where `agents` remains correct

Skills, commands, subagents, and MCP configs are **workspace surfaces**, not
distribution units. They have a genuine cross-vendor shape — `SKILL.md`
frontmatter and standard `mcp.json` are near-identical across vendors — and they
are consumed in place rather than shipped. `.agents/skills/`, `.agents/mcp.json`,
and the `<git-root..cwd>/.agents/` walk stay valid production targets
(`agent-spec-tree-map.md`, quick-reference block).

The split, stated once:

| | Unit | `agents` as a write target |
|---|---|---|
| Skills, commands, subagents, MCP | workspace surface, consumed in place | **yes** — a real cross-vendor convention |
| Plugins | distribution unit, shipped to an ecosystem | **no** — belongs to no ecosystem |

### 1.4 `--platform`, and splitting the resolver

One flag, `--platform claude|github`, replaces `--support-platform`
(`main.ts:433-452`). Per `AGENTS.md` ("do not preserve backward compatibility
unless the user explicitly asks") this is a rename, not an alias.

The resolver must split, because the two consumers now have opposite rules and
today share one function (`platform-targets.ts:88`, consumed by
`propose-plugin.ts:136` and `scaffold.ts:121,182,253`):

```ts
/** Plugin production. Never returns "agents". Defaults to claude. */
resolvePluginPlatforms(explicit?): ("claude" | "github")[]

/** Workspace scaffolds (/new-skill, /new-agent, /new-command). "agents" allowed. */
resolveWorkspacePlatforms(explicit?): MarketplacePlatform[]
```

`resolvePluginPlatforms` resolution order: explicit per-call → session
`--platform` → `["claude"]`. Passing `agents` to it is an error, not a silent
drop — a caller asking for an unpublishable artifact should hear about it.

Default `claude` rather than erroring keeps the autonomous capability-gap flow
unbroken, and §1.5 shows it is also the only platform where the local loop
actually closes.

### 1.5 The two platforms are not symmetric

| | Claude Code | Copilot CLI |
|---|---|---|
| Hand-writable auto-load path | `~/.claude/skills/<id>/` — documented, what `claude plugin init` writes, loads with no install step | none. `~/.copilot/installed-plugins/` is manager-owned, "should not be edited manually" |
| Local dev loop | `--plugin-dir`, `--plugin-url`, or the skills-dir path above | `--plugin-dir` only; direct installs from local paths are **deprecated** ("only `plugin@marketplace` installs will be supported in a future release") |
| Route into the ecosystem | drop-in, or submission form + review | marketplace publish, increasingly the only route |

Sources: `code.claude.com/docs/en/plugins`;
`docs.github.com/.../copilot-cli-reference/cli-config-dir-reference`;
`github.com/github/copilot-cli/discussions/3685`. All fetched 2026-08-09.
The GitHub rows are second-hand — `docs.github.com` is blocked by this
environment's egress proxy, so they came via search results and the upstream
discussion. **Re-verify against the primary reference before implementing.**

Consequence for the plan: for `claude`, authoring produces a plugin that is live
in Claude Code on its next session. For `github`, there is no such path, so
"production" means the publish lane in §3.2 and nothing shorter. That asymmetry
is upstream reality, not a design choice, and the plan should not pretend
otherwise.

---

## 2. Standard drift

### 2.1 The seam exists; the contents are stale

`PluginFormatAdapter` (`formats/types.ts:124-152`) is the right seam: tracking a
vendor means editing one adapter file. The Copilot adapter is current — it probes
all four documented manifest locations (`formats/copilot.ts:73-77`).

The Claude adapter is not. Reading `code.claude.com/docs/en/plugins` (fetched
2026-08-09) against `formats/claude.ts` + `formats/jsonManifest.ts`:

| Claude Code convention | hoocode today |
|---|---|
| Manifest is **optional** — a directory with `skills/` alone is a plugin | `detectPlugin` requires `.claude-plugin/plugin.json` (`jsonManifest.ts:52-54`) |
| A single `SKILL.md` at plugin root = a one-skill plugin | not detected |
| `.lsp.json` — LSP servers | not parsed |
| `monitors/monitors.json` — background monitors | not parsed |
| `bin/` — added to Bash `PATH` while enabled | not parsed |
| plugin-root `settings.json` (`agent`, `subagentStatusLine`) | not parsed |
| Skills namespaced `/plugin-name:skill-name` | not applied |

"If the standard changes" is not hypothetical — it already changed in at least
seven places and nothing noticed. The defect is the missing *signal*, not a
missing doc-fetcher.

**§1 raises the stakes on this table.** Under a vendor-neutral production model
the Claude adapter only had to *read* well. As a production target it has to
*write* conforming artifacts into someone else's ecosystem. Every unparsed
surface above is also an unemittable one, and a plugin we emit that Claude Code
rejects is a worse failure than one we merely fail to read.

### 2.2 The ladder

**Tier 0 — pinned adapters stay the only writer.** Emission must be
byte-reproducible. A doc-inferred or model-inferred *write* turns one bad guess
into a corrupt artifact in someone's repo, possibly published. Non-negotiable,
and more so now that emission targets a foreign ecosystem.

**Tier 1 — lenient reader that never loses data.** Add to `NormalizedPlugin`
(`manifest.ts:51-82`):

```ts
/** Manifest keys present on disk that this adapter does not model. */
unknownFields?: Record<string, unknown>;
/** On-disk surfaces present but unhandled, e.g. [".lsp.json", "monitors/"]. */
unsupportedSurfaces?: string[];
```

`ListPlugins` then reports `foo@1.2.0 [claude] — skills, hooks (2 surfaces
present but unsupported: .lsp.json, monitors/)`. Near-zero cost; turns silent
drift into a visible diagnostic the moment a drifted plugin is installed.

**Tier 2 — offline drift check, human-ratified.** A scheduled/CI job fetches the
vendor reference pages, diffs documented field and path sets against each
adapter's declared sets, opens a report. Same discipline `AGENTS.md` mandates for
`packages/ai/src/models.generated.ts`: the network informs the codegen input,
never the runtime.

**Tier 3 — model fallback, quarantined.** When a directory parses to nothing and
Tier 1 found unmodeled surfaces, the model may read the vendor docs and draft an
*adapter patch*. It lands as a proposal for human review, never a live parse.

### 2.3 Why not "pass it to the model at runtime"

1. **Trust.** A model-inferred parse of `hooks/` or `.mcp.json` is a
   code-execution decision derived from fetched web text —
   `plugin-system-spec.md` already forbids that shape under the injection
   carve-out.
2. **Determinism.** The same directory could parse differently across sessions,
   and the failure is silent.
3. **Hot path.** Format interpretation runs at session start
   (`loader.ts:644-649`); a network round-trip there is a latency and
   availability regression for a case that is rare by construction.

---

## 3. Automation, and the two use cases

### 3.1 Consume / inherit — mostly built, two gaps

Working today: curated indices clone lazily on first search
(`install.ts:88-101`), install is autonomous within a trusted marketplace
(`tools/plugins.ts:239-255`), passive capabilities activate live the same turn
(`propose-plugin.ts:326`), and the runtime arms reuse cues into `SearchPlugins`
(`tools/plugins.ts:108-111`).

- **Indices clone once and never refresh** (`install.ts:91-92` skips when the
  directory exists), so inheritance goes stale silently. Add a TTL plus explicit
  refresh. Refreshing an *index* is read-only — it changes what is discoverable,
  not what is installed. The no-auto-update rule for installed plugins stands.
- **Discovery is substring matching** (`tools/plugins.ts:96`). Fine for two
  curated marketplaces; fails at community-catalog scale. That is §6's job.

Consumption is format-agnostic and stays that way: hoocode installs Claude,
Copilot, and native plugins alike, into its own home (§5.3).

### 3.2 Produce / publish — absent, and now the only GitHub route

No packaging, no index-entry generation, no validation, no publish path. But
"publishing is a human act" does not mean "no tooling" — Claude Code ships
`claude plugin validate` and an automated submission pipeline; the human part is
the *approval*, not the mechanics.

Pipeline, autonomous until the last step:

1. **`PackagePlugin`** (autonomous) — take the authored plugin, emit the target
   platform's layout, generate `README.md`, produce the `marketplace.json` entry
   stanza. Writes to staging. Touches no remote.
2. **`PluginEval`** (autonomous) — §4. Must be green.
3. **Publish** (human) — opens the PR against the marketplace repo, or prepares
   the Claude community submission.

The two platforms diverge at step 3, and the pipeline must model that rather than
paper over it:

- **claude** — the plugin is already live locally via `~/.claude/skills/<id>/`
  (§5.3), so publishing is optional and additive. Community submission goes
  through the web form and review; approved plugins are pinned to a commit SHA
  and the catalog syncs nightly.
- **github** — with local installs deprecated, publishing is the *only* way the
  plugin reaches the ecosystem. The publish lane is therefore load-bearing for
  GitHub in a way it is not for Claude, and should be built with that platform's
  flow as the primary case.

### 3.3 Against total automation of the last step

The ask was total automation. For step 3 the answer should be no, and the reason
is worth stating rather than deferring to policy: an agent that can autonomously
publish executable code into a marketplace that other agents autonomously install
from is a supply-chain compromise primitive. Install is *deliberately* autonomous
(`plugin-system-spec.md`, trust model) on the strength of the marketplace
boundary vouching for the code. Automating publish dissolves the thing install is
trusting.

Claude Code holds the same line with more machinery than we have — submission
form, review pipeline, safety screening, SHA pinning. We have none of it, so our
line has to be at least as conservative. Everything before the button is
automatable, and should be.

---

## 4. Eval — the missing green signal

### 4.1 What is missing

Nothing runs between "model writes plugin draft" and "plugin is live":
`writePluginDraft` is followed directly by `ctx.activatePlugin(dest)`
(`propose-plugin.ts:323-326`). No check that it parses, that the hook's binary
exists, that the MCP server starts, or that the skill ever triggers.
`plugin-system-spec.md:203-209` lists round-trip checks as *tests*; there is no
runtime gate.

Claude Code's analogue is `claude plugin validate`, run pre-submission and again
in review. For them that is proportionate — their plugins are human-written. Ours
are machine-written, activate in the same turn, and now land in *their*
ecosystem. Same idea, earlier in the pipeline, stricter bar.

### 4.2 Four gates

**G1 — Structural and conformant** (always, ~ms). Manifest schema, id/slug
validity, no absolute or machine-specific paths, no high-entropy or `sk-`/`AWS_`
strings. Portability is currently only a `promptGuidelines` suggestion
(`propose-plugin.ts:298`); G1 makes it enforced.

Critically, conformance is measured against the **target platform**, not against
ourselves. Round-tripping through `parsePluginDir` proves our emitter and our
parser agree — it says nothing about whether Claude Code will accept the
artifact. Two checks, in order:

- round-trip through `parsePluginDir` (cheap, always available);
- **shell out to `claude plugin validate <dir>` when the binary is present**,
  `--strict` on the publish path. That is the vendor's own validator, it is the
  exact check their review pipeline runs, and it costs us one `execFile`. For a
  `--platform claude` artifact it is the authoritative green signal, and we
  should not invent a second opinion where the vendor ships one.

For `--platform github` no equivalent validator is known; fall back to
round-trip plus schema until one is confirmed.

**G2 — Static safety** (always, ~ms). Hook commands parse; `argv[0]` resolves on
`PATH`; reject `rm -rf`, `curl | sh`, writes outside the workspace. MCP server
command resolves. Subagent allowlists run through `classifyAllowlist`
(`authoring.ts:51`), which today only *picks a gate* and should also be a line in
the record.

**G3 — Behavioral smoke** (executable capabilities only, seconds). Hook: run in a
sandboxed temp cwd against a synthetic event payload; require exit 0 and no
writes outside the sandbox. MCP: spawn, complete `initialize` + `tools/list`,
kill. Catches the most common real failure — a plugin that installs cleanly and
breaks the session at the next tool call.

**G4 — Trigger eval** (passive capabilities, one model call, opt-in). Whether an
authored skill is worth anything comes down to whether its `description` fires on
the situations it is for and stays quiet otherwise. Score against a small gold
set of positive and negative prompts.

G4 must **reuse `core/search/eval-harness.ts`** rather than re-implement it. That
module already solved the three problems any second eval subsystem rediscovers:
the corpus moves under you, nothing gets recorded, and a degraded run reads like
a real one (`search/eval-harness.ts:1-22`).

### 4.3 Where the gate binds

| Path | Gates | On failure |
|---|---|---|
| Authored, passive only | G1, G2 | Do not activate. Return the failure to the model — this is the loop that makes autonomous authoring converge instead of emitting silent garbage. |
| Authored, executable | G1–G3 **before** the confirm prompt | Never reach the human with a draft that does not run. |
| Marketplace install | G1 (round-trip only), G2 post-clone, pre-activate | Source trust is not content trust. |
| Publish | G1–G4, `--strict` | No green, no PR. |

### 4.4 The real argument for it

`buildReview` (`propose-plugin.ts:191-203`) shows the human a hook command string
and asks them to approve it. Someone confronted with
`jq -r '.tool_input.file_path' | xargs npm run lint:fix` at 2am has no basis to
say yes or no. The confirmation is a formality.

`G3: ran in sandbox, exit 0, no writes outside sandbox, no network` is what makes
that confirmation mean something. The eval is not primarily a quality gate — it
is what converts the existing human trust gate from theater into a decision.

---

## 5. Scope and location

### 5.1 The rule

> A surface is **repo-scoped** iff its content is a fact about *this codebase*.
> It is **user-scoped** iff its content is a fact about *how this person works*.

### 5.2 Applying it

**Skills, commands, agents, MCP, AGENTS.md — legitimately both, no change.**
"Run `bun run check`, never `bun test`" is a fact about this repo and belongs in
`./.agents/skills/`. "How I like commit messages" is a fact about the person.
Both scopes resolve correctly today (`skills.ts:485-494`; `mcp.json` at user and
project level).

**Plugins — user-scoped.** Portable, versioned, reusable across projects. Claude
Code draws the same line: standalone `.claude/` for "project-specific
customizations", plugins for "reusable across projects". Copilot CLI is per-user
only today (project scoping is an open upstream request).

### 5.3 Three locations, not one

The §1 split means "where does a plugin live" has different answers for
consumption and production.

| Role | Location | Why |
|---|---|---|
| **Consumption home** (marketplace installs) | `~/.agents/plugins/<id>/` | hoocode installing for itself. Format-agnostic. Must not leak into a vendor's directory — installing a plugin for hoocode should not silently add it to Claude Code. |
| **Production, `--platform claude`** | `~/.claude/skills/<id>/` | The documented drop-in. Live in Claude Code next session, and hoocode reads it back through the claude adapter. |
| **Production, `--platform github`** | `~/.agents/staged-plugins/<id>/` | No vendor drop-in exists (§1.5). Staging feeds `PackagePlugin` → publish, and `copilot --plugin-dir <path>` for local testing. |

### 5.4 The bug this replaces

`installedPluginsDir(cwd)` returns `<cwd>/.agents/plugins` (`install.ts:34-36`),
and `writePluginDraft` inherits it. **Every install and every autonomous
authoring writes into the user's repo working tree.** Three consequences:

1. The agent dirties `git status` with content unrelated to the change it was
   asked to make.
2. A capability gained in repo A is invisible in repo B, so the model re-authors
   it. The reuse flywheel the subsystem exists to spin never spins across
   projects — which is most of its value.
3. `marketplaceCacheDir` puts full git clones at
   `<cwd>/.agents/marketplace-cache/` (`install.ts:79`) — clones inside the
   user's clone.

The read side is already right: `defaultPluginDirs` searches project *and* global
scopes (`loader.ts:662-670`). Only the write side is wrong, which keeps the fix
small. `marketplace-cache/` and `marketplaces.json` move to the agent dir
unconditionally — a cache is never repo content.

### 5.5 Project scope, and the authoring decision rule

Keep an explicit `scope: "project"` opt-in for the real case: a team pinning a
plugin into the repo, committed deliberately.

Make scope selection a *decision rule* in `ProposePlugin`'s guidelines, not a
style note: **if the capability references this repo's paths, build system, or
conventions, it is a project skill, not a plugin.** The existing "author for
portability" guideline (`propose-plugin.ts:298`) gestures at this; it should be
the criterion.

### 5.6 The tension, stated

Writing to `~/.claude/skills/` means an autonomous action mutates another tool's
configuration directory. That is a genuine widening and should not be glossed.
Mitigations: it is announced, reversible via `UninstallPlugin`, lands in a
documented user-plugin path rather than arbitrary `$HOME`, and Claude Code's own
`plugin init` writes exactly there. Against it, today's behavior silently edits
the one directory the user is definitely about to `git add`. Both are widenings;
the current one is worse and less visible.

---

## 6. Embed search behind ToolSearch

### 6.1 Current state

Deferral (`plugin-system-spec.md` §2) is implemented, but only half the cost is
deferred:

- `formatDeferredCatalog` renders **the entire catalog** into the resolver tool's
  description (`mcp-deferred.ts:23-40`). Schemas are saved; name + description
  for every tool is still paid on every request. Against `AGENTS.md`'s ~2,710
  token default tool-schema budget, an unbounded catalog dominates.
- `selectResolvable` matches **exact strings only** (`mcp-deferred.ts:47-61`).

Those two are locked together: because the matcher is exact-only, the catalog
*must* be dumped in full for the resolver to be usable. Fix the matcher and the
dump becomes optional.

Meanwhile `core/embsearch/` and `core/search/hybrid-search.ts` (dense + BM25 +
RRF + cross-encoder rerank) exist and point only at repo files.

### 6.2 One capability index

Documents: `{ id, kind, name, description, source, deferred }` with
`kind ∈ {mcp-tool, skill, command, agent, plugin-available, plugin-installed}`.
Four producers, one index, one retrieval tool.

- **Separate embsearch store**, keyed on a hash of the capability set — not the
  repo store. Different lifecycle, different invalidation, and the repo service
  goes dormant below its byte threshold (`embsearch-service.ts:1-15`). A
  capability index must work in a small repo.
- **Hybrid, not dense-only.** Tool descriptions are short; pure dense retrieval
  on eight-word strings is weak, and exact-name lookup must stay exact.
  `hybrid-search.ts` already fuses BM25 and dense via RRF — `select:Read,Edit`
  resolves through the lexical leg, "something that sends email" through the
  dense leg.

### 6.3 `deferred` as a policy knob

Today deferral is one global flag (`mcp-loader.ts:638`). It should be
per-capability and policy-driven: capability count, measured token cost,
used-this-session, plugin-provided vs built-in.

Rule of thumb: **eager below the threshold, deferred above it.** Below roughly
20–30 entries a flat catalog is both cheaper than a retrieval round-trip and
strictly more deterministic. `plugin-system-spec.md` already reasons this way for
subagents; this generalizes it and makes the threshold measurable via
`hoocode --print-token-surface`.

### 6.4 Retrieval is a determinism regression

A flat catalog guarantees the model can see every capability. A retrieval tool
guarantees only that it sees the top-k for a query it thought to write. That is
exactly why `plugin-system-spec.md` says "Do NOT add `SearchSkills`", and that
call should stand until skill count forces it. Rollout: build the index, wire
**MCP tools first** (already deferred, exact-match already a known limitation, so
strictly an improvement), leave skills/commands/agents eager, promote by
measurement.

---

## 7. Comparison — Claude Code, Copilot CLI, hoocode

Claude rows from `code.claude.com/docs/en/plugins`; Copilot rows via search and
`github/copilot-cli` discussions (primary docs proxy-blocked here). Both
2026-08-09.

| | Claude Code | Copilot CLI | hoocode (proposed) |
|---|---|---|---|
| Formats read | Claude | 4 locations incl. `.claude-plugin/` | **all three** |
| Format written | Claude | Copilot | **target platform only, never `agents`** |
| Who installs | human | human | **model**, within trusted marketplaces |
| Who authors | human (`plugin init`) | human | **model** (`ProposePlugin`) |
| Validation | `claude plugin validate` | — | G1–G4, delegating to `claude plugin validate` |
| Plugin scope | user | user | user (consumption + production split, §5.3) |
| Local drop-in | `~/.claude/skills/<id>/` | none; local installs deprecated | follows the platform |
| MCP schema loading | eager | eager | **deferred** |

### 7.1 Where we align, and why alignment is the default

Manifest shape, marketplace index shape, hook events, `SKILL.md` frontmatter,
`metadata.pluginRoot`, SHA pinning, curated official marketplace — and now the
production model itself. hoocode's stated value is reading *every* vendor
convention (`agent-spec-tree-map.md`), so format divergence is pure cost.
Compatibility is the product.

### 7.2 Where we differ, and why

1. **The agent authors and installs its own capabilities.** The one real
   divergence; everything below is downstream. Claude's and Copilot's plugins are
   human-authored artifacts a human installs. hoocode's thesis is that the agent
   closes its own capability gaps mid-task.
2. **Risk computed from content, not inferred from authorship.** Claude can rely
   on "a human wrote and reviewed this." We cannot, so we classify the draft
   (`authoring.ts:51`) and let content pick the gate.
3. **Reading all three formats** while writing one. Each vendor reads and writes
   only its own. We are the only one that has to normalize across them, which is
   what the native format is *for* — a read-side normalization target, not a
   distribution format.
4. **Deferred MCP schemas.** Neither vendor does this. Justified by the per-turn
   token budget discipline in `AGENTS.md`.
5. **Eval before activation.** Claude validates before *submission*; we validate
   before *activation*, because our plugins are machine-authored and go live in
   the same turn. Same idea, earlier position, forced by the autonomy — and we
   delegate to their validator where it exists rather than inventing our own.

Previously listed as a divergence and now retired: "vendor-neutral `.agents` as
the write target." §1 replaces it with alignment.

### 7.3 What we deliberately do not adopt

- **Autonomous publish** — supply-chain (§3.3).
- **Autonomous marketplace add** — the human trust boundary, unchanged.
- **Autonomous update of marketplace plugins** — benign v1 → hostile v2. Claude
  mitigates with review, SHA pinning, and CI pin bumps; we have none, so our rule
  stays stricter.

### 7.4 Compatibility gaps to close

From §2.1: `.lsp.json`, `monitors/monitors.json`, `bin/` on `PATH`, plugin-root
`settings.json`, manifest-less directories, single-`SKILL.md` plugins, skill
namespacing, and a `--plugin-dir` equivalent. These are now **production**
blockers, not just read-side gaps.

---

## 8. End-to-end review, and the revised order

### 8.1 What the production-model correction changed

| Area | Before | After |
|---|---|---|
| Write target | `agents` default, vendor opt-in | platform-only, default `claude`, `agents` rejected |
| Resolver | one shared `resolveAuthoringPlatforms` | split plugin vs workspace |
| Flag | `--support-platform` | `--platform` |
| Location | one global home | consumption home + per-platform production target (§5.3) |
| G1 | round-trip through our parser | target-platform conformance, delegating to `claude plugin validate` |
| Claude adapter | step 4, read-side nicety | near-front, production blocker |
| Publish lane | optional flywheel endpoint | the *only* GitHub route |
| §7 divergence #3 | a deliberate difference | retired; now alignment |

Two things the correction did **not** change: §6 (retrieval is orthogonal to
format), and the §5.1 scope rule (plugins were already user-scoped; only the
destination moved).

### 8.2 Open items before scoping

1. **Verify the GitHub rows** in §1.5 and §5.3 against the primary Copilot CLI
   reference from an unblocked network. The whole `github` branch of the plan
   rests on second-hand sourcing.
2. **Migration.** Plugins already installed at `<cwd>/.agents/plugins/` need a
   one-time move or a deprecation read-path. `defaultPluginDirs` already reads
   both, so a read-only grace period is nearly free.
3. **Does `--platform` accept multiple values?** `github/copilot-plugins` ships
   both a `.github/plugin/` and a `.claude-plugin/` index, so dual-emission is a
   real shape. Recommend allowing it but never defaulting to it.
4. **`~/.claude/skills/<id>/` collision** with a plugin the user installed
   through Claude Code itself. Needs a namespace convention or a pre-write check.

### 8.3 Revised order

Resequenced so each step unblocks the next; the first three are now one coherent
change rather than three independent ones.

| Step | Work | Why here |
|---|---|---|
| 1 | Split the resolver; add `--platform`; reject `agents` for plugins | Everything else reads from this. Small and mechanical. |
| 2 | Production + consumption locations (§5.3), migration read-path | Same change in practice as step 1 — format and destination are one decision |
| 3 | Claude adapter catch-up (§2.1 table) | Now a production blocker. Emitting non-conforming Claude plugins is worse than failing to read them |
| 4 | G1 + G2, delegating to `claude plugin validate` | Cheapest real green signal; step 3 makes it pass for the right reasons |
| 5 | Tier 1 lenient reader + `unsupportedSurfaces` | Turns the *next* drift into a signal instead of a silent gap |
| 6 | G3 sandboxed smoke | Makes the executable confirm gate meaningful |
| 7 | `PackagePlugin` + publish lane, GitHub flow primary | Depends on eval being green-signal-capable; load-bearing for GitHub |
| 8 | Capability index + MCP retrieval (§6) | Largest build, fully independent — can run in parallel from step 1 |
| 9 | G4 trigger eval, Tier 2 drift CI | Ongoing quality, not blocking |
