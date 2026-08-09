# Plugin System Architecture — production model, drift, automation, eval, scope, retrieval

**Status:** agreed plan; steps 0–2 landed, steps 3+ not started (§8.5)
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
(`plugins/marketplace.ts:188-244`, `formats/types.ts:124-152`). Nothing there
changes.

Writing was wrong. `DEFAULT_AUTHORING_PLATFORMS = ["agents"]` made the portable
native layout the default production target for authored plugins, with vendor
layouts as opt-in interop — inverting the actual relationship. **Fixed in step 1**:
`DEFAULT_PLUGIN_PLATFORMS = ["claude"]` and `agents` is no longer reachable as a
plugin target.

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

One flag, `--platform claude|github`, replaces `--support-platform`. Per `AGENTS.md` ("do not preserve backward compatibility
unless the user explicitly asks") this is a rename, not an alias.

The resolver had to split, because the two consumers have opposite rules and
shared one function (`resolveAuthoringPlatforms`, consumed by plugin authoring
and by `scaffold.ts`):

```ts
/** Plugin production. Never returns "agents". Defaults to claude. */
resolvePluginPlatforms(explicit?): PluginPlatform[]

/** Workspace scaffolds. Session value or undefined — callers keep their own
 *  fallback (scaffolds fall back to `.hoocode/`, not to a platform). */
getWorkspacePlatforms(): MarketplacePlatform[] | undefined
```

As built, the workspace side needed no resolver at all: `resolveAuthoringPlatforms`
turned out to have exactly one consumer group — plugin authoring — while the
scaffolds read the raw session value and supply their own `.hoocode/` fallback.
So the "split" is one renamed-and-narrowed function plus one accessor.

`resolvePluginPlatforms` resolution order: explicit per-call → session
`--platform` → `["claude"]`.

`agents` is handled two ways, deliberately. An **explicit** `agents` throws — a
caller asking for an unpublishable artifact should hear about it. A
**session-level** `agents` is filtered instead, because `--platform` also drives
workspace scaffolds where `agents` is a perfectly good choice; a user who set it
for that reason should not have plugin authoring blow up. If filtering leaves
nothing, the default applies.

Default `claude` rather than erroring keeps the autonomous capability-gap flow
unbroken, and §1.5 shows it is also the only platform where the local loop
actually closes.

### 1.5 The two platforms are not symmetric

| | Claude Code | Copilot CLI |
|---|---|---|
| Hand-writable auto-load path | `~/.claude/skills/<id>/` — documented, what `claude plugin init` writes, loads with no install step | none — installs copy into `~/.copilot/installed-plugins/`, nothing is discovered in place |
| Local dev loop | `--plugin-dir`, `--plugin-url`, or the skills-dir path above | `copilot plugin install ./dir` (copies; reinstall per change), or `--plugin-dir` |
| Route into the ecosystem | drop-in, or submission form + review | marketplace publish |

**Sourcing.** Claude rows: `code.claude.com/docs/en/plugins` and
`/plugins-reference`, fetched directly. Copilot rows: the `github/docs` source
repo (§1.7), because `docs.github.com` is blocked by this environment's egress
proxy. Both 2026-08-09.

Two Copilot claims remain **second-hand** and are deliberately not load-bearing
anywhere in this plan:

- `~/.copilot/installed-plugins/` being "managed by the plugins themselves and
  should not be edited manually" — from search results, not the fetched files.
- The deprecation warning on direct installs ("only `plugin@marketplace` installs
  will be supported in a future release") — from
  `github.com/github/copilot-cli/discussions/3685`.

The first-hand facts alone already settle the decision that matters: installs
*copy* into a cache and require a reinstall per change (§1.7), so there is no
in-place drop-in to target regardless of whether the deprecation lands.

Consequence: for `claude`, authoring produces a plugin live in Claude Code on its
next session. For `github` there is no such path, so "production" means the
publish lane in §3.2 and nothing shorter. That asymmetry is upstream reality, not
a design choice.

### 1.6 Skills-directory plugins — the verified contract

Verified against `plugins-reference` §"Skills-directory plugins" (2026-08-09),
because the whole `claude` production target rests on it.

> "Any folder under a skills directory that contains a `.claude-plugin/plugin.json`
> manifest is loaded as a plugin named `<name>@skills-dir` on the next session,
> with no marketplace and no install step … discovered in place rather than
> copied into the plugin cache."

A skills tree holds three distinct things:

| On disk | What it is |
|---|---|
| `<skills-dir>/foo/SKILL.md`, no manifest | plain skill `foo` |
| `<skills-dir>/foo/.claude-plugin/plugin.json` | plugin `foo@skills-dir` |
| `<plugin>/skills/bar/SKILL.md` | skill `bar` **packaged inside** a plugin |

Three consequences that shape §5.3 and §5.7:

1. **The manifest is the promoter.** That is exactly `parsePluginDir` returning
   null for a manifest-less directory, so adding `~/.claude/skills` to
   `defaultPluginDirs` reproduces Claude's rule with no special-casing.
2. **Row 3 does not surface as a top-level skill.** hoocode's scanner recurses
   without bound (`skills.ts:277`), so today it would. See §5.7.
3. **Scope differs in kind, not just path.** `~/.claude/skills/` (personal) has
   no restrictions. `<cwd>/.claude/skills/` (project) loads only after the
   workspace trust dialog; its MCP servers need per-server approval, LSP needs
   trust, and monitors do not load at all. Project-scope skills-dir plugins also
   **do not walk up to the repo root** the way plain skills do.

Removal is "delete its folder" (or `claude plugin disable <name>@skills-dir`) —
there is no uninstall step, which is already what `uninstallPlugin` does
(`install.ts:339-348`). No change needed there.

### 1.7 GitHub Copilot — the verified contract

Verified against `github/docs` at
`content/copilot/reference/copilot-cli-reference/cli-plugin-reference.md` and
`content/copilot/how-tos/copilot-cli/customize-copilot/plugins-creating.md`
(2026-08-09). Fetched from the docs source repo because `docs.github.com` is
blocked by this environment's egress proxy. Everything in this section is
first-hand; the two remaining second-hand claims are quarantined in §1.5.

**Manifest probe order** (first match wins):
`.plugin/plugin.json` → `plugin.json` → `.github/plugin/plugin.json` →
`.claude-plugin/plugin.json`.

**Manifest component fields:** `agents`, `skills`, `commands`, `hooks`,
`extensions`, `mcpServers`, `lspServers`. No `themes`.

| Component | Location |
|---|---|
| Agents | `agents/<name>.agent.md` (frontmatter `tools` as a YAML list) |
| Skills | `skills/<name>/SKILL.md` |
| Hooks | `hooks.json` **or** `hooks/hooks.json` |
| MCP | `.mcp.json` **or** `.github/mcp.json` |
| LSP | `lsp.json` **or** `.github/lsp.json` |
| Installed plugins | `~/.copilot/installed-plugins/<marketplace>/<name>` or `.../_direct/<source-id>/` |
| Marketplace cache | `~/.cache/copilot/marketplaces/` (Linux), `~/Library/Caches/copilot/marketplaces/` (macOS) |

**Loading precedence** is not uniform: agents and skills are *first-found-wins*
(project/personal override plugins), while MCP servers are *last-wins* (plugins
override user config). hoocode is first-wins throughout.

**Runtime variables:** `${PLUGIN_ROOT}` for paths inside the plugin directory,
and `${COPILOT_PLUGIN_DATA}` (also spelled `${CLAUDE_PLUGIN_DATA}`) — "a
persistent, writable directory unique to each installed plugin … instead of paths
inside the installed-plugins cache directory."

**There is no `copilot plugin validate`.** This confirms rather than assumes the
§4.2 fallback for `github`.

**Local loop, confirmed:** `copilot plugin install ./my-plugin` works today but
*copies* into `~/.copilot/installed-plugins/_direct/<source-id>/`; component
changes require a reinstall. Combined with the CLI's own deprecation warning on
direct installs, there is no in-place drop-in — which is what makes the
hoocode-owned production home in §5.3 the right target rather than a workaround.
`copilot plugin uninstall <name>` keys off the manifest `name`, not the path.

---

## 2. Standard drift

### 2.1 The seam exists; the contents are stale

`PluginFormatAdapter` (`formats/types.ts:124-152`) is the right seam: tracking a
vendor means editing one adapter file. Both adapters have drifted, the Claude one
much further. Component lists are taken from each vendor's authoritative
file-locations table, not inferred.

#### Claude Code (`formats/claude.ts` + `formats/jsonManifest.ts`)

| Claude Code convention | hoocode today |
|---|---|
| Manifest optional **for marketplace / `--plugin-dir` plugins** — components auto-discovered, name derived from the directory | `detectPlugin` requires a manifest (`jsonManifest.ts:52-54`) |
| A single `SKILL.md` at plugin root = a one-skill plugin | not detected |
| `workflows/` — workflow scripts | not parsed |
| `output-styles/` — output style definitions | not parsed |
| `.lsp.json` — LSP servers | not parsed |
| `monitors/monitors.json` — background monitors | not parsed |
| `bin/` — added to the Bash tool's `PATH` while enabled | not parsed |
| plugin-root `settings.json` (`agent`, `subagentStatusLine` only) | not parsed |
| Skills namespaced `plugin-name:skill-name` | flat namespace, first-wins + collision diagnostic |

`themes/` we do support. Note the manifest row is **scope-dependent** and an
earlier draft of this doc got it wrong: the manifest is optional for marketplace
and `--plugin-dir` plugins, but for skills-directory plugins it is precisely what
promotes a folder from skill to plugin (§1.6). Two different rules.

#### GitHub Copilot (`formats/copilot.ts`)

Materially closer to spec. Correct today: hooks at both `hooks.json` and
`hooks/hooks.json` (`copilot.ts:183-185`); MCP at both `.mcp.json` and
`.github/mcp.json` (`shared.ts:111`, `copilot.ts:186`); `<name>.agent.md` with a
YAML-list `tools`; marketplace index locations; and components emitted at the
plugin root rather than under `.github/` — that prefix is for *workspace*
artifacts, and the adapter gets the distinction right.

| Copilot CLI convention | hoocode today |
|---|---|
| Probe order `.plugin/` → `plugin.json` → `.github/plugin/` → `.claude-plugin/` | reordered: `.github/plugin/` first, `.plugin/` third (`copilot.ts:73-78`) |
| `extensions` field — `{ paths, exclusive }`, can suppress built-ins | not modeled |
| `lspServers` field; `lsp.json` / `.github/lsp.json` | not parsed |
| `${PLUGIN_ROOT}` | only `CLAUDE_`/`AGENTS_PLUGIN_ROOT` (`plugins/index.ts:28`, `hooks-bridge.ts:36`) |
| `${COPILOT_PLUGIN_DATA}` / `${CLAUDE_PLUGIN_DATA}` | not provided **on either platform** |
| MCP servers resolve *last-wins*; agents/skills *first-found-wins* | first-wins throughout |

Two notes. `.github/mcp.json` is labelled a legacy fallback in our source
(`copilot.ts:91`) but the reference lists it as a current location — a comment
fix, not a functional one. And `themes` is read for Copilot (`copilot.ts:179`)
though it is not in the Copilot schema: a harmless dead read.

The `*_PLUGIN_DATA` row is the one that spans both vendors. A plugin using it
today gets an unexpanded literal under hoocode regardless of platform.

"If the standard changes" is not hypothetical — it already changed in at least
fifteen places across the two adapters and nothing noticed. The defect is the
missing *signal*, not a missing doc-fetcher.

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

1. **`PackagePlugin`** (autonomous) — generate `README.md` and the
   `marketplace.json` entry stanza *in the plugin's production home* (§5.3). No
   third location and no copy: the artifact is already in its final layout by the
   time it gets here. Touches no remote.
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

For `--platform github` there is **no** equivalent validator — confirmed absent
from the reference (§1.7), not merely unknown — so G1 there is round-trip plus
schema.

#### G1 forces a draft-then-promote write

Today `writePluginDraft` writes straight to the destination and
`ProposePlugin` activates it (`propose-plugin.ts:323-326`). Under D3 that
destination is `~/.claude/skills/<id>/` — a **live vendor directory**. Writing
there and validating afterwards means a failed G1 leaves a broken plugin loading
in Claude Code, and a declined confirm gate leaves one that was never approved.

So authoring becomes draft → validate → promote:

```
1. emit into an ephemeral draft dir (temp; never a configured location)
2. G1 (+ G2, + G3 for executables) run against the draft
3. executable content: confirm gate, showing the eval result
4. only then: atomic move into the resolved production home
5. activate
```

"Draft dir" is deliberately a third term: it is neither the **consumption home**
nor a **production home** (§5.3), it is ephemeral and is deleted on any failure.

This changes `writePluginDraft`'s contract rather than adding a call, so the
draft-and-promote mechanism ships with the location work in §8.5 step 2 — ahead
of the gates that need it. Until the gates land, step 2 promotes unconditionally;
the point is that by the time G1 exists there is already somewhere safe to run
it.

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
| Authored, passive only | G1, G2 | Discard the draft — nothing reaches a production home. Return the failure to the model: this is the loop that makes autonomous authoring converge instead of emitting silent garbage. |
| Authored, executable | G1–G3 against the draft, **before** the confirm prompt | Never reach the human with a draft that does not run; a decline discards it. |
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
| **Production, `--platform claude`** | `~/.claude/skills/<id>/` | The documented drop-in (§1.6). Live in Claude Code next session, and live in hoocode once §5.7 lands. |
| **Production, `--platform github`** | `~/.agents/publish/github/<id>/` | No vendor drop-in exists — `copilot plugin install ./dir` copies into a cache and needs a reinstall per change (§1.7). This is a real home, not a scratch area: `PackagePlugin` works in it, publish reads from it, and `copilot plugin install <that path>` is the local test loop. |

A `github` artifact writes its manifest to **root `plugin.json`** — probe
position #2 and the layout GitHub's own plugin-creating guide teaches. Today we
emit `.github/plugin/plugin.json` (`copilot.ts:80`) while the adjacent comment
claims root "is the canonical location (Copilot CLI spec)"; the code and its
comment disagree and the comment is right.

**Id collisions are checked per target location, not globally.** The same id may
exist as a `claude` artifact and a `github` artifact — they are separate
ecosystem artifacts, not duplicates. `pluginExists` therefore takes the resolved
target rather than scanning everywhere, and `ListPlugins` must show the location
so two same-named entries are distinguishable.

### 5.4 The bug this replaced

**Fixed in step 2.** `installedPluginsDir(cwd)` returned `<cwd>/.agents/plugins`
and `writePluginDraft` inherited it, so **every install and every autonomous
authoring wrote into the user's repo working tree.** Three consequences:

1. The agent dirties `git status` with content unrelated to the change it was
   asked to make.
2. A capability gained in repo A is invisible in repo B, so the model re-authors
   it. The reuse flywheel the subsystem exists to spin never spins across
   projects — which is most of its value.
3. `marketplaceCacheDir` puts full git clones at
   `<cwd>/.agents/marketplace-cache/` (`install.ts:79`) — clones inside the
   user's clone.

The read side was already right: `defaultPluginDirs` searched project *and*
global scopes, so only the write side needed changing. `marketplace-cache/` and
`marketplaces.json` moved to the agent dir unconditionally — a cache is never
repo content — and `<cwd>/.agents/plugins` is still *read*, so plugins installed
by older versions keep loading (§8.6 item 4).

One thing the plan did not anticipate: `mergePluginDraft` and `removeFromPlugin`
share the writer with creation, so promoting on every write would have silently
relocated a legacy plugin into a production home the first time it was edited —
the same hazard `existingLayout` guards in step 1. The writer now separates
*creation* (draft → promote) from an *in-place edit* (write over the plugin
wherever it already lives).

### 5.5 Project scope, and the authoring decision rule

Make scope selection a *decision rule* in `ProposePlugin`'s guidelines, not a
style note: **if the capability references this repo's paths, build system, or
conventions, it is a project skill, not a plugin.** The existing "author for
portability" guideline (`propose-plugin.ts:298`) gestures at this; it should be
the criterion.

**Project-scope *production* is out of scope, and that needs saying** — an
earlier draft kept a vague `scope: "project"` opt-in with no destination, which
does not survive contact with the rest of the plan. Two reasons:

- §5.3 defines production targets per platform, and neither is repo-local. The
  only coherent project destination would be `<cwd>/.claude/skills/<id>/`.
- §5.9 loads project-scope plugins **passive-only**. Authoring a project plugin
  carrying a hook or an MCP server would therefore produce an artifact that
  cannot fully run in the tool that wrote it.

So: authored plugins are user-scoped, full stop. A team that wants a plugin
pinned in the repo commits it by hand — a deliberate human act, consistent with
§3.3. The repo-local case the rule above points at is a **project skill**, which
already has a working home (`./.agents/skills/`) and no such contradiction.

### 5.6 The tension, stated

Writing to `~/.claude/skills/` means an autonomous action mutates another tool's
configuration directory. That is a genuine widening and should not be glossed.
Mitigations: it is announced, reversible via `UninstallPlugin`, lands in a
documented user-plugin path rather than arbitrary `$HOME`, and Claude Code's own
`plugin init` writes exactly there. Against it, today's behavior silently edits
the one directory the user is definitely about to `git add`. Both are widenings;
the current one is worse and less visible.

### 5.7 Making `~/.claude/skills/<id>/` actually work

Writing there is not sufficient on its own. `defaultPluginDirs`
(`loader.ts:662-670`) does not include `~/.claude/skills`, so hoocode would never
parse the artifact as a plugin — while `loadSkills` *does* scan that path
(`skills.ts:485`) and recurses without bound (`skills.ts:277`). The result is an
artifact that is fully live in Claude Code and, in hoocode, loads its skills as
loose top-level entries while **hooks, MCP servers, subagents and commands
silently do not load at all**. Two changes fix it:

1. **Add `~/.claude/skills` and `<cwd>/.claude/skills` to `defaultPluginDirs`.**
   Manifest-less directories still return null from `parsePluginDir` and stay
   plain skills, which is Claude's own rule (§1.6). This implements
   skills-directory plugins. Project scope is discovered here but loads
   passive-only (§5.9), and — matching the vendor — does **not** walk up to the
   repository root the way plain skills do.
2. **Stop the plain-skill scan descending into a plugin root.** Otherwise a
   plugin's inner `skills/bar/SKILL.md` is picked up as a loose skill `bar`,
   contradicting row 3 of §1.6. Without this, change 1 makes things *worse*: the
   skill loads on both paths, `realPath` dedup lets the plain-scan copy win, and
   it is never attributed to its plugin.

### 5.8 Namespacing plugin skills

Once §5.7 lands, plugin skills arrive only through the plugin path, so they can
carry their origin. Adopt Claude's `plugin-name:skill-name`.

Today plugin skills land flat in `skillMap` keyed by bare name
(`plugins/index.ts:104-111` → `skills.ts` `addSkills`), first-wins with a
collision diagnostic. Two plugins shipping a `review` skill silently lose one.
Namespacing removes the entire collision class and matches the vendor.

This is a breaking change to how plugin skills are referenced, which
`AGENTS.md` permits ("do not preserve backward compatibility unless the user
explicitly asks"). No bare-name alias: it would double the catalog entries in the
prompt surface, which the token budget rules care about.

### 5.9 Project scope: read both, gate executables

Consumption reads every format, so `<cwd>/.claude/skills/` is discovered as a
plugin source too — but only its **passive** capabilities (skills, commands,
subagents) load. Hooks and MCP servers from project scope are skipped and
reported.

The reason is a capability gap, not a preference: **hoocode has no workspace
trust mechanism.** Every `trust` reference in the source is TLS CA trust
(`utils/tls-ca.ts`), marketplace source trust (a curated list, not a per-folder
gate), or prose. There is no trusted-directories setting in `settings-types.ts`,
and `ui.confirm` is called in exactly one place in the entire codebase
(`propose-plugin.ts:261`). Claude gates project-scope skills-dir plugins behind a
workspace trust dialog, restricts their MCP servers to per-server approval, and
refuses to load their monitors at all (§1.6). Loading project-scope hooks and MCP
ungated would mean a cloned repo can register shell hooks and spawn servers with
no confirmation — a real escalation over reading skill text, which is all
`.claude/skills/` gets today.

Upgrade path when someone wants it: `ctx.ui.confirm` is already the primitive,
and `propose-plugin.ts:252-260` sets the headless precedent — no UI means refuse,
not proceed.

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
Producers: the MCP loader, the skills loader, the slash-command registry, the
agent registry, the marketplace lister, and `discoverPlugins`. One index, one
retrieval tool.

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

Both vendor columns are now first-hand: Claude from
`code.claude.com/docs/en/{plugins,plugins-reference}`, Copilot from the
`github/docs` source repo (§1.7). Fetched 2026-08-09.

| | Claude Code | Copilot CLI | hoocode (proposed) |
|---|---|---|---|
| Formats read | Claude | 4 locations incl. `.claude-plugin/` | **all three** |
| Format written | Claude | Copilot | **target platform only, never `agents`** |
| Manifest home | `.claude-plugin/plugin.json` | root `plugin.json` | per target platform |
| Who installs | human | human | **model**, within trusted marketplaces |
| Who authors | human (`plugin init`) | human | **model** (`ProposePlugin`) |
| Validation | `claude plugin validate` | none | G1–G4, delegating to `claude plugin validate` where it exists |
| Plugin scope | user | user (project scoping is an open upstream request) | user (consumption + production split, §5.3) |
| Local drop-in | `~/.claude/skills/<id>/`, discovered in place | none — install copies to cache, reinstall per change | follows the platform |
| Precedence | n/a | skills/agents first-wins, MCP last-wins | first-wins throughout |
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

**Claude:** `workflows/`, `output-styles/`, `.lsp.json`, `monitors/monitors.json`,
`bin/` on `PATH`, plugin-root `settings.json`, manifest-less directories,
single-`SKILL.md` plugins, skill namespacing, a `--plugin-dir` equivalent.

**Copilot:** documented probe order, `extensions`, `lspServers` + `lsp.json`,
`${PLUGIN_ROOT}`, MCP last-wins precedence.

**Both:** `${COPILOT_PLUGIN_DATA}` / `${CLAUDE_PLUGIN_DATA}`.

These are now **production** blockers, not read-side gaps: we emit into these
ecosystems, so an unmodeled surface is an unemittable one.

---

## 8. Agreed plan

### 8.1 Decisions

Settled in review. Each row is a fork that was actually open.

| # | Decision | Chosen | Rationale |
|---|---|---|---|
| D1 | Production format | Platform-specific; `agents` rejected for plugins | §1.2 — a distribution unit must belong to an ecosystem |
| D2 | Default when `--platform` is absent | `claude` | Keeps the autonomous gap-fill unbroken; the only platform where the local loop closes (§1.5) |
| D3 | Authored-plugin location | Per platform (§5.3): Claude's drop-in; a hoocode-owned production home for GitHub, which has no drop-in | Producing in a vendor format and hiding it from that vendor defeats the purpose |
| D4 | Flag rename scope | Session flag only | `supportPlatform` names two things; the manifest/marketplace data field is vendor on-disk data we do not own (§8.2) |
| D5 | Id collision | Per target location | A `claude` `foo` and a `github` `foo` are separate ecosystem artifacts, not duplicates |
| D6 | Plugin skill namespacing | `plugin-name:skill-name` | Claude parity; removes the collision class outright (§5.8) |
| D7 | Project-scope skills-dir plugins | Read, but load passive capabilities only | We have no trust gate to honor the vendor's restrictions with (§5.9) |
| D8 | `github` manifest home | Root `plugin.json` | Probe #2 and what the vendor's plugin-creating guide teaches; our code emits `.github/plugin/` while its own comment says root is canonical |
| D9 | Plugin runtime variables + data dir | In scope, both platforms | `*_PLUGIN_DATA` is documented by both vendors and provided by neither of our adapters; plugins using it get an unexpanded literal today |
| D10 | Project-scope plugin *production* | Dropped — authored plugins are user-scoped, full stop | Resolved during final review, not in discussion: the earlier `scope: "project"` opt-in had no destination under §5.3 and would have produced artifacts that §5.9 refuses to fully load. Overridable, but it needs a coherent answer to both before it comes back (§5.5) |
| D11 | When authoring validates | Draft dir → validate → atomic promote | D3 makes the destination a live vendor directory; writing first and validating after leaves broken or unapproved plugins loading in Claude Code (§4.2) |

### 8.2 Blast radius

Measured, not estimated.

**`supportPlatform` is two things sharing a name.** D4 renames only the first:

| Renamed — session write-target flag | Untouched — on-disk vendor data |
|---|---|
| `main.ts:433-452`, `cli/args.ts`, `settings-manager.ts`, `settings-types.ts` | `NormalizedPlugin.supportPlatform` (`manifest.ts`) |
| `platform-targets.ts`, `scaffold.ts:121,182,253` | `NormalizedMarketplace.supportPlatform` (`plugins/marketplace.ts`, 16 refs) |
| `propose-plugin.ts:132-137` | `marketplace.json` `supportPlatform` field, `formats/types.ts` |

A blind rename across all 25 files corrupts the parsed data model.
`support-platform.test.ts` (29 refs) covers both halves and has to be split the
same way.

**Location coupling.** `installedPluginsDir(cwd)` is the single assumed home for
five roles in `authoring.ts` — `isAuthoredPlugin`, `writePluginDraft`,
`pluginExists`, `getPlugin`, `removeFromPlugin`. All five must resolve a target
rather than a constant. `UpdatePlugin` and `RemovePluginCapability` need to know
*which* location holds the plugin.

**Two pre-existing defects sitting in the blast radius**, both worth fixing
first because the location change touches them anyway:

- ~~`extensions/core/marketplace.ts` defined its own `storePath`, `cacheDir` and
  `sanitizeForDir`, while its docstring claimed the shared mechanics live in
  `install.ts` "so this command and the model-facing lifecycle tools never
  drift".~~ **Fixed in step 0**: `marketplaceStorePath` and a new
  `marketplaceCacheRoot` are exported from `install.ts` and the three local
  copies are gone, so the docstring is now true.
- ~~`activatePlugin` hardcoded `scope: "project"` for every plugin including
  global ones, mislabelling provenance in the config selector
  (`components/config-selector.ts:77` renders it verbatim).~~ **Fixed in step 0**: scope is
  derived from whether the plugin root sits under the workspace.

**Tests that move:** `plugin-authoring.test.ts` (11 hardcoded
`.agents/plugins` paths), `plugin-lifecycle.test.ts`, `plugin-e2e-official.test.ts`,
`plugins.test.ts`, `marketplace.test.ts`, `support-platform.test.ts`, and
`plugin-e2e-copilot.test.ts` — the last breaks on D8 alone, since the emitted
manifest path changes.

### 8.3 Vocabulary

Three locations, deliberately named apart — an earlier draft called all three
"staging" and that is exactly the kind of collision that produces the wrong
directory in code.

| Term | Path | Lifetime |
|---|---|---|
| **Draft dir** | temp | Ephemeral. Holds an authored plugin while the gates run; deleted on any failure (§4.2) |
| **Consumption home** | `~/.agents/plugins/<id>/` | Persistent. Marketplace installs, format-agnostic |
| **Production home** | `~/.claude/skills/<id>/` or `~/.agents/publish/github/<id>/` | Persistent. Where an authored plugin lives, per platform (§5.3) |

### 8.4 What survived every review round

`§6` (retrieval) is untouched — format and location are orthogonal to it, and it
can be built in parallel from day one. The `§5.1` scope rule also held: plugins
were already user-scoped, only the destination moved. Everything else in §5 was
revised at least once.

### 8.5 Order

Steps 1–3 are one coherent change; splitting them leaves the tree in a state
where authored plugins are half-live.

| Step | Work | Why here |
|---|---|---|
| 0 | ~~Delete the `/plugin` duplicates; fix `activatePlugin` scope metadata~~ **done** | Pre-existing defects directly under the later steps |
| 1 | ~~Split the resolver; `--platform` (D4 scope); reject `agents` for plugins~~ **done** | Everything reads from this |
| 2 | ~~Locations (§5.3): consumption home, per-platform production, per-target `pluginExists`, migration read-path, draft-then-promote (§4.2)~~ **done** — new `plugins/locations.ts` owns all three roles | Same decision as step 1. The mechanism shipped here, not with the gates, so that by the time G1 exists there is already somewhere safe to run it |
| 3 | `~/.claude/skills` + `<cwd>/.claude/skills` discovery, stop the plain scan at plugin roots (§5.7); namespace plugin skills (§5.8); project scope passive-only (§5.9) | Without this the D3 target is half-live |
| 4 | Adapter catch-up, both vendors (§2.1): Claude surfaces; Copilot probe order, `extensions`, `lspServers`, root-manifest emit (D8) | Now a production blocker: we emit into these ecosystems |
| 4b | Runtime variables + per-plugin data dir (D9), both platforms | Small, self-contained, and unblocks any plugin that uses them |
| 5 | G1 + G2, delegating to `claude plugin validate` (no Copilot equivalent exists) | Cheapest real green signal; step 4 makes it pass for the right reasons, step 2 gave it somewhere safe to run |
| 6 | Tier 1 lenient reader + `unsupportedSurfaces` | Turns the *next* drift into a signal |
| 7 | G3 sandboxed smoke | Makes the executable confirm gate meaningful |
| 8 | Marketplace index TTL + explicit refresh (§3.1) | Small; the inherit story goes stale silently without it |
| 9 | `PackagePlugin` + publish lane, GitHub flow primary | Needs eval green-signal-capable; the only GitHub route |
| 10 | Capability index + MCP retrieval (§6) | Independent — can run in parallel from step 1 |
| 11 | G4 trigger eval; Tier 2 drift CI | Ongoing quality, not blocking |

### 8.6 Still open

1. **MCP last-wins precedence for Copilot plugins** (§1.7). Copilot resolves
   agents/skills first-found-wins but MCP servers last-wins, so a plugin's MCP
   server overrides user config. We are first-wins throughout. Adopting the
   asymmetry is a consumption-side behavior change with real blast radius
   (`registerExtensionMcpServers`), so it is deliberately *not* in the plan
   above — decide it separately.
2. **Does `--platform` accept multiple values?** `emitForPlatforms` already
   handles it and dedupes by path (`formats/index.ts:69-80`), and
   `github/copilot-plugins` ships both index formats, so dual-emission is a real
   shape. Recommend allowing it, never defaulting to it.
3. **`~/.claude/skills/<id>/` collision** with a plugin Claude Code installed
   itself, and with a user's hand-written skill of the same name. Needs a
   pre-write check; `UninstallPlugin` must never delete a directory it did not
   author (the `.authored.json` marker already distinguishes them).
4. **Migration policy for `<cwd>/.agents/plugins/`.** `defaultPluginDirs` already
   reads it, so a read-only grace period costs nothing — but the plan does not
   say whether existing plugins are ever *moved*, or left in place indefinitely
   while new writes go elsewhere. Leaving them means `git status` stays dirty for
   anyone who already hit the §5.4 bug. Recommend: keep reading indefinitely,
   offer a one-shot `/plugin migrate`, never move silently.
