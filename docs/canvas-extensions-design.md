# Design Note: Canvas extensions — hosting GitHub Copilot canvases

**Status:** Proposed. Nothing in this document is implemented. No canvas code
exists in the repo today, and `.github/extensions/` is not read anywhere
(`grep -rn "\.github/extensions" .` is clean). The facts about the Copilot
side were verified against `@github/copilot-sdk@1.0.11` (npm) and the
`github/awesome-copilot` reference extension `pr-artifact-explorer`, both read
directly rather than from documentation. The module-resolution and lifecycle
contract (§4.1, §6.1) is quoted from `docs/extensions.md` inside the SDK package
and cross-checked against all 23 extensions in the catalog.

**Motivation:** hoocode is a TUI. Every rich review surface it could offer — a
plan you reorder, a diff you approve hunk by hunk, a queue of pending
permission decisions with their context — currently squeezes through a terminal
prompt or does not exist. A *canvas* is a shared interactive surface that an
agent and a person both drive: the agent mutates state and navigates the view,
the person reads and edits on the same surface. GitHub shipped canvases for the
Copilot app in 2026, and a community catalog already exists.

Two payoffs, in order of value:

1. **Consume the catalog.** A Copilot canvas extension is a directory of plain
   `.mjs` files with **no dependencies** except one host-provided import. If
   hoocode satisfies that import, community canvases run unmodified. We inherit
   other people's work for the cost of one compatibility layer.
2. **A GUI surface without an IDE.** Once the runner exists, hoocode can author
   its own canvases, and they run in the Copilot app too.

The strategic point is not the canvas. hoocode already claims cross-vendor
plugin compatibility (`docs/plugin-format-mapping.md`). A surface that runs
unchanged in both hosts is that claim made concrete.

---

## 1. What a canvas actually is

Verified by reading `github/awesome-copilot/extensions/pr-artifact-explorer`
(~10 `.mjs` modules, ~90KB). Its anatomy is the reference for everything below.

```
extension.mjs      215 lines  canvas declaration; pure delegation, no logic
server.mjs          30 KB     loopback HTTP server, SSE, routing, auth gate
github.mjs          22 KB     GitHub API client — server-side only
cache.mjs           16 KB     artifact download + ZIP index
render.mjs                    HTML as a template string
accounts.mjs preview.mjs state.mjs zip.mjs memory-cache.mjs security.mjs
assets/            app.js app.css primer-*.css asciinema-player.min.js octicons/
```

**No `package.json`. No `tsconfig.json`. No build step.** The only external
import in the entire extension is `@github/copilot-sdk/extension`. Primer CSS
and the asciinema player are vendored as files. The HTML is a template string,
not a static `index.html` — the capability token is injected by replacing a
marker in a `<meta>` tag, specifically so the page needs no inline `<script>`
and can run under a strict CSP.

That zero-dependency property is not incidental. It is what makes a
compatibility shim sufficient, and it is the single most important fact in this
document.

### 1.1 The transport

```
open()  → startInstance(instanceId)
          ├─ createServer().listen(0, "127.0.0.1")   ephemeral port, loopback
          ├─ token = randomBytes(32).toString("base64url")   per instance
          └─ return { url, title, status }
                     url = http://127.0.0.1:<port>/?token=<token>

agent → UI    SSE on /events; broadcast(entry, "navigate"|"cache"|"refresh", …)
UI → server   fetch POST /api/…, token in header
UI ← server   JSON
```

There is no `postMessage` bridge and no host-provided message protocol. The
host does exactly one thing: it loads the URL that `open()` returns, in a panel.

The load-bearing idea: **the agent never renders.** An action mutates state and
then calls `navigateInstance(instanceId, "#/artifact/owner-name/0/12345")`,
pushing the person's view to the right route over SSE. Human navigation and
agent navigation use the identical mechanism. That is what makes the surface
genuinely shared without any co-editing machinery.

### 1.2 What the reference canvas is not

Worth naming, because it should temper our own ambitions: `pr-artifact-explorer`
is a rich **read** surface that the agent navigates. Its shared state is a cache
and a route. Neither party co-edits content. The flagship community canvas
sidesteps concurrent editing entirely — which is evidence about what is easy and
what is not.

---

## 2. The contract is a wire protocol, not the Node API

This section is the compatibility policy. Everything else is implementation.

`@github/copilot-sdk@1.0.11` ships `dist/canvas.d.ts` — **126 lines**, fully
typed. Every declaration in it carries:

> `@experimental` Canvas types are part of an experimental wire-protocol surface
> and may change or be removed in future SDK or CLI releases.

So drift is not a risk to be assessed; it is stated policy. But the same file
tells us where the contract actually lives:

> The runtime sends provider callbacks as `canvas.open`, `canvas.close`, and
> `canvas.action.invoke` JSON-RPC requests … routed by `canvasId`.

> Node intentionally uses a per-canvas factory pattern (…) where other SDKs
> (Rust, Python, Go, .NET) expose a single `CanvasHandler` per session that
> switches on `canvasId`. **Both shapes target the same JSON-RPC wire protocol;
> the divergence is API ergonomics only.**

And `dist/sdkProtocolVersion.d.ts`:

```ts
export declare const SDK_PROTOCOL_VERSION = 3;
```

`createCanvas`/`joinSession` is one of five ergonomic wrappers over a
three-method protocol. GitHub treats the wire protocol as the contract and the
language API as taste.

**Therefore: bind to the wire protocol, not to the Node sugar.** The drift
surface is three JSON-RPC methods and one integer — and that integer is a
negotiated handshake version, so protocol drift fails loudly at connect time
rather than silently at runtime.

### 2.1 Three tiers of ownership

The confusion this policy resolves is that "stay portable" and "own it so we can
improve it" appear to conflict. They do not; they apply to different layers.

| Layer | Owner | Drift exposure |
|---|---|---|
| Third-party canvases (`pr-artifact-explorer`, catalog) | GitHub and authors. **We consume, never fork.** | **Zero** — upstream updates arrive by `git pull` |
| Wire protocol: `canvas.open` / `canvas.close` / `canvas.action.invoke` + `SDK_PROTOCOL_VERSION` | GitHub defines, **we implement** | **Bounded and loud** — 3 methods, versioned handshake |
| Runner, shim, trust gate, TUI integration, our own canvases, our extras | **Us, entirely** | None |

Portability costs us three methods. Ownership above that line is unconstrained.
Tier 1 is pure upside precisely because we never touch it.

### 2.2 Two rules that keep the tiers from collapsing

- **Never patch a third-party canvas in place.** If `pr-artifact-explorer` needs
  a fix, upstream it or wrap it. Forking it means owning 90KB of someone else's
  GitHub API client forever.
- **hoocode-only capabilities go in a separate import** — `@hoocode/canvas`,
  feature-detected — never as additions to the `@github/copilot-sdk/extension`
  surface our shim provides. A canvas authored for hoocode then still opens in
  the Copilot app, minus the extra. Extend *beside* the contract, never *inside*
  it. This is the rule that stops "we can improve it" from becoming a fork.

### 2.3 Drift becomes a failing build

The SDK ships full `.d.ts`, so this is mechanical rather than a vigilance
problem:

1. **`@github/copilot-sdk` as a devDependency only, never runtime.** Its
   dependencies are `@github/copilot`, `zod`, `vscode-jsonrpc`, and **`koffi`**
   (native FFI — the package carries `dist/ffiRuntimeHost.d.ts`). Native FFI must
   not enter hoocode's runtime tree. We need the package for types and tests.
2. **Type-level conformance test.** Assert our shim satisfies their
   `CanvasOptions`, `CanvasAction`, `CanvasError`, and `createCanvas`. `bun run
   check` already runs `tsgo --noEmit`, so a shape change on version bump becomes
   a failing typecheck with a readable diff. Highest-leverage item in the plan.
3. **Assert `SDK_PROTOCOL_VERSION === 3`** in a test. When it becomes 4 we learn
   it from a test name, not a user report.
4. **Canary.** `pr-artifact-explorer` pinned at a commit, opened headless in CI,
   actions invoked. Catches behavioural drift the types cannot see.
5. **Renovate on that one package.** Bump → typecheck + canary → known in a day.

### 2.4 Residual risk, stated plainly

`@experimental` on every line means breakage is a question of when. Three things
make that acceptable:

- It is **loud** — handshake version plus typecheck, not silent misbehaviour.
- When GitHub breaks the protocol, **their own canvases break too** and get
  updated. We are a fast follower on a three-method surface: a day of work.
- The alternative — our own protocol — carries 100% of the surface forever *and*
  yields none of the catalog. More cost to avoid a smaller risk.

If GitHub removes canvases entirely we keep tiers 2 and 3 — runner, loopback
pattern, our own canvases — and lose the catalog. Survivable. It is also the
argument for keeping our own canvases inside the plain part of the contract.

---

## 3. Canvas is not a plugin capability

The instinct is to add `canvas` to the plugin capability tree next to skills and
commands. That is wrong, and the repo shows why.

`src/core/extensions/plugins/manifest.ts` normalizes `skills`, `commands`,
`agents`, `hooks`, `mcpServers`, plus `skillsDir`/`commandsDir`/`agentsDir`.
Every one of those is a **declarative file the loader reads and hands to the
runtime**. That is why `plugins/formats/copilot.ts` is 304 clean lines: it maps
vendor paths onto that tree and nothing more.

A canvas has no declarative form. It is:

- a **process** with a lifecycle (`joinSession`, `open`, `onClose`),
- a **JSON-RPC channel** where stdout is reserved (hence `session.log()`),
- **actions that must reach the agent's tool list at runtime**, not load time,
- a **UI surface** with an origin, a capability token, and a server to tear down.

So canvas is **MCP-shaped, not skill-shaped**: a spawned process whose tools
appear dynamically. The precedent to follow is
`src/extensions/core/mcp-loader.ts`, which spawns a child (`:121`) and speaks
JSON-RPC 2.0 over stdio. `plugins/formats/copilot.ts` gains detection only —
"this directory is a canvas, here is its entry file" — and no capability
mapping.

### 3.1 The in-process trap

`src/core/extensions/loader.ts:373` loads hoocode extensions **in-process**:

```ts
const module = await jiti.import(extensionPath, { default: true });
```

Copilot runs canvas extensions as **child processes over JSON-RPC**. If the shim
is built on the in-process path, we `jiti.import` arbitrary third-party code
straight into the agent runtime — full access to the tool registry, the
permission gate, and provider credentials, with no boundary. For a project whose
headline claim is that every edit and shell command passes a permission gate,
importing a stranger's canvas in-process is the one thing we cannot ship.

**The canvas runner spawns; it does not import.** This also matches Copilot's
model, so a canvas behaves identically in both hosts, and we inherit the reason
their stdio discipline exists instead of inventing our own.

---

## 4. Architecture

```
packages/coding-agent/src/core/canvas/
  runner.ts       spawn child process, JSON-RPC over stdio, lifecycle   ← the work
  protocol.ts     canvas.open / canvas.close / canvas.action.invoke; version check
  registry.ts     live instances, idle reaper, action→tool bridge
  discovery.ts    locate canvas directories, identify entry file
  sdk-shim/       child-side @github/copilot-sdk/extension surface
```

Four responsibilities, deliberately separated so that only `protocol.ts` and
`sdk-shim/` face GitHub:

- **`protocol.ts`** — the entire tier-2 surface. Three request types, one
  version handshake. If GitHub moves, this file and the shim move; nothing else
  does.
- **`sdk-shim/`** — implements `createCanvas`, `joinSession`, and `CanvasError`
  as a thin translation onto `protocol.ts`. Roughly what their SDK does onto
  theirs.
- **`runner.ts`** — forks the entry file, owns stdio framing, keeps stdout clean,
  maps `session.log` to hoocode's logging, and — the hard requirement from §4.1 —
  **injects the shim as a module resolver** so the child's bare
  `@github/copilot-sdk/extension` import resolves without anything being written
  into the extension directory.
- **`registry.ts`** — instance table keyed by `instanceId`, plus the two things
  the host must provide that a browser cannot: an idle reaper and SSE-liveness
  heartbeat (§6).

### 4.1 Module resolution is the load-bearing mechanism

Settled against GitHub's own documentation, shipped inside the SDK package as
`docs/extensions.md`:

> **Launch**: Each extension is forked as a child process with
> `@github/copilot-sdk` available via an automatic module resolver.

> The `@github/copilot-sdk` import is resolved automatically — you don't install
> it.

So a canvas extension never installs the SDK, and `package.json` plays no part
in resolution. Empirically confirmed across all 23 extensions in
`github/awesome-copilot`: **not one ships `node_modules`**, and the three that
omit `package.json` entirely work the same as the twenty that include it.

Consequences, all now requirements rather than options:

- **The shim is an injected resolver, not a file on disk.** hoocode must make
  `@github/copilot-sdk/extension` resolvable inside the forked child — a
  `node:module` `register()` resolve hook mapping that specifier onto
  `sdk-shim/`. We never write into the extension directory; a canvas pulled from
  the catalog stays byte-identical to upstream, which is what tier 1 of §2.1
  requires.
- **Discovery keys off `<dir>/extension.mjs`, nothing else.** The entry file is
  required, must be named `extension.mjs`, and must be an ES module. All 23
  catalog extensions comply; where `package.json` exists, its `main` is always
  `extension.mjs` anyway.
- **hoocode installs nothing.** If a child imports a bare specifier we do not
  provide, the import fails; surface an error naming the missing module and the
  extension's README. Running a package install on behalf of repository-supplied
  code would defeat §5.

### 4.2 The one catalog extension with a real dependency

`chromium-control-canvas` declares `playwright: ^1.60.0` and imports it bare.
Its README instructs the user to run `npm install` plus `npx playwright install
chromium` by hand. It is the **only** third-party dependency anywhere in the
catalog — every other extension imports nothing but `@github/copilot-sdk` and
`node:` builtins.

So "clone it and it works" holds for 22 of 23, and the exception fails the same
way in the Copilot app until a person installs its dependencies. Our behaviour
should match: fail with a clear message, never install.

Worth noting as evidence that `package.json` is decorative here: three further
extensions (`arcade-canvas`, `backrooms-canvas`, `flight-map-canvas`) tell users
to run `npm install` even though their only declared dependency is the
auto-resolved SDK. The catalog's own instructions are inconsistent with GitHub's
documented behaviour, which is another reason not to key any of our logic off
`package.json`.

### 4.3 Discovery paths

Discovery reads, in precedence order:

| Path | Origin | Notes |
|---|---|---|
| `.agents/extensions/` | hoocode native, `.agents/`-first policy | authored here by default |
| `.github/extensions/` | Copilot project scope | **arrives with a clone — gated (§5)** |
| `~/.copilot/extensions/` | Copilot user scope | the catalog's install target |

This follows the policy in `docs/plugin-format-mapping.md` §0: `.agents/` is
read first and written by default; vendor conventions are compatibility inputs.
Note that hoocode's *existing* extension locations are `./.hoocode/extensions`
and `~/.hoocode/extensions` (`docs/agent-spec-tree-map.md`, `loader.ts:420-500`)
— those stay in-process and are unrelated to canvases.

---

## 5. Trust

`src/core/extensions/plugins/trust.ts` already contains the exact argument, for
the exact reason:

> Its skills and commands are text the model reads, which is no worse than
> reading the repository itself, but its **hooks and MCP servers are processes**
> that start on session load.

A canvas is a process **that also opens a listening socket**. It belongs in the
workspace-trust record on the same grounds, with no new mechanism needed:

- A canvas found under `.github/extensions/` in an untrusted workspace is
  **detected and listed but never spawned**.
- Granting trust stays a human act. Per `trust.ts`, the autonomous install path
  never grants it — and a model deciding to execute a canvas that arrived in a
  clone is precisely the decision that record exists to keep with a person.

This is a differentiator, not overhead. `.github/extensions/` arriving via `git
pull` is a supply-chain edge for Copilot users too; we have the machinery to
gate it and the Copilot app does not.

---

## 6. The TUI gap

hoocode has no side panel. `open()` returns a loopback URL and we hand it to a
browser — print it, or open it on request. Acceptable, but it costs us the
lifecycle signal the Copilot app provides:

| Signal | Copilot app | hoocode |
|---|---|---|
| Instance opened | panel opens | browser tab opened or URL printed |
| Instance closed | `onClose(ctx)` | **nothing** |
| Host shutdown | process teardown | session end |

Without a close signal, loopback servers and child processes leak across a long
session. `registry.ts` therefore owns, from the start and not as a retrofit:

- an **SSE-liveness heartbeat** — an instance with no connected client for N
  seconds is idle,
- an **idle reaper** that calls `canvas.close` and reaps the child,
- **teardown on session end** for everything still live.

### 6.1 The process lifecycle to mirror

The SDK's `docs/extensions.md` states the CLI's contract, which our runner should
match so a canvas cannot tell the hosts apart:

| Stage | Copilot CLI behaviour |
|---|---|
| Launch | forked child process; JSON-RPC over stdio |
| Connect | `joinSession()` attaches to the user's **current foreground session** |
| Reload | on `/clear`, or when the foreground session is replaced |
| Shutdown | on CLI exit — **SIGTERM, then SIGKILL after 5s** |

The reload trigger matters for us: hoocode's session lifecycle is not the Copilot
CLI's, so `registry.ts` must decide what counts as "foreground session replaced"
and reload canvases on the same boundary hoocode already uses for session reset.

### 6.2 Known gap: extensions are not only canvases

`joinSession({ tools, hooks, canvases })` — a Copilot extension may register
agent **tools** and **hooks** as well as canvases. Phase 1's shim implements
canvases only. It should accept the `tools` and `hooks` keys, ignore them, and
warn once naming the extension, so a catalog extension that uses them degrades
visibly rather than half-working. Supporting them is a separate decision: hoocode
already has its own tool and hook systems, so this is a mapping question, not a
missing-feature question.

---

## 7. Actions are tool schemas, so they cost tokens

`AGENTS.md` sets a prompt-surface budget: ~4,140 tokens baseline, ~2,710 of it
tool schemas, and everything in an active tool's schema is re-sent on **every**
request. Canvas actions become agent-callable tools. `pr-artifact-explorer`
declares six, several with JSON Schema.

Two rules follow directly:

1. **Actions register only while an instance is open.** A canvas that is not
   open contributes zero tokens. This is the main reason actions bind at runtime
   rather than load time, and it is why canvas cannot be a load-time plugin
   capability (§3).
2. **Action results are truncated.** The reference implementation does this
   deliberately — `entries.slice(0, 200)` with an `entriesTruncated` flag.
   Everything an action returns lands in the model's context window.

Measure with `hoocode --print-token-surface` before and after a canvas opens.

---

## 8. Security posture to copy

`pr-artifact-explorer`'s server is a good reference and its choices should be
defaults in `registry.ts` and in anything we author:

1. **Secrets never cross the boundary.** Tokens stay in the extension process;
   the page receives sanitized metadata only, and every authenticated call is
   server-side.
2. **Untrusted content gets its own origin.** Static previews bind a *separate*
   ephemeral `127.0.0.1` port with their own CSP, so they cannot reach the canvas
   API or its token.
3. **Three-layer request gate.** Canonical `Host` check (DNS rebinding),
   `Sec-Fetch-Site` check (CSRF), then per-instance capability token. Plus a 1MB
   body cap.
4. **CSP without inline script.** `default-src 'self'; object-src 'none';
   script-src 'self' 'wasm-unsafe-eval'; frame-src http://127.0.0.1:*`, with the
   token delivered via a `<meta>` marker replacement.
5. **Typed errors.** `CanvasError(code, message)` gives the agent a
   machine-readable code rather than a string to interpret.

One rule of our own, following from hoocode's permission gate: **a canvas grants
the agent no new capability.** Actions read or mutate canvas state. Anything a
canvas *proposes* still passes the existing permission gate when executed. This
keeps the prompt-injection surface flat — external text rendered into a canvas
can at worst produce a proposal a person still approves.

---

## 9. Phases

Ordering is driven by one principle: validate against **someone else's** canvas
before authoring our own. A third-party artifact is a far better test of the
compatibility layer than anything we write against our own assumptions.

### Phase 1 — Host a canvas at all

`protocol.ts`, `sdk-shim/`, `runner.ts`, `registry.ts`, `discovery.ts`, plus
detection in `plugins/formats/copilot.ts`.

**Acceptance:** `pr-artifact-explorer`, copied unmodified from
`github/awesome-copilot`, opens in hoocode; `open_pull_request` and
`inspect_artifact` both work; closing the tab reaps the child within the idle
window. Plus the §2.3 conformance test and protocol-version assertion.

### Phase 2 — Trust gate

Canvas directories under `.github/extensions/` in an untrusted workspace are
listed but never spawned. Ships with Phase 1 or immediately after. Not optional.

### Phase 3 — `/create-canvas` authoring

A skill plus a scaffold template — in Copilot this is a skill, not machinery, and
it should be the same here. `src/extensions/core/scaffold.ts` already handles
`--platform copilot`. Last, because until Phase 1 exists there is nothing to run
what it scaffolds.

### Phase 4 — Our first canvas

A **plan canvas**: the agent drafts steps and navigates; the person reorders,
toggles status, and adds a note the agent reads back. Deliberately asymmetric —
§1.2 is evidence that concurrent text editing is the unexplored, expensive part.
Per-field last-write-wins on small values with a monotonic document version;
stale writes are rejected by returning current state so the agent retries. No
merge algorithm, because we would not have one.

State placement, split deliberately: the plan document is a reviewable work
artifact and belongs in the repo under `.agents/canvas/`. Runtime prefs and any
cache go to `.hoocode/`, following the two-homes rule in
`docs/plugin-format-mapping.md` §0. The reference implementation keeps everything
in `$COPILOT_HOME`; it is right for cache and prefs, and machine-local state must
not enter git.

Explicit non-goals throughout: multi-user realtime, auth beyond the per-instance
token, anything on a blocking control path, any UI build step.

### Deliberately not first: the permission queue

The pending-permission board is the best canvas hoocode could eventually have —
it is the differentiator, and it is where a terminal prompt is genuinely worse
because the diff context is lost. It is a bad early phase: it sits on the
blocking control path, and "best-effort local state" under "the agent cannot
proceed until this resolves" produces a hung agent. Earn it in a phase where an
outage costs nothing.

---

## 10. Open questions

Settled — see §4.1, §4.2, §6.1: the SDK import is resolved by the host's
automatic module resolver and is never installed; `package.json` plays no part in
resolution and discovery must key off `extension.mjs`; the launch, reload and
shutdown contract is documented and mirrorable.

Still open:

1. **Argv and environment.** The documented contract covers fork, stdio, reload
   and signals, but not what the CLI passes on the command line or in the
   environment. `pr-artifact-explorer` reads `COPILOT_HOME` (defaulting to
   `~/.copilot`); a canvas may read more. Decide which variables hoocode sets and
   which it deliberately does not.
2. **Multiple canvases per extension.** `joinSession({ canvases: [...] })` takes
   an array; `registry.ts` must key by `(extension, canvasId, instanceId)`.
3. **Resolver mechanism under the Bun binary.** `loader.ts` already special-cases
   Bun with `virtualModules`/`tryNative` (see `isBunBinary` there). The canvas
   runner forks a child rather than importing, so it is a different problem, but
   the shim's resolve hook needs verifying under both the Node dev path and the
   packaged Bun binary.
4. **`tools` and `hooks` mapping** (§6.2) — whether to support them at all.

---

## File map

Proposed:

| Path | Role |
|---|---|
| `src/core/canvas/protocol.ts` | 3 JSON-RPC methods + version handshake — the entire GitHub-facing surface |
| `src/core/canvas/sdk-shim/` | child-side `@github/copilot-sdk/extension` implementation |
| `src/core/canvas/runner.ts` | spawn, stdio framing, `session.log` bridge |
| `src/core/canvas/registry.ts` | instances, idle reaper, heartbeat, action→tool bridge |
| `src/core/canvas/discovery.ts` | locate canvas dirs and entry files |

Touched:

| Path | Change |
|---|---|
| `src/core/extensions/plugins/formats/copilot.ts` | canvas **detection** only, no capability mapping |
| `src/core/extensions/plugins/trust.ts` | canvases join the process-gated set |
| `src/extensions/core/scaffold.ts` | `/create-canvas` template (Phase 3) |
| `docs/agent-spec-tree-map.md` | record `.github/extensions/` and `~/.copilot/extensions/` |
| `docs/plugin-format-mapping.md` | note canvas as a non-capability Copilot surface |

Reference (read-only, external):

| Source | Used for |
|---|---|
| `@github/copilot-sdk@1.0.11` `dist/canvas.d.ts`, `dist/sdkProtocolVersion.d.ts` | the contract; devDependency for types and conformance tests |
| `github/awesome-copilot` `extensions/pr-artifact-explorer/` | transport pattern, security posture, Phase 1 acceptance test |
