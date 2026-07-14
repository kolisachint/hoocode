# Plugin System: discovery, lifecycle, lazy loading, and capability authoring

**Status:** implemented (see `docs/loop-and-plugin-system.md` §2.7 for what
shipped: lifecycle tools, authoring, deferred MCP schemas — default on — plus
single-turn live activation and the curated well-known marketplace list)
**Scope:** `packages/coding-agent` plugin + agent subsystem

## Goal

Let the model discover, install, manage, *and author* plugins so heavy/rare
capabilities (skills, subagents, MCP servers, commands, hooks bundled in a
plugin) aren't injected into context eagerly, and so the agent can fill
capability gaps itself. **Install is the "load act."**

---

## Trust model (non-negotiable)

- **Adding a marketplace is the human trust boundary.** Marketplaces are
  git-hosted plugin indices; plugins bundle `hooks` + `mcpServers` (code
  execution). Registering a new marketplace stays a human/config action —
  never an autonomous model tool.
- **Within a trusted marketplace, install is the model's discretion**
  (package-manager model). Installs must be **transparent and reversible,
  never silent** — the model announces "installing X to do Y," and the plugin
  is uninstallable.
- **Injection carve-out:** if the impetus to install traces to untrusted
  external content (PR comment, fetched web text, injected task), require a
  human check before installing.
- **Two distinct trust axes — keep them straight:**
  - *Install* = trust the **source** (marketplace boundary vouches for remote code).
  - *Author* = trust the **content / capability grant** (model wrote it
    in-session, so it's visible; gate on whether the human has seen the
    executable code or the tool grant before activation).

---

## Section 1 — Model-facing lifecycle tools

| Tool | Notes |
|---|---|
| `SearchPlugins` | Query indices of all registered marketplaces. Read-only. |
| `SuggestPluginInstall` | Proactively surface "there's a plugin for this." Suggests only. |
| `InstallPlugin` | Runs hooks + MCP; gated by transparency + reversibility + injection carve-out. Source trust covered by marketplace. |
| `UninstallPlugin` | Low risk (removing capabilities can't execute attacker code). The "reversible" half of install; enables self-cleanup. |
| `ListPlugins` | Read-only; see what's installed before installing a dup. |

**NOT model tools:**

- `AddMarketplace` — human/config only (trust boundary).
- `UpdatePlugin` **(marketplace)** — supply-chain sensitive (benign v1 → hostile
  v2); model has no good judgment about accepting a new *remote* version.
  CLI/config only; at most a `SuggestPluginUpdate` nudge. No auto-update from a
  marketplace.
  - **Carve-out (implemented):** a model-facing `UpdatePlugin` *does* exist for
    **locally authored** plugins (§3). It merges **inline caller-supplied**
    content — it never fetches a remote — so the "benign v1 → hostile v2" vector
    is structurally absent. Executable additions still pass the §3 confirm gate.
    Authored plugins are identified by a `.authored.json` provenance marker
    stamped at scaffold time; marketplace installs land in the same
    `.agents/plugins/` directory but lack the marker and are refused (they also
    don't round-trip losslessly through the authoring emitters).
- `SearchMarketplace` — redundant with `SearchPlugins`, or a trust-crossing
  ramp to registering new sources. Drop.

---

## Section 2 — Lazy loading

Install already gates the plugin tier (don't load what isn't installed;
`SearchPlugins` finds the rest). At the *installed* tier, lazy-load
**capabilities, not plugins**. Each capability has its own natural laziness —
respect it, don't build a generic registry:

- **Skills** — already two-tier lazy (`formatSkillsForPrompt` injects name +
  description; body read on demand). Leave as-is. Do NOT add `SearchSkills`.
- **Commands** — same shape (name + description surface, body on use), cheap.
  Leave.
- **Subagents (agents)** — **already two-tier lazy, same pattern as skills**:
  the *catalog* (name + description + `tools:` list) is injected eagerly; the
  **system prompt / body loads on dispatch**. Preserve this — never eagerly
  inject agent bodies. Note the eager *catalog* cost per agent runs **higher
  than a skill's**, because agent descriptions are typically multi-sentence.
  So agents hit the scale threshold *sooner* than skills — but the answer is
  the same: keep the catalog eager (cheap enough + deterministic; the model
  can't dispatch an agent it can't see) until agent count makes the catalog
  itself the bottleneck. Do NOT add a `SearchAgents` tool before that point —
  deferring the catalog reintroduces the determinism problem for no real
  saving.
  - **Dispatch ↔ MCP-schema interaction:** when a subagent's allowlist
    includes MCP tools, those deferred schemas (below) must resolve *at
    dispatch time* so the agent can actually use them. Wire agent dispatch to
    trigger schema resolution for the tools in its allowlist.
- **Hooks** — can't be lazy (an event listener must be registered to fire) and
  near-zero context cost. Leave.
- **MCP tool schemas** — the ONLY genuinely heavy thing, and the one in-house
  lazy mechanism to build. When an installed plugin's MCP server activates,
  every tool's full JSON schema lands in context. **Defer these:** inject tool
  *names* only, mark them deferred, fetch the schema on demand — the same
  deferred-tool / `ToolSearch` pattern the harness already uses. Lives at the
  MCP-tool-schema boundary, not the plugin boundary.

Do NOT build a bespoke per-capability lazy-loading registry — skills /
commands / agents already carry the right shape; only MCP schemas need new
code.

---

## Section 3 — Capability authoring (`ProposePlugin`)

Completes the spectrum: discover (`SearchPlugins`) → acquire (`InstallPlugin`)
→ **author (`ProposePlugin`)**, for when no marketplace plugin fits a gap.
Gate on the *content / capability-grant* trust axis, **not** the *source* axis
used for install.

> **Revised (implemented): one tool, computed gate.** The original spec said
> "escalating-risk paths — build them separately, do not collapse into one
> tool." That was interpreted too literally as *two tools*. Two tools force the
> model to *pre-declare* risk by tool choice and make a mixed plugin (skill +
> hook) impossible to author in one call. The current implementation keeps the
> **escalating risk gate** but in a **single `ProposePlugin`**: risk is computed
> from the draft's *content* (via `classifyAllowlist` + presence of
> hooks/MCP servers), so a hook can never be mis-routed through a "passive"
> path, and passive-plus-executable plugins author in one call — the executable
> portion alone triggers the confirm gate. `UpdatePlugin` (local, merge-only)
> reuses the same gate.

| Authored capability | Risk shape | Treatment |
|---|---|---|
| **Skill** (SKILL.md, pure instructions) | passive text, loaded lazily | Autonomous + transparent; reversible. Near-zero risk. |
| **Command** (prompt template) | passive text | Light touch, same as skill. |
| **Subagent** (agent definition) | passive definition, but grants an **autonomous actor** scoped by its `tools:` allowlist | Gate on the **allowlist**, not the file (below). |
| **Hook** | active code, runs on tool events | **Propose → show the code → human confirms → activate.** Bar ≥ install. |
| **MCP server** | active code | Same draft → display → confirm flow. |

**Subagent authoring — allowlist-driven gate:** the definition file doesn't
execute, but invoking it spawns an autonomous actor with whatever `tools:`
grants. Classify the requested allowlist and set the bar automatically:

- Read-only grant (Read / Grep / Glob / WebFetch) → autonomous + transparent,
  as safe as a skill.
- Mutating / exec / network grant (Bash, Write, Edit, MCP) or `tools: *` →
  **show the definition, especially the `tools:` line, and require human
  confirmation** (bar = hook/MCP path).
- **Implementation lever:** reuse the existing `allowed-tools`
  parser/normalizer (`normalizeTools`, `skills.ts:~331–336`) on the authored
  allowlist to classify read-only vs. mutating and pick the gate from the
  frontmatter — compute the risk, don't guess it.

**Privilege-amplification guardrail:** never include capability-acquisition
tools (`InstallPlugin`, `ProposePlugin`, `UninstallPlugin`, marketplace tools)
in an authored subagent's default allowlist. Keep plugin-system tools on the
**top-level agent only** — otherwise a low-trust authored agent can bootstrap
privilege in an author → spawn → install → author loop.

**Two build paths:**

1. **Scaffold-a-skill/command/subagent** — thinner. The model can already
   `Write` these into `.claude/` dirs (skills via `formatSkillsForPrompt`,
   agents via `agentsDir`). Value added is *scaffolding*: correct manifest +
   directory layout so the result is a proper, publishable plugin. Skill /
   command autonomous; subagent follows the allowlist gate.
2. **Propose-executable-plugin (hook/MCP, or high-privilege subagent)** — the
   risk-bearing path. Draft the code/definition, **display it** (and the tool
   grant), require explicit human confirmation, *then* activate.

**Flywheel + boundary:** a proposed local plugin that proves useful is raw
material for publishing to a marketplace — but publishing is outward-facing
distribution and stays a **human act**, never autonomous.

---

## Relevant existing code (read first)

- `packages/coding-agent/src/core/extensions/plugins/manifest.ts` —
  `NormalizedPlugin` (bundling unit: `skillsDir`, `commandsDir`, `agentsDir`,
  `hooks`, `mcpServers`, `providers`) and `parsePluginDir`. Authoring must emit
  a manifest this parser round-trips.
- `packages/coding-agent/src/core/extensions/plugins/marketplace.ts` —
  `parseMarketplaceDir`, `resolvePluginSource`, `readMarketplaceStore` /
  `writeMarketplaceStore`. `SearchPlugins` reads here; install resolves /
  fetches sources here.
- `packages/coding-agent/src/core/skills.ts` — `formatSkillsForPrompt`
  (two-tier lazy pattern to preserve; target for authored skills) and
  `normalizeTools` / `allowed-tools` parsing (~331–336; reuse for the subagent
  allowlist gate).
- Wherever agents are loaded and their catalog is injected + dispatched (search
  for the agent/subagent registry) — the agent lazy-load treatment and the
  dispatch ↔ MCP-schema wiring live here.
- The agent's tool registry (search where existing tools are registered) —
  lifecycle tools, deferred-MCP-schema path, and `ProposePlugin` all hook in
  here.

---

## Scope + process

- Ship one curated **default marketplace** so source-level trust is meaningful
  out of the box, plus the existing human-add path. Treat "marketplaces stay
  curated/narrow" as a maintained invariant.
- Add tests alongside `packages/coding-agent/test/plugins.test.ts`:
  - authored skill/subagent round-trips through `parsePluginDir`;
  - executable authoring (hook/MCP) blocked until confirmation;
  - subagent with a mutating allowlist blocked until confirmation while a
    read-only one is not;
  - authored subagents cannot carry plugin-system tools;
  - agent bodies are not injected until dispatch.
- **Before coding, confirm:**
  1. how the tool registry injects schemas and how deferred/on-demand schema
     loading works in this harness;
  2. how agents are cataloged + dispatched today, and whether bodies are
     already deferred;
  3. whether an install/fetch path for plugin sources exists (wrap) or needs
     building (in scope);
  4. where the human-confirmation gate for executable authoring,
     subagent-allowlist authoring, and injection-triggered install lives.

  Then propose tool input/output schemas and the three `ProposePlugin` paths'
  signatures for review before implementing.
