# Plugin System Architecture — drift, automation, eval, scope, retrieval

**Status:** proposal (nothing here is implemented yet)
**Scope:** `packages/coding-agent` plugin + capability subsystem
**Companions:** `docs/plugin-system-spec.md` (what shipped),
`docs/plugin-format-mapping.md` (format tables),
`docs/agent-spec-tree-map.md` (on-disk surfaces)

Follows the `agent-spec-tree-map.md` rule: every "hoocode does X" claim carries a
`file:line` citation. Vendor claims are cited to the doc page they came from and
dated, because they are the thing most likely to go stale.

---

## 0. Summary of the six positions

| # | Question | Position |
|---|---|---|
| 1 | What if the vendor plugin standard changes? | Read leniently, write pinned, learn offline. Never let a runtime network fetch decide how to interpret executable content. |
| 2 | Total automation of capability inheritance + publishing? | Total up to the trust boundary, hard stop at it. Automate everything about publishing except the publish. |
| 3 | Missing eval / green signal | Four gates, cheap→expensive. Its real job is not quality — it is making the existing human confirm prompt non-theatrical. |
| 4 | Which surfaces are per-repo? | Repo-scoped iff the content is a fact about *this codebase*. Plugins are not. Today we write them into the user's working tree — that is a bug. |
| 5 | Embed search behind ToolSearch | One capability index, hybrid retrieval, per-capability `deferred` policy with a measured threshold. Retrieval is a scale mechanism, not an upgrade. |
| 6 | Aligned or different from Claude / GitHub? | Aligned on every format detail. Different on exactly one axis — the agent authors its own capabilities — and every other divergence is downstream of that. |

---

## 1. Standard drift

### 1.1 The seam exists; the contents are stale

`PluginFormatAdapter` (`formats/types.ts:124-152`) is the right seam: adding or
tracking a vendor means editing one adapter file. The Copilot adapter is current
— it probes all four documented manifest locations
(`formats/copilot.ts:73-77`), matching the Copilot CLI plugin reference.

The Claude adapter is not. Reading `code.claude.com/docs/en/plugins` (fetched
2026-08-09) against `formats/claude.ts` + `formats/jsonManifest.ts`:

| Claude Code convention | hoocode today |
|---|---|
| Manifest is **optional** — a directory with `skills/` alone is a plugin | `detectPlugin` requires `.claude-plugin/plugin.json`, returns false otherwise (`jsonManifest.ts:52-54`) |
| A single `SKILL.md` at plugin root = a one-skill plugin | not detected |
| `.lsp.json` — LSP servers | not parsed |
| `monitors/monitors.json` — background monitors | not parsed |
| `bin/` — added to Bash `PATH` while enabled | not parsed |
| plugin-root `settings.json` (`agent`, `subagentStatusLine`) | not parsed |
| Skills namespaced `/plugin-name:skill-name` | not applied |

So "if the standard changes" is not hypothetical. It already changed, in at
least seven places, and nothing in the system noticed. That is the actual defect
— not the absence of a doc-fetching mechanism, but the absence of a *signal*.

### 1.2 The ladder

Four tiers, ordered by how much determinism each one costs.

**Tier 0 — pinned adapters stay the only writer.** Emission must be
byte-reproducible and must round-trip through our own parser. A model-inferred
or doc-inferred *write* turns one bad guess into a corrupt artifact in someone's
repo, possibly published. Non-negotiable.

**Tier 1 — lenient reader that never loses data.** Add to `NormalizedPlugin`
(`manifest.ts:51-82`):

```ts
/** Manifest keys present on disk that this adapter does not model. */
unknownFields?: Record<string, unknown>;
/** On-disk surfaces present but unhandled, e.g. [".lsp.json", "monitors/"]. */
unsupportedSurfaces?: string[];
```

`ListPlugins` then reports `foo@1.2.0 [claude] — skills, hooks (2 surfaces
present but unsupported: .lsp.json, monitors/)`. This is the whole fix for the
signal problem, it costs near-zero, and it turns silent drift into a visible
diagnostic the moment a drifted plugin is installed.

**Tier 2 — offline drift check, human-ratified.** A scheduled/CI job fetches the
vendor reference pages, diffs the documented field and path sets against each
adapter's declared sets, and opens a report. This is the same discipline
`AGENTS.md` already mandates for `packages/ai/src/models.generated.ts` ("never
modify the generated file; update the generator"): the network is allowed to
inform the codegen input, never the runtime.

**Tier 3 — model fallback, quarantined.** When a directory parses to nothing and
Tier 1 found unmodeled surfaces, the model may read the vendor docs and draft an
*adapter patch*. It lands as a proposal for human review, never as a live parse.

### 1.3 Why not "pass it to the model at runtime"

Three reasons, in order of severity:

1. **Trust.** A model-inferred parse of `hooks/` or `.mcp.json` is a
   code-execution decision derived from fetched web text. `plugin-system-spec.md`
   already forbids exactly this shape under the injection carve-out. Runtime
   doc-fetching would route around it.
2. **Determinism.** The same plugin directory could parse differently in two
   sessions. Debugging that is miserable and the failure is silent.
3. **Hot path.** Format interpretation runs at session start
   (`loader.ts:644-649`). A network round-trip there is a latency and
   availability regression for a case that is rare by construction.

Runtime leniency + offline learning gets the same coverage without any of the
three.

---

## 2. Automation, and the two use cases

### 2.1 Read/inherit — mostly built, two gaps

Working today: curated indices clone lazily on first search
(`install.ts:88-101`), install is autonomous within a trusted marketplace
(`tools/plugins.ts:239-255`), passive capabilities activate live the same turn
(`propose-plugin.ts:326`), and the runtime arms reuse cues that surface into
`SearchPlugins` (`tools/plugins.ts:108-111`).

Two gaps:

- **Indices are cloned once and never refreshed** (`install.ts:91-92` skips when
  the directory exists). The inherit story goes stale silently. Add a TTL plus
  an explicit refresh. Refreshing an *index* is read-only and safe to automate —
  it changes only what is discoverable, not what is installed. The no-auto-update
  rule for installed plugins stands unchanged.
- **Discovery is substring matching** (`tools/plugins.ts:96`). Fine for two
  curated marketplaces; it fails at community-catalog scale. This is §5's job.

### 2.2 Publish — absent, and should stay human at exactly one step

There is no packaging, no index-entry generation, no validation, no publish path
at all. But "publishing is a human act" does not mean "no tooling" — Claude Code
ships `claude plugin validate` and an automated submission/review pipeline; the
human part is the *approval*, not the mechanics.

Proposed pipeline, autonomous until the last step:

1. **`PackagePlugin`** (autonomous) — take an authored `.agents/plugins/<id>/`,
   emit the requested vendor layouts, generate `README.md`, produce the
   `marketplace.json` entry stanza. Writes to a staging directory. Touches no
   remote.
2. **`PluginEval`** (autonomous) — §3. Must be green.
3. **Publish** (human) — opens the PR against the marketplace repo.

### 2.3 Against total automation of the last step

The request was total automation. For step 3 the answer should be no, and the
reason is worth stating rather than hiding behind policy: an agent that can
autonomously publish executable code into a marketplace that other agents
autonomously install from is a supply-chain compromise primitive. The install
side is *deliberately* autonomous (`plugin-system-spec.md`, trust model) on the
strength of the marketplace boundary vouching for the code. Automating the
publish side dissolves the thing the install side is trusting.

Claude Code holds the same line with more machinery than we have — human
submission form, review pipeline, automated safety screening, commit-SHA pinning
in the community catalog. We have none of that, so our line has to be at least
as conservative.

Everything before the button is automatable, and should be.

---

## 3. Eval — the missing green signal

### 3.1 What is missing

Nothing runs between "model writes plugin draft" and "plugin is live in this
session": `writePluginDraft` is followed directly by `ctx.activatePlugin(dest)`
(`propose-plugin.ts:323-326`). No check that it parses, that the hook's binary
exists, that the MCP server starts, or that the skill ever triggers.
`plugin-system-spec.md:203-209` lists round-trip tests as *tests* — there is no
runtime gate.

Claude Code's analogue is `claude plugin validate`, run before submission and
again in review. That is static validation, and for Claude it is proportionate:
their plugins are human-written. Ours are machine-written and activate in the
same turn. Same idea, but it has to sit earlier in the pipeline and go further.

### 3.2 Four gates

All must pass. Ordered cheap→expensive so the common case is milliseconds.

**G1 — Structural** (always, ~ms). `parsePluginDir(dest)` returns non-null and
reproduces every capability in the draft. Manifest schema, id/slug validity, no
absolute or machine-specific paths in emitted content, no high-entropy strings
or `sk-`/`AWS_` patterns. Note that portability is currently only a
`promptGuidelines` suggestion (`propose-plugin.ts:298`) — G1 makes it enforced.

**G2 — Static safety** (always, ~ms). Hook commands parse; `argv[0]` resolves on
`PATH`; reject `rm -rf`, `curl | sh`, writes outside the workspace. MCP server
command resolves. Subagent allowlists run through the existing
`classifyAllowlist` (`authoring.ts:51`) — which today only *picks a gate*; it
should also be a line in the eval record.

**G3 — Behavioral smoke** (executable capabilities only, seconds). Hook: run in
a sandboxed temp cwd against a synthetic event payload; require exit 0 and no
writes outside the sandbox. MCP: spawn, complete `initialize` + `tools/list`,
kill. This catches the most common real failure mode — a plugin that installs
cleanly and breaks the session at the next tool call.

**G4 — Trigger eval** (passive capabilities, one model call, opt-in). Whether an
authored skill is worth anything comes down to whether its `description` fires
on the situations it is for and stays quiet otherwise. Score it against a small
gold set of positive and negative prompts; report precision/recall of selection.

G4 must **reuse `core/search/eval-harness.ts`**, not re-implement it. That module
already solved the three problems any second eval subsystem would rediscover:
the corpus moves under you, nothing gets recorded, and a degraded run reads like
a real one (`search/eval-harness.ts:1-22`). Pinned corpus, machine-readable run
record, and a writer that refuses to hide degradation — inherit all three.

### 3.3 Where the gate binds

| Path | Gates | On failure |
|---|---|---|
| Authored, passive only | G1, G2 | Do not activate. Return the failure to the model — this is the loop that makes autonomous authoring converge instead of emitting silent garbage. |
| Authored, executable | G1–G3 **before** the confirm prompt | Never reach the human with a draft that does not run. |
| Marketplace install | G1, G2 post-clone, pre-activate | Source trust is not content trust. Cheap insurance. |
| Publish | G1–G4, `--strict` | No green, no PR. |

### 3.4 The real argument for it

Today `buildReview` (`propose-plugin.ts:191-203`) shows the human a hook command
string and asks them to approve it. Someone confronted with
`jq -r '.tool_input.file_path' | xargs npm run lint:fix` at 2am has no basis to
say yes or no. The confirmation is a formality.

`G3: ran in sandbox, exit 0, no writes outside sandbox, no network` is what makes
that confirmation mean something. The eval is not primarily a quality gate. It is
what converts the existing human trust gate from theater into a decision.

---

## 4. Surface scoping — repo-owned vs inherited

### 4.1 The rule

> A surface is **repo-scoped** iff its content is a fact about *this codebase*.
> It is **user-scoped** iff its content is a fact about *how this person works*.

### 4.2 Applying it

**Skills, commands, agents, MCP, AGENTS.md — legitimately both, no change
needed.** "Run `bun run check`, never `bun test`" is a fact about this repo and
belongs in `./.agents/skills/`. "How I like commit messages" is a fact about the
person. Both scopes already exist and resolve correctly (`skills.ts:485-494`;
`mcp.json` at user and project level per `AGENTS.md`).

**Plugins — user-scoped.** A plugin is by definition portable, versioned, and
reusable across projects. Claude Code draws the same line explicitly: standalone
`.claude/` for "project-specific customizations", plugins for "reusable across
projects" (`code.claude.com/docs/en/plugins`, fetched 2026-08-09).

### 4.3 The bug

`installedPluginsDir(cwd)` returns `<cwd>/.agents/plugins`
(`install.ts:34-36`), and `writePluginDraft` inherits it. **Every install and
every autonomous authoring writes into the user's repo working tree.**

Three consequences:

1. The agent dirties `git status` in a repo it was asked to work on, with content
   unrelated to the change.
2. A capability gained in repo A is invisible in repo B, so the model re-authors
   it. The reuse flywheel the entire subsystem exists to spin never spins across
   projects — which is most of its value.
3. `marketplaceCacheDir` puts full git clones of marketplace repos at
   `<cwd>/.agents/marketplace-cache/` (`install.ts:79`) — clones inside the
   user's clone.

The read side is already right: `defaultPluginDirs` searches project *and*
global scopes (`loader.ts:662-670`). Only the write side is wrong, which makes
the fix small.

### 4.4 Proposal

- Default write target for `InstallPlugin` and `ProposePlugin` → global
  `~/.agents/plugins/`.
- Move `marketplace-cache/` and `marketplaces.json` to the agent dir
  unconditionally. A cache is never repo content.
- Keep project scope as an explicit `scope: "project"` opt-in for the real case:
  a team pinning a plugin into the repo, committed deliberately.
- Make scope selection a *decision rule* in `ProposePlugin`'s guidelines, not a
  style note: **if the capability references this repo's paths, build system, or
  conventions, it is a project skill, not a plugin.** The existing "author for
  portability" guideline (`propose-plugin.ts:298`) gestures at this; it should be
  the criterion.

### 4.5 The tension, stated

Global-by-default means an autonomous action mutates state outside the repo the
user pointed the agent at. That is a genuine widening, and worth naming rather
than glossing. Mitigations: it is announced, reversible via `UninstallPlugin`,
and lands in the agent's own config directory, not arbitrary `$HOME`. Against
that, the current behavior silently edits the one directory the user is
definitely about to `git add`. Global is the safer of the two.

Claude Code's `claude plugin init` already defaults to `~/.claude/skills/`, so
this change is us re-aligning after drifting, not innovating.

---

## 5. Embed search behind ToolSearch

### 5.1 Current state

Deferral (`plugin-system-spec.md` §2) is implemented, but only half the cost is
actually deferred:

- `formatDeferredCatalog` renders **the entire catalog** into the resolver tool's
  description (`mcp-deferred.ts:23-40`). The JSON schemas are saved; name +
  description for every tool is still paid on every request, forever. Against
  `AGENTS.md`'s ~2,710-token default tool-schema budget, an unbounded catalog
  dominates the surface.
- `selectResolvable` matches **exact strings only** (`mcp-deferred.ts:47-61`).
  The model must already know the name.

Those two are locked together: because the matcher is exact-only, the catalog
*has* to be dumped in full for the resolver to be usable. Fix the matcher and the
dump becomes optional.

Meanwhile `core/embsearch/` and `core/search/hybrid-search.ts` (dense + BM25 +
RRF + cross-encoder rerank) exist and point only at repo files.

### 5.2 One capability index

Documents: `{ id, kind, name, description, source, deferred }` with
`kind ∈ {mcp-tool, skill, command, agent, plugin-available, plugin-installed}`.
Four producers (MCP loader, skills loader, agent registry, marketplace lister),
one index, one retrieval tool.

Two implementation constraints that matter:

- **Separate embsearch store**, keyed on a hash of the capability set — not the
  repo store. Different lifecycle, different invalidation, and critically the
  repo service goes dormant below its byte threshold
  (`embsearch-service.ts:1-15`). A capability index must work in a small repo.
- **Hybrid, not dense-only.** Tool descriptions are short; pure dense retrieval
  on eight-word strings is weak, and exact-name lookup must stay exact.
  `hybrid-search.ts` already fuses BM25 and dense via RRF, which is exactly the
  right shape: `select:Read,Edit` resolves through the lexical leg, "something
  that sends email" through the dense leg.

### 5.3 The `deferred` field as a policy knob

Today deferral is one global flag (`mcp-loader.ts:638`). It should be
per-capability and policy-driven, with inputs: capability count, measured token
cost, used-this-session, and plugin-provided vs built-in.

Rule of thumb: **eager below the threshold, deferred above it.** Below roughly
20–30 entries a flat catalog is both cheaper than a retrieval round-trip and
strictly more deterministic. `plugin-system-spec.md` already reasons this way for
subagents ("keep the catalog eager … until agent count makes the catalog itself
the bottleneck"); this generalizes it and makes the threshold measurable via
`hoocode --print-token-surface` rather than a judgment call.

### 5.4 Retrieval is a determinism regression

Worth being blunt about, because it argues for a narrow rollout. A flat catalog
guarantees the model can see every capability. A retrieval tool guarantees only
that it sees the top-k for a query it thought to write. That is precisely why
`plugin-system-spec.md` says "Do NOT add `SearchSkills`" — and that call should
stand until skill count actually forces it.

Rollout, therefore:

1. Build the index and hybrid retrieval.
2. Wire **MCP tools** first — already deferred, and the exact-match matcher is a
   known limitation, so it is strictly an improvement there.
3. Leave skills, commands, and agents eager.
4. Let the measured threshold promote them when a real deployment crosses it.

---

## 6. Comparison — Claude Code, Copilot CLI, hoocode

Vendor rows from `code.claude.com/docs/en/plugins` and the Copilot CLI plugin
reference, both fetched 2026-08-09.

| | Claude Code | Copilot CLI | hoocode |
|---|---|---|---|
| Manifest | `.claude-plugin/plugin.json`, optional | 4 probe locations incl. `.claude-plugin/` | `.agents-plugin/`, reads all three |
| Marketplace index | `.claude-plugin/marketplace.json` | `.github/plugin/marketplace.json` + `.claude-plugin/` | reads all three, native first |
| Who installs | human (`/plugin install`) | human | **model**, within trusted marketplaces |
| Who authors | human (`claude plugin init`) | human | **model** (`ProposePlugin`) |
| Validation | `claude plugin validate` pre-submission | — | **none today** (§3) |
| Default scope | `~/.claude/skills/` (user) | user | `<cwd>/.agents/plugins` (§4 — wrong) |
| MCP schema loading | eager | eager | **deferred** |
| Publish | human form + review + SHA pinning | marketplace repo PR | none today |

### 6.1 Where we align, and why alignment is the default

Manifest shape, marketplace index shape, hook event names, `SKILL.md` frontmatter,
`metadata.pluginRoot`, SHA pinning, a curated official marketplace. hoocode's
stated value is reading *every* vendor convention (`agent-spec-tree-map.md`), so
divergence in the format is pure cost with no upside. Format compatibility is the
product.

### 6.2 Where we differ, and why

1. **The agent authors and installs its own capabilities.** This is the one real
   divergence; everything below is downstream of it. Claude's and Copilot's
   plugins are human-authored artifacts a human installs. hoocode's thesis is
   that the agent closes its own capability gaps mid-task.
2. **Risk computed from content, not inferred from authorship.** Claude can rely
   on "a human wrote and reviewed this." We cannot, so we classify the draft
   (`authoring.ts:51`) and let the content pick the gate.
3. **Vendor-neutral `.agents` as the write target** while reading all three
   formats. Each vendor writes its own layout; the one system reading everyone's
   needs its native format to be a superset.
4. **Deferred MCP schemas.** Neither vendor does this. Justified by the per-turn
   token budget discipline in `AGENTS.md`, which is a hoocode-specific constraint
   we have chosen to take seriously.
5. **Eval before activation** (§3, proposed). Claude validates before
   *submission*; we must validate before *activation*, because our plugins are
   machine-authored and go live in the same turn. Same idea, earlier position,
   forced by the autonomy.
6. **Global-default plugin scope** (§4, proposed). Not a divergence — a
   correction back toward what Claude already does.

### 6.3 What we deliberately do not adopt

- **Autonomous publish** — supply-chain (§2.3).
- **Autonomous marketplace add** — the human trust boundary, unchanged.
- **Autonomous update of marketplace plugins** — benign v1 → hostile v2. Claude
  mitigates with review, SHA pinning, and CI pin bumps; we have none of those, so
  our rule stays stricter.

### 6.4 Compatibility gaps to close (from §1.1)

`.lsp.json`, `monitors/monitors.json`, `bin/` on `PATH`, plugin-root
`settings.json`, manifest-less plugin directories, single-`SKILL.md` plugins,
skill namespacing, and a `--plugin-dir`-equivalent for local testing. The last is
worth prioritizing beyond parity: a local plugin-dir load is the natural place to
run `PluginEval`.

---

## 7. Suggested order

Sequenced by (value ÷ cost), and because each step makes the next safer.

| Step | Work | Why first |
|---|---|---|
| 1 | §4 scope fix (write target, cache relocation) | Small, fixes an active bug, unlocks cross-repo reuse |
| 2 | §3 G1+G2 gates | Small, closes the widest correctness hole, no new subsystems |
| 3 | §1 Tier 1 lenient reader + `unsupportedSurfaces` | Small, turns future drift into a visible signal |
| 4 | §1.1 Claude adapter catch-up | Now measurable via step 3 |
| 5 | §3 G3 sandboxed smoke | Makes the executable confirm gate meaningful |
| 6 | §5 capability index + MCP retrieval | Largest build; deferred MCP is the only surface that needs it today |
| 7 | §2.2 package + publish pipeline | Depends on eval being green-signal-capable |
| 8 | §3 G4 trigger eval, §1 Tier 2 drift CI | Ongoing quality, not blocking |
