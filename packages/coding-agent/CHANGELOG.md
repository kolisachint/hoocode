# Changelog

## [Unreleased]

## [0.5.27] - 2026-08-22

### Added

- hoocode can answer questions about itself. The startup banner already promised
  "hoocode can explain its own features and look up its docs", but nothing put a
  docs path into model context — `getDocsPath()`'s only caller printed paths to
  the human on an auth failure. The agent could not find them on its own either:
  its cwd is your project, so searching there turns up your docs, never
  hoocode's.

  The system prompt now lists the shipped docs (~215 tokens, filenames only),
  and a `SearchHooCode` tool retrieves them at the heading level, returning a
  file path and line number to read. It also indexes the session's skills,
  slash commands, and subagents, so "what can you do?" has one place that
  answers it. MCP tools remain with `ResolveMcpTools`.

- New documentation for features that had none: `plugins.md` (marketplaces,
  the trust model, plugin formats), `mcp.md` (server config, transports,
  deferred schemas), `modes.md` (ask/plan/build/debug and the plan → grill →
  approve → goal workflow), and `canvas.md`. `usage.md` gains the 13 slash
  commands that were shipping undocumented, including `/plugin`, `/mode`,
  `/loop`, `/canvas`, `/cost`, and the `/new-*` scaffolds, plus the 12 CLI
  flags that were missing from the reference — the optional tool bundles
  (`--enable-todowrite`, `--enable-webtools`, `--enable-search-tool`,
  `--enable-plugintools`), the subagent flags, `--light`,
  `--print-token-surface`, `--platform`, `--team`, and `--disallowed-tools`.
  Every slash command and CLI flag hoocode accepts is now documented.

### Fixed

- Capability search could not match a plural: a query for "theme" missed a
  heading reading "Themes". Singular forms are now indexed alongside the
  originals, so exact tool names still match exactly.

- Capability search ranked on question filler. "how do I add a custom theme"
  was won by a section matching "add" and "how" over the one titled "Creating a
  Custom Theme". Function words are now dropped from queries.

- The dense capability index is a shared store keyed on a content hash, but the
  MCP loader seeded it from only its own tools, evicting any other producer's
  vectors. It now seeds from the full registered set.

- Corrected the global config directory throughout the docs. Paths were written
  as `~/.pi/agent/` or `~/.hoocode/agent/`; the real directory is `~/.hoocode`,
  with no `agent/` segment. Every global path — `settings.json`,
  `keybindings.json`, `auth.json`, `sessions/`, `skills/`, `extensions/`,
  `themes/` — pointed somewhere that does not exist.

## [0.5.26] - 2026-08-21

### Added

- `/new-canvas` now takes a description, not just a name, and builds what you
  describe. `/new-canvas a kanban board for the release checklist` scaffolds the
  extension, derives and reports a directory name, opens the canvas, and hands
  the agent a brief to build it — you steer it like any other turn. The old
  shapes still work: `/new-canvas my-board` gives you the template to edit by
  hand and starts no build, and `/new-canvas my-board: <description>` lets you
  name it yourself.

- `/canvas reload [extension]` and a `reload_canvas` tool, so a canvas can be
  changed while you are looking at it. Editing an extension's code used to do
  nothing at all while it was open — the running process was forked from the old
  code, so neither the open page nor a newly opened second instance saw the
  change, and the only way to see one was to restart the session. Now the agent
  reloads after each edit and the new capability is immediately callable.

  The new code is forked and asked for its declarations *before* the old process
  is stopped, so an edit that does not run leaves the canvas you are looking at
  exactly as it was and reports the error. Instances keep their ids and the input
  they were opened with, but each gets a **new url** — the extension binds a new
  port and mints a new token on every open — so the previous browser tab is dead
  and the replacement url is printed.

- An arrow-key games canvas, hoocode's first canvas of its own, at
  `.agents/extensions/arrow-key-games/`. Open it with
  `/canvas open arrow-key-games`. Snake and a randomly carved Maze are
  yours alone; **Duel** is a turn-based coin race you play against the model —
  you move with the arrow keys, it moves through `invoke_canvas_action`, and the
  board updates live for both of you. It was built by running
  `/new-canvas create lightweight games that can be played with keyboard arrow keys`
  and iterating with `reload_canvas`, which is what the two entries above are for.

- `/canvas rename <extension> <new-name>` and `/canvas remove <extension>`. A
  canvas's name lives in four places — the directory (which *is* the extension
  id), the canvas's own `id`, its `displayName`, and its header comment — and
  getting the `id` wrong by hand drops the canvas you are looking at on the next
  reload. Rename does all of it at once, closes what was open first, and prints
  every line it rewrote. It only touches a string that is *entirely* the old
  name, so a sentence mentioning the canvas is reported rather than rewritten.
  Remove asks before deleting, refuses when there is no surface to ask on, and
  both refuse a canvas that came from a plugin, pointing at `/plugin` instead.

- Reloading now reports which actions the edit **added, removed or changed**, so
  writing a new action tells you whether the host can see it. Previously a typo
  inside `actions: [...]` failed silently — the action just was not there.
  "Nothing changed" is reported too, since silence reads as success.

- `/canvas list` names the actions of each open canvas. They were visible only to
  the model, so the person steering the session could not see the surface they
  were being asked about.

### Changed

- `/new-canvas` moved from the `/new-*` scaffold family to the canvas surface. It
  is no longer a file-writing command: it opens what it creates and drives the
  agent loop.

- `/new-canvas` derives a much better name from a description. It was naming the
  *request* rather than the thing — `create lightweight games…` became
  `create-lightweight-games`, `help me compare two benchmark runs` became
  `help-compare-two` — because opening words like "create", "build", "show" and
  "help me" landed in the directory name, and `-ing`/`-ed` words crowded out the
  nouns. On twelve realistic descriptions, seven were wrong; all of them now read
  as names for the thing: `lightweight-games-keyboard`, `dashboard-flaky-tests`,
  `compare-benchmark-runs`.

- The scaffolded template names itself once, in `ID` and `NAME` at the top,
  instead of repeating the name in six places. Existing canvases are unaffected —
  rename handles both shapes.

- The build brief now tells the model to leave the canvas's `id` alone and rename
  `displayName` instead. Renaming the id drops the instance the person is
  watching on the next reload — correct behaviour, since the canvas the instance
  was opened against no longer exists, but a trap the model walks into because
  the scaffold names the canvas after a directory derived from a sentence.

## [0.5.25] - 2026-08-20

### Added

- `/new-canvas <name>` — scaffold a canvas extension. Writes
  `.agents/extensions/<name>/extension.mjs` (or `.github/extensions/` with
  `--platform github`), pre-wired with a declared canvas, one agent-callable
  action, and a token-gated loopback server. It runs as scaffolded, so
  `/canvas open <name>` works immediately — no `/reload`, because canvases are
  discovered when `/canvas` runs rather than loaded at session start.

  Scaffolding trusts the workspace, the same grant `/plugin install --scope
  project` makes and for the same reason: a canvas lands in the working tree,
  which is where the trust gate looks, so without it the canvas you just asked
  for would be refused with "came with this repository" seconds after you
  created it. The grant is wider than the one canvas — plugins committed here
  may then run hooks and MCP servers — so it is stated in the output and
  `/plugin untrust` reverses it.

- Plugins can ship canvases, and `/canvas` now finds them. Both real catalog
  layouts are read: the Copilot manifest's `"extensions": "<dir>"` path key
  (`Redth/mobile-canvas-ghcp`) and the Agent Plugins vendor content namespace
  `com.github.copilot/extensions/<id>/` (`github/awesome-copilot`). A plugin
  whose root itself carries `extension.mjs` counts as one canvas named for the
  plugin. Installed canvases are listed and opened exactly like hand-placed
  ones, and a canvas from a project-scoped plugin goes through the same
  workspace-trust gate as one in `.github/extensions/` — it arrives in every
  collaborator's clone the same way.

- Subagents in the `com.github.copilot/agents/` namespace load. This is where
  `github/awesome-copilot` publishes all 125 of its plugin agents; they were
  previously read as nothing.

- Canvases appear in the startup and `/reload` surface — a counted cell, and a
  detail row per canvas carrying either `/canvas open <id>` or the reason it is
  withheld. Installing a canvas plugin used to report "1 plugin" and give no
  sign a canvas existed, which is a large part of why the gap below went
  unnoticed.

### Fixed

- `github/awesome-copilot` plugins install with their content. hoocode cloned
  the repository's default branch, where a `plugins/<name>/` directory holds
  only `plugin.json` and `README.md` — the real content lives in top-level
  trees and is materialized into each plugin directory by CI on the
  `marketplace` branch, which is the branch the vendor's own installer reads.
  Every entry therefore installed as an empty shell and reported that it
  contributed no capabilities. The marketplace is now pinned to that branch,
  and a cache left on the wrong ref is re-cloned rather than kept up to date
  forever. `/plugin install arcade-canvas@awesome-copilot` now lands a working
  canvas and says so.

  The pin names a branch hoocode does not control, so a retired one falls back
  to the default branch with a note saying entries there may be unbuilt, rather
  than dropping the marketplace outright — and a fallback clone stays put
  instead of being discarded and re-downloaded on every run.

- `/plugin list` and `/plugin install` refresh the curated marketplace indices
  before reading them, as `SearchPlugins` already did. The human path never did,
  so a stale — or, after the fix below, a *wrong-ref* — cache could persist
  indefinitely: someone who only ever typed `/plugin install` would have kept
  installing unbuilt stubs with no way to know why. TTL-respecting, so it is
  free when the cache is fresh, and it says so before a fetch that will take a
  moment.

- Starting a new session, resuming one, or forking one shows the same loaded
  resources `/reload` does. The listing was rendered — extensions rebind before
  the transcript is reset — and then wiped a moment later by that reset, so
  every session change but `/reload` reported nothing.

## [0.5.24] - 2026-08-20

### Fixed

- Extension loading no longer depends on the sibling workspace packages having
  been built. `getAliases()` resolved every workspace alias eagerly and threw if
  any one of them was missing, so a single unresolvable specifier took down every
  extension — including extensions that import nothing. The fallback also used
  `import.meta.resolve`, which is undefined once the module goes through a
  bundler or test transform (`__vite_ssr_import_meta__.resolve is not a
  function`). Aliases now resolve via `require.resolve` and an unresolvable one
  is omitted rather than fatal.

## [0.5.23] - 2026-08-19

### Added

- `/canvas` — run GitHub Copilot canvas extensions in hoocode. A canvas is a shared
  interactive surface an agent and a person both drive: the agent mutates state and
  navigates the view, the person reads and edits on the same page. Copilot canvas
  extensions are directories of plain `.mjs` with no dependencies except one
  host-provided import, so satisfying that import runs the community catalog
  unmodified — verified against `pr-artifact-explorer` from `github/awesome-copilot`,
  which opens byte-identical to upstream with nothing written into its directory.

  `/canvas list` shows what is installed, what is open, and what is being withheld;
  `/canvas open <extension>[:<canvas>]` opens one behind a spinner you can cancel with
  escape; `/canvas close <instanceId>` closes it. Extensions are read from
  `.agents/extensions/`, `.github/extensions/`, and `~/.copilot/extensions/`.

  An extension that arrived in the repository is **listed but never run** until the
  workspace is trusted (`/plugin trust`), on the same grounds as a plugin's hooks and
  MCP servers: it is a process that also binds a listening socket. Unlike a plugin it
  has no passive half — even its name comes from running its code — so it is withheld
  whole rather than partially.

  While a canvas is open the agent gets two tools, mirroring the shape Copilot names
  in its own SDK: `list_canvas_capabilities` and `invoke_canvas_action`. There is
  deliberately no tool to *open* a canvas — that forks a process, which stays a
  person's decision. The pair costs ~235 tokens of prompt surface and nothing at all
  in a session that never opens one.

  Canvas support needs a reachable Node 20.6 or newer, which is what `module.register`
  requires: the npm install forks itself, and the standalone binary forks a `node`
  from PATH. Where none is reachable, `/canvas` says so instead of half-working.
  See `docs/canvas-extensions-design.md`.

## [0.5.22] - 2026-08-18

### Fixed

- Dispatching a subagent no longer duplicates rows in the TUI. `beginDispatch`
  wrote its `[DISPATCH]` line straight to the terminal with `console.error`;
  stdout is reserved for the event stream, but stderr is not free either while
  the TUI owns the screen. The renderer is differential and repositions by
  relative row deltas from its own bookkeeping (`hardwareCursorRow`), so a write
  it did not make scrolled the terminal underneath it and every later partial
  repaint landed a row off — painting a second `Agent [explore]` row for a
  single dispatch, wedging fragments of the log line into the transcript, and
  scrambling the frame outright with two or more concurrent spawns. The damage
  persisted until something forced a full repaint, which is why expanding and
  collapsing tool output (`ctrl+O`) appeared to "fix" it.

  Operational log lines now go through a small `agentLog` sink (`core/agent-log.ts`)
  that stays silent while a TUI is attached and is unchanged in every other mode
  (`--print`, RPC, CI). Nothing is lost interactively: the dispatch is already on
  screen as its tool row and task-panel row, every field of the `[DISPATCH]` line
  is persisted to `dispatch-log.json`, and warm/inherited-model fallbacks already
  surface as a `⚠` note on the task row. `HOOCODE_DEBUG_AGENT_LOG=1` tees the
  suppressed lines to `hoocode-debug.log`. The same hazard is closed for the
  `[WARM]` cold-spawn fallbacks, the inherited-model retry notice, and the
  lifeguard's stall report, which could corrupt the frame the same way.

## [0.5.21] - 2026-08-16

### Fixed

- Plugin hooks no longer crash the process when a hook exits without reading its
  stdin. `runHookCommand` wrote the JSON payload inside a `try`, which catches
  only synchronous throws: a hook that exits first (`exit 2`, any script
  ignoring its input) made the write fail asynchronously, and `child.stdin`
  emitted an unhandled `EPIPE`. Intermittently fatal in real sessions, and the
  cause of the `bun-test (coding-agent)` failure on `main` where every test file
  passed but the run still exited 1.

## [0.5.20] - 2026-08-16

## [0.5.19] - 2026-08-16

### Breaking Changes

- The `workflow` category is gone, and `learnMinWorkflowRepeats` with it. It
  mined tool sequences and proposed them as skills, and in practice it proposed
  nothing at all: on a real corpus, 30 candidates were mined and not one ever
  cleared its threshold, because the threshold was three identical
  model-invented names for a tool sequence across separate sessions. Nor is it
  worth reimplementing deterministically — counting tool trigrams over the same
  corpus yields 170 "workflows" led by `bash -> bash -> bash` at 1142
  occurrences. Tool names carry no procedural meaning.
- `learnMinWorkflowRepeats` is replaced by `learnMinRequestRepeats` (default 3),
  which gates the new `request` category below.

### Added

- `/learn` proposes **slash commands** for work you keep asking for by name. The
  miner was explicitly told not to report task requests — and those discarded
  requests were the strongest unexploited signal in the corpus. A directive is
  *how* work should be done; a request repeated across sessions is *what you
  keep asking for*, which is the definition of a slash command. Held to a higher
  bar than directives: a rule stated twice is a rule, but a job asked for twice
  may just be a job that came up twice.
- The digest routes a proposal three ways instead of two — rule, skill, or slash
  command — and names the destination for each, along with what it costs: a
  context file is loaded every turn, a skill's description is always loaded but
  its body on demand, and a slash command costs nothing until it is invoked.
- The digest states the boundary with `ProposePlugin`: skills and commands
  proposed from mined evidence are local habits written under `.agents/`;
  `ProposePlugin` packages something already proven useful into a portable
  artifact. Hooks and MCP servers are never proposed from this evidence, which
  records what was said and what failed — far too weak a warrant for anything
  that executes.

### Changed

- `/learn` names occurrences in one global pass instead of asking the miner to
  name them per session. The miner was told to emit a label — its canonical name
  for what was meant — and the reduce step grouped on exact label equality, but
  a session mined in isolation is being asked to hit a shared vocabulary it has
  never seen. On a real corpus that produced 188 distinct labels from 191
  candidates: `use-bun-not-npm` and `prefer-bun-over-npm` are the same rule and
  never met, and nothing ever reached the repeat threshold. Naming now happens
  once, with every candidate visible at the same time, which is the only
  vantage point from which "is this the same point as that" is answerable.
- Cached candidates no longer carry a label, which makes the mining cache
  model-independent. A label was frozen at mining time, so changing the `fast`
  model tier forked the vocabulary permanently — old sessions and new ones
  naming the same thing differently, with every count split across the seam and
  no sign that it had happened.
- The naming pass is given the labels already on record and told to reuse them.
  State keys are `directive:<label>`, so a label that drifts between runs
  silently breaks suppression and re-proposes everything already decided on.
- A failed naming call falls back to naming each candidate after its own
  wording, which groups identical sentences and nothing else — the behaviour the
  pipeline had before the pass existed. An outage costs recall, not the run.

### Fixed

- `/learn stale` audited nothing when run from a package subdirectory. Context
  files are collected by walking up from the working directory, so in a monorepo
  the repo's `AGENTS.md` sits *above* the package you are in — and the audit
  anchored on the working directory, declared that file "outside this working
  tree", skipped it, and reported a clean run. Running from a package root is
  the normal case, so the check was passing by checking nothing. Referents now
  resolve against the project root.
- A cluster formed by merging two labels carrying the same sentence took its
  name from whichever session came first, and session order changes whenever a
  session is added. The name is the state key, so it drifted between runs, the
  bookmark stopped matching, and items already decided on came back as new. The
  merged name is now a pure function of what was merged.
- The clustering vocabulary is trimmed from both ends rather than by taking a
  prefix. It holds labels already on record followed by names coined earlier in
  the same run, which anchor different things — one keeps the bookmark matching
  across runs, the other stops a split window inventing rival names for one
  point. A prefix dropped the second exactly when a window was large enough to
  be split, which is the only time it mattered.
- Request quotes are flattened to one line and capped. A request is a whole task
  message rather than a sentence — a slash-command body runs to thousands of
  characters — so rendering several raw swamped the digest and a multi-line one
  broke the list it sat in.
- `/learn` no longer mines text the user never typed. A `user`-type slash
  command is persisted as an ordinary user message holding the whole template
  body, with nothing marking it as machinery, so running `/pr` thirty times
  wrote the same two thousand characters into thirty transcripts — making it the
  most repeated "user statement" in a real corpus by a wide margin. Every
  proposal in a recent 100-session run turned out to be `/pr` template text,
  counted as though it had been said aloud. User turns matching a slash-command
  body are now skipped. Detection is retroactive rather than a flag written at
  turn time, so it fixes transcripts that already exist instead of only future
  ones.
- `/learn` no longer reads successful tool output. It was fed 600 characters per
  call, so lines out of plan files and configs were mined as directives and
  attributed to the user — a tenth of them on a real corpus. Tool calls and
  errors are still shown, which is what fix detection actually needs.
- Candidates whose quote cannot be found in what the user said are dropped. The
  miner is told to quote verbatim and the digest renders every quote inside
  quotation marks, but on a real corpus a third of them appeared nowhere in the
  session. A quote that cannot be located is evidence that cannot be shown. This
  also backstops the case above: a command file deleted since a transcript was
  written leaves no fingerprint to match, but its text is still unfindable in
  anything the user said.
- Two labels carrying the same sentence are merged into one proposal, before the
  repeat threshold rather than after. The model labels each session
  independently and cannot see what it called the same thing last time, so
  identical quotes arrived under two labels and were proposed twice, word for
  word — each below the threshold alone.
- The mining cache version is bumped: entries mined before these changes were
  read from a different transcript than the pipeline now produces, so the next
  run re-reads the window once.
- An item is dated by when it was said, not by when its session was opened.
  Suppression compares an item's newest occurrence against the last run, and the
  occurrence was taking the session header's timestamp — so in a session opened
  yesterday and worked in today, something said minutes ago read as older than
  the last run and was held back as "nothing new". It now takes the session's
  last activity.
- The repeat threshold counts distinct sessions rather than total occurrences.
  Saying a thing twice inside one session is the commonest thing in a transcript
  and usually means the opposite of durable — the agent ignored it the first
  time, so it was restated.
- Coverage rules are sent to the judge with their heading path and scope
  (`[repo] Git Rules > - Stage only your own files`) instead of bare lines.
  Headings were stripped and the lines under them sent alone, which asks the
  model to decide whether a proposal is in scope using text with the scope
  removed. Fenced code blocks are dropped: a sample illustrates a rule, it is
  not one.
- A run whose coverage judge failed no longer writes the bookmark. Everything
  reads `new` when that call fails, and recording that as "was not written down
  when shown" made a later run report proposals as passed over that were never
  shown. Skipping the write costs one round of re-proposing.
- An empty run now reports its funnel: occurrences read, distinct points named,
  and how many fell below the repeat threshold. The pipeline filters hard —
  replayed command bodies, tool output, unfindable quotes, then a
  distinct-session bar — and all of it was silent, so "nothing to propose" could
  not be told from "over-filtered" and gave the reader no idea which knob to
  reach for. The below-threshold figure is computed by running the same reduce
  with the threshold at 1, so it is what the threshold cost rather than an
  estimate of it.
- Proposals cut by the per-run cap are reported instead of vanishing. Eight
  shown out of twenty read as "twenty was all there was", which sends the reader
  to the wrong knob.
- Added an evaluation harness for the naming pass (`test/support/learn-eval.ts`
  over a hand-grouped corpus). Every existing test injected a clusterer that
  already agreed with itself, so none of them could fail on the assumption the
  design rests on — which is how a 1.6% collision rate shipped and stayed for
  months. The harness scores merges, false merges, and cluster ratio, and takes
  a real model's clusterer so the assumption can be measured rather than
  assumed.

- `/learn stats` no longer reports an adoption rate, and no longer makes a model
  call. It re-judged coverage and called the delta "adopted", which moved with a
  failed judge at either end and could not tell a proposal correctly rejected as
  junk from one ignored; it shipped with two disclaimers explaining how not to
  misread it. It now reports what has been proposed, and the always-loaded token
  cost of the context files, which is the question the number was reaching for
  and one the filesystem answers exactly.

- `/learn stale` — the subtractive half of `/learn`. The mining path can only
  ever propose additions, so nothing in the command moved the always-loaded
  token surface down: a rule naming a deleted workflow or a command that was
  removed keeps costing tokens on every request, and the agent believes it. The
  audit resolves every backticked path and `bun run` script named by the context
  files in force against the working tree and every package root in it, and
  reports the lines whose referents are gone, priced in tokens. It is
  deterministic — no model call, no cache, no state — so it costs nothing and
  can be run often. On this repo it found four: a documented slash command
  deleted three months ago, and three contribution-gate workflows that do not
  exist.
- Precision comes from exclusions rather than cleverness, because a noisy
  report is one nobody reads: only path-like referents with a separator are
  checked (a bare `stream.test.ts` in a monorepo is under-specified, not stale),
  paths resolve against every package root rather than the repo root alone, and
  lines that assert absence ("was removed", "e.g. `x.ts`") are skipped, since
  they are correct precisely because the file is missing. Findings are presented
  to the model as candidates to verify, not as a licence to delete.

## [0.5.18] - 2026-08-15

### Changed

- Every surface that shows measurable progress now uses one progress bar. The footer (tool
  downloads, the semantic index, `/learn`) and the voice panel each had their
  own, written months apart, and they had drifted three ways: one drew `▰▱` and
  the other `·` over `·`, one said `2.0 MB` and the other `2 MB`, and each
  carried its own copy of the percent-and-detail layout. Bar width is still per
  surface — a footer line shares its row, a panel has room to be finer — but
  nothing else is.
- Progress bars distinguish filled from empty by shape (`▰▱`) rather than by
  colour alone, matching the context gauge beside them. The previous `·`-over-`·`
  fill put the entire reading in the colour, so at a glance, on a low-contrast
  theme, or anywhere styling is dropped, 10% and 90% looked identical. A started
  bar also keeps one filled cell instead of rounding down to empty, so the first
  of forty files reads differently from none of them.
- `/learn` now reads session transcripts with a model instead of pre-filtering
  them with regexes. The old extractor only considered user turns matching a
  whitelist of imperative words, so anything phrased another way — "we're on bun
  now", "that's not how our error handling works", a constraint stated once in
  passing — was not ranked low, it was invisible. Length and line-count caps
  dropped long explanations too, which is where the reasoning behind a rule
  usually is. Every user turn now goes to the model whole.
- Grouping is semantic rather than lexical. The model labels each occurrence
  with what was *meant*, so "we're on bun now" and "stop using npm" count as one
  recurring point instead of two unrelated ones. Counting still happens in code:
  a model asked to count across a long context is approximately right, and the
  count is what the digest is for.
- Coverage — whether a proposal is already written down — is a model judgement
  instead of word overlap. The old 0.6-overlap test both called unrelated rules a
  match (marking a working rule `restated`) and missed real paraphrases that
  picked different vocabulary (proposing a duplicate).
- Mining results are cached per session file, keyed on content hash, so each
  transcript is read once in its life. Counts are still recomputed over every
  session in the window on every run, so caching the expensive step never costs
  the cross-session evidence. A run reports what it read versus reused, and asks
  before reading more than a few new sessions.
- The digest names the mode it ran in, so "nothing new since last time" and
  "nothing here at all" no longer read alike.
- `/learn`'s per-directory memory is discarded once on upgrade. Its keys used to
  be normalized directive text and are now the miner's semantic label, so old
  entries could never match a new proposal — harmless for suppression, but every
  one of them would have counted in `/learn stats` as a proposal that was never
  adopted, holding the rate down permanently. The cost of discarding is one round
  of re-proposing.

### Added

- `/learn` shows a progress bar in the footer while it reads transcripts — the
  same one the semantic index uses — with the count of sessions done, how many
  came from cache, and the reminder that escape stops it. Cached sessions count
  as done: the bar measures progress through the window, so a mostly-cached run
  looks nearly finished from the start, which it is.
- A long backfill can be stopped with escape. Everything read up to that point is
  already cached, so resuming picks up where it left off. A stopped run neither
  shows nor records its proposals: it counted only part of the window, so its
  numbers are low, and bookmarking them would hide those items on the next
  complete run.
- `/learn` reads transcripts with the `fast` model category — the same tier
  subagents already use for bulk reads — rather than the session's model. Set
  `modelCategories.fast` to change it; left unset it is derived from the models
  you have, so nothing here is provider-specific. `/learn settings` names the
  model it resolved and how much it sends per call.
- Transcripts are chunked to fit the reading model's context window instead of a
  fixed 120k characters. Rendering already compresses the two real transcripts in
  this repo from 0.93 MB and 2.26 MB to roughly 47k and 68k tokens, so on a
  200k-token model each is now a single call rather than two and three. Fewer
  boundaries also means fewer blind spots: a failure and the fix that resolved it
  can otherwise land on opposite sides of one.
## [0.5.17] - 2026-08-14

### Fixed

- The `Working...` line no longer strobes, the hardware cursor no longer
  flickers at the end of it, and a long working message is truncated instead of
  wrapping onto a second line. See the tui changelog for details — the same
  fixes cover the compaction, retry, branch-summary and bash-command loaders.

## [0.5.16] - 2026-08-13

### Fixed

- `/learn` no longer reports "no recent sessions in this directory" while a full
  history sits in `~/.hoocode/sessions/`. It searched only the directory the
  live session manager pointed at, which is the right one in the ordinary case
  and wrong in three: an in-memory session (`--no-session`) reports no directory
  at all and was taken literally, `--session <path>` points at wherever that
  file lives, and a custom `sessionDir` setting points at one directory shared
  by every project. The per-cwd session directory is now always searched
  alongside it, and a session reachable from both is counted once.
- `/learn` no longer discards sessions whose recorded working directory is the
  same directory spelled differently — reached through a symlink, in different
  case on the case-insensitive filesystems macOS and Windows ship by default, or
  with a trailing separator. Any of these silently dropped the entire history the
  command exists to read.
- `/learn`'s per-directory memory is keyed on the working directory rather than
  on the live session's directory, so a session with no directory of its own no
  longer lands every project on one shared state file, and projects configured
  to share a `sessionDir` no longer share one bookmark.
- Warnings and errors in the TUI are rendered as one filled block instead of two
  different shapes. A warning previously had neither the blank line nor the left
  padding an error had, so it collided with whatever preceded it and hung off the
  margin, and neither had a background to separate it from ordinary output.

### Added

- `/learn` explains an empty result instead of asserting there is no history. It
  names the directories it searched, how many transcripts they hold, and why each
  was passed over — outside the age window, beyond the session cap, or recorded
  under a different working directory. Those have different fixes, and the single
  sentence it printed before could not tell them apart.
- `/learn settings` — the five thresholds with the values in force, the user and
  project `settings.json` paths to set them in, the session directory being read,
  and the state file. The settings existed but had no discoverable home; every
  message that now reports a disappointing result points at them.
- A **Learning** category in the `/settings` pane holding the same five
  thresholds. Top level rather than folded into Advanced, since these are what
  decide whether `/learn` finds anything and burying them is what made them
  undiscoverable. A value set by hand in `settings.json` that is not one of the
  presets stays in the cycle, so a keypress cannot silently snap the window
  down to the first preset.

## [0.5.15] - 2026-08-13

### Fixed

- A repo-committed plugin no longer stops hoocode from starting. Withholding a
  working-tree plugin's hooks and MCP servers was reported as a load *error*,
  and any error diagnostic exits before the TUI opens — so the notice told you
  to run `/plugin trust` while making the prompt that runs it unreachable. The
  notice is a warning now: the session starts, the plugin's skills and commands
  load, and `/plugin trust` is there to grant the rest.
- An MCP server whose command cannot be spawned no longer crashes the process on
  startup. `ChildProcess` emitted `error` (ENOENT) with no listener attached,
  which is fatal in Node; the failure is now caught and reported per server as
  `command not found`. Windows launches `npx`/`npm`-style commands through the
  shell as well, since those are `.cmd` shims that `spawn()` cannot execute
  directly — the case that produced `spawn npx ENOENT` for `@playwright/mcp`.

## [0.5.14] - 2026-08-13

### Added

- `~/.agents/AGENTS.md` is read as user-scope instructions, alongside the
  existing `~/.hoocode/AGENTS.md`. The two are additive and ordered least
  specific first, so the native home wins on conflict and nobody has to migrate
  an existing file. Closes the long-standing gap in `docs/agent-spec-tree-map.md`
  against the one cross-vendor convention forming at user scope. Override the
  directory with `HOOCODE_USER_AGENTS_DIR`.
- `/learn` — promotes what recent sessions actually taught into durable rules
  and skills. A deterministic extractor reads session transcripts off disk and
  reports three ranked lists with their evidence: directives you have restated
  across sessions, failures where a command failed and later succeeded
  unchanged, and repeated tool sequences. The model then proposes edits to the
  repo `AGENTS.md`, to `~/.agents/AGENTS.md`, or as a new skill. Reading from
  disk rather than the live context is what makes cross-session counts possible
  and what keeps the command working after compaction. Items already covered by
  a rule that you keep restating are flagged as rewrites rather than additions,
  so the file does not only grow.
- `/learn stats` — what became of past proposals, and the closest thing to an
  eval this has without a gold set. No new instrumentation was needed: each
  history entry already records whether the item was written down when it was
  shown, and coverage now is recomputable, so the pair reconstructs adoption.
  Directives only, since fixes and workflows have no coverage signal and
  counting them would mean inventing an outcome. The readout says plainly that
  near-100% means the bar is too low rather than that every proposal was good.
- `/learn` now remembers what it has already shown, per directory, in
  `~/.hoocode/learn/`. An item returns only once it recurs *after* it was last
  surfaced. This fixes a case where a rule accepted from a previous run came
  back flagged `restated` — accusing a working rule of not working, because its
  occurrences were still inside the window — and stops a declined proposal from
  reappearing unchanged every run. Items shown before that are still not written
  down anywhere are marked as previously declined, inferred from what is on disk
  rather than by asking. `/learn all` ignores the memory and re-proposes
  everything. Coverage reads both user scopes and loaded skills as well as the
  repo file, so a proposal routed to `~/.agents/AGENTS.md` or turned into a
  skill counts as adopted either way.
- `/learn`'s two weakest categories were corrected against real transcripts.
  Tool-sequence steps kept only the head of a command, and since nearly every
  agent command is wrapped in `cd <path> && …`, every bash step collapsed to
  `bash:cd <path>` and the sequences were content-free. Occurrences were counted
  per sliding-window position, turning one stretch of edit/test churn into
  "25x". And a sequence only had to be *any* three tool calls, so the digest
  filled with eight rotations of the same edit/test rhythm. Steps now name the
  real command, occurrences are counted without overlap, overlapping variants
  collapse to one representative, and a sequence must chain two distinct
  doing-commands to qualify as a procedure.
- Failure signatures are taken from the error region rather than the whole
  output. Build tools lead with an identical banner, so signing everything made
  unrelated failures of one command collide on their shared preamble and showed
  the reader npm's header instead of the error. Aborted and empty results are
  skipped — an abort is a change of mind, not a problem that was solved.
- A directive already covered by a skill and still being asked for by hand comes
  back marked `has-skill` rather than as a candidate rule, with the advice to
  sharpen the skill's `description` so it triggers — a rule duplicating an
  existing skill is cost with no benefit. The bar for a skill match is set
  higher than for a rule line, since a skill description is a much larger
  haystack and matches a short directive by chance far more easily.
- `learnMaxSessions` (default 20), `learnMaxAgeDays` (default 30),
  `learnMinRepeats` (default 2), `learnMinWorkflowRepeats` (default 3) and
  `learnMaxProposals` (default 8) configure what `/learn` mines and how much it
  proposes. Read per-invocation and project-overridable, so a repo can carry its
  own window; non-positive values fall back to the default rather than silently
  mining nothing.
- Aggregate budget across all loaded context files, on top of the existing
  per-file limits: a warning past ~6k tokens, and trimming of the least specific
  scope first past ~16k. Trimmed files stay visible with a notice rather than
  disappearing. The startup listing now shows the total tokens-per-turn the set
  costs, which the per-file annotation could not surface.

## [0.5.13] - 2026-08-13

## [0.5.12] - 2026-08-12

## [0.5.11] - 2026-08-12

### Added

- Optional `warningBg` theme token: the surface behind a warning notice. Every
  built-in theme sets it to a low-chroma tint of its own warning hue; themes
  that omit it fall back to `customMessageBg`. It joins the other surfaces in
  `test/theme-contrast.test.ts`, so all six accessible themes keep every
  foreground at AAA against it.

### Fixed

- The Anthropic subscription billing caveat had two renderings behind one
  once-per-session latch, and the compact one claimed the latch first. On a
  normal startup you only ever saw the fragment `anthropic sub — billed as
  extra usage`; the sentence explaining the consequence ("not against plan
  limits") only appeared on a quiet startup. There is now one message, and it
  says what it costs.
- The same caveat was resolved with an `await` in front of the startup summary,
  so the splash blocked on an auth lookup that can reach the keychain. It now
  resolves in the background, as it always did on the other code path.

### Changed

- Warnings that cost money if missed render as a `warningBg`-filled block with a
  bold title, instead of one more coloured line in a startup already full of
  them. `showWarning` is unchanged for everything else.
- The startup summary dropped its session state line. Model and thinking level
  were on it and in the footer, which is on screen for the whole session and
  also carries the provider.
- The collapsed details block on the startup summary dropped its `ctrl+o
  details` hint. The banner already shows `ctrl+o more`, and one keypress
  expands both.

## [0.5.10] - 2026-08-12

### Fixed

- `vox-light`'s `ruleMuted` was one point per channel away from `rule`
  (`#8b8263` against `#8a8062`), so `border` and `borderMuted` rendered as the
  same colour and the primary-vs-secondary divider distinction never appeared.
  `rule` also backs `mdCodeBlockBorder`, `mdQuoteBorder` and `mdHr`, so every
  rule in the theme read at one weight. `ruleMuted` is now `#958b6d`, matching
  the separation `vox-dark` already had and staying above 3:1 against the
  theme's cream canvas
  ([#174](https://github.com/kolisachint/hoocode/issues/174))

## [0.5.9] - 2026-08-12

### Fixed

- The session tree selector's highlight stopped wherever the entry text
  stopped, because the row was wrapped in `selectedBg` without being padded
  first — a 17-cell band on a 120-column terminal. It now fills the row, like
  the `/resume` picker beside it.
- The resource picker (`/config`) marked its selected row with bold alone — no
  accent, no highlight — which on a light theme was close to invisible. It now
  carries the same accent and band as every other picker.

### Changed

- **The selected row is highlighted the same way in more of the app.** Until now
  only the `/resume` session picker filled its selected row with `selectedBg`;
  every other picker marked selection with an arrow and accent text alone, and
  the band in `/resume` reached the right edge only because that component
  right-aligns a timestamp column. `getSelectListTheme()` and
  `getSettingsListTheme()` now supply a `selectedRow` background, so the theme
  picker, thinking picker, show-images picker, the `/settings` screens and the
  editor's autocomplete all draw the same full-width highlight. The accent
  styling stays: `accent` clears 4.6:1 against `selectedBg` in every shipped
  theme, while `muted` and `dim` fall to 2.8:1 and 1.9:1 on `dark`, so the band
  is added to the existing marker rather than replacing it. The hand-rolled
  pickers now do the same, via `SelectedRowList` (below).
- **Every picker marks its selected row with the same cursor.** `→`, `›` and
  `>` were all in use — four glyphs counting the one `SelectList` hardcoded —
  so which one you saw depended on which list you had opened. They all use `›`
  now, from a single `SELECT_CURSOR`. In the ask-for-input pane this also stops
  the row cursor colliding with the `>` that prompts its custom-answer field.
- Unselected rows are indented by `SELECT_GUTTER`, derived from the cursor's
  visible width, rather than by a hardcoded two columns. The two agreed before
  only because every cursor in use happened to be two columns wide.
- The model, scoped models, oauth and extension pickers build their rows
  through the new `SelectedRowList` component instead of adding a `Text` child
  per row. Those rows were assembled in an update method that never sees the
  terminal width — width only arrives later, when the container renders each
  child — so there was no point at which a row could be padded before being
  painted. `SelectedRowList` holds the rows and renders them lazily, which puts
  the width back in reach.
- The fork-from-message picker highlights the message line the cursor is on
  (its metadata line stays unpainted) and colors it with `accent`, which it
  previously left to bold alone.
- The ask-for-input pane keeps its plain rows: it is a question form with a
  live text field, not a scrolling list, and a band under an input caret reads
  as a rendering artifact rather than a selection.
- The `/resume` session picker and the session tree selector paint their
  selected row through a shared `paintSelectedRow` helper rather than each
  calling `theme.bg` on a row they had padded (or not padded) themselves.
  `/resume` no longer depends on its right-aligned timestamp column to reach
  the right edge.

## [0.5.8] - 2026-08-12

### Changed

- **Capability listings share one layout.** `/plugin list`,
  `/plugin marketplace list`, `SearchPlugins` and `ListPlugins` each built their
  own `name [a, b] — description` line, so the same plugin printed three ways
  and the model's view never matched the user's. They now go through a single
  row formatter (`core/format-list.ts`, `core/extensions/plugins/listing.ts`):
  aligned name column, facts beside it, description wrapped with a hanging
  indent instead of restarting at column 0, and plugins grouped by the
  marketplace that offers them. `/plugin list` marks what is already installed.
- `SearchPlugins` takes a `limit` (default 10). With no query it previously
  returned every plugin in every registered marketplace — dozens of entries,
  several lines each — into the chat and the context.
- The five plugin tools define `renderResult`, as every other built-in tool
  already did. Results are bounded to 20 lines with an expand hint rather than
  dumped whole.
- `ListPlugins` reports the manifest format its description already promised,
  counts `themes` and `providers` as capabilities (a plugin shipping only a
  theme reported none), and carries the plugin description through.
- `[Agents]` in the startup summary is one line per agent, truncated to the
  terminal, instead of a 200-character summary wrapping into a paragraph.
- `brand.ts` moved to `core/` so non-interactive surfaces can label capability
  classes; added a `marketplaces` glyph.

### Fixed

- Status messages carrying their own colors are no longer wrapped in a blanket
  dim. `theme.fg` closes with `\x1b[39m` (reset, not restore), so the dim died
  at the first inner span and — because the wrapper carries that state across
  newlines — every line after it lost the dim too.
- `InstallPlugin` no longer erases its own announcement. It notified "installing
  X because Y", then notified the outcome; consecutive info notifications
  coalesce into one chat line, so the second replaced the first and the
  transparency the tool guidelines require never reached the screen.
- Truncation in the new listings does not emit escape codes. The TUI's
  `truncateToWidth` appends `\x1b[0m` even for plain input — a full reset that
  would both leak into model-facing text and clear any enclosing style mid-line.

### Added

- `ExtensionUIContext.columns` exposes the terminal width to extensions, so a
  listing can wrap to the actual terminal. Optional, and undefined outside a
  terminal (RPC, print), which is the signal to emit unwrapped text rather than
  wrap to someone else's width.

## [0.5.7] - 2026-08-12

### Added

- **`vox-dark` and `vox-light` themes** — explanatory-journalism styling: one
  loud yellow used sparingly against a newsprint-quiet palette. The yellow is
  reserved for four roles (`accent`, `mdHeading`, `selectedBg`, `borderAccent`)
  and never carries a status, so `warning` is orange rather than the usual
  yellow — a yellow `◐` or context gauge stops reading as a signal when yellow
  is already the accent. Both are AA, not AAA, and are not covered by
  `test/theme-contrast.test.ts`, but they are held to the same standards the
  default `light` theme gained in 0.5.6: every text token clears 4.5:1 on every
  surface it paints, rules and inactive chrome clear 2.8:1, and no two tokens
  that mean different things sit closer than ΔE 11. Rules stay neutral rather
  than saturated — a separator carries no meaning through hue — which is the one
  deliberate departure, documented in `docs/themes.md`.
- Optional `brandBg` / `brandText` theme tokens, honored only as a pair, paint
  the footer's brand mark as a filled chip instead of accent-colored text.
  Themes that omit them are unchanged. Added for light themes, where a brand
  hue vivid enough to be recognizable is rarely legible as text on a light
  canvas. `vox-light` is the only built-in that sets them.

## [0.5.6] - 2026-08-12

### Changed

- **The default `light` theme now uses saturated, deep hues instead of pastels.**
  Its colors were pale enough to blur into the grays around them on a light
  terminal: `success` and `bashMode` were a 20%-saturation sage, `error` a dusty
  rose, `thinkingHigh` and `mcp` near-gray mauves, and almost every token sat at
  4.0–4.9:1 against the page. Every hue is now 65%+ saturated, every foreground
  clears WCAG AA (4.5:1) on all six message/tool surfaces plus the export
  canvases, and rules and inactive chrome (`borderMuted`, `mdHr`,
  `thinkingOff`) went from a near-invisible 2.0:1 to 3.0:1. The tool-state
  backgrounds are tinted far enough apart to tell pending, success, and error
  apart at a glance, and unlanguaged code blocks render as dark ink rather than
  green so syntax highlighting carries the color. `test/theme-contrast.test.ts`
  enforces both the contrast floor and the saturation floor.
- **The six accessible themes no longer paint different things the same color.**
  They all cleared AAA, but contrast says nothing about whether two tokens read
  as the *same* color, and crushing a palette dark enough to clear 7:1 on white
  is exactly what collapses distinct hues together. Measured with CIEDE2000,
  `mcp` was byte-identical to an agent color in three themes — an MCP row and a
  subagent row in the same task panel drew their tags in one color — and the two
  quietest thinking levels were 2.0-2.6 apart, below the threshold where they
  read as different at all. Every group a user reads against itself (core UI,
  the agent palette, syntax, thinking levels, diff) now holds ΔE 11 or better,
  up from 0.0 at worst.
- **`colorsafe-light` now delivers what its name promises.** It picked
  color-blind-safe hues, but those palettes lean on lightness differences and
  darkening every token for AAA removed exactly that. Simulated, `error` and
  `warning` were ΔE 2.8 apart under deuteranopia, `syntaxFunction` and
  `syntaxNumber` were identical, and an added diff line was ΔE 3.1 from
  unchanged context. Its palette is rebuilt on blue/violet/magenta, where every
  semantic pair now holds ΔE 12 or better under all three dichromat
  simulations. The theme's `vars` are renamed to describe the colors they
  actually hold, and its description no longer claims Okabe-Ito hues.

### Known limitations

- `colorsafe-dark` still shares one color between `mcp` and `agent2`. Six agent
  identities already spend the color-blind-safe hues available at AAA on a dark
  ground, and every remaining candidate degenerates to near-white, which reads
  as plain text. Tracked in `theme-contrast.test.ts` rather than papered over.
- In 256-color terminals (Apple Terminal, GNU screen), `warm-light` draws
  `accent` and `warning` with the same palette index. The 6x6x6 cube has almost
  no resolution below channel value 135, so any palette dark enough for AAA on
  white loses distinctions there. The default `light` theme is checked against
  this and stays clear; the AAA themes cannot be.

## [0.5.5] - 2026-08-10

### Added

- **Six built-in themes for low vision**, three light and three dark:
  `high-contrast-dark` / `high-contrast-light` (maximum contrast),
  `warm-dark` / `warm-light` (low-glare warm palettes), and
  `colorsafe-dark` / `colorsafe-light` (Okabe-Ito hues, no red/green pairs).
  Every token in all six clears WCAG AAA (7:1) against every surface the TUI
  paints — page, selected row, user message, and the three tool-box states —
  and none of them fall back to the terminal's default foreground, so contrast
  does not depend on the terminal's own palette. `test/theme-contrast.test.ts`
  enforces the floor.
- Themes may declare an optional `description`, shown next to the name in the
  `/settings` theme picker. All built-in themes now carry one.
- `test/test-theme-colors.ts theme <name>` previews any theme by name, not just
  `light` and `dark`.

### Changed

- Built-in themes are discovered by scanning the shipped theme directory instead
  of loading `dark.json` and `light.json` by name, and are validated against the
  theme schema on load.

### Fixed

- HTML export picked its fallback text color by checking whether the theme was
  literally named `light`, so any other light-background theme exported dark text
  on a light card. It now derives the choice from the theme's own background
  luminance.
- The theme file watcher treated only `dark` and `light` as built-in, so it would
  try to watch the custom-themes directory for any other shipped theme.

## [0.5.4] - 2026-08-09

## [0.5.3] - 2026-08-09

### Fixed

- **The webtools request timeout never reached the binary.** `webtools.timeoutSecs`
  and `HOOCODE_WEBTOOLS_TIMEOUT` were resolved and clamped, then used only to size
  the subprocess kill timer — `--timeout` was never passed, so every request ran on
  the binary's own default regardless of the setting. It is now forwarded. The kill
  timer also accounts for the binary's whole-run budget (a fetch is bounded at three
  times `--timeout`, a search may try a fallback provider), so hoocode no longer kills
  a request the binary would have completed.
- **A blocked search was reported as an unexplained exit code.** `webtools search`
  signals a bot challenge or rate limit by exiting non-zero with its JSON on stdout
  and nothing on stderr, which surfaced as `webtools search exited with code 1`. The
  status is now read back off stdout and reported as a blocked search.
- **A JavaScript-rendered page was indistinguishable from an empty one.** `webfetch`
  now reads the `status` field the binary reports (`ok` / `empty` / `needs_js` /
  `too_complex`) and appends a one-line note when extraction produced nothing, so an
  HTML shell that needs a browser is no longer read as a page with nothing to say.
- **webtools could never be downloaded on ARM Linux.** The asset name was built as
  `aarch64-unknown-linux-gnu`, but the upstream release matrix publishes aarch64
  Linux as a musl build, so the download 404'd on every arm64 Linux host.

### Changed

- `websearch` reports which backend answered (`provider`), so a silent fallback from
  a keyed provider to DuckDuckGo is visible, and its description no longer claims
  DuckDuckGo is the only backend — Brave, Tavily and SearXNG are used when configured.

## [0.5.2] - 2026-08-08

## [0.5.1] - 2026-08-08

## [0.5.0] - 2026-08-08

## [0.4.168] - 2026-08-08

## [0.4.167] - 2026-08-08

## [0.4.166] - 2026-08-08

## [0.4.165] - 2026-08-08

## [0.4.164] - 2026-08-08

### Removed

- **Browser tools and document tools are gone.** `browser_run`/`browser_continue`
  and `DocRead`/`DocEdit`/`DocWrite`/`DocScan`/`DocGrep`/`DocPeek` have been
  removed along with the `--enable-browsertools`, `--enable-browser-live-preview`
  and `--enable-filetools` flags, the `enableBrowserTools`,
  `enableBrowserLivePreview` and `enableFileTools` settings, the `browsertools`
  and `filetools` managed-binary downloads, and their entries in the interactive
  tool-group picker. Both groups were off by default, so sessions that never
  enabled them are unaffected; sessions that did will now see the tools absent.
  `read` still short-circuits on OOXML/PDF rather than dumping binary, but now
  says to extract via `bash` instead of pointing at `DocRead`.

### Changed

- Trimmed ~402 tokens off the fixed per-turn surface (~8.8%) by removing text
  that was being sent twice. The `edit`, `grep`, and `read` tools each restated,
  in `promptGuidelines`, rules already carried by their own parameter schemas or
  by the canonical search-vs-grep and file-exploration guidelines that
  `buildSystemPrompt` emits — so both copies shipped on every request. Also
  compressed the `find` and `ls` schemas (`find` was the single most expensive
  built-in schema at 1,154 chars) and dropped the duplicated half of `bash`'s
  prompt snippet. No capability change: every rule that was removed is still
  stated exactly once. Verify with `--print-token-surface`.

## [0.4.163] - 2026-08-07

## [0.4.162] - 2026-08-07

## [0.4.161] - 2026-08-07

## [0.4.160] - 2026-08-07

## [0.4.159] - 2026-08-07

## [0.4.158] - 2026-08-07

### Added

- The input editor is now a bordered box by default, replacing the two horizontal
  rules. The rules were ambiguous at more than one line: a wrapped prompt read as
  loose text between two dividers, with nothing tying the continuation lines to the
  input. Side borders make the field's extent explicit, and the scroll indicators
  now close (`┌─── ↑ 3 more ───┐`) instead of reading as section rules. The box
  costs two columns of text width, so `editorBorder: "rule"` (also under
  `/settings` → Interface → Editor border) restores the previous rendering.
  `editorPaddingX` now defaults to `1` to match.
- The input prompt is now `❯` (U+276F) instead of `>`. Inside the box the ASCII
  `>` read as a quote marker rather than a prompt; the ornament is unambiguous,
  is one cell wide in every terminal (it is not East Asian ambiguous and has no
  emoji presentation, so it cannot desync the box borders), and is the glyph most
  shell prompts already use. Bash mode keeps its green `!`.

- Each request now ends with its own cost in the transcript (`↑3.9k ↓450 · 1.1s ·
  $0.015 · ◇1 explore 12.4k`). This is where a retrospective number belongs: the
  task panel is a live instrument that resets on the next message, and the footer
  carries cumulative session vitals, so neither could answer what a given request
  cost after the fact. Delegated runs are reported separately because subagents
  bill against their own sessions.

### Changed

- The task panel header is now a lens tab strip on the left, replacing the state
  stamp, progress bar and turn delta on the right. Each tab carries its own
  `done/total`, so unfinished work in the lens you are *not* viewing stays visible
  (`tasks 1/3  subagents 0/2`), and the selected tab fills only while its lens has
  live work. With a single lens there is nothing to switch to and the header
  disappears entirely — a plain TodoWrite session is now just its rows. The cycle
  hint reads from the configured keybinding rather than a hardcoded key, and the
  tab strip now survives narrowing instead of being the first thing dropped.
- An in-progress task renders the static `◐` its glyph table specifies. The panel
  used to animate a braille spinner that duplicated the transcript loader's own
  spinner — same frames, independent timer, permanently out of phase — and
  re-rendered the whole component tree eight times a second. The only motion left
  is a 1s run clock on delegated rows, and it runs only while one is live.
- Rows no longer restate their glyph in words (`queued`, `running…`, `cancelled`).

### Fixed

- TodoWrite rows no longer sit at `in_progress` after the request that owned them
  has ended. The model marks its own plan items and routinely drops the final call
  that completes the last one, so the panel kept claiming live work until the next
  user message wiped it. Dangling items now settle when the agent goes idle: a
  clean stop means the model believed it was finished, so they settle to done; an
  abort, error or length cutoff settles them to cancelled (`⊘`) rather than
  fabricating a completion. Only main-plan items are touched, and nothing settles
  while a subagent is still running or a follow-up message is queued.
- A live TodoWrite row no longer shows an elapsed time. Every item in a plan shares
  a `createdAt` (the tool creates the whole list in one batch), so the timer was
  reporting the age of the *plan*, not of the task — item #3 included all the time
  spent on #1 and #2. Delegated rows keep their clock, where `createdAt` is the
  dispatch time and the number is real.
- The footer and the new per-request cost line share one usage sum, so they can no
  longer disagree about what a session has spent.

## [0.4.157] - 2026-08-07

### Changed

- The startup page is one column and states the session instead of narrating it.
  Everything on it now sits on a single left rail under the banner, and the model,
  thinking level, and Anthropic subscription-billing caveat collapse into one
  state line (`claude-sonnet-4-5 · thinking high · anthropic sub — billed as extra
  usage`) rather than three separate status/warning lines that the footer already
  carries for the rest of the session.
- MCP servers are reported as a capability, not as boot chatter. A successful
  connect no longer prints a line above the summary; live servers get an `⧉` cell
  in the counted grid and a `[MCP]` details row with tool count, background mode,
  and schema deferral. Failures and authorization prompts still surface inline,
  since those need action. The count now covers every source (`mcp.json`,
  per-server files, plugins) instead of only plugin-contributed servers.
- The agents cell counts dispatchable subagents from the agent registry (and is
  hidden when the Task tool is off). It previously counted only agents contributed
  by package manifests, so a session with five usable agents could show none.
- Context files are priced where they are listed: an oversized `AGENTS.md` is
  annotated inline (`~3.6k tokens · consider trimming`) instead of repeating the
  file on a separate warning line. Size is now structural data on the loaded file
  (`tokens`, `size`), and `warnings` carries only real failures.
- The banner tagline is `coding agent`.

### Removed

- `hoo-core` no longer appears in the loaded-resources summary or details. It is
  built-in plumbing that is always present, so listing it as "1 extension" said
  nothing about the session.

## [0.4.156] - 2026-08-07

### Added

- `/goal` — works autonomously toward a completion condition. `/goal <objective>`
  takes the objective directly; bare `/goal` reads it from the plan's Goal
  section, and the plan's Verification section becomes the completion condition
  either way, which is the difference between a run that stops when it feels
  finished and one that stops when something it can execute says so. Accepts
  `--max-turns N` for the turn budget. Deliberately linear: a single thread of
  execution that iterates until it reports done or exhausts its budget, with no
  task graph and no subtask fan-out. The run inherits whatever the active mode
  already permits (`enabled_tools`, `allowed_bash_commands`,
  `allowed_write_paths`, `auto_allow`), so how much rope an unattended goal gets
  stays a mode-config decision.
- `/grill` — stress-tests the current plan before `/approve` executes it. Two
  phases: `/grill me` surfaces the request's unconfirmed assumptions as
  `ask_options` questions, and `/grill plan` attacks the plan file itself,
  verifying its claims against the codebase and revising it where it is weak.
  Bare `/grill` runs both, in that order: underspecification sits upstream of
  plan weakness, so critiquing a plan built on a misread request only produces a
  well-reviewed plan for the wrong job — the user's answers are folded into the
  critique instead. Unlike `/approve` the command only injects a follow-up
  message, so there is no session switch, no mode change, and no config write.
  While a `/loop auto` run is active nobody is present to answer a question, so
  the interrogation phase is dropped and the plan critique runs on its own.

### Changed

- Plan mode now points at the whole path out of a plan rather than only
  `/approve`: it offers `/grill` to stress-test first, then `/approve` to execute
  step by step or `/goal` to work toward the plan autonomously, and recommends
  grilling whenever the plan carries real risk. Previously the two new commands
  were reachable only if you already knew to type them.
- The autonomous loop can now be started over the event bus. `LOOP_AUTO_START`
  mirrors the existing `LOOP_HALT` channel in the opposite direction, letting an
  extension drive a run without reaching into state private to `/loop`, and its
  payload carries an optional `continuePrompt` so a caller with a concrete
  completion condition can restate it on every continuation instead of relying
  on the generic nudge. `/loop auto` now shares that same starter; its behaviour
  is unchanged.

## [0.4.155] - 2026-07-29

## [0.4.154] - 2026-07-23

### Added

- Configurable voice trailing-silence window. The pause length tolerated before
  voice capture auto-stops was a hardcoded 800 ms; it's now the `voice.silenceMs`
  setting (default 800, clamped 300–10000) with a `VOICETOOLS_SILENCE_MS` env
  override (env wins, like `VOICETOOLS_BIN`) and a picker under Settings →
  Advanced. The effective value is resolved once and drives both the binary
  cutoff (`voicetools serve --silence-ms`) and the on-screen countdown, so the
  two stay in sync; changing it live drops any idle warm daemon so the next
  capture respawns with the matching cutoff. Someone who pauses to think can
  lengthen the window — up to 10 s — without editing code.
- Configurable web-tools request timeout. The `webfetch`/`websearch` binary
  timeout was a hardcoded 15 s; it's now the `webtools.timeoutSecs` setting
  (default 15, clamped 1–120) with precedence settings → `HOOCODE_WEBTOOLS_TIMEOUT`
  env → default (mirroring the TLS resolver, where the setting wins over env),
  and a picker under Settings → Advanced. The effective value is resolved once
  next to the TLS config in each tool factory and threaded into `runWebtools`
  (whose spawn headroom already scales with it).

## [0.4.153] - 2026-07-23

### Added

- Startup progress bars for external tool binaries. First-run downloads of `fd`
  and `ripgrep`, and the semantic-search index (its binary download **and** the
  index build), now render a determinate progress bar as a transient line in
  the footer — `received MB / total MB` with a percent, matching the voice
  download bar — with an indeterminate byte count when the server sends no
  `Content-Length`. The bars are fed by `ensureTool`'s existing byte-count
  callback, run in the background so a slow first-run download never blocks
  first paint, and each clears as its work settles. They live in a dedicated
  `startupProgress` store, never as task-panel plan rows.

### Changed

- The startup semantic-search index no longer surfaces as a task-panel row. It
  was created via `taskStore.create()` — indistinguishable from a main-agent
  `TodoWrite` plan item — so its finished row lingered above the prompt for the
  whole first turn. It now reports through the shared `startupProgress` footer
  channel alongside the tool-download bars, dropped on `ready`/`skipped` and
  shown as a transient notice on failure. The non-interactive stderr log path
  is unchanged.
- Renamed the user-facing semantic-index startup messages from `embsearch`
  jargon to plain language (e.g. `Building semantic search index –
  {done}/{total} files ({pct}%)`, `Semantic search index ready ({chunkCount}
  chunks)`, `... skipped`/`unavailable ({reason})`). Flags, settings, and
  module paths are unchanged.
- The interactive startup preload no longer fetches the semantic-index binary:
  when the index is enabled, `EmbsearchService` already downloads and builds it
  through the same progress channel, so preloading it separately only fetched
  the binary twice and raced.

## [0.4.152] - 2026-07-23

## [0.4.151] - 2026-07-23

## [0.4.150] - 2026-07-22

### Added

- At-call-time read de-duplication (gated on `contextGc.enabled`, the same
  setting as the post-hoc GC). When the model issues a `read` whose requested
  line range is already fully covered by an earlier, still-live read in the
  session, the tool returns a short pointer instead of re-fetching the file.
  This complements `evictSupersededReads` (which stubs superseded reads on the
  way out) by preventing the redundant re-fetch up front. The guard is
  deliberately conservative: a cap-truncated earlier read only covers what it
  actually delivered, a whole-file read must have been delivered untruncated to
  count, reads before a compaction boundary (summarized out of the live
  context) are ignored, and any read superseded by a later edit/write or
  overlapping read — predicted with the same declared-range test the GC uses —
  is never treated as covering. Its pointer results are excluded from the GC's
  supersession bookkeeping so they never stub the read they point at.

### Changed

- Raised the default `toolOutput.maxBytes` cap from 16KB to 32KB (and the
  matching `DEFAULT_MAX_BYTES` constant). Dense prose files (e.g. a ~17KB
  markdown doc) now load in a single `read` instead of being truncated
  mid-file by the byte cap and paged back in via 4+ overlapping
  offset-reads. The 800-line cap is unchanged, and the value remains
  configurable via the `toolOutput` setting.

## [0.4.149] - 2026-07-22

## [0.4.148] - 2026-07-21

### Fixed

- Context GC no longer evicts a `read` result just because the same file was read again over a **different** line range. Eviction is now range-aware: a read is superseded only by a later read whose `offset`/`limit` window overlaps it (or by any later edit/write). This fixes a read loop where alternating reads of two disjoint regions of one large file would repeatedly stub each other, and the "Re-read the file if you need its current contents" hint would drive the model to re-read indefinitely.

## [0.4.147] - 2026-07-20

### Changed

- Surfaced the `search` tool more prominently in the default system prompt: it is now listed first in file-exploration guidance, its description no longer frames it as "grep-backed", and its guidelines encourage starting with search for discovery while reserving `grep` for exact-line/regex enumeration.

## [0.4.146] - 2026-07-19

### Changed

- The `search` tool is now **always available** by default, not gated behind
  `--enable-search-tool` / `enableEmbsearchTools`. It answers "find where X
  lives" with ranked results and degrades to grep-backed lexical retrieval
  when no semantic index is present, so it needs no binary to be useful. The
  `enableEmbsearchTools` flag/setting now controls only the **semantic index
  layer**: when on, semantic hits from the embsearch index are fused into
  search results; when off, search runs lexical-only. `grep`/`find` are
  unchanged — `grep` still owns exact/regex matching, call sites, and counts.
- **Semantic search is now ON by default** (`enableEmbsearchTools` defaults to
  `true`), and the repo size threshold is removed (`embsearchThresholdBytes`
  defaults to `0`, so every repo is indexed regardless of size). On session
  start the embsearch binary indexes the repo in the background; search runs
  lexical-only until the index is ready, then fuses semantic hits. If the
  embsearch binary is unavailable (offline, unsupported platform), search
  degrades to lexical-only without error. Set `enableEmbsearchTools` to
  `false` to opt out, or raise `embsearchThresholdBytes` to skip small repos.

## [0.4.145] - 2026-07-18

## [0.4.144] - 2026-07-18

## [0.4.143] - 2026-07-18

### Added

- Hybrid code retrieval (docs/hybrid-retrieval-design.md): the new unified
  `search` tool replaces `semantic_search`, with `mode: auto | lexical |
  semantic | hybrid`. Hybrid mode runs grep-backed lexical retrieval and the
  local embedding index in parallel and fuses results with Reciprocal Rank
  Fusion (rank-only, deterministic tie-breaks; `k=2` per the eval gate below). Lexical hits are
  mapped to indexed chunk ids (with `rel#L<line>` fallbacks for uncovered
  files), collapsed, and re-ranked before fusion. Results are expanded into
  line-window snippets under a token budget; full per-call diagnostics go to
  `search-trace.jsonl` in the embsearch store dir, never into model context.
- `--enable-search-tool` CLI flag (alias of the legacy
  `--enable-embsearchtools`; same `enableEmbsearchTools` setting).
- Retrieval eval gate (`scripts/search-eval.mjs` + a repo-grounded gold set
  in `test/fixtures/search-eval.json`): Recall@5/10/50 per mode with an RRF
  `k` sweep, gold answers matched by span overlap. Lexical baseline recorded
  in docs/hybrid-retrieval-design.md.
- Lexical retrieval quality: unindexed grep hits now coalesce into per-file
  clusters (no more near-duplicate windows of the same file), candidates are
  ranked by distinct query terms matched instead of directory order, one
  file contributes at most 8 candidates, and ripgrep runs with `--sort path`
  so capped result sets are deterministic across runs. Search traces rotate
  at 5 MB.
- Deterministic reranker over the fused top-50 (on by default): term
  coverage of each candidate's expanded window, path affinity with an
  exact-path bonus (a query naming a file now ranks that file first), and
  the fused order as prior. Measured on the eval gate with a full index:
  the default search path went from R@5 42% / R@10 58% to R@5 63% /
  R@10 83% (Recall@50 92%).
- Eval-driven defaults: RRF `k` is now 2 (beat the folklore k=60 on every
  differing gold query), only the top 20 lexical candidates vote in hybrid
  fusion, and `auto` no longer routes path-like queries to lexical (they
  score 0% lexically, 100% in hybrid).

### Changed

- When the embedding index is unavailable (disabled, under threshold, missing
  binary, or still building) the `search` tool degrades to lexical retrieval
  with a notice instead of erroring like the old `semantic_search` tool.
- `grep`'s no-match hint now points at the `search` tool.

## [0.4.142] - 2026-07-18

## [0.4.141] - 2026-07-18

### Added

- Semantic code search behind the `--enable-embsearchtools` flag and the
  `enableEmbsearchTools` setting (default off). When enabled, repositories over
  `embsearchThresholdBytes` (default 10 MB) are chunked and embedded in the
  background by the `embsearch` binary; results are persisted to
  `~/.hoocode/embsearch/<hash>` for reuse across sessions. A new
  `semantic_search` tool returns ranked `path:start-end` hits. `grep` and
  `find` behavior is unchanged for small repos or when the feature is off.
- `embsearchBinaryPath` and `embsearchThresholdBytes` settings for controlling
  the optional embedding engine.

### Changed

- `grep` now appends a one-line hint to `No matches found` when a semantic
  index is available, pointing the model at `semantic_search` without altering
  any match output.

## [0.4.140] - 2026-07-17

## [0.4.139] - 2026-07-17

### Fixed

- GitHub Copilot plugin authoring (`ProposePlugin` / `UpdatePlugin` with
  `--support-platform github`) now emits custom agents as
  `agents/<name>.agent.md` with a YAML-list `tools` frontmatter, matching the
  current GitHub Copilot plugin spec (docs.github.com custom agents /
  plugins-creating; verified 2026-07) so Copilot recognizes them. Previously
  they were written as bare `agents/<name>.md` with a Claude-style comma-string
  `tools`, which Copilot does not load as agents. The reader still accepts the
  bare `.md` layout (and the legacy `.github/chatmodes/` fallback), so
  previously authored plugins keep loading.
- GitHub Copilot plugin authoring now maps a plugin's commands to Copilot
  prompt files — `.github/prompts/<name>.prompt.md` — instead of Claude-style
  `commands/<name>.md`, matching the Copilot/VS Code prompt-file convention and
  this adapter's workspace command layout. The reader loads them from
  `.github/prompts/` (and still honors a manifest `commands` path or a
  `commands/` dir). Relatedly, command-name derivation now strips a `.prompt.md`
  suffix, so a `greet.prompt.md` prompt file loads as the command `greet`
  (previously `greet.prompt`).

## [0.4.138] - 2026-07-17

### Changed

- Authored plugins are now portable-by-default. `ProposePlugin` and
  `UpdatePlugin` write one vendor-neutral native (`.agents-plugin`) artifact
  instead of forking into Claude + Copilot layouts, and no longer expose a
  per-call `platforms` parameter. Vendor layouts remain available only through
  the `--support-platform` session flag (an opt-in interop choice), which is
  now authoritative — the model cannot override it. Both tools' descriptions
  and guidelines now steer toward self-contained, reusable, vendor-neutral
  content.

## [0.4.137] - 2026-07-16

### Added

- `--light` flag (and `light` setting): a minimal, low-token preset for
  small/local models. Restricts the session to exactly the read/write/edit/bash
  tools with shortened descriptions and stripped parameter schemas (search
  happens via bash instead of grep/find/ls), replaces the system prompt with a
  terse three-line prompt, and disables subagents, TodoWrite, skills, context
  files, plugin tools, and the hoo-core mode-prompt appendix. The fixed
  per-turn surface (system prompt + serialized tool schemas) measures ~293
  tokens, down from ~2000+ in full mode. Explicit flags such as `--tools` and
  `--system-prompt` still win over the preset. Inspect the surface of any
  session with the new `--print-token-surface` flag.

## [0.4.136] - 2026-07-16

### Added

- Completion chime: interactive mode can now ring the terminal bell when an
  assistant turn finishes after you have likely stepped away — specifically when
  a turn runs longer than 10s (measured from the turn's start to when the agent
  goes truly idle), or when the agent blocks awaiting your input (the
  `ask_options` pane). The cue is a single BEL byte, so it is output-only with no
  new dependencies, and it is debounced so rapid turns do not spam it. Off by
  default; enable via the `terminal.chimeOnTurnComplete` setting.

## [0.4.135] - 2026-07-16

## [0.4.134] - 2026-07-16

## [0.4.133] - 2026-07-14

### Changed

- Plugin authoring is now a single risk-gated `ProposePlugin` tool, replacing
  the `ProposePlugin` / `ProposeExecutablePlugin` split. The human-confirmation
  gate is computed from the draft's content — hooks, MCP servers, or a
  mutating-subagent allowlist trigger it; passive skills/commands/read-only
  subagents author autonomously — instead of being pre-declared by tool choice,
  so a mixed passive+executable plugin authors in one call and executable
  content can never ride in through a "passive" path. Authored plugins now
  carry a `.authored.json` provenance marker at their root.

### Added

- `UpdatePlugin` tool: merge inline-authored capabilities into an existing
  locally authored plugin — skills/commands/subagents are added or replaced by
  name, hooks and MCP servers are unioned with what's on disk. Additive-only
  and no remote fetch; executable additions require the same human confirmation
  as authoring, and marketplace-installed plugins are refused (they don't carry
  the authored provenance marker and don't round-trip losslessly through the
  authoring emitters).
- `RemovePluginCapability` tool: remove named capabilities from a locally
  authored plugin — skills/commands/subagents/MCP servers by name, hooks by
  event (narrowed by matcher/command). The subtractive half of `UpdatePlugin`;
  runs autonomously since removal is the low-risk direction (deleting
  capabilities cannot execute code). Also the supported way to *change* a hook
  (hooks have no name to replace by): remove the old one, then add the new one
  via `UpdatePlugin`.
- `ListPlugins` accepts an optional `id` parameter to look up a single
  installed plugin.

## [0.4.132] - 2026-07-14

## [0.4.131] - 2026-07-14

## [0.4.130] - 2026-07-14

### Fixed

- The Copilot adapter now reads every plugin manifest and marketplace location
  accepted by the official Copilot CLI plugin reference, keeping
  `.github/plugin/` as the preferred home (matching the real-world plugins
  indexed by github/copilot-plugins). Manifests are probed as
  `.github/plugin/plugin.json`, root `plugin.json`, `.plugin/plugin.json`, then
  legacy `.github/copilot-plugin.json`; marketplace indexes as
  `.github/plugin/marketplace.json`, legacy `.github/marketplace.json`, root
  `marketplace.json`, then `.plugin/marketplace.json` — previously only the
  `.github/` locations were read. Copilot plugins with root `hooks.json` (CLI
  convention) now load, manifest `author` is emitted as an object
  (`{ "name": ... }`) per both vendors' schemas, and `metadata.pluginRoot`
  (shared by the Claude Code and Copilot CLI marketplace schemas) is applied to
  relative plugin sources.

### Added

- `--support-platform <list>` CLI flag (and `supportPlatform` setting): pick
  which vendor layout(s) hoocode targets when it **writes** artifacts. Tokens:
  `claude`, `copilot` (aliases `github`, `gh`), `agents` (alias `native`);
  comma-separated and/or repeated. Applies to authored plugins
  (ProposePlugin / ProposeExecutablePlugin — overrides the claude+github
  default target set) and to the `/new-skill` `/new-agent` `/new-command`
  scaffolds, which then land in each platform's workspace conventions instead
  of `.hoocode/`: Copilot gets `.github/skills/<name>/SKILL.md`,
  `.github/agents/<name>.agent.md` (frontmatter `tools` as a YAML list, per the
  current custom-agents spec), and `.github/prompts/<name>.prompt.md`; Claude
  gets `.claude/skills|agents|commands/`; native gets the `.agents/`
  equivalents. Implemented as a per-adapter `WorkspaceLayout` on the plugin
  format registry, so each vendor's conventions stay a one-file concern
  (`formats/<vendor>.ts`) and new platforms plug in without touching callers.

## [0.4.129] - 2026-07-14

## [0.4.128] - 2026-07-14

## [0.4.127] - 2026-07-14

### Added

- `--enable-plugintools` CLI flag: toggle the autonomous plugin system
  (`enablePluginTools`) per session without editing settings.json, matching
  the existing `--enable-webtools` / `--enable-filetools` pattern.

## [0.4.126] - 2026-07-14

## [0.4.125] - 2026-07-13

### Added

- Runtime "plugin reuse nudge": a reactive extension
  (`extensions/core/prompt-reactive`) that watches tool output and turn text for
  reusability cues (e.g. "active voice", "avoid repetition", "prefer JSON", an
  explicit capability gap) and attaches a matching, plugin-facing note to the
  next turn via the ephemeral `context` hook — instead of relying only on the
  static plugin guidance folded into the system prompt once at session build.
  The cue → nudge table in `prompt-reactive/policy.ts` is the single source of
  truth; armed nudges are also surfaced to `SearchPlugins` so a reusability
  signal reaches the plugin layer even when no tool asked for it. Conservative
  by design: one note per turn, at most once per category per session, never
  blocking. Wired once from hoo-core via a static import (Bun-bundle-safe) with
  an idempotency guard so composing default extensions twice is harmless.

### Changed

- `enablePluginTools` is now the master switch for the **whole autonomous plugin
  system** — the plugin lifecycle tools (SearchPlugins, InstallPlugin, …),
  ProposePlugin, and the new runtime reuse nudge — and **defaults to off**. Set
  it to `true` in settings.json to opt in; both the tool surface and the nudge
  flip together.

## [0.4.124] - 2026-07-13

### Fixed

- Compiled binary: the standalone (Bun `--compile`) build now bundles the
  hoo-core built-in extension, restoring `/loop`, `/plugin`, `/mode`, `/cost`,
  the scaffold commands, and the MCP loader (remote `type: "http"`/`sse`
  servers plus the OAuth browser flow) in the packaged executable. hoo-core
  was only referenced from the node entry (`bin/hoocode.js`) via a dynamic
  `import()` the compiler could not follow, so it was silently dropped from
  the binary. The built-in factory list now lives in one place — a
  `DEFAULT_EXTENSION_FACTORIES` default in `main()` reached by a static import
  — so both the node CLI and the compiled binary load it identically, while
  callers that pass their own `extensionFactories` (downstream embedders) are
  unaffected.

## [0.4.123] - 2026-07-13

## [0.4.122] - 2026-07-13

## [0.4.121] - 2026-07-13

## [0.4.120] - 2026-07-13

### Added

- Remote MCP servers: the MCP loader now speaks Streamable HTTP
  (`{ "type": "http", "url": ..., "headers": ... }`) and the legacy SSE
  transport (`"type": "sse"`) in addition to stdio, across every config
  source — standard `mcp.json` files, `~/.hoocode/mcp-servers/*.json`, and
  plugin `.mcp.json` registrations in both the Claude (`mcpServers`) and
  Copilot / VS Code (`servers`) shapes. This closes the 0.4.119 known
  limitation: remote plugin servers (e.g. workiq from the Copilot directory,
  or Atlassian Rovo) are connected instead of skipped. Built on the official
  `@modelcontextprotocol/sdk` client transports; a `type: "http"` endpoint
  that rejects streamable HTTP with a 4xx automatically falls back to the
  legacy SSE transport.
- MCP OAuth: remote servers that demand authorization get the full MCP auth
  flow — RFC 9728/8414 discovery, dynamic client registration, browser-based
  authorization code + PKCE via a loopback redirect listener, and automatic
  token refresh — with per-server-URL state persisted under
  `~/.hoocode/mcp-auth/` (0600). When interactive sign-in is needed the
  session keeps starting; the server's tools connect and register as soon as
  the browser flow completes. The headless `loadMcpTools` in
  `@kolisachint/hoocode-agent-core` accepts the same remote entries plus
  `McpRemoteOptions` (storage dir, browser opener, auth callbacks), and the
  transport/provider are exported as `connectHttpMcpServer` /
  `McpFileOAuthProvider`.

## [0.4.119] - 2026-07-13

### Added

- **Single-turn capability loop**: `InstallPlugin` and `ProposePlugin` now
  activate the plugin in the live session — skills, slash commands, and
  subagents are usable on the model's very next request, in the same turn, with
  no `/reload`. Plugins bundling executable capabilities (hooks, MCP servers)
  trigger an automatic reload when the turn ends. `UninstallPlugin` schedules
  the same idle reload so cleanup is autonomous too.
- Mid-run context refresh: tool or system-prompt changes made while a run is
  streaming (deferred MCP schema resolution via `ResolveMcpTools`, live plugin
  activation) now reach the next provider request within the same run, via the
  agent loop's `prepareNextTurn` hook. Previously the loop's context was frozen
  at run start, so `ResolveMcpTools`-resolved tools were not callable until the
  next user prompt.
- Well-known marketplaces: the official Claude plugins directory
  (`anthropics/claude-plugins-official`, 250+ plugins) is registered as a
  curated, trusted marketplace out of the box. Its index is cloned lazily into
  `.agents/marketplace-cache/` on first `SearchPlugins` call (offline degrades
  gracefully) and is never auto-updated.
- Reusability sensing: system-prompt guidance now nudges the model to
  `SearchPlugins` before hand-rolling a missing capability, and to author a
  reusable recipe as a plugin with `ProposePlugin` autonomously when no
  marketplace plugin covers it.

- Real-world GitHub Copilot plugin support: the Copilot adapter now reads the
  convention established by `github/copilot-plugins` — plugin manifests at
  `.github/plugin/plugin.json` over a Claude-mirror capability tree, marketplace
  indices at `.github/plugin/marketplace.json`, manifest dir-path overrides
  (`"skills": "./skills/"`), and the `{ "source": "github", "repo", "path" }`
  source shorthand. The legacy authored layout (`.github/copilot-plugin.json` +
  prompts/chatmodes) still parses. `github/copilot-plugins` joins the well-known
  trusted marketplaces (searchable with `platform: "github"`).
- Manifest-less marketplace plugins (bare capability trees, e.g.
  copilot-plugins' `spark`) now install: a native manifest is synthesized from
  the marketplace entry so the standard loader carries them.
- Hooks + bundled scripts verified end to end through the install path: script
  exec bits survive install, the `{ description, hooks }` `hooks.json` wrapper
  parses, `${CLAUDE_PLUGIN_ROOT}` resolves to the installed root, event JSON
  arrives on stdin, and exit-code 2 blocks the tool call.

### Changed

- `deferMcpSchemas` now defaults to **on**: MCP tool schemas are deferred
  (names only in context, resolved on demand via `ResolveMcpTools`), cutting
  the context cost of MCP-heavy plugins. Set `deferMcpSchemas: false` to
  restore eager schema registration.
- `ProposePlugin`'s Copilot output now follows the real-world convention: one
  shared capability tree plus a `.github/plugin/plugin.json` marker manifest
  (previously `.github/copilot-plugin.json` + `.prompt.md`/`.chatmode.md`
  files).

### Known limitations

- Remote MCP servers (`{ "type": "http", "url": ... }` in a plugin's
  `.mcp.json`, e.g. workiq) are not yet supported — the MCP loader is
  stdio-only; such servers are skipped at plugin load.

## [0.4.118] - 2026-07-12

## [0.4.117] - 2026-07-12

### Fixed

- Fix `SearchPlugins`, `SuggestPluginInstall`, and `InstallPlugin` crashing with
  `source.trim is not a function` when a marketplace contains structured source
  objects (Claude and GitHub Copilot marketplace formats). The parser now
  normalizes `url` and `git-subdir` source objects, and installation supports
  full-repo and subdirectory git clones with optional `ref`/`sha`.

## [0.4.116] - 2026-07-12

### Removed

- Remove `amazon-bedrock`, `cloudflare-workers-ai`, `cloudflare-ai-gateway`, and `mistral` provider entries from model resolver, display names, SDK headers, and login controller.

## [0.4.115] - 2026-07-12

## [0.4.114] - 2026-07-12

### Changed

- **Merged the `glob` tool into `find`.** The two shipped side by side in the
  default tool set and did near-identical fd-backed work, adding selection
  ambiguity and duplicate schema tokens on every request. `find` — the canonical
  tool (the Claude `Glob`/`Find` alias target, a member of the agent tool
  allowlist, and the tool with typed extension events) — now absorbs `glob`'s
  capabilities: `pattern` accepts an array for OR logic, plus optional `exclude`,
  `type` (files/dirs/symlinks), `depth`, and `compress`. Existing single-pattern
  `find` calls are unchanged (flat output, fd parse errors still surface). The
  `glob` tool is removed; Claude Code's `Glob` continues to normalize to `find`.

## [0.4.113] - 2026-07-07

### Performance

- **Streaming long messages no longer re-parses the whole text per tick.**
  While streaming, assistant text and thinking blocks over 2KB are segmented
  at stable markdown block boundaries (never inside fences, loose lists,
  tables, or indented continuations; disabled entirely when link-reference
  definitions are present), so each update re-lexes only the growing tail —
  measured 34x faster over a 13KB streamed message. The final render collapses
  back to one canonical Markdown, so any segmentation artifact is transient by
  construction; equivalence tests assert segmented and single renders match.
- **Subagent stdout no longer floods the parent event loop.** A spawned
  subagent now filters its JSON stdout to the events the parent actually
  consumes (progress + `message_end` usage), dropping the per-delta
  `message_update` / `tool_execution_update` firehose at the source. Under
  concurrent subagents this removes hundreds of main-thread `JSON.parse` calls
  per second — a major cause of overall TUI lag while delegating. The top-level
  `--json` stream is unchanged (still emits every event).
- **Bounded transcript memory for long sessions.** Finished tool blocks that
  scroll far out of view (beyond a live window) are frozen: their rendered lines
  are kept while the heavy source payloads — full tool output, duplicate base64
  image copies, per-renderer state, child component caches — are released. The
  session data stays intact, so a theme toggle or reload restores full fidelity;
  frozen blocks are re-truncated to the terminal width on render so a resize can
  never overflow.

## [0.4.112] - 2026-07-04

### Changed

- The subagent tool is now **enabled by default** (`enableSubagent` defaults to
  `true`), so the root session gets the `Task` and `TaskOutput` tools without
  `--enable-subagents`. Disable per session with the new `--no-subagents` flag or
  set `enableSubagent: false`.
- Default subagent nesting depth raised to **2** (`maxSubagentDepth`), so a
  spawned subagent may itself delegate one more level (depth-2 grandchildren
  still cannot). Was 1 (no nesting). Override with `--max-subagent-depth` or the
  setting.
- Trimmed always-on subagent prompt tokens (~370–420 fewer per turn in a default
  setup): the `Task` tool description is cut to mechanics (the when-to-use /
  when-not guidance lived there **and** in the system-prompt block — now only the
  block carries it, ~150–200 tok), and the background/barrier guidance in that
  block is compressed from three verbose bullets to two tight ones (~220 tok),
  collapsing to a single concise line when the project has no background-capable
  agents.

- Plugin loading now prefers the cross-vendor `.agents/` surface first: plugins
  are discovered from `.agents/plugins/` ahead of `.hoocode/plugins/` (project
  before global, first-wins by id), `/plugin install` writes to
  `.agents/plugins/<name>`, the added-marketplace registry lives at
  `.agents/marketplaces.json` (falling back to the legacy `.hoocode/` path), and
  `/plugin remove` deletes from both. `.hoocode/plugins/` stays discovered.
- `/loop` scheduled tasks now persist to `.agents/scheduled_tasks.json`; a legacy
  `.hoocode/scheduled_tasks.json` is read once and migrates forward on the next
  persist.

### Added

- Native `.agents-plugin/marketplace.json` marketplace index format (preferred
  over Claude `.claude-plugin/` and GitHub `.github/marketplace.json`).
- `docs/plugin-format-mapping.md`: reference mapping of the native, Claude, and
  GitHub/Copilot plugin & marketplace formats, plus the `.agents/`-first
  packaging/install/storage/loading rules.
- Optional `supportPlatform` field on marketplace manifests (top-level and per
  plugin entry). When a repo carries conflicting index formats (e.g. both
  `.github/marketplace.json` and `.claude-plugin/marketplace.json`), the parse
  result now records every platform present in `NormalizedMarketplace.supportPlatform`
  instead of silently dropping the others; precedence still selects one `format`.
  The field is optional and informational — omitting it changes nothing. Tokens
  are `agents` | `claude` | `github` (aliases `copilot`/`gh` → `github`,
  `native` → `agents`); `/plugin marketplace list` surfaces multi-platform repos.

## [0.4.111] - 2026-07-04

### Added

- Agent identity colors: each subagent type hashes to a stable hue from six new
  theme tokens (`agent1`-`agent6`, optional in custom themes with an `accent`
  fallback), applied consistently to the chat's `Agent [type]` line, task-panel
  row tags/glyphs and roster names, and TaskOutput's call line and roster.
- The task panel's flat ("tasks") lens now nests each dispatched subagent run
  under the TodoWrite item it was dispatched for (recorded when exactly one
  item is in_progress), with tree connectors, live activity, and a per-run
  timer — the plan and the agents executing it read as one picture.
- Each running task row shows its own live elapsed timer next to its activity.
- User-initiated cancellation now propagates to subagents: aborting a turn
  kills the dispatched run's whole process tree, queued runs settle
  immediately, and the run reports a distinct `cancelled` status (dim ⊘ in the
  panel/TaskOutput) instead of a red failure.
- Team-focus keys are configurable (`app.team.nudge`, `app.team.attach`;
  defaults `n`/`a`), and the attached-panel's nudge key now honors the same
  binding instead of a hardcoded `n`. Both panels' hint lines use the shared
  dim-key/muted-description hint style and reflect the configured keys.

### Changed

- Task-panel roster rows are keyed per dispatch (pool task id, labeled
  `explore#1`) instead of per agent type, so concurrent same-type subagents no
  longer share one row with colliding state/activity/stats.
- The panel header's elapsed is the wall-clock span of the visible batch, not
  the sum of per-task spans (which ticked at 2x with two concurrent subagents).
- The panel and TaskOutput share one duration format.
- `turn_end` now reads "thinking" in the task panel, matching TaskOutput.
- TodoWrite reconciles the incoming list against existing items by content
  identity first (position only as a fallback), so reordering or shrinking the
  plan keeps task ids — and the subagent runs linked to them — attached to the
  same items; the panel still renders the plan in list order.

### Fixed

- Glyph rendering: the warning cue (⚠), team-focus cursor (▶), and
  team-attach pause/resume markers (⏸/▶) now carry the text-presentation
  selector (VS15) so terminals with emoji font fallback render them as
  single-cell text instead of double-width emoji that misaligned their rows;
  the voice panel's mic carries VS16 so its measured width matches the
  two-cell emoji terminals draw. The task panel's pending marker is now a
  hollow ○ (matching the selectors' ○/◉ convention), leaving ● exclusive to
  the chat's tool status dot.
- Concurrent subagents no longer trample each other's panel state; a stale
  warning note (⚠) clears on the next state change; subagents-lens header
  counts always match the rendered rows (orphaned children render as roots,
  `parentTaskId` cycles cannot hang the walk); running tasks show advancing
  elapsed time instead of freezing at ~0s.
- Subagent reliability: children spawn detached and are killed by process
  group/tree so grandchildren cannot be orphaned; the lifeguard emits one
  stalled event per reap instead of one per tick; retries keep the cumulative
  token budget and its listeners; stdout readers/streams are cleaned up on
  failure paths too; `result.json`/`output.json` are written atomically so a
  mid-write SIGKILL cannot turn a finished run into a torn-file failure; the
  child's stdout is parsed by a single UTF-8-safe line reader with bounded
  buffers (the token budget no longer runs a second chunk parser that could
  split multi-byte characters).

## [0.4.110] - 2026-07-03

## [0.4.109] - 2026-07-03

## [0.4.108] - 2026-07-03

## [0.4.107] - 2026-07-02

## [0.4.106] - 2026-07-01

### Changed

- Voice-to-text (`ctrl+r`) now allows a longer thinking pause before a capture
  auto-stops. The trailing-silence window is raised from 600ms to 3s and is now
  passed through to `voicetools serve` via `--silence-ms`, so the real cutoff
  matches the on-screen countdown (previously the hoocode-side value only drove
  the cosmetic countdown while the binary used its own 600ms default).
- Voice daemon idle shutdown: after 60s with no capture, the warm `voicetools
  serve` process exits automatically, releasing the ~900 MB resident ASR model
  from memory. The next `ctrl+r` pays a cold-start respawn cost. Controlled
  by `VOICE_IDLE_TIMEOUT_MS` (default 60,000; 0 disables).

## [0.4.105] - 2026-07-01

### Added

- Voice-to-text (`ctrl+r`) now uses `voicetools serve` when available: the
  first press loads the model once and keeps it warm for the rest of the
  session, so later presses skip the cold start and jump straight into
  listening. Binaries that don't support `serve` fall back to the previous
  per-press `transcribe` behavior automatically.
- Live voice-input panel with words-as-you-speak. With a streaming
  `voicetools serve` (v0.1.4+), the multi-line panel shows the transcript
  growing word by word as you talk (`PARTIAL`), alongside a mic glyph, an
  elapsed timer, a scrolling waveform driven by `LEVEL` events, and a
  shrinking "cutting off in Ns" countdown when trailing silence begins. The
  finished utterance (`FINAL`) is committed to the editor in one piece and
  the panel collapses. Older non-streaming binaries just show a spinner for
  the batch phases; the committed text still lands the same way.

## [0.4.104] - 2026-07-01

### Fixed

- Voice-to-text (`ctrl+r`) no longer fails with `spawn voicetools ENOENT` when
  the binary is not preinstalled. `voicetools` is now a managed tool that is
  auto-downloaded from the `kolisachint/voicetools` GitHub release on demand
  (like `webtools`/`filetools`), resolving from `VOICETOOLS_BIN`, then
  `~/.hoocode/bin/voicetools`, then `PATH`, then download. A missing platform
  asset degrades to a clear error message instead of a raw spawn failure.

## [0.4.103] - 2026-07-01

### Added

- Voice-to-text input: press `ctrl+r` in the editor to record from the mic and
  stream transcribed text into the input via the external `voicetools` binary.
  Press again to cancel. The binary is resolved from `VOICETOOLS_BIN`, then
  `~/.hoocode/bin/voicetools`, then `PATH`.

## [0.4.102] - 2026-07-01

## [0.4.101] - 2026-06-28

### Breaking Changes

- Renamed the browser automation tools for a clearer, parallel start/continue
  pair: `browser_flow` -> `browser_run` and `browser_resume` -> `browser_continue`.
  The old tool names are removed (no aliases); update any flows, scripts, or
  `tools` allowlists that referenced them. The `--enable-browsertools` flag and
  the underlying browsertools serve RPC (`flow_start`/`flow_resume`) are
  unchanged.

### Fixed

- `browser_run` no longer starts a second live viewer or auto-opens another OS
  browser tab when it reuses the parked-idle serve client across calls. The
  live-view URL is cached on the serve client; a reused client reports
  "Live view already open at: <url>" instead of re-issuing `live_view_start`.

## [0.4.100] - 2026-06-28

### Changed

- `browser_flow`/`browser_resume` now render a `NeedsParent` suspension as a compact,
  readable block (request kind, goal/expected_state/fields, page title/url, named
  controls with stable selectors, and a one-line headings summary) instead of
  pretty-printing the entire `ParentRequest` JSON. On busy pages the raw observation
  dump was thousands of mostly-empty entries straight into the model context; the
  screenshot already conveys the page, so only the actionable text layer is kept,
  cutting the per-suspension output by an order of magnitude.

## [0.4.99] - 2026-06-28

### Fixed

- `browser_flow` now reuses a single `browsertools serve` process (one Chromium
  instance and one live-view port) across successive `browser_flow`/`browser_resume`
  calls instead of spawning a new process, browser window, and live-view port on
  every call. The serve client is parked as idle on flow completion and reclaimed
  by the next call when the browser config (headful/browserPath) matches; it is
  disposed on config change, idle timeout, or session shutdown.
- `browser_flow` inline-flow validation: the `flow` parameter now documents the
  full flow/action schema (including that `extract_semantic.fields` must be a
  string array, not a map, and takes no `goal`), and `invalid inline flow` errors
  from the binary are enriched with the action-schema hint so the model corrects a
  malformed flow in one turn instead of guessing across several rounds.

## [0.4.98] - 2026-06-28

### Changed

- Expanded the `browser_flow` and `browser_resume` tool descriptions to document the
  agentic screenshot loop: build flows from `decide`/`extract_semantic` steps that
  suspend with a screenshot, answer with `browser_resume`, and keep looping until the
  outcome is `complete` rather than falling back to `webfetch` to read page content.
  Added the per-kind `ParentResponse` shapes, the `next_action` Action schema, and
  guidance to always pass a `fallbacks` selector array for click/fill.

## [0.4.97] - 2026-06-28

### Added

- `--enable-browser-live-preview` flag (and the `enableBrowserLivePreview` setting)
  defaults the streamed live viewer on for `browser_flow` runs and auto-opens it in
  your default browser. The viewer streams the page plus the agent's tool-call log
  over a local WebSocket. Set `HOOCODE_BROWSERTOOLS_NO_OPEN=1` to print the URL
  without opening (CI/SSH). Per-call `live_view`/`headful` params on `browser_flow`
  override the default; `headful` launches a real on-screen Chromium window
  (requires `BROWSERTOOLS_HEADFUL` support in the browsertools binary).
- `HOOCODE_BROWSERTOOLS_BINARY` env override points tool resolution at a locally
  built `browsertools` binary, bypassing the tools-dir/PATH download (useful for
  testing unreleased binary changes). Generalized as `HOOCODE_<TOOL>_BINARY`.

## [0.4.96] - 2026-06-28

### Fixed

- `--enable-browsertools` (and the `enableBrowserTools` setting) had no effect:
  the flag was parsed and stored but never forwarded into session creation in
  `main.ts`, so the `browser_flow`/`browser_resume` tools were never added to the
  active tool set. Wired `enableBrowserTools` through the
  `createAgentSessionFromServices` call alongside `enableWebTools`/`enableFileTools`.
- The `grep` tool now hints at the `literal: true` option when ripgrep rejects the
  pattern with a regex parse error, instead of surfacing only the raw ripgrep error.

## [0.4.95] - 2026-06-27

## [0.4.94] - 2026-06-27

### Changed

- The document discovery loop (`DocScan`/`DocGrep`/`DocPeek`) now reaches cell and
  text content for **all** supported formats, including xlsx cell values and pptx
  slide text. This picks up the upstream `filetools` `v0.1.7` fix ("reach full
  cell/text content via scan/grep/read for all formats"), which closes the
  xlsx-only gap tracked in [#78](https://github.com/kolisachint/hoocode/issues/78)
  where `DocGrep`/`DocPeek` previously surfaced sheet structure only. The
  `DocGrep`/`DocPeek` prompt guidelines that steered spreadsheet cell work to
  `DocRead`/`DocEdit` are dropped, and the coverage matrix in
  `docs/doc-tools-flow.md` / `docs/doc-tools-scoping-design.md` is updated
  (xlsx and pptx now ✅ across the loop). Re-verified against the v0.1.7 binary on
  hand-built xlsx and pptx fixtures. `filetools` is resolved as the latest release,
  so no version pin change is needed.

## [0.4.93] - 2026-06-27

### Added

- Document discovery tools `DocScan`, `DocGrep`, and `DocPeek` (off by default;
  enabled with `--enable-filetools` alongside `DocRead`/`DocEdit`/`DocWrite`).
  They wire up the `filetools` binary's token-sensitive loop so large
  structured/binary documents can be navigated without a full `DocRead`:
  `DocScan` returns a paginated manifest of block previews (structural-path
  ids), `DocGrep` locates blocks by literal text and returns the editable `el_`
  node ids for a direct `DocEdit`, and `DocPeek` hydrates specific blocks by
  their `DocScan` path id (or pages through all with offset/limit). All three
  are read-only and print JSON the agent renders in the same id-addressed
  dialect as `DocRead`. Verified working for XML, docx, and PDF; for xlsx the
  loop surfaces sheet structure only (cell values still go through
  `DocRead`/`DocEdit`). A token-cost benchmark (`test/filetools-token-cost.test.ts`)
  measures the loop at ~4× cheaper than a full `DocRead` on a large document.

### Changed

- Clarified the recommended ordering for the document tools in their prompt
  guidance: scan first with `DocRead readonly:true` (cheap, analysis-only), do a
  full writable `DocRead` only when about to edit (it carries the id-map and is
  token-heavy), then `DocEdit`/`DocWrite` with minimal patches — and do not
  re-run `DocRead` between edits, since the edit tools re-extract on their own.
  Documented the flow in `docs/doc-tools-flow.md`.

## [0.4.92] - 2026-06-26

### Changed

- `DocEdit`/`DocWrite` no longer hard-fail when there is no prior `DocRead` or when
  the document changed on disk (e.g. a script rewrote the binary). They now
  re-extract automatically, validate the patch's node ids against the current
  extract, and only fail when the patch targets ids that no longer exist — in which
  case the error includes the current id-addressed structure so the agent can
  re-issue the patch without a separate `DocRead`. `DocEdit` results now include the
  list of nodes affected by the patch (`details.affected`).

## [0.4.91] - 2026-06-26

## [0.4.90] - 2026-06-26

## [0.4.89] - 2026-06-26

### Added

- Document tools `DocRead`, `DocEdit`, and `DocWrite` (off by default; enable with
  `--enable-filetools` or the `enableFileTools` setting). They shell out to the
  `filetools` binary (resolved from PATH or downloaded from GitHub releases) to
  losslessly project structured/binary documents (XML, drawio, docx/xlsx/pptx,
  PDF) into editable, id-addressed JSON: `DocRead` extracts a document to an
  id-addressed envelope, `DocEdit` applies an id-based RFC-6902 patch in place and
  re-extracts, and `DocWrite` reconstructs a patched document to a new path,
  leaving the source untouched. `read` now redirects to `DocRead` when given an
  OOXML or PDF file instead of dumping binary bytes.

## [0.4.88] - 2026-06-24

## [0.4.87] - 2026-06-24

## [0.4.86] - 2026-06-24

## [0.4.85] - 2026-06-23

### Added

- The `webfetch`/`websearch` tools can now run behind a TLS-intercepting proxy by
  forwarding a CA bundle to the `webtools` binary: set `HOOCODE_WEBTOOLS_CA_CERT`
  to a readable PEM file and it is passed through as `--ca-cert`. An unreadable or
  missing path is warned about once and ignored (not forwarded). As a strictly
  opt-in last resort, `HOOCODE_WEBTOOLS_INSECURE=1` forwards `--insecure` to
  disable the binary's TLS verification, warning once per run while active. Both
  can also be supplied programmatically via the tool factory options (e.g. from
  settings.json), which take precedence over the environment. This is the
  webtools-binary counterpart to hoocode's own app-level CA trust, which does not
  reach that separate binary.

- App-level TLS CA trust so hoocode's own outbound traffic (provider API calls,
  the GitHub API, and on-demand tool downloads) works behind corporate
  TLS-intercepting proxies **with certificate verification kept on** — replacing
  the insecure `NODE_TLS_REJECT_UNAUTHORIZED=0` workaround. Trust is additive to
  Node's bundled roots and fails closed (a missing/unreadable CA warns once and
  is skipped; there is no trust-all or trust-on-first-use). A custom PEM bundle is
  trusted via `--ca-cert <path>` (or `HOOCODE_CA_CERT` / `NODE_EXTRA_CA_CERTS`, in
  that precedence), and the OS/system trust store is trusted only when opted in
  with `--use-system-ca` / `HOOCODE_USE_SYSTEM_CA=1`. The resolved CA set is
  installed on the global HTTPS agent and threaded into the undici dispatcher. If
  `NODE_TLS_REJECT_UNAUTHORIZED=0` is set, hoocode now warns once on startup. This
  does not cover the `webfetch`/`websearch` tools (separate `webtools` binary).

### Changed

- `webfetch` and `websearch` now run as normal foreground (blocking) tools.
  They were previously dispatched in the background (non-blocking); reverting to
  foreground execution means the agent waits for the result inline instead of
  continuing to reason while the fetch/search runs.

### Fixed

- Hardened on-demand tool downloads (`tools-manager`) so a failed or truncated
  transfer can no longer leave a corrupt partial archive — or a broken binary —
  in place (the root cause of `webtools` silently never installing). Downloads
  now go to a unique temp path (`<asset>.<pid>.<rand>.part`), are validated, then
  atomically renamed to the final archive; the shared archive path is never
  written directly. `downloadFile()` captures `Content-Length` and asserts the
  bytes written match it (throwing on a short/truncated transfer), and does a
  best-effort SHA-256 check against `<downloadUrl>.sha256` (verifying on HTTP 200,
  skipping on 404). The download + verify step retries once (2 attempts total),
  and the temp archive plus temp extract directory are now cleaned up on any
  failure, not just extraction errors. `ensureTool()` still degrades to
  `undefined` on ultimate failure.

## [0.4.84] - 2026-06-23

## [0.4.83] - 2026-06-22

## [0.4.82] - 2026-06-22

## [0.4.81] - 2026-06-22

### Added

- **Web tools: `webfetch` and `websearch`** (off by default). Enable with
  `--enable-webtools` or the `enableWebTools` setting. Both shell out to the
  `webtools` CLI (auto-downloaded from `kolisachint/webtools` releases, or used
  from PATH), which returns token-efficient, reference-style output (`[N]`
  markers plus a trailing reference block). `websearch` uses DuckDuckGo and needs
  no API key. Results are cached in-process for 15 minutes. Both tools go through
  the permission gate (interactive prompt) and can be restricted with a
  `.webtoolsignore` file (gitignore syntax) that blocks hosts for `webfetch` and
  filters blocked domains out of `websearch` results. SSRF/private-address
  protection is enforced by the `webtools` binary.

## [0.4.80] - 2026-06-22

## [0.4.79] - 2026-06-22

### Changed

- **Reverted the `Task` → `ExecuteTask` rename from 0.4.78.** The subagent
  delegation tool is `Task` again (no deprecated alias), keeping parity with
  Claude Code. The never-wired `item_id` parameter and the duplicate `complexity`
  field on TodoWrite (both added in 0.4.78) are removed.
- **Background subagents are now notify-and-pull** instead of forced-background.
  A background `Task` posts a compact one-line notification and retains the body
  in a new subagent inbox; the model pulls the full result with `TaskOutput`.
  Background is opt-in per agent with a per-call `background: true|false`
  override. This keeps a wide swarm of subagents from flooding the parent's
  context with N full summaries.
- **`TaskOutput` reworked into a status-aware probe + swarm barrier** (replacing
  0.4.78's `wait_for_completion` approach). It never errors on a valid handle: a
  running task reports its status/activity, a finished one returns its body, an
  already-read one says so. New modes: `list: true` lists every background
  subagent, and `wait: true` blocks until a named task — or, with no `task_id`,
  all outstanding subagents — finish. Tasks are addressable by a friendly label
  (`explore#1`) or their task id.
- **`Task` keeps the optional `complexity` tier** (`"fast"`/`"standard"`/
  `"capable"`), now passed straight through as the dispatch model so the pool's
  precedence applies it only when the agent's model is `inherit`; a pinned-model
  agent ignores it.
- **Model categories are provider-neutral**: the tiers resolve only from
  `settings.modelCategories` with no hardcoded fallback, and an unconfigured tier
  is a no-op (keep the agent's or parent's default model). Built-in agent
  templates select by category (`explore: fast`, `general-purpose`/`plan:
  standard`).
- **Lower per-turn prompt cost**: the available-agents roster renders once (was
  up to three times — the `<available_agents>` block, the buildTaskMainPrompt
  appendix, and the Task tool description) and as a one-line summary per agent
  instead of the full description; the Task tool description no longer re-embeds
  the roster or re-explains parameters covered by their schemas.
- The on-spawn placeholder is a single compact line instead of a multi-line
  explainer, so dispatching several subagents at once no longer floods the TUI.

## [0.4.78] - 2026-06-21

### Breaking Changes

- **Task tool renamed to ExecuteTask**. The subagent delegation tool is now
  called `ExecuteTask` instead of `Task`. Agent definitions and prompts that
  reference "Task tool" must be updated. The old name is kept as a deprecated
  alias for backward compatibility.
- **TodoWrite schema extended with `complexity` field**. Each todo item can now
  carry a `complexity` parameter (`"fast"`, `"standard"`, `"capable"`) that
  maps to a model category via `settings.modelCategories`. This is optional;
  omitting it uses the agent's default model.
- **ExecuteTask schema extended with `complexity` and `item_id` fields**. The
  new `complexity` parameter selects a model category from config. The new
  `item_id` parameter links the dispatch to a TodoWrite item for tracking.

### Fixed

- TodoWrite reconciliation now filters to root main-agent tasks only, excluding
  MCP-sourced and delegated tasks that `taskOwnerId()` folded under "main". This
  prevented silent data corruption of MCP task rows when the TodoWrite list was
  shorter than the combined main+MCP task count.
- TaskOutput now waits for a running/queued subagent to finish (up to 120s)
  instead of returning "call again later" and requiring an extra LLM round-trip
  per poll. Foreground Task completions are also visible to TaskOutput via a new
  `wait_for_completion` API on the subagent pool.

### Changed

- TaskStore gains a `batch(fn)` method that defers listener notifications until
  the batch callback completes. TodoWrite reconciliation and child-task-tree
  merging now emit a single render invalidation instead of one per item.

## [0.4.77] - 2026-06-21

## [0.4.76] - 2026-06-21

### Fixed

- Tool calls that shell out to `fd`/`rg` (`grep`, `find`, `glob`) no longer
  re-run a synchronous `spawnSync(<tool>, ["--version"])` probe on every
  invocation. When a tool resolves from `PATH` instead of `~/.hoocode/bin`,
  this blocking probe ran on each call and stalled the event loop; the resolved
  path is now cached for the process lifetime.
- `ls` tool uses `readdir` with `{ withFileTypes: true }` on the default local
  filesystem backend, replacing N sequential `stat()` syscalls with a single
  `readdir` call that returns file type info inline. Extension backends that
  only implement the existing `readdir` string return continue to work via the
  fallback path.
- `grep` tool output now shows each filename once per file instead of repeating
  it on every matching line, reducing token usage for multi-file matches.
- `edit` tool now matches blocks even when leading indentation differs (for
  example the model emits 2-space indentation against a tab-indented file). A
  third indentation-tolerant matching tier runs only after exact and fuzzy
  matching fail, comparing whole lines with leading/trailing whitespace ignored,
  while keeping the uniqueness guardrail and replacing in the original content so
  surrounding formatting is preserved. This addresses frequent "Could not find
  the exact text" failures caused by whitespace drift.
- `edit` tool no longer re-normalizes the entire file just to count occurrences
  on the exact-match path; occurrence counting now reuses the resolved match
  spans.
- File mutation queue caches resolved realpaths instead of running a blocking
  `realpathSync.native` syscall on every edit/write.
- `read` tool reuses the original file string for whole-file reads instead of
  splitting into lines and rejoining them.
- `write` tool reports the actual UTF-8 byte count instead of the JavaScript
  string length (which differs for multibyte content).
- `OutputAccumulator.snapshot()` caches its result and invalidates the cache
  only when new output is appended or the stream finishes, so the streaming UI's
  timer-driven polling no longer re-runs tail truncation/compression on every
  tick when no new data has arrived.
- `execCommand` (extension command helper) now decodes child process output with
  streaming `TextDecoder`s and joins collected chunks, fixing corruption of
  multibyte UTF-8 sequences split across chunk boundaries and avoiding quadratic
  string concatenation for large output.
- Hoisted the retryable-error regex (`agent-session`) and the inherited-model
  fallback regex (`subagent-pool`) to module-level constants so they are compiled
  once instead of on every error check.
- Session tree sorting parses each entry timestamp once (decorate-sort-undecorate)
  instead of allocating two `Date` objects per comparison.
- Branch-session label creation reuses a single collision-id set instead of
  rebuilding a `Set` on every label iteration.
- `getTextOutput` partitions tool result content in a single pass instead of
  filtering the same array twice.

### Changed

- Extracted the duplicated fd path-normalization and glob-argument helpers from
  `find` and `glob` into a shared `fd-utils.ts` module.

## [0.4.75] - 2026-06-21

## [0.4.74] - 2026-06-21

### Removed

- Dropped the `shadow-executor` local-inference routing mode. It was an
  unimplemented no-op (the live path always used the primary and the
  measurement mirror was never built), so removing it deletes a misleading
  option from `routing.mode` / `HOOCODE_ROUTING_MODE` with no behavior loss.
  Setting it now falls through to the default activated mode.

### Changed

- Default system prompt now includes output-constraint guidelines that trim
  primary-model tokens without fighting hoocode's design: no preamble/postamble
  or task restatement, no filler closers, no narration of routine tool
  calls/results (the permission gate already surfaces them), and matching the
  surrounding code's conventions for comments/docstrings/types rather than
  adding or stripping them by default.
- Generalized the local-executor routing docs beyond MLX on Apple Silicon:
  documented hosted executors (e.g. a free `opencode` model, no `server`
  block), Windows/Linux runtimes (llama.cpp, Ollama, LM Studio, vLLM), and
  clarified that the `maxBytes` size-band cap is a local-hardware OOM guard that
  should be raised for hosted or large-memory executors.
- Local-inference size band is now wired entirely from the models.json executor
  config — the hardcoded `DEFAULT_MIN_BYTES`/`DEFAULT_MAX_BYTES` (2048/8192)
  defaults were removed. An omitted bound is no longer applied (`minBytes` → 0,
  `maxBytes` → unbounded), so enabling routing without a configured band now
  routes inputs of any size instead of silently gating to 2048–8192 bytes. Set
  `minBytes`/`maxBytes` in `routing.executor` to gate by size.

## [0.4.73] - 2026-06-20

### Fixed

- Local-inference compaction routing no longer serializes the conversation on
  the common primary-only path. `_compactWithRouting` computed the conversation
  byte size unconditionally, which both did needless work when routing is off
  and broke compaction in environments that mock `compaction/index.js` without
  `serializeConversation`. The size is now measured only when an executor is
  actually selected for summarization.

## [0.4.72] - 2026-06-20

### Changed

- Local-inference routing (`--enable-local-inference`) narrowed after post-ship
  measurement on an 8 GB M1:
  - Tool-result compression now targets `bash` only. `read` was removed: on real
    source code it compressed ~0% (every line is a keep-line under the extractive
    prompt) while adding 60-90s of latency. `bash` still compresses noisy command
    output ~85% at full fact retention.
  - Added a global input size band for all local inference (compaction and bash
    compression). Only inputs within the band are routed to the executor
    (`minBytes`/`maxBytes` in the executor config, default 2048-8192). Oversized
    inputs fall back to the primary model; this prevents the GPU OOM that large
    compaction inputs caused on small machines. Replaces the old
    `toolResultMinBytes` setting.

## [0.4.71] - 2026-06-20

## [0.4.70] - 2026-06-20

## [0.4.69] - 2026-06-18

## [0.4.68] - 2026-06-18

### Fixed

- `truncated-tool` example extension: run ripgrep via `spawnSync` with an argv
  array instead of `execSync(args.join(" "))`. Joining into a single string and
  running it through a shell broke on search paths or patterns containing
  spaces, parentheses, or other shell metacharacters (e.g. a home directory like
  `/home/user (admin)/project` produced `syntax error near unexpected token '('`).

### Changed

- Bumped workspace packages to `0.4.68` so the reported version reflects the
  current build.

### Documentation

- Corrected the README config paths from `config.json` to `hoo-config.json`, the
  filename the agent actually reads/writes (`src/init.ts`,
  `src/extensions/core/hoo-core.ts`). A `config.json` placed in `~/.hoocode/` or
  `.hoocode/` was silently ignored.

## [0.4.67] - 2026-06-16

### Added

- Public API surface for downstream apps that build their own agents on top of
  hoocode's prompt/tool machinery:
  - System prompt: `buildSystemPrompt`, plus the built-in mode prompts
    `DEFAULT_MODE_PROMPTS` and `DEFAULT_MODE` (extracted to
    `core/mode-prompts.ts`).
  - Tool registry: `createTool`, `createToolDefinition`, `createAllTools`,
    `createAllToolDefinitions`, `createCodingToolDefinitions`,
    `createReadOnlyToolDefinitions`, `allToolNames`, and the `Tool` / `ToolDef`
    / `ToolName` types.
  - Opt-in tools: `createTaskToolDefinition`, `createTaskOutputToolDefinition`,
    `createTodoWriteToolDefinition`, and `buildTaskMainPrompt`.
  - Prompt templates: `loadPromptTemplates`, `expandPromptTemplate`,
    `tryExpandPromptTemplate`, `parseCommandArgs`, `substituteArgs`, and their
    option/result types.
  - Skills/agents: `LoadSkillsOptions`, `AgentRegistry`, `loadAgentRegistry`,
    `LoadAgentRegistryOptions`, `formatAgentsForPrompt`, `AgentDefinition`, and
    `HOOCODE_TOOL_NAMES`.
- `loadAgentRegistry` now accepts an explicit `agentPaths` option (files or
  directories, resolved against `cwd` with `~` expansion), mirroring
  `skillPaths`/`promptPaths` on the skills and prompt-template loaders. These
  override discovered agents by name and yield to CLI `--agent` paths.
  - Canonical opt-in tool-name constants `TASK_TOOL_NAME` and
    `TODO_WRITE_TOOL_NAME`, now the single source of truth at the tool
    definition sites and in system-prompt gating, so downstream callers
    reference a value instead of hardcoding case-sensitive strings.

## [0.4.66] - 2026-06-16

## [0.4.65] - 2026-06-16

### Added

- Release workflows now build and attach a 64-bit Windows standalone binary
  (`hoocode-windows-x64.zip`) to each GitHub release. A `binaries` job in
  `release.yml` and `merge-release.yml` runs the bun-only
  `scripts/build-binaries.sh` (`bun build --compile --target=bun-windows-x64`),
  stages the runtime assets and the koffi native module next to `hoocode.exe`,
  zips them, and uploads the archive to the release.

### Changed

- `scripts/build-binaries.sh` is now bun-only: dropped the `npm run build` step
  (replaced with `bun run build`) and the obsolete `hoist-bun-deps.mjs` step
  (the hoisted linker pinned in `bunfig.toml` already yields a flat
  `node_modules`). Example sources are copied without their `node_modules` to
  avoid unresolved bun workspace symlinks.

## [0.4.64] - 2026-06-16

### Fixed

- bun CI (`bun-check`/`bun-build`) failed on `main` with TS7016/TS7006 errors
  because `bun install --frozen-lockfile` never installed coding-agent's
  `@types/*` devDeps. The workspace `packages/coding-agent` is named
  `@kolisachint/hoocode-agent`, which collided with the root `package.json`
  self-dependency `@kolisachint/hoocode-agent: ^0.2.0`; bun resolved that name
  to the registry package and dropped the local workspace (and its devDeps)
  from `bun.lock`. Removed the unused root self-dependency (root is private and
  `tsconfig.json` already maps the name to the workspace source) and
  regenerated `bun.lock` so the workspace and its `@types/proper-lockfile` /
  `@types/hosted-git-info` devDeps are captured.

### Added

- `build:bun-binary` script: builds a self-contained standalone executable with
  `bun build --compile` (embeds the Bun runtime; no Node.js/Bun required to run).
  Stages the runtime assets (themes, HTML export templates, docs, examples,
  templates, photon wasm, package.json) next to the executable in
  `dist/bun-binary/`, and supports cross-compilation via `--target` (e.g.
  `bun-linux-x64`, `bun-darwin-arm64`, `bun-windows-x64`).

## [0.4.63] - 2026-06-16

## [0.4.62] - 2026-06-16

### Changed

- Trimmed the built-in subagent roster to match Claude Code: `explore`, `plan`, and
  `general-purpose` ship by default (the `doc`, `edit`, `review`, and `test` agents
  were removed — author them under `.hoocode/agents/` if needed). `explore` and `plan`
  are strictly read-only, and `general-purpose` inherits the parent model and sets
  `delegate: true` so it can spawn subagents when nesting is enabled.
- `embed-templates` now formats its generated output with biome, so regenerating the
  embedded templates can no longer break the CI lint check.

### Added

- Built-in read-only `plan` subagent (research that backs plan mode), matching Claude
  Code's Plan agent.
- `disallowedTools` agent frontmatter field and `--disallowed-tools` CLI flag — a tool
  denylist subtracted from the allowlist/default set (Claude Code's allow+deny model).
- Fork subagents: a `fork: true` agent inherits the parent's full conversation (via a
  forked session that reuses the parent's prompt cache) instead of starting from a fresh
  context, matching Claude Code's fork subagents. Ships `examples/agents/fork-reviewer.md`.
- `nestedSubagentConcurrency` setting (default 2) to tune how many subagents a nested
  pool runs concurrently.
- Configurable subagent nesting via `maxSubagentDepth` (default `1`, opt-in) or the
  `--max-subagent-depth <n>` CLI flag (overrides the setting). At the default cap
  behavior is unchanged — subagents cannot spawn subagents. Raising it
  (e.g. `"maxSubagentDepth": 2`) lets a subagent delegate one further level. Fan-out
  stays bounded: the cap is seeded into the environment so every process in the tree
  agrees, and nested pools (depth ≥ 1) run with a reduced concurrency. The requested
  cap is clamped to a hard ceiling of 3 (worst case ≈ 35 live processes) so a
  mis-configuration can't exhaust the host, with no shared state to leak on crash. The
  child's depth is recorded in the `[DISPATCH]` log line and `dispatch-log.json`.
- Per-agent delegation opt-in via the `delegate` agent frontmatter flag. A delegating
  agent spawned below the nesting cap has `Task`/`TaskOutput` added to its tool allowlist
  and subagents enabled, so it can dispatch one further level; every other agent keeps its
  declared sandbox. `delegate: true` allows any subagent type; `delegate: explore, plan`
  scopes delegation to those types only (the Task tool rejects out-of-scope dispatches),
  matching Claude Code's `Agent(types)` syntax. Ships an example `examples/agents/orchestrator.md`.

## [0.4.61] - 2026-06-15

## [0.4.60] - 2026-06-15

## [0.4.59] - 2026-06-15

### Changed

- TodoWrite tool is now enabled by default (`enableTodoWrite` defaults to `true`).
  Set `"enableTodoWrite": false` in settings to opt out.

## [0.4.58] - 2026-06-15

## [0.4.57] - 2026-06-14

## [0.4.56] - 2026-06-13

## [0.4.55] - 2026-06-12

### Added

- Approval gates inline in the attach panel: when the attached role pauses
  (`task_paused`), the question and its options render inside the side panel —
  right where the stream stopped — instead of the editor's INPUT NEEDED pane.
  While a gate is open it owns the panel's keyboard (`q`/`n` type into the
  custom row, esc skips); answering stamps "✓ answered: …" into the stream and
  resumes it. Detaching mid-gate falls back to the options pane so the
  question isn't lost, and a gate answered elsewhere still dismisses itself.
  The panel also renders orchestrator task lifecycle lines now
  (`task_started`/`task_paused`/`task_resumed`/`task_finished`).

## [0.4.54] - 2026-06-12

### Added

- Approval gates for `--team`: when a hooteams orchestrator pauses a task
  (`task_paused` over the shared `/events` stream, or already pending on
  attach via `GET /tasks/pending`), the question and its options surface in
  the INPUT NEEDED pane (free-form answers allowed). The answer goes back as
  `POST /tasks/:id/resume`; first answer across surfaces wins, and a gate
  answered from hoocanvas (or another hoocode) dismisses itself with a notice.
  Paused roles show as "waiting" with an "awaiting approval: …" task row in
  the teams view, and orchestrator `task_started`/`task_finished` events now
  drive role state too.

## [0.4.53] - 2026-06-12

### Added

- Team focus for `--team`: `alt+n` (`app.team.focus`) focuses the task panel's
  teams roster. With a role focused, `↑/↓` move the ▶ cursor, `n` opens an
  inline editor that nudges the role (POST `/steer`), `a` opens an attach side
  panel, and `q`/`esc` return to the prompt. Outside team focus all keys behave
  exactly as before.
- Attach side panel: `a` on a focused role streams that role's live events in
  the style of `hooteams attach <role>` (themed, bounded buffer). `q` detaches
  (the role keeps running), `n` nudges the attached role. The panel filters the
  existing team SSE stream — no second connection — and detaching leaks no
  subscribers.
- `--team auto`: walks up from cwd for `.agents/teams/default.json` or
  `hooteams.config.json`, spawns `hooteams start --config <path> --port <free>`
  (from PATH, falling back to `bunx`), waits for `/health`, then proceeds as if
  `--team http://localhost:<port>` was passed. The child is reaped on exit,
  clean or signalled. Missing config or launcher fails with a clear error;
  `--team <url>` is unchanged.

## [0.4.52] - 2026-06-11

## [0.4.51] - 2026-06-11
### Fixed

- `--team` no longer corrupts the interactive TUI. Team-view warnings went to
  raw stderr while the TUI owned the screen, scribbling over the render until
  the editor stopped echoing input; an `/events` endpoint that answered 200
  but closed without streaming repeated that warning every 5s retry. Warnings
  now surface in the chat, and a reconnect only counts as recovered once the
  stream actually delivers data.
- Idle team roles no longer pin the task pane at "◐ WORKING". The mirror gave
  every role a `pending` task that never finished, so the pane never collapsed
  and task numbering never reset. Roles now get a task row only while they are
  actually doing something (or failed); an all-idle team leaves the pane
  collapsed.

### Changed

- Default key for `app.tasks.cycleView` is now `alt+t` (with `shift+ctrl+t`
  kept as an alias). Windows Terminal intercepts `ctrl+shift+t` as its own
  "new tab" shortcut, so the old default never reached the app there.

## [0.4.50] - 2026-06-11

### Fixed

- Task panel: the `teams` view is now a real lens distinct from `subagents`
  instead of an alias for the same grouped path. `subagents` filters out
  `kind: "role"` agents and `teams` filters to only role agents; non-matching
  agents and their tasks no longer leak between lenses. The `teams` view also
  renders forward-handoff connectors (`└──→ next`) between successive role
  agents and keeps a placeholder header for queued roles with no tasks yet.
- Task store: `update()` and `addAgentStats()` now log a `console.warn` when
  called with an unknown task or agent id. Previously straggler completions
  (late MCP/SSE events whose owner had been reaped) silently no-opped, hiding
  observability data.
- Task store: `reset()` now preserves agents with non-zero accumulated stats
  across user turns. Previously an agent that finished all of its tasks before
  the next user message would be wiped along with its cross-turn cost
  accounting; subsequent re-dispatches restarted from zero.
- Task panel: animation timer is now guarded against post-`dispose()` revival.
  A `disposed` flag short-circuits both `render()` and `ensureAnimation()` so a
  late render call cannot resurrect the spinner interval after teardown.

## [0.4.49] - 2026-06-11

## [0.4.48] - 2026-06-11
### Added

- `--team <url>`: mirror a running hooteams server (read-only) into the task
  panel's teams view. Roles from `GET /status` register as `kind="role"`
  agents and the single `GET /events` SSE stream maps TeamEvents onto live
  task/agent patches. Connection failures and drops log a warning and never
  block the main agent.

## [0.4.47] - 2026-06-11

## [0.4.46] - 2026-06-11

## [0.4.45] - 2026-06-11

### Fixed

- Subagent pool now correctly marks tasks as failed when a child process writes a
  valid `result.json` with `status: "failed"` and exits non-zero. Previously the
  pool treated a well-formed but failed result as a clean completion, causing
  `result.ok` to be `true` and suppressing the concrete failure reason.

### Added

- The task panel now has three views over the same task list, cycled with
  shift+ctrl+t (`app.tasks.cycleView`) and shown as a `tasks · subagents · teams`
  switcher in the ledger header: flat (unchanged), subagents (tasks grouped by
  owning agent: ◆ main orchestrator + ⊕ workers), and teams (grouped by named
  role-agent: ▸, with lifecycle `[state]` tags and handoff arrows). Group headers
  carry each agent's own token/cost totals and done/total count; grouped rows sit
  on a faint indent guide and drop their per-row origin tag. Subagent dispatches
  register themselves in the new `TaskAgent` roster on the task store
  (`upsertAgent`/`patchAgent`/`addAgentStats`), which external orchestrators
  (e.g. hooteams) can also feed to drive the teams view.

## [0.4.44] - 2026-06-09

## [0.4.43] - 2026-06-05

### Fixed

- Built-in subagents now fall back to the parent's inherited model on a
  credits/auth/quota failure even when no provider was threaded through the
  dispatch. Previously the inherited-model retry required both a model and a
  provider from the parent; when the harness routed through a gateway without an
  explicit provider, a preferred-model failure (e.g. `CreditsError: Insufficient
  balance`) was reported as a hard failure instead of retrying on the inherited
  model. The retry now requires only the parent model and lets the child resolve
  the provider from its own default.

## [0.4.42] - 2026-06-05

### Fixed

- MCP tools failed with `MCP server "<name>" is not connected` once the server's
  connection was torn down (server process exit, host process churn between turns,
  or a racing teardown). The tool call gave up immediately with no recovery, and
  the server config was not retained so it could not reconnect. The loader now
  retains each server's config and a tool call lazily reconnects from it before
  failing. The client also now sends the spec-required `notifications/initialized`
  after `initialize`, the handshake (`initialize`/`tools/list`) is bounded by a
  timeout so a dead server can't hang startup, and spawned MCP servers are killed
  when the host process exits so they no longer linger as orphans.
- Quick-finishing subagents were reported as "stalled". A spawned subagent
  (`--mode json` with a task id) finished its work and wrote a valid
  `result.json`, but the child process did not exit on its own: lingering open
  handles in its runtime kept the event loop alive, so it sat idle until the
  parent lifeguard SIGKILLed it at the 60s heartbeat threshold (`exit_code: null`,
  `status: "stalled"`). Spawned subagents now force a clean, flushed exit as soon
  as their work is done, so they terminate in seconds instead of hanging until the
  reap. As a defense in depth, the pool now treats a child as successfully
  completed whenever it produced a verified `result.json`, regardless of exit
  code, so a kill that races a finished child still returns the real answer.

## [0.4.41] - 2026-06-05

### Added

- Slash commands are now discovered from `.agents/commands/` (project
  ancestor-walk up to the git root, plus user-level `~/.agents/commands/`), so
  commands written under the cross-vendor `.agents/` tree round-trip. Precedence
  is first-match-wins: project `.hoocode` > project `.claude` > project
  `.agents` (cwd-first) > user `.hoocode` > user `.agents` > user `.claude`.

### Fixed

- Background subagents that finished cleanly were sometimes reported as "stalled".
  A late lifeguard heartbeat-miss could fire SIGKILL just as a healthy child was
  already exiting; the kill was a no-op, the child still exited 0 and wrote a valid
  result.json, but the pool honored the stale stall verdict and discarded the real
  success. The exit handler now only honors a stalled/timeout kill when the child
  did not actually complete (non-zero exit or no verified result.json), so a
  genuine completion always wins over a racing stall verdict.

## [0.4.40] - 2026-06-05

## [0.4.39] - 2026-06-05

## [0.4.38] - 2026-06-05

### Fixed

- Subagents no longer get killed under heavy load. When many subagents (plus
  background MCP tools) run at once, CPU contention starved the parent event loop
  so it missed a healthy child's heartbeat and SIGKILLed it as "stalled" (or hit a
  wall-clock timeout that inflated faster than real work). The lifeguard now scales
  the heartbeat-miss and hard-timeout budgets by the number of concurrent
  subagents and forgives measured event-loop lag, so contention alone no longer
  reaps a working subagent. A genuinely stuck agent is still reaped at a hard
  ceiling.

### Changed

- Background tools now explain themselves verbosely and consistently in chat. Both
  background subagents and background MCP tools get the same shape: a verbose
  "started" line that names the subagent/MCP tool and summarizes its arguments, and
  a matching "finished/failed" follow-up message using the same label. Subagent and
  MCP background work already appear in the task pane (pending → in_progress →
  done/failed); the chat side is now in sync with it.
- MCP tool calls now render a clean, prefixed title in chat — `MCP [server › tool]
  <args>` — parallel to the subagent `Task [type] <desc>` line, instead of falling
  back to the raw `mcp_<server>_<tool>` tool name. The args summary reuses the same
  helper as the background start/finish messages so the title stays in sync with them.
- The task pane now shows a single-cell source glyph before the id — ⚙ for subagent
  rows, ⧉ for MCP rows (plain tasks reserve a blank cell) — so the two kinds of
  background work are distinguishable at a glance. The pane stays tag-free: the
  subagent *mode* tag (e.g. "[explore]") is still not shown.
- Task-pane ids are padded to a uniform column width, so titles stay aligned across
  rows regardless of id digit count (#1 vs #10 vs #100) instead of jagging right.

## [0.4.37] - 2026-06-05

## [0.4.36] - 2026-06-05

### Added

- MCP tools now run in background mode by default (non-blocking), since MCP servers are external processes that may have high latency. Configure per-server with `"background": false` in mcp.json.

### Fixed

- Slash commands in `.hoocode/commands/` now take precedence over prompt templates in `.hoocode/prompts/` when there is a name collision, instead of the reverse.

## [0.4.35] - 2026-06-05

## [0.4.34] - 2026-06-05

### Changed

- Built-in `explore`, `review`, `test`, and `doc` subagents now run in the background by default (their templates set `background: true`), so delegating to them no longer blocks the parent turn — it keeps reasoning while the subagent runs and the answer arrives as a follow-up message.

### Fixed

- Task pane no longer truncates task titles early or unevenly: rows now use the full available width (the previous budget subtracted the row prefix, so titles clipped ~5-7 columns short and differently per task-id width).
- Subagent fallbacks and exhaustion skips now surface as a compact ⚠ cue in the task pane (e.g. "ran on inherited model", "<provider> exhausted") instead of relying on a chat message.

## [0.4.33] - 2026-06-04

### Fixed

- Built-in subagents now retry with the inherited parent model when their preferred model fails due to model availability, auth, quota, or rate-limit errors.

## [0.4.32] - 2026-06-04

### Changed

- Background subagents (`Task` with an agent whose definition sets `background: true`)
  now run via the agent loop's non-blocking tool mechanism. The parent keeps reasoning
  while the subagent runs and its final answer is injected automatically as a follow-up
  message when it finishes — no `TaskOutput` polling required. The injected answer is a
  dedicated `backgroundTask` custom message (distinct styling) rather than a plain user
  message.

## [0.4.31] - 2026-06-03

## [0.4.30] - 2026-06-03

## [0.4.29] - 2026-06-03

## [0.4.28] - 2026-06-03

## [0.4.27] - 2026-06-03

## [0.4.25] - 2026-06-03

### Added

- Options pane (`ask_options`) options can now be flagged `recommended: true`, which renders a green `(recommended)` marker next to the option label to guide the user's choice.
- File-based slash commands: reusable command Markdown invoked with `/name`, auto-discovered from `~/.hoocode/commands/*.md` (global) and `.hoocode/commands/*.md` (project), plus the `slashCommands` settings array and the repeatable `--slash-command <path>` flag. A `type` frontmatter field controls how the command is injected: `user` (default, sent as a user message), `system` (appended to the system prompt), or `context` (added as a hidden context message). Disable discovery with `--no-slash-commands` (`-nsc`).
- Claude Code slash commands are now imported natively (D7) from `.claude/commands/*.md` (project) and `~/.claude/commands/*.md` (user), at lower precedence than `.hoocode/commands/`. Discovery respects `--no-slash-commands`.

### Changed

- Renamed the `--subagent` CLI flag to `--enable-subagents` for readability (the `enableSubagent` setting and `/subagent` command are unchanged).
- Release workflow no longer publishes binaries; releases now ship only GitHub's auto-generated source code archives.

### Fixed

- Removed the leading blank line inside the edit tool's output box (call render box paddingY changed from 1 to 0).

## [0.4.24] - 2026-06-03

### Added

- Added the `general-purpose` built-in subagent (the default dispatch target) with an explicit `read, bash, edit, write, grep, find, ls` tool allowlist so it can investigate and act end to end.
- Subagent dispatch now skips spawning when the inherited provider is flagged exhausted. When the main session's own turn fails with a persistent usage/quota/rate-limit error, the provider is recorded as exhausted for a short window (cleared on the next successful response, self-expiring after a TTL); the Task tool then returns a clear "provider appears exhausted" message and records a failed task instead of burning another doomed spawn (subagents inherit the parent's provider).

### Fixed

- Subagent failures now surface the real cause. A non-zero-exit subagent's `result.json` summary (which carries the provider/model error, e.g. "usage limit reached") is attached to the pool result, and the failed run's summary now embeds the provider error message, so the Task tool reports the concrete reason instead of a generic "subagent failed".

## [0.4.23] - 2026-06-02

### Added

- Added an options pane: the agent can call the new `ask_options` tool to ask you one or more decisions inline in the transcript. Move between options with up/down, confirm and advance to the next question with right (left to go back), quick-pick with number keys, or drop onto a custom row to type your own answer when none fit; esc skips. Answered steps stay on screen as a breadcrumb. Added `tui.select.next` (right) and `tui.select.back` (left) keybindings and an `askOptions()` method on the extension UI context.

### Changed

- Redesigned the interactive task panel to be more legible and informative. Added a state-colored left rail (working/reviewed/stopped), a deterministic block-glyph progress bar with a done/total count, total elapsed time in the header, an animated braille spinner on the active task, and `queued`/`running…` tags on unsettled rows.

## [0.4.21] - 2026-06-01

### Fixed

- Removed leading space from startup warning lines (`showWarning` paddingX changed from 1 to 0).

## [0.4.20] - 2026-06-01

## [0.4.19] - 2026-06-01

### Added

- Startup banner now renders the Hoo owl symbol as colored half-block ANSI art above the ASCII wordmark on truecolor terminals (wide layout only). Non-truecolor or narrow terminals keep the existing wordmark/app-name banner. The art is generated from `assets/symbol.svg` via `scripts/generate-wordmark-symbol.ts` into `src/core/wordmark-symbol.generated.ts`.

## [0.4.18] - 2026-06-01

### Changed

- Subagent dispatch dirs are now deleted on a clean, verified success. When a subagent exits 0 and its `result.json` passes verification, `SubagentPool` removes `.hoocode/dispatch/<task_id>/` entirely (session.jsonl, result.json, dispatch-log.json, budget.json); the in-memory result still carries `result_data`, so callers lose nothing. Dirs for failed/partial/stalled/timeout tasks are retained for debugging. Trade-off: `resume_task_id` now only works for non-successful tasks, since a completed task's persisted session is discarded.

## [0.4.17] - 2026-06-01

### Changed

- Consolidated subagent prompts to a single source of truth. The duplicate `templates/subagent/**` prompt set (and the generated `EMBEDDED_SUBAGENT_PROMPTS` map) is removed; `templates/agents/**` (the frontmatter registry) is now the only prompt source. The built-in agent set is the canonical five: `explore`, `edit`, `test`, `review`, `doc`. The unreachable `fix` mode (never exposed by the Task tool, `/subagent`, or routing) was dropped.
- Trimmed `DispatchEvaluator` to its only live responsibilities: the nested-delegation depth guard and a complexity estimate for the dispatch log. Delegation is fully description-driven (the parent agent chooses the agent), so the dead keyword-routing/auto-split surface was removed. Updated `docs/routing.md` to match.
- Moved built-in subagent tool allowlists into agent frontmatter (`tools:` in `templates/agents/*.md`), making the agent registry the single source of truth for each agent's prompt, tools, and model. The hardcoded `SubagentMode` enum and `MODE_TOOLS` map are gone; `SubagentPool` reads the allowlist solely from the resolved definition.

### Removed

- Removed unused exports: `isSubagentRecommended` (tools/subagent), `SubagentPool.dispatchBatch`, and the `DispatchEvaluator` routing helpers (`classifyWithConfidence`, `shouldSplit`, `canHandleInline`, `getReason`).
- Removed the `core/subagent.ts` module (`SubagentMode`, `SUBAGENT_MODES`, `MODE_TOOLS`); its role is now served by the frontmatter agent registry.

## [0.4.16] - 2026-06-01

### Fixed

- Fixed the available-agents list shown to the model (in the Task tool description and the main-session subagent prompt) collapsing to a repeated, useless header line. Every built-in agent description opens with the same `Use this subagent ONLY when:` line, and only that first line was surfaced. The list now condenses each description to a meaningful one-line summary (first "when to use" bullets, or first prose line), so agents are distinguishable.
- Fixed the footer and startup Resources list not reflecting an enabled subagent. The wiring checked for a tool named `subagent`, but the model-facing tool was renamed to `Task`, so `--subagent` (or the `enableSubagent` setting) never showed `mode + subagent` in the footer nor `subagent_system_prompt` under Resources.

## [0.4.15] - 2026-05-31

### Added

- Data-driven subagents: agents are now defined by frontmatter `.md` files (`name`, `description`, optional `tools`/`model`/`maxTurns`) loaded from a registry with precedence project > user > built-in. For drop-in Claude Code compatibility, `.claude/agents/` (project) and `~/.claude/agents/` (user) are also discovered, and Claude tool names are normalized to hoocode's tools (`Read`→`read`, `Glob`→`find`, `LS`→`ls`, etc.; unsupported tools are dropped with a diagnostic).
- Every spawned subagent now runs under a hard turn cap (`--max-turns`, default 50 when a definition sets no `maxTurns`). Near the cap the agent is asked to wrap up and return its findings; if it reaches the cap it is stopped and its partial findings are returned as a `partial` result instead of a failure.
- Background (non-blocking) subagents: an agent definition can set `background: true`. The `Task` tool then dispatches it detached and returns a `task_id` immediately instead of blocking. A new `TaskOutput` tool polls a background subagent by `task_id` and collects its final answer once finished.
- Resume capability: subagents now persist their session, and the `Task` tool accepts an optional `resume_task_id` to continue a previous subagent run with a follow-up `prompt` (full prior transcript intact). Partial results surface their resume handle so the parent can continue interrupted work.

### Changed

- Renamed the model-facing `subagent` tool to `Task`, mirroring Claude Code. Parameters are now `description`, `prompt`, and `subagent_type` (any registry agent name). Delegation is description-driven: the model chooses when and which agent to use, with no blocking dispatch gate. The `--subagent` flag, `enableSubagent` setting, and `/subagent` command are unchanged (the latter now validates against the registry).
- The subagent token budget is now advisory. It still emits `budget_warning` (80%) and `budget_exceeded` (100%) events for telemetry, but never kills or fails a subagent; the per-subagent turn cap is the guaranteed hard stop. Lifeguard stall/timeout kills are unchanged.
- Relocated subagent runtime dispatch state from `.hoocode/agents/<task_id>/` to `.hoocode/dispatch/<task_id>/`, freeing `.hoocode/agents/` to hold agent definitions.
- Subagent sessions are now persisted to `.hoocode/dispatch/<task_id>/session.jsonl` (previously ephemeral via `--no-session`) so a finished or interrupted subagent can be resumed.
- Nesting guard: the `Task`/`TaskOutput` tools are never registered inside a spawned subagent process (`--task-id` present), even when a project's `enableSubagent` setting is on, so subagents cannot recursively dispatch.

## [0.4.14] - 2026-05-31

### Changed

- Task panel restyled to the hoocode design system: the `#id` recedes (dim), completed task titles fade to muted while active/failed stay full foreground, each finished row's token count sits one step brighter than its elapsed time, and the header turn delta uses muted framing with bright numbers. The reviewed/deterministic stamp is now quiet (dim) rather than green; only the active "watching" state keeps a tint. Pure visual change, no logic changes.
- Subagent routing is now deterministic on ties: the dispatch evaluator uses an explicit agent-priority order instead of incidental object-iteration order, exposes a normalized `confidence` on each analysis, and defaults an ambiguous delegated task to `explore`. Tightened the parent's subagent guidance to prefer inline handling for small/quick tasks.
- Raised default subagent token budgets to stop hard-stopping agents mid-task: explore 35k, edit 60k, test 45k, fix 45k, review 35k, doc 30k (fallback 35k). Real per-event usage tracking is unchanged.
- Tool execution blocks (bash commands, diffs, file reads, etc.) are tighter: dropped the box vertical padding so consecutive tools are separated by a single blank line instead of three, saving vertical space in the TUI.

## [0.4.13] - 2026-05-30

## [0.4.12] - 2026-05-30

### Changed

- Subagents now run as isolated `hoocode` child processes through `SubagentPool` instead of in-process loops. The `subagent` tool and `/subagent` command dispatch through a shared pool with bounded concurrency, per-mode tool allowlists, token budgets, and lifeguard monitoring (heartbeat + hard timeout).
- Added a `--task-id` CLI flag (internal) and made `--mode json` subagents emit a `{"ping":true}` heartbeat and write a verified `result.json` on exit, so the parent pool can monitor liveness and validate output.

### Removed

- Removed the in-process `runSubagent` path; subagent execution is now exclusively pool/child-process based.

### Fixed

- Tool execution status dot (`●`) is now rendered inline with the tool/command on the first line instead of on its own line above it. Renderer-backed tools (e.g. bash) previously stacked the dot as a separate line.

## [0.4.11] - 2026-05-30

### Changed

- Task panel: subagent task titles are now limited to ~4–8 words so they stay legible in the pane.
- Task panel: finished tasks show combined token usage and elapsed time (`tokens · time`).
- Task panel: header shows a per-turn token + cost delta (`turn ↑in ↓out $cost`) summed across the turn's tasks.
- Task panel: the `[mode]` tag (e.g. `[explore]`) is no longer shown per row — the task title is the meaningful label.
- Finished subagent tasks now persist in the task panel until the next user message, instead of retiring when the main agent starts its next turn.

## [0.4.10] - 2026-05-30

## [0.4.8] - 2026-05-30

### Added

- TokenBudget class tracking per-agent-type token budgets with 80% warning and 100% hard-stop, persisting usage to disk.
- OutputVerifier validating subagent result.json after exit.
- SubagentLifeguard with heartbeat monitoring, hard timeouts, parent-exit cleanup, and startup sweep.
- Named `SUBAGENT_MAIN_PROMPT` system prompt appendix loaded when subagent tooling is enabled.

### Changed

- Footer displays active mode plus `+ subagent` when the subagent system prompt is active.
- Loading page `[Resources]` section always shows `mode/{activeMode}` and `subagent_system_prompt` when applicable.

### Fixed

- Editor wordmark first-line indentation preserved (array join instead of `String.raw` + `.trim()`).

## [0.4.7] - 2026-05-30

## [0.4.6] - 2026-05-30

### Changed

- Task panel now shows all subagent tasks with every status (pending, in_progress, done, failed) instead of only active ones. Finished tasks keep their final status icon and retire only when the main agent moves on to its next turn after a parallel subagent spawn.
- Manual and auto compaction now share a single `_applyCompaction` core, removing ~80 lines of duplicated extension-hook, persistence, and summary-extraction logic that could drift between the two paths.
- `/compact` no longer pre-checks message count in the UI; the session is the single source of truth and surfaces the precise reason ("Already compacted", "Nothing to compact") via the compaction event.

### Fixed

- Compaction now fails loudly instead of writing an empty summary when the summarizer model returns no usable text, preventing silent loss of conversation history.

## [0.4.3] - 2026-05-29

### Changed

- Active subagent tasks are now rendered in a dedicated task panel just above the editor prompt instead of the footer. The panel shows only running tasks (pending / in_progress), uses LIFO ordering (newest closest to the prompt), and collapses to zero lines when idle.

### Added

- Clearer guidance on when the model should invoke the `subagent` tool (self-contained work, parallel investigation, discrete tasks, long-running commands).

## [0.4.2] - 2026-05-29

### Added

- Optional `subagent` tool: delegates a self-contained task to a fresh, isolated agent loop (clean minimal prompt, no parent conversation history) and returns only the subagent's final answer. Modes: `explore`, `edit`, `test`, `fix`, `review`. Opt in with the `--subagent` flag or the `enableSubagent` setting. Active subagent tasks are tracked in a task store and shown in the TUI footer. Replaces the former `examples/extensions/subagent` extension.

## [0.3.1] - 2026-05-29

### Added

- `ExtensionFactory` functions can now declare an optional `displayName` (e.g. `mode/build`). When present, the TUI and CLI show that name instead of the synthetic `<inline:N>` path in loaded resources and error messages.

## [0.3.0] - 2026-05-29

### Fixed

- Moved `AGENTS.md` / `CLAUDE.md` size warnings from `console.error` into the TUI startup screen so they are visible inside the app instead of being buried in shell stderr.
- Suppressed empty "What's New" sections on startup when the changelog has no new entries or only whitespace.
- Collapsed loaded resources into a single `[Resources]` line when the total count of context files, skills, prompts, extensions, and themes is 5 or fewer, saving vertical real-estate on sparse projects.

### Changed

- `ResourceLoader.getAgentsFiles()` now returns `{ agentsFiles, warnings }` where `warnings` contains any size/truncation notices. Implementations of `ResourceLoader` must update their return shape. `DefaultResourceLoaderOptions.agentsFilesOverride` and `loadProjectContextFiles()` have matching signature changes.

## [0.2.7] - 2026-05-29

## [0.2.6] - 2026-05-29

### Fixed

- Fixed startup changelog occasionally re-displaying entries the user had already seen. `getChangelogForDisplay()` recorded the app `VERSION` as the last-seen version even when that version overshot the latest entry actually in `CHANGELOG.md` (e.g. shipping `0.2.6` with an empty `[Unreleased]`). When entries for that version were appended later, they were filtered out as already-seen. Both code paths (fresh install and subsequent runs) now record the latest changelog entry's version instead, falling back to `VERSION` only if the changelog is empty.
- Stopped injecting the 7-line HooCode documentation block into the system prompt on every project. It is now gated on the cwd being inside the hoocode source repo (detected by walking up for a `package.json` with `name: "hoocode-monorepo"`). Saves ~150 tokens per turn on every unrelated project, across every provider.
- Added soft warning at 8 KB and hard truncation at 40 KB for context files (`AGENTS.md` / `CLAUDE.md`). Previously a pasted spec would silently bloat every request for the lifetime of the project. Truncation appends a marker so the agent knows content was elided.

### Removed

- Removed the `agent` mode (template, default auto-allow entry, `/mode agent` command target, and `KNOWN_MODES` registration). It overlapped with `build` in intent; users wanting more autonomy should add tools to `modes.build.auto_allow` in `~/.hoocode/config.json` or `.hoocode/config.json` instead. Existing configs that pin `active_mode: "agent"` will still run, but with no mode-prompt layer — switch to `/mode build` to restore a guided prompt.
- Deleted the stray `packages/ai/bedrock-provider.js` and `packages/ai/bedrock-provider.d.ts` root-level shims. They were one-line re-exports of `./dist/bedrock-provider.js` accidentally committed in the first release. The `package.json` `exports."./bedrock-provider"` already points directly at `dist/`, and the `files` field only ships `dist/` — so the shims were never published or imported.
- Slimmed the release pipeline to npm publish + a single Windows standalone zip (`hoocode-windows-x64.zip`). Dropped the macOS/Linux `tar.gz` binaries, the winget standalone `.exe`/manifest, the duplicate `build-binaries.yml` workflow, and the `submit-winget.yml` workflow. Install on macOS/Linux via `npm i -g @kolisachint/hoocode-agent`.

## [0.2.5] - 2026-05-15

### Fixed

- Fixed `init.ts` seeding the global extensions directory at the wrong path. It created `~/.hoocode/extensions/` while the extension loader (`core/extensions/loader.ts` and `core/resource-loader.ts`) reads from `agentDir/extensions` = `~/.hoocode/agent/extensions/`. As a result, extensions placed in the freshly seeded directory were silently invisible. `initConfig()` now creates `~/.hoocode/agent/extensions/` directly, and the redundant `mkdir("agent")` call was dropped since the recursive mkdir on `agent/extensions` already covers it.

## [0.2.4] - 2026-05-15

### Fixed

- Fixed Windows standalone `.exe` (winget) and zip binary still failing to seed default `modes/` and `profiles/` on first run. The Bun-compiled entry point (`src/bun/cli.ts`) never invoked `initConfig()` — only the Node wrapper `bin/hoocode.js` did — so `~/.hoocode/{modes,profiles}` stayed empty for users installing via zip or winget. Added the `initConfig()` call to the Bun entry, and embedded the seed templates into the compiled binary itself (new `scripts/embed-templates.mjs` generates `src/init-templates.generated.ts` at build time, and `init.ts` now writes from these constants instead of reading the on-disk `templates/` folder). This also lets the standalone `.exe` self-seed without a sibling `templates/` directory.
- Dropped the redundant `cp -r templates …` step from `scripts/build-binaries.sh` now that seed content ships inside the binary; this slims every release archive.

## [0.2.3] - 2026-05-15

### Fixed

- Fixed `ERR_UNSUPPORTED_ESM_URL_SCHEME` when running `hoocode` / `hoo` on Windows after `bun add -g`. The bin shim passed raw `path.join(...)` results to dynamic `import()`; Node's ESM loader requires a `file://` URL specifier on Windows. Each `import()` argument is now wrapped with `pathToFileURL().href`.
- Fixed Windows binary missing default `modes/` and `profiles/` after extracting the release zip. Two causes: `init.ts` resolved templates from `__dirname`, which doesn't map to a real disk path inside Bun-compiled binaries; and `scripts/build-binaries.sh` never copied `templates/` next to the executable. Added `getTemplatesDir()` (mirrors `getThemesDir()` etc.), routed `init.ts` through it, replaced silent `try/catch` with a visible warning, and updated the build script to ship `templates/` in every platform archive.

## [0.2.2] - 2026-05-13

### Added

- Added external mode/profile search paths so modes and profiles can ship outside `~/.hoocode/`. Three sources feed the lookup: `HooConfig.mode_paths`/`profile_paths` (in `~/.hoocode/agent/hoo-config.json` or `./.hoocode/config.json`), repeatable `--mode-path <dir>` / `--profile-path <dir>` CLI flags, and new `pi.addModeSearchPath` / `pi.addProfileSearchPath` extension API methods (with matching getters). Resolution order: `./.hoocode/{modes,profiles}/{name}/...` → `~/.hoocode/{modes,profiles}/{name}/...` → external dirs in declared order → built-in defaults.

### Fixed

- Fixed `ERR_SUPPORTED_ESM_URL_SCHEME` error on Windows with Bun by normalizing file URLs to file paths in path resolution functions.

## [0.1.4] - 2026-05-11

### Added

- Added NVIDIA provider support with `NVIDIA_API_KEY` authentication and default model `meta/llama-3.3-70b-instruct`.
- Added mode-based tool filtering via `enabled_tools` configuration option (mode takes priority over profile).
- Added write path restrictions via `allowed_write_paths` configuration option for granular file access control.

### Fixed

- Fixed branding: changed default share viewer URL from `pi.dev` to `hoocode.dev`.
- Fixed branding: changed APP_TITLE from lowercase `hoocode` to `HooCode`.
- Fixed changelog showing old entries on every login by resetting changelog for new project.
