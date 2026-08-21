# Design Note: Canvas extensions — hosting GitHub Copilot canvases

**Status:** Phase 1 partly implemented. Shipped in
`packages/coding-agent/src/core/canvas/`: `protocol.ts` (wire contract),
`sdk-shim/` (child-side SDK surface), `resolver.ts` (module resolution),
`runner.ts` (fork and lifecycle), `registry.ts` (children, instances, reaping,
action inventory), `discovery.ts`. The Phase 1 acceptance test passes —
`pr-artifact-explorer` from `github/awesome-copilot` opens unmodified and answers
actions (`test/canvas-acceptance-catalog.test.ts`).

Phase 2 is also in: `trust.ts` gates repository-supplied canvases and the
registry enforces it at the single point where a process could start.

Every design question that blocked reachability is now decided, with evidence, in
§11 — availability, environment, timeouts, and the agent-facing tool shape. Canvases
run on every install path where a Node ≥ 20.6 is reachable, including the standalone
binary, which forks a Node child (§11.1).

`launch.ts` implements the availability decision and the production launch path is
verified end to end: a compiled shim forked with `execArgv: []` — no TypeScript
loader — runs the real `pr-artifact-explorer`, serves its page under its own CSP, and
enforces its capability token.

The two agent-facing tools are in (§11.5), and so is the interactive surface:
`/canvas list | open <extension>[:<canvas>] | close <instanceId>`, implemented as a
built-in extension over `core/canvas/session.ts` — a facade that holds no TUI types so
every decision it makes is testable without a terminal. Opening runs behind a
cancellable loader whose `AbortSignal` reaches the abandon path (§11.6), so escape
releases a port the extension may already have bound.

**This is the first user-visible behaviour**; everything before it was subsystem work.

One constraint discovered while wiring it: `registerTool` has no counterpart to remove
a tool, so the two canvas tools register on the first successful open and stay for the
session. A session that never opens a canvas still pays nothing, which is the case that
matters, and they answer honestly when nothing is open — but the earlier claim that they
are absent *whenever* nothing is open is only true until the first open.

The facts about the Copilot side were verified against `@github/copilot-sdk@1.0.11`
(npm) and the `github/awesome-copilot` reference extension `pr-artifact-explorer`,
both read directly rather than from documentation. The module-resolution and
lifecycle contract (§4.1, §6.1) is quoted from `docs/extensions.md` inside the SDK
package and cross-checked against all 23 extensions in the catalog.

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
4. **Canary — shipped.** The `canvas-canary` job in `.github/workflows/ci.yml`
   sparse-checks-out `pr-artifact-explorer` and `arcade-canvas` at a pinned SHA
   and runs `test/canvas-acceptance-catalog.test.ts` against them: forked, opened,
   actions invoked. Catches behavioural drift the types cannot see. Pinned to a
   SHA rather than a branch — a canary aimed at a moving target reports upstream
   edits as our regressions, and the job executes third-party code, so the
   revision should change only when someone bumps it here deliberately. The test
   *fails* rather than skips when the checkout is missing; a canary that can go
   green having run nothing is worse than no canary.
5. **Renovate on that one package — not set up.** Bump → typecheck + canary →
   known in a day. Items 2–4 are the checks; this is what would make them fire on
   the day the SDK changes rather than whenever we next look. See §10 item 4.

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
runtime**. That is why `plugins/formats/copilot.ts` stays a thin mapping file: it
maps vendor paths onto that tree and nothing more.

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

- **The shim is an injected resolver, not a file on disk.** Implemented in
  `resolver.ts`: a `node:module` `register()` resolve hook that maps the package
  and its subpaths onto `sdk-shim/`, delivered entirely through `--import`
  **`data:` URLs**. Nothing is written to disk, so there is no `.mjs` asset to
  copy into `dist/` or embed in the packaged binary — the whole resolver exists
  only as command-line arguments. Verified: after forking `pr-artifact-explorer`
  and driving it through open plus an action, `git status` in the catalog clone is
  clean and no `node_modules` appeared, which is what tier 1 of §2.1 requires.
- **Discovery keys off `<dir>/extension.mjs`, nothing else.** The entry file is
  required, must be named `extension.mjs`, and must be an ES module. All 23
  catalog extensions comply; where `package.json` exists, its `main` is always
  `extension.mjs` anyway. Some extensions also carry a `copilot-extension.json`,
  but it is optional and uninformative — 8 of 23 have one, each holds exactly
  `{ name, version }`, and `name` always equals the directory name.
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
| installed plugins | every plugin on `defaultPluginDirs` | resolved from the manifest, not from position |

This follows the policy in `docs/plugin-format-mapping.md` §0: `.agents/` is
read first and written by default; vendor conventions are compatibility inputs.

The fourth row is not a search root and cannot be one. A plugin's canvases are
named by its **manifest** rather than by where they sit, so the plugin readers
resolve them (`NormalizedPlugin.canvasExtensions`) and `plugin-canvases.ts`
presents the result in the shape the three roots produce — which is what lets
the trust gate, the listing and `open` stay unaware a plugin was involved. Two
layouts are read, both real:

| Layout | Seen in |
|---|---|
| `"extensions": "<dir>"` manifest key + `<dir>/<id>/extension.mjs` | `Redth/mobile-canvas-ghcp` |
| `com.github.copilot/extensions/<id>/extension.mjs` | `github/awesome-copilot`, all 24 canvas entries |

A plugin root that itself carries `extension.mjs` is one canvas, named for the
plugin — there is no directory below the root to take a name from.

Precedence puts plugins last: anything a person placed in a search root by hand
shadows a same-named canvas that arrived inside a package. Scope comes from
where the *plugin* lives, not the canvas directory — a plugin installed at
project scope travels in every clone whichever subdirectory its canvas sits in,
and that is the only question §5 asks.
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

- A canvas found under `.agents/extensions/`, `.github/extensions/`, or a
  project-scoped plugin home (`.agents/plugins/`, `.hoocode/plugins/`,
  `.claude/skills/`) in an untrusted workspace is **detected and listed but
  never forked**.
- Granting trust stays a human act. Per `trust.ts`, the autonomous install path
  never grants it — and a model deciding to execute a canvas that arrived in a
  clone is precisely the decision that record exists to keep with a person.
- **`/new-canvas` grants it**, on the same footing as `/plugin install --scope
  project`: a person typing that command in that directory *is* the human act,
  and the alternative is absurd — the gate refusing a file the person created
  seconds earlier, with the words "came with this repository". The grant is
  wider than the one canvas, since it also releases the hooks and MCP servers of
  plugins already committed there, so the command says so and names
  `/plugin untrust`. Scaffolding is the only authoring path that grants; nothing
  a *model* does ever will.

### 5.1 The gate is shared with the plugin gate

`isRepositorySupplied` and `shouldWithholdRepositorySupplied` live in
`plugins/trust.ts` and serve both gates. Each caller supplies only its own
project-scope roots — `.claude/skills` + `.agents/plugins` + `.hoocode/plugins` for
plugins, and those same three plus `.agents/extensions` + `.github/extensions`
for canvases, since a plugin can ship one (§4.3) — because that
list is the only thing that differs. `loader.ts`'s `isProjectSuppliedPlugin` and
`shouldWithholdExecutables` keep their names and behaviour and now delegate; the
existing plugin trust tests pass unchanged, which is what makes the consolidation
safe to assert.

### 5.2 One difference from plugins

`shouldWithholdExecutables` withholds a plugin's hooks and MCP servers while still
loading its skills, because a plugin has passive capabilities worth having.
**A canvas has none.** Its declaration, its actions and its UI all come from
running its code — even listing what a canvas offers requires forking it, since
the declaration arrives in the child's `ready` message. There is nothing to
partially allow, so `shouldWithholdCanvas` withholds the extension whole.

What survives is discovery: `discovery.ts` only reads directory entries, so an
untrusted canvas can still be named and offered. `gateCanvasExtensions` returns
`{ runnable, withheld }` rather than filtering silently, because the point of the
gate is that a person can see what a repository offers and choose — not that the
offer disappears.

Enforcement lives in `CanvasRegistry`, in the private `child()` method every path
funnels through. Callers are still expected to filter with `gateCanvasExtensions`
so they can explain a refusal; the registry check is the backstop that makes a
forgetful caller safe rather than dangerous.

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

- an **idle reaper** that calls `canvas.close` and reaps the child,
- a **per-canvas instance cap**, so a loop cannot open ports without bound,
- **teardown on session end** for everything still live.

An earlier draft of this section also called for an "SSE-liveness heartbeat — an
instance with no connected client for N seconds is idle". **That was wrong and is
withdrawn.** It is not implementable: the SSE endpoint and its client set live
inside the extension's own HTTP server (`entry.sseClients` in
`pr-artifact-explorer`'s `server.mjs`), and the host never sees them. Learning
otherwise would take either proxying the canvas URL — which breaks the token,
origin and CSP model the extension built for itself — or adding a liveness call to
the contract, which breaks tier-2 portability (§2.1). Neither is worth it for a
reaper.

So `registry.ts` defines idleness narrowly and honestly: **time since hoocode last
touched the instance**, meaning opened it or invoked one of its actions. A person
reading a canvas in a browser tab is invisible to us. That makes the reaper
advisory cleanup rather than a claim about whether anyone is watching, and it is
why the default timeout is generous (30 minutes) and the child lingers after its
last instance closes — reopening a warm extension should not pay for a fork.

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

`pr-artifact-explorer`'s server is a good reference. Read the list below with one
correction the smoke test forced (§12.1): **points 1–4 are things the extension's
own HTTP server does, and the host cannot default them.** `arcade-canvas` serves
its page with no capability token at all, and that is its author's call to make.
So this is a standard for *anything we author* — the `/new-canvas` template, and
any canvas hoocode ships — plus the posture we should expect when reviewing one,
not a property `registry.ts` can enforce. Point 5 and the rule at the end are
genuinely ours:

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

What `/new-canvas` scaffolds, and what it leaves out: the template implements the
per-instance capability token (point 3) and typed `CanvasError`s (point 5), and
stops there. A scaffold is read as much as it is run, so it demonstrates the one
mechanism a canvas cannot work without and points here for the rest — the `Host`
and `Sec-Fetch-Site` checks, the body cap, the separate origin for untrusted
content, and the CSP are all worth adding to a canvas that grows past a demo.

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

### Phase 3 — `/new-canvas` authoring — shipped

Named `/new-canvas`, not `/create-canvas` as this section first proposed: the
three sibling scaffolds are `/new-skill`, `/new-agent` and `/new-command`, and
one odd verb in that set is a worse cost than a rename before anyone depends on
it.

A scaffold template in `src/extensions/core/scaffold.ts`, writing
`.agents/extensions/<name>/extension.mjs` by default and `.github/extensions/`
under `--platform github`. Deliberately *not* routed through the format
registry's `WorkspaceLayout` like `/new-skill` and friends: there is no Claude
canvas convention, and a layout method returning nothing for one adapter would
be a worse lie than naming the two real homes.

The template is complete rather than a stub, and that is the point — a canvas has
no passive half, so a scaffold that does not run teaches nothing and cannot be
checked. `test/new-canvas.test.ts` forks what it writes through the production
runner and drives the protocol against it.

Scaffolding grants workspace trust, for the reason in §5 — without it the gate
withholds the canvas the moment it is written, which is how this shipped first
and was wrong.

**Acceptance:** `/new-canvas x` then `/canvas open x` works with no `/reload`
in between; canvases are discovered when `/canvas` runs, not loaded at session
start.

**Superseded in part by §13.** Measured against Copilot's `/create-canvas`, a
scaffold is only the first third of the command: it now takes a description,
opens what it creates, and hands the agent a brief to build it — and, the gap
that actually mattered, an edit to an open canvas can be made live.

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

Settled since the first draft — see §11 for each decision and its evidence:
availability and how the child is launched (§11.1), the child's environment
(§11.2), `tools`/`hooks` (§11.3), timeout policy (§11.4), and how actions reach
the agent (§11.5). Multiple canvases per extension is handled: `registry.ts` keys
instances by `(extensionId, canvasId, instanceId)`.

Settled since: the **TUI surface for opening and cancelling**. `/canvas open`
draws a `BorderedLoader`, and its `AbortSignal` is what carries a person's Esc
into the abandon path (§11.6) rather than merely hiding a spinner — verified in
§12's smoke test. One cosmetic gap remains and is recorded there, not here.

Still open:

1. **Removing the Node requirement entirely** would need a Bun-native resolver, since
   the `module.register` hook path is disproven. Not worth it while every install path
   can reach a Node; recorded in case that changes.
2. **The shim is TypeScript, so tests fork it from `src/` under `tsx`** while a release
   resolves built JavaScript. `CanvasRuntime` takes the shim URL as a parameter exactly
   so the two differ by configuration and not by code, but the path CI exercises is
   still not byte-for-byte the path a release runs. See `test/canvas-test-runtime.ts`.
3. **Should the host validate action input against the declared `inputSchema` before
   dispatch?** Observed in §12.1: `arcade-canvas` normalises an unknown `gameKey` to a
   default, so a misspelled field silently *looks* like a success. Validating would turn
   that into an honest error, at the cost of rejecting input the canvas would have
   accepted. Left open on purpose — the case against is that the canvas owns its own
   contract, and `list_canvas_capabilities` already hands the model the schemas to read
   (§11.5).
4. **Nothing announces an SDK bump.** §2.3 items 2–4 are the right checks and all three
   are in place; what is missing is item 5, the Renovate rule that makes them run on the
   day `@github/copilot-sdk` changes rather than whenever someone next bumps it by hand.

---

## 11. Decisions

### 11.1 Availability: canvases require a reachable Node ≥ 20.6

Two facts, both established by running the code rather than reading about it.

**Bun does not honour `node:module` resolve hooks.** It exports `register` as a
function, so the call succeeds and nothing warns — but the child then resolved the
*real* `@github/copilot-sdk` out of Bun's global install cache instead of the shim:

```
error: joinSession() is intended for extensions running as child processes of the Copilot CLI.
  at /root/.bun/install/cache/@github/copilot-sdk@1.0.11@@@1/dist/extension.js:18
```

Silent wrong-resolution, not an error. So re-executing the packaged binary with the
same `--import` resolver is not an option; it would need a Bun-native mechanism
(`Bun.plugin` in a `--preload`), which is unbuilt work of unknown size.

**`process.versions.node` cannot be used to detect Node.** Bun reports
`process.versions.node = "24.3.0"` alongside `process.versions.bun = "1.3.11"`.
A version check alone passes under Bun and then fails silently per the above, so
detection must key on `process.versions.bun` being **absent**.

The decision: **the child must run under Node ≥ 20.6; hoocode itself need not.**

An earlier draft of this section conflated those and excluded the standalone binary
altogether. That was wrong. The Bun finding rules out running the *child* under Bun;
it says nothing about a Bun-compiled *parent* forking a Node child, which works —
verified by running the real `pr-artifact-explorer` from a Bun parent through a Node
child, page serving under its own CSP and capability token enforced. So the question
`launch.ts` asks is "is a usable Node reachable?", not "are we Node?":

| Install path | Child forks |
|---|---|
| `npm install -g` (`engines: node >= 20.6.0`) | `process.execPath` — we are already Node, no PATH lookup |
| bun / source checkout | `process.execPath` when it is Node, else PATH |
| standalone binary | `node` from PATH, discovered on first use |

Only "no Node anywhere" degrades, and it degrades honestly: `resolveCanvasRuntime`
returns either a `CanvasRuntime` or a sentence the user can act on, and never throws.
Two ordering details matter — the shim is checked before PATH is probed so the more
actionable reason wins, and forking ourselves requires `process.versions.bun` to be
*absent* rather than merely a satisfying Node version.

Pleasant alignment: the npm package already declares `engines: node >= 20.6.0`, which
is exactly `module.register`'s minimum, so for npm users the requirement is guaranteed
by the package rather than an extra ask.

Resolution can spawn `node --version`, so it is not cached here: callers resolve once
per session and hold the result.

**Running from source.** A checkout has no `dist`, so the built shim does not exist and
canvas support would be off for exactly the people most likely to be writing a canvas.
`launch.ts` therefore pairs each shim candidate with the argv needed to import it: the
built `index.js` needs none, and the TypeScript source is offered with a `tsx` loader —
but **only when `tsx` actually resolves**, so this stays a verified capability rather
than a hopeful one. The loader path is absolute, because Node resolves a bare
`--import` specifier against the *child's* working directory.

**Shipping the shim.** The child imports the shim from a real path on disk, which the
Bun binary's virtual filesystem is not. `config.ts` gains `getCanvasDir()` beside the
existing `getThemesDir()` and `getExportTemplateDir()`, following the same shape, and
`scripts/build-binaries.sh` copies `dist/core/canvas` next to the executable — the
whole directory, so `sdk-shim/index.js`'s import of `../protocol.js` still resolves.
That is not a new distribution mechanism: the binary zip already ships
`export-html/`, `theme/`, `assets/`, a wasm blob and a `node_modules/koffi` tree with
a native module beside the exe.

### 11.2 The child's environment

Evidence, from the SDK's own `dist/extension.js`: the real `joinSession` reads
exactly one variable, `SESSION_ID`, and throws without it. Its transport is
`connectToParentProcessViaStdio()`, which writes to `process.stdout` — confirming
stdout is the protocol channel, as §1.1 assumed.

- **`SESSION_ID` is set.** Our shim does not need it, but a real SDK-using
  extension throws without it, so setting it is free fidelity.
- **`COPILOT_HOME` is left alone**, defaulting to `~/.copilot`. Pointing it at a
  hoocode-private directory would isolate state but would also make the hosts
  distinguishable and lose an extension's existing cache — and a canvas being
  unable to tell the hosts apart is the whole portability thesis (§2).

### 11.3 `tools` and `hooks`: closed by measurement

**Zero of the 23 catalog extensions declare `tools` or `hooks` alongside a canvas.**
The shim's accept-and-warn-once behaviour (§6.2) is therefore sufficient, and
mapping them onto hoocode's own tool and hook systems is not worth designing until
a real extension needs it.

### 11.4 Timeouts are per method

A single 30s ceiling was a guess, and wrong for `canvas.open` on a canvas that
downloads an artifact before returning a URL. Ceilings are per provider method:
`canvas.open` generous, `canvas.action.invoke` tighter, `canvas.close` short —
each overridable. Cancelling a slow `open` rather than merely waiting it out is
handled by the abandon path in §11.6.

### 11.5 Actions reach the agent through fixed tools

Copilot names its own agent-facing shape in the SDK types: `list_canvas_capabilities`
for discovery and `invoke_canvas_action` for invocation. Two fixed tools, whatever is
open.

hoocode mirrors that, for its own reason as well as fidelity: `AGENTS.md` budgets the
prompt surface at ~4,140 tokens with ~2,710 of it tool schemas, and everything in an
active tool's schema is re-sent on every request. One tool per open action would make
that surface scale with how many canvases are open. Two fixed tools keep it flat, and
`registry.activeActions()` feeds the discovery response rather than the tool registry.

Measured: `list_canvas_capabilities` ~75 tokens, `invoke_canvas_action` ~144 — and
`createCanvasToolDefinitions` returns an empty array while nothing is open, so a
repository without canvases pays nothing at all. A test asserts each schema stays under
the ~250-token per-tool budget, so a future description cannot quietly start costing
every request. The invariant the tests protect is that the surface is *fixed*, not that
it is two: one tool per open action is the thing ruled out.

Two consequences worth stating:

- **There is no "open a canvas" tool.** Opening forks a process and binds a listening
  socket, which is a person's decision behind the trust gate (§5). The agent drives a
  surface a human already opened. That also keeps the injection surface flat: text
  from an issue title rendered into a canvas can at most cause an action on an
  instance somebody chose to open.
- **There *is* a `reload_canvas` tool**, added with §13, and the distinction is what
  keeps the bullet above intact. Reloading restarts an extension the person already
  opened, in a workspace they already trusted, from a path the host already resolved;
  the model cannot reach a new extension through it, and a poisoned string in some
  canvas's data still cannot cause one to start. It is not a boundary on the file's
  *contents* and is not described as one — whatever wrote `extension.mjs`, through the
  permission gate, is what runs. It is hoocode's, not Copilot's: their host reloads the
  panel itself. Measured at ~59 tokens, so the three together are ~278.
- **Instances are addressed by `instanceId` alone.** It is a UUID unique across every
  canvas, so the schema needs three fields instead of five; the registry resolves the
  extension and canvas ids itself.

**The tool description makes no safety claim.** An earlier version told the model
actions "cannot edit files or run commands". That is false: a canvas extension is
arbitrary Node code running with the user's privileges — `pr-artifact-explorer`
downloads artifact ZIPs to disk and calls the GitHub API — and none of it passes
hoocode's permission gate, because the gate sits in front of hoocode's own tools, not
inside a forked extension. The workspace-trust gate (§5) is the control here. Never
state a safety property the runtime does not enforce; a test asserts the description
does not regress to one.

Failures are thrown, matching the built-ins — the loop turns a rejection into the
model's result. `CanvasError.code` survives the process boundary as
`CanvasCallError.code`, but only an `Error`'s *message* is rendered to the model, so
the tool folds the code into the message rather than letting the typed-error intent
(§8) stop at the tool boundary.

### 11.6 Cancelling: one abandon path, and a signal that is honoured

The provider protocol has no cancel verb, so aborting a call only ends *our* wait —
the child may have finished opening and be holding a port. What makes cancelling safe
is that `registry.open` generates the `instanceId` **before** calling `canvas.open`,
so it can close an instance it never saw open.

`abandon(extensionId, canvasId, instanceId)` serves a person's cancel and a timeout
identically: send `canvas.close`; if that goes unanswered the child is wedged, and
terminating it is the only lever left — but that kills every instance of that
extension, so it happens only when no sibling instance is live. When siblings exist the
child is left running and the possible leak is reported, rather than paid for by
someone else's open canvas.

`invoke_canvas_action` now passes the turn's `AbortSignal` through
`registry.invokeAction` into the runner. A signal that is already aborted rejects
without writing anything to the child, and a late answer for an abandoned call is
dropped rather than mistaken for the next one — the runner ignores any response whose
id is no longer pending.

Per-method ceilings are also reachable now. They had been defined in `runner.ts` but
`CanvasRegistryOptions` did not forward them, so nothing that goes through the registry
— which is everything real — could set them. The timeout test is what caught it.

---

## 12. Smoke test, and one known gap

Run in a real terminal (`./hoocode-test.sh`, tmux per `AGENTS.md`) against the fixture
canvas installed at `~/.copilot/extensions/`, plus a repository-supplied copy under
`.agents/extensions/`:

| Path | Result |
|---|---|
| `/canvas list` | lists user-scope extensions, and the repository-supplied one as `[withheld: untrusted workspace]` with a pointer to `/plugin trust` |
| `/canvas open <ext>` | opens; prints the loopback URL |
| the URL | serves 200 with its token, **403 without** |
| `/canvas close <id>` | closes; the port is refused afterwards, so the extension released it |
| `/canvas open <withheld>` | refused, quoting the trust reason |
| escape during a slow open | cancels; the extension logs `closed <id> (known=false)` — the abandon path (§11.6) reaching an instance it never finished opening |

Two bugs it caught that no unit test could: `CATEGORY_GLYPH` is a map, so the listing
rendered `[object Object]`; and the cancel path needed to decide from
`loader.signal.aborted` rather than from which of the abort and the rejection landed
first.

**Known gap.** The "Canvas open cancelled." confirmation does not render when the cancel
came from the loader's own escape handling. Every *effect* of cancelling is correct and
verified above. Ruled out: the continuation runs and `ctx.ui.notify` works on those
lines (the failure path renders its error, and the canvas's own diagnostic arrives
through the same function moments later), and deferring a tick did not help. Left
documented rather than papered over with a sleep — the loader disappearing is itself the
signal.

### 12.1 A second catalog canvas

`arcade-canvas` (a different author) was taken through the same path — installed to
`~/.copilot/extensions/` exactly as its README says, then opened from `/canvas`,
driven through the two agent tools, and closed. It is a useful second case because it
differs where it matters:

| | `pr-artifact-explorer` | `arcade-canvas` |
|---|---|---|
| `package.json` | none | present, declaring `@github/copilot-sdk` |
| README install step | copy the folder | copy **and `npm install`** |
| loopback server | capability token enforced | **no token** |
| content | GitHub API + ZIP index | static Phaser game bundle |

It ran with **no `node_modules` anywhere and the `npm install` step skipped**, which is
§4.1 and §4.2 confirmed against a second extension: the declared dependency is
decorative because the host resolves the import. Its page and every static asset it
references served, path traversal was refused, `select_game` switched games, and two
instances stayed independent on their own ports.

Two things worth carrying forward:

- **The security posture in §8 is each extension's choice, not something the host
  imposes.** `arcade-canvas` serves its page with no capability token at all. Nothing is
  wrong with that for a game, but it means "canvases are token-gated" is an observation
  about one extension rather than a property of the surface.
- **Nothing validates action input against the declared schema** — not hoocode, and not
  necessarily the extension. `select_game` normalises an unknown `gameKey` to a default,
  so a wrong or misspelled field silently *looks* like a success. That is precisely why
  `list_canvas_capabilities` returns the schemas (§11.5): the model is meant to read
  them rather than guess. Whether the host should validate input against
  `inputSchema` before dispatch is an open question — it would turn a silent default
  into an honest error, at the cost of rejecting input a canvas would have accepted.

Incidental confirmation of §11.5's honesty fix: merely running `pr-artifact-explorer` in
tests created `~/.copilot/extensions/pr-artifact-explorer/artifacts/cache`. A canvas
really does write outside itself, and none of it passes the permission gate. Discovery
correctly ignores that directory, because it keys off `extension.mjs` and nothing else.

---

## 13. Parity with `/create-canvas`: authoring is a loop, not a file

`/new-canvas` shipped as a scaffold (§9 Phase 3) and was measured against the
thing it is the counterpart of — Copilot's `/create-canvas`, which "lets you
create interactive interfaces directly from a conversation": you describe what
you want in the prompt box, the agent generates the extension, it opens in the
right panel, and **you keep iterating by asking for capability or UI changes**.

Three gaps, each reproduced before it was fixed:

| | before | after |
|---|---|---|
| `/new-canvas a kanban board for the release checklist` | refused: *"name must be lowercase a-z, 0-9, and hyphens only"* | scaffolds `kanban-board-release`, and says which name it derived |
| after scaffolding | printed *"Open it now with /canvas open x"* | already open, url in the same message |
| editing `extension.mjs` while it is open | **changed nothing at all** | `reload_canvas` / `/canvas reload` makes it live |

The third was the one that mattered, and it was silent. The registry hands back
the child it already forked, so an edit reached neither the open page nor a
freshly opened *second* instance; the only way to see a change was to end the
session. "Ask for a UI change and watch it appear" — the entire point of
authoring a canvas in a session — was not reachable at any speed.

### 13.1 Reloading

`registry.reload(extensionId)` forks the edited code, and only once the new child
has answered with its declarations does it close the old instances, kill the old
child, and re-open each instance against the new one.

That order is the whole design. A broken edit is the *normal* failure while
iterating — a syntax error, a throw at module scope — and paying for it with the
person's open canvas would make the loop hostile. On failure the old child is
still registered and still serving, and the reload throws.

Two consequences are stated rather than hidden:

- **Instance ids survive; urls do not.** The extension binds a fresh ephemeral
  port and mints a fresh capability token inside `open()`, and the host has no
  way to make it reuse either — that is the extension's business (§1.1). So a
  reload always hands back new urls, and both the command and the tool
  description say to give them to the person: the tab in front of them points at
  a closed port. This is the one place hoocode is visibly worse than a host with
  a panel it can re-point.
- **In-process state is gone**, because a process really did restart. The `input`
  each instance was opened with is replayed so the canvas comes back rather than
  coming back empty, but anything the extension kept in memory does not survive.

`onExit` needed a guard for this: during a reload two children of one extension
are briefly alive, and the probe dying before it is adopted must not clear the
live child's instances. Only the *currently registered* child may clear the
table.

### 13.2 Describing instead of naming

`/new-canvas` now reads three shapes: a bare slug (unchanged, and tested first so
a one-word request can never be re-read as a description), `name: description`,
and a bare description whose directory name is derived. Derivation drops grammar
only, never subject matter — "a board for tracking the release checklist" becomes
`board-tracking-release` — and gives up rather than inventing `canvas-1` when
nothing usable survives, because a name we made up hides that we did not
understand the request.

When a description is given, the command opens the canvas and then sends the
model a build brief (`canvasBuildBrief`) as a follow-up turn. A scaffold plus a
sentence is not a canvas; the gap between them is the agent's work, and the brief
is what makes it a task rather than a hope. It states the three contract rules
whose symptom does not name the cause: an installed dependency (§4.1 — the
resolver already provides the SDK), a `console.log` (corrupts the JSON-RPC
channel), and a write without a reload (changes nothing).

A bare `/new-canvas my-board` still means "give me the template". Starting a build
nobody asked for would burn a turn and overwrite the file they were about to edit.

### 13.3 What moved, and why

`/new-canvas` left `extensions/core/scaffold.ts`, where it sat beside
`/new-skill`, `/new-agent` and `/new-command`. It is not a file-writing command
any more: it opens what it writes and drives the agent loop, so it needs the
canvas session and `pi.sendUserMessage`, neither of which belongs in a scaffold.
The command lives in `extensions/core/canvas.ts`; every decision it makes — the
parse, the name derivation, the homes, the template, the brief — is in
`core/canvas/scaffold.ts`, testable without a terminal, a fork or a model.

### 13.4 What building one with it turned up

The command was then used for its own purpose — `/new-canvas create lightweight
games that can be played with keyboard arrow keys` — and the canvas it produced
is committed at `.agents/extensions/arrow-key-games/`, covered by
`test/canvas-arrow-key-games.test.ts`. That makes it §9 Phase 4's "a canvas of
our own", though not the plan board that section describes.

Four things the loop surfaced that no unit test had:

- **Renaming the canvas `id` drops the open instance.** The scaffold names the
  canvas after the directory, the directory is named from a sentence, and the
  first thing a model wants to do is pick a better name. The reload reported it
  exactly right — *"the reloaded extension no longer declares canvas X (declares:
  Y)"* — but it still cost the canvas the person was watching, twice. The brief
  now says to change `displayName` and leave `id` alone. Auto-adopting a sole
  replacement was considered and rejected: it is a guess, and a wrong one
  whenever an extension genuinely declares several canvases.
- **A broken edit really is free.** A syntax error in the first draft made the
  reload refuse; `/canvas list` showed the canvas still open on its original
  port, unchanged. That is §13.1's ordering doing its job on an accident rather
  than in a test.
- **The token gate needs to cover what the page itself loads.** The first draft
  served `/app.js` and `/app.css` behind the token but linked them without one,
  so every asset 403'd. `curl` checks that append the token by hand cannot see
  this; a browser hits it on the first paint. The lesson generalises to any
  canvas that serves more than one route.
- **Real time is what an agent cannot join.** Snake and the maze are the
  person's alone — an agent acting in tool calls seconds apart cannot share a
  130ms clock. The third game, Duel, is turn-based precisely so it can be
  shared, and that is the general shape: **a canvas an agent co-plays has to be
  turn-based, or it has to give the agent something other than reflexes to
  contribute.**

One thing the canvas needed that the host does not provide: a way to *wait*. The
agent has no event channel — nothing tells it the person moved — so its only
options are to poll `get_state` or to be told. Duel solves it inside the
extension with an `await_turn` action that blocks until the turn flips and
returns just under the host's 30s action ceiling. It works, and it is worth
noting that every canvas wanting the same thing has to invent it again; a
host-level "wait for this canvas to change" has an obvious shape and does not
exist.

### 13.5 The rest of the lifecycle

Create and iterate were built first because they were missing outright. Reviewing
the surface afterwards found that everything *after* creation was still
hand-work, and one piece of it was a trap.

**Naming was wrong more often than right.** Measured on twelve realistic
phrasings, seven produced a name for the *request* rather than the thing:
`create-lightweight-games`, `build-dashboard-showing`, `want-review-pull`,
`help-compare-two`. Two rules fix it, both grammar-only:

- **Request words are dropped anywhere** — `create`, `build`, `make`, `show`,
  `help`, `want`, `please`, `something`. People type this command as an
  instruction, and the instruction was landing in the directory name.
- **Nouns are preferred over `-ing`/`-ed` words**, but only while at least two
  survive. A participle sits between the request and its subject and pushes the
  subject out of a three-word name: `dashboard-showing-flaky` was losing "tests".
  Where dropping them would leave nothing, the participle *is* the subject —
  "a canvas for onboarding" — and it stays.

Every one of the fifteen sample phrasings now improves or holds, and the table is
the test (`test/canvas-lifecycle.test.ts`): a heuristic asserted case by case
invites tuning one case at the expense of the rest.

**`/canvas rename <extension> <new-name>`.** A canvas has more identity than a
file: the directory name *is* the extension id, since discovery keys off
position, and the canvas separately declares an `id`, a `displayName` and
whatever its header comment tells the reader to type. Renaming by hand means
getting all of them right, and the `id` is not cosmetic — §13.4 records losing an
open canvas to it twice.

The rewriting is narrow on purpose: **a string literal whose entire content is
the old name**, plus `/canvas <verb> <old>` in a comment. That covers `id`,
`displayName`, `title` and the scaffold's `ID`/`NAME` constants without knowing
any of their names, and it cannot touch a sentence that merely mentions the
canvas — `"the board is the point of the board"` is not the string `"board"`. A
whole-word replacement would have rewritten that sentence, which is worse than
leaving it. Everything else is reported by line number, never edited.

The template changed to meet it halfway: it named itself in six places, so a
rename left four stale mentions behind. It now names itself once, in `ID` and
`NAME` at the top, and a scaffolded canvas renames with nothing left over. Both
shapes rename correctly, since the rule is about literals rather than about the
template.

**`/canvas remove <extension>`.** Confirmed, because deleting source is not
undoable from here — and *refused* rather than assumed outside a terminal, so a
piped `--print` session cannot delete a directory. Both it and rename close what
the extension had open and stop its child first: a process outliving its own
source keeps serving code that is nowhere on disk, which is the most confusing
state a canvas can be in.

Both refuse a canvas that arrived inside a plugin. A plugin's canvases are named
by its manifest rather than by position, so moving the directory would break the
plugin rather than rename anything; the refusal points at `/plugin`. The test is
positional and cheap: an extension is ours to edit when its directory sits
directly inside a search root.

**Editing capabilities needed steering, and got it from the reload.** Adding,
removing or reshaping an action was invisible — the reload answered "which
canvases exist", which is not the question an author has. A typo inside
`actions: [...]`, an action attached to the wrong canvas, and a handler that
throws at declaration time all fail the same silent way: the action is simply not
there, and the next `invoke_canvas_action` reports it missing with no hint why.

`reload` now diffs the action inventory and reports **added / removed / changed**,
through the tool and through `/canvas reload`. `changed` is separated because it
is the case where a `list_canvas_capabilities` result the model still holds has
become *wrong* rather than merely incomplete. "Nothing changed" is said out loud
too, since silence would read as success.

**`/canvas list` names the actions of open instances.** They were visible only to
the model, through `list_canvas_capabilities` — so the person steering the
session could not see the surface they were being asked about. Only for open
instances, because actions come from running the code (§5.1); an extension that
has never been forked has no actions to report, and none knowable is different
from zero.

### 13.6 Still not parity

Copilot renders the canvas in a panel it owns. hoocode hands you a URL for your
own browser, so it cannot re-point a tab on reload, cannot follow a
`navigateInstance` call, and cannot tell whether anyone is looking (§6, and the
registry's correction to it). Those follow from not having a panel, not from
anything above.

---

## File map

Shipped:

| Path | Role |
|---|---|
| `src/core/canvas/protocol.ts` | 3 provider methods + version + payloads + NDJSON codec — the entire GitHub-facing surface |
| `src/core/canvas/sdk-shim/` | child-side `@github/copilot-sdk/extension` implementation |
| `src/core/canvas/resolver.ts` | `data:`-URL `--import` hook mapping the SDK specifier onto the shim |
| `src/core/canvas/runner.ts` | fork, stdio framing, request correlation, `session.log` bridge, SIGTERM→SIGKILL |
| `src/core/canvas/registry.ts` | children, instances, idle reaper, instance cap, action inventory, trust enforcement |
| `src/core/canvas/trust.ts` | withholds repository-supplied canvases in an untrusted workspace |
| `src/core/canvas/discovery.ts` | locate canvas dirs by `extension.mjs` |
| `src/core/canvas/launch.ts` | whether canvases can run here (§11.1), and the runtime to fork |
| `src/core/canvas/session.ts` | the session facade: what is there, can it run, open this, close that — with no TUI in it |
| `src/core/canvas/plugin-canvases.ts` | canvases resolved out of installed plugins, in discovery's own shape (§4.3) |
| `src/extensions/core/canvas.ts` | `/canvas list \| open \| reload \| close` and `/new-canvas`, the cancellable loader, and tool registration on first open |
| `src/core/canvas/scaffold.ts` | what `/new-canvas` was asked for, where it writes, the template, and the model's build brief (§13) |
| `src/core/canvas/lifecycle.ts` | renaming and removing: which places hold a canvas's name, and which are prose (§13.5) |
| `src/core/tools/canvas.ts` | `list_canvas_capabilities` + `invoke_canvas_action` + `reload_canvas`, created only while a canvas is open |
| `src/modes/interactive/resource-display.ts` | canvases in the startup / `/reload` summary — counted, and named with how to open them |
| `test/canvas-acceptance-catalog.test.ts` | acceptance against two real catalog extensions (§12.1); the CI canary runs it (§2.3) |
| `test/canvas-reload.test.ts` | the iterate loop end to end: edit an open canvas, reload through the agent's own tools, drive the capability that edit added (§13) |
| `.agents/extensions/arrow-key-games/` | hoocode's own canvas — arrow-key Snake and Maze, plus Duel, a turn-based game the agent plays (§13.4) |
| `test/canvas-arrow-key-games.test.ts` | that canvas's contract: declarations, the token gate covering its own assets, and Duel's turn rules |
| `test/canvas-lifecycle.test.ts` | naming measured against fifteen real phrasings, plus rename and remove through a live session (§13.5) |

Still to build:

| Path | Role |
|---|---|
| the plan board | §9 Phase 4. `.agents/extensions/arrow-key-games/` is now a canvas of our own (§13.4), but it is a games canvas — the reviewable plan surface that phase is actually about is still unbuilt |

Touched:

| Path | Change |
|---|---|
| `src/core/extensions/plugins/formats/copilot.ts` | canvas **detection** only, no capability mapping; plus the `com.github.copilot/` content namespace |
| `src/core/extensions/plugins/trust.ts` | gained the shared `isRepositorySupplied` / `shouldWithholdRepositorySupplied` used by both gates |
| `src/core/extensions/loader.ts` | plugin gate delegates to the shared pair; its private `isUnderDir` removed |
| `src/utils/paths.ts` | gained `isPathInside`; four private copies of it already existed elsewhere |
| `src/extensions/core/scaffold.ts` | `/new-canvas` template (Phase 3) — ✅ shipped |
| `src/core/extensions/plugins/formats/shared.ts` | `detectCanvasExtensions`, shared by all three readers |
| `.github/workflows/ci.yml` | the `canvas-canary` job (§2.3 item 4) |
| `docs/agent-spec-tree-map.md` | record `.github/extensions/` and `~/.copilot/extensions/` |
| `docs/plugin-format-mapping.md` | note canvas as a non-capability Copilot surface |

Reference (read-only, external):

| Source | Used for |
|---|---|
| `@github/copilot-sdk@1.0.11` `dist/canvas.d.ts`, `dist/sdkProtocolVersion.d.ts` | the contract; devDependency for types and conformance tests |
| `github/awesome-copilot` `extensions/pr-artifact-explorer/`, `extensions/arcade-canvas/` | transport pattern, security posture, acceptance tests |
