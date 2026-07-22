<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="../assets/hoocode.svg">
    <img alt="HooCode" src="../assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Product &amp; visual testing methodology for the HooCode CLI.</p>

# Product + UI Testing

This is a **discussion / design document**. It defines how the **HooCode-Test**
agent validates HooCode as a *product* — by running the real compiled binary,
driving the terminal UI, and treating **captured output** and **UI element
changes** as the evidence of pass/fail. It complements, but does not replace,
the unit and integration suites described in [`AGENTS.md`](../AGENTS.md) and
[`npm-packages.md`](./npm-packages.md).

The distinction that drives everything below:

- A **unit test** proves a function returns the right value.
- A **product test** proves the compiled `hoocode` CLI does the right thing
  end-to-end, with real data, in a real terminal.
- A **visual test** proves the *pixels the user sees* — menus, prompts, status
  bar, spinners, colour, alignment — render correctly and stay readable.

For product and visual tests the deliverable is not a green assertion; it is
**evidence**. Every feature check below names (a) the concrete action, (b) the
**outcome captured as evidence** (a terminal snapshot, a diff, a JSON stamp),
and (c) the **UI elements that must change** for the feature to count as
working.

---

## 1. The HooCode-Test agent

**Identity.** HooCode-Test is an autonomous testing persona whose job is to
validate both code correctness and product behaviour. It does not trust the
source; it runs the binary and looks at the screen.

**Responsibilities.**

- Execute the unit, integration, and end-to-end suites.
- Run the **real** `hoocode` CLI in an isolated temp environment.
- **Visually inspect** the TUI: modes, permission prompts, the task panel, the
  footer, diffs, selectors, spinners, colour output, and layout alignment.
- Validate agent orchestration — mode switching, tool calling, subagent
  dispatch — through observable UI state, not internal logs.
- Detect regressions across three surfaces at once: **code**, **CLI
  behaviour**, and **visual output**.

**Testing philosophy (four layers).**

| Layer | Proves | Where |
|---|---|---|
| Unit | functions work in isolation | `packages/*/test/*.test.ts` (vitest) |
| Integration | modules connect correctly | `packages/coding-agent/test/suite/` + faux provider |
| **Product** | the compiled binary works end-to-end | real `hoocode` run in a temp dir |
| **Visual** | UI elements render and are human-readable | tmux capture + snapshot diff |

**Non-negotiables.**

- Product and visual runs use a **deterministic, non-paid** path — the faux
  provider or a recorded fixture, never real API keys or paid tokens (see
  [`AGENTS.md`](../AGENTS.md), *Testing*). `./hoocode-test.sh --no-env` unsets
  every provider key so a run cannot silently reach a live provider.
- Fix the terminal geometry (`-x 80 -y 24`) so captures are reproducible and
  diffable.
- No test mutates the developer's real working tree — everything runs in an
  isolated temp directory.

---

## 2. Test environment & harness

HooCode is a TypeScript / Bun monorepo; the interactive UI is a custom ANSI
renderer (the `@kolisachint/hoocode-tui` differential renderer — see
[`ui-map.md`](./ui-map.md)), **not** Ratatui or Ink. Visual testing therefore
means capturing the rendered terminal cells, not screenshotting a GUI.

The project already ships the primitives this methodology depends on:

- `./hoocode-test.sh` — runs the CLI **from source** via `tsx`, with a
  `--no-env` flag that strips all provider credentials.
- The **tmux capture flow** documented in [`AGENTS.md`](../AGENTS.md) — a
  controlled terminal at a fixed size that can be driven with `send-keys` and
  read back with `capture-pane`.

### 2.1 Canonical capture loop

```bash
# 1. Fresh, sized terminal — geometry is part of the fixture.
tmux new-session -d -s hoocode-test -x 80 -y 24

# 2. Launch the product in an isolated temp dir, no live credentials.
tmux send-keys -t hoocode-test \
  "cd $(mktemp -d) && <repo-root>/hoocode-test.sh --no-env" Enter

# 3. Let it start, then capture the frame as evidence.
sleep 3 && tmux capture-pane -t hoocode-test -p > evidence/00-startup.txt

# 4. Drive a feature.
tmux send-keys -t hoocode-test "/mode plan" Enter
sleep 1 && tmux capture-pane -t hoocode-test -p > evidence/01-mode-plan.txt

# 5. Special keys (Esc, ctrl+o, etc.) exercise chrome and dialogs.
tmux send-keys -t hoocode-test Escape
tmux send-keys -t hoocode-test C-o

# 6. Always tear down.
tmux kill-session -t hoocode-test
```

Each `capture-pane` frame under `evidence/` **is** the outcome. A visual test
passes when the current frame matches the approved baseline; it fails with an
attached inline diff of the two frames.

### 2.2 Component-level visual tests (fast path)

Not every UI check needs a live binary. Many TUI components already expose a
`render(width)` method returning styled lines, and the suite snapshots them
directly (e.g. `task-panel.test.ts`, `footer-width.test.ts`,
`tool-execution-component.test.ts`, `diff` rendering). Prefer this path for
pure-layout regressions — it is deterministic, sub-second, and needs no tmux.
Reserve the full binary capture (2.1) for behaviour that only emerges when the
real agent loop, permission gate, and renderer run together.

### 2.3 Evidence layout

```
evidence/<run-id>/
  frames/          # capture-pane .txt frames, zero-padded & ordered
  diffs/           # inline unified diffs vs. baseline (only on failure)
  report.md        # human-readable pass/fail/block report
  report.json      # CI-consumable metadata (see §5)
```

---

## 3. Feature-by-feature test matrix

Each feature below is a self-contained check. **Action** is what HooCode-Test
does; **Outcome (evidence)** is the artifact captured; **UI elements changed**
is the visual delta that must appear for the feature to pass. Component paths
reference [`ui-map.md`](./ui-map.md).

### 3.1 Mode switching — Ask · Plan · Build · Debug

- **Action:** launch (defaults to `build`), then `/mode ask`, `/mode plan`,
  `/mode debug`.
- **Outcome (evidence):** one frame per mode showing the mode indicator, plus a
  behavioural probe — in `ask`, a "modify this file" prompt must be *refused*
  (read-only); in `plan`, the agent writes `.hoocode/plan.md`.
- **UI elements changed:** the **footer** mode segment (`footer.ts`) updates its
  label/colour per mode; the **keybinding hint strip** (`keybinding-hints.ts`)
  reflects the active mode's affordances. Evidence = before/after footer frames.

### 3.2 Permission gate — Yes (once) / No (block) / Always

- **Action:** in `build`, ask the agent to run a shell command and to edit a
  file; drive the gate with each of the three answers.
- **Outcome (evidence):** frame of the gate prompt with all three options
  visible; a follow-up frame proving `No` **blocked** the action and `Always`
  suppressed the prompt on the next identical action.
- **UI elements changed:** the permission prompt appears as a modal row over the
  transcript, then the **tool-execution row** (`tool-execution.ts` /
  `bash-execution.ts`) resolves to an allowed/blocked state. This is HooCode's
  core "you stay in control" guarantee — it gets a dedicated, always-run check.

### 3.3 File tools — `read` · `write` · `edit` and the diff view

- **Action:** have the agent create a file (`write`), then make an exact-text
  `edit`, including a `replaceAll` edit.
- **Outcome (evidence):** the rendered **unified diff** frame for the edit;
  on-disk verification that the temp file's contents match.
- **UI elements changed:** the **diff component** (`diff.ts`) renders
  added/removed lines with the correct gutter and colour; a multi-replacement
  edit shows every hunk. Snapshot the diff frame against baseline.

### 3.4 Search tools — `grep` · `find` · `ls`

- **Action:** run each against a seeded temp tree containing a `.gitignore`d
  directory (e.g. `node_modules`).
- **Outcome (evidence):** captured tool-result rows; assert `grep`/`find`
  **respect `.gitignore`** and `ls` honours its `ignore` list.
- **UI elements changed:** the **tool-execution row** shows the tool name, the
  query, and a result summary that collapses/expands correctly at 80 columns.

### 3.5 Subagents (Task) & TodoWrite — the task panel

- **Action:** trigger a subagent dispatch and a TodoWrite list.
- **Outcome (evidence):** frames cycling the **task panel's** three views
  (flat → subagents → teams) via `ctrl+n`.
- **UI elements changed:** `task-panel.ts` shows status icons, usage stamps, the
  warning cue, and correct grouping by owning agent. Baselined per view. Reuse
  the existing `task-panel.test.ts` / `task-panel-team-focus.test.ts` component
  snapshots for the fast path.

### 3.6 Model selection — `/model` and `/models`

- **Action:** open the single `/model` picker and the scoped `/models`
  enable-set picker; cycle models with the cycle key.
- **Outcome (evidence):** selector frames; the **footer** available-provider
  count before/after.
- **UI elements changed:** `model-selector.ts` / `scoped-models-selector.ts`
  open as modal pickers; `footer.ts` updates the active model and provider
  count. (`model-controller.ts` owns this flow.)

### 3.7 Auth flows — `/login` and `/logout`

- **Action:** open `/login` (drive with the **faux** provider — never real
  OAuth or keys), then `/logout`.
- **Outcome (evidence):** the auth-type selector and API-key/OAuth dialog
  frames; post-logout state.
- **UI elements changed:** `login-dialog.ts` / `oauth-selector.ts` render;
  `login-controller.ts` drives the sequence. Reuse `oauth-selector.test.ts` for
  the component-level snapshot.

### 3.8 `ask_options` decision prompt

- **Action:** force a run where the agent needs a decision (interactive), and a
  `-p` non-interactive run of the same scenario.
- **Outcome (evidence):** interactive frame shows the options pane with the
  `recommended` marker; the `-p` frame proves it **falls back to proceeding on
  its own** with no prompt.
- **UI elements changed:** `ask-options.ts` renders the multiple-choice pane
  interactively and is absent in `-p`. (`ask-options.test.ts` /
  `ask-options-loop.test.ts` cover the logic.)

### 3.9 Bash prompt mode — `!cmd`

- **Action:** enter `!` prompt mode, run a command, and run one *while the agent
  is streaming*.
- **Outcome (evidence):** frame of streamed output in the
  `BashExecutionComponent`; a second frame showing a command started
  mid-stream **parked in the pending area**, then moved into the transcript when
  the turn ends.
- **UI elements changed:** `bash-execution-controller.ts` +
  `bash-execution.ts`; verify width handling with `bash-execution-width.test.ts`.

### 3.10 Off-by-default tool groups — Web · Browser · Documents · Plugins

- **Action:** confirm each group is **absent** by default, then enable via flag
  (`--enable-webtools`, `--enable-browsertools`, `--enable-filetools`,
  `--enable-plugintools`) and confirm it appears.
- **Outcome (evidence):** paired "off" / "on" frames of the resource/tool
  listing (`resource-display.ts`).
- **UI elements changed:** the startup resource listing gains the enabled
  group's tools; browser/plugin flows surface their own rows when exercised.

### 3.11 Chrome & ambient UI — footer, hints, spinners, theme

- **Action:** idle, stream a response, open the theme selector
  (`theme-selector.ts`), resize considerations at 80×24.
- **Outcome (evidence):** frames of the **footer** (`footer.ts`), **hint strip**
  (`keybinding-hints.ts`), and an in-flight **loader/spinner**
  (`bordered-loader.ts` / `cancellable-loader`).
- **UI elements changed:** spinner animates without corrupting the frame; footer
  segments (mode, model, usage) stay aligned; switching themes recolours the
  whole transcript via `theme.fg(...)`. Alignment is validated by
  `footer-width.test.ts` and the width helpers in `tui/src/utils.ts`.

### 3.12 Layout / truncation safety at 80 columns

- **Action:** feed long paths, wide tables, and CJK/emoji content.
- **Outcome (evidence):** frames proving no line exceeds the terminal width and
  no wrap corrupts the differential render.
- **UI elements changed:** truncation ellipsis appears where expected;
  `visibleWidth` / `truncateToWidth` (`tui/src/utils.ts`) keep multi-byte and
  ANSI-styled text correctly measured.

---

## 4. Regression: UI element changes as outcome

A visual regression is defined as **any unintended change to a captured
frame**. The workflow:

1. **Baseline.** For each feature above, store the approved frame(s) under
   `evidence/baseline/`.
2. **Re-run.** On each test pass, capture fresh frames at the identical geometry
   and driving sequence.
3. **Diff.** Compare byte-for-byte (after stripping volatile fields — see
   below). A mismatch is a candidate regression.
4. **Adjudicate.** An *intended* UI change updates the baseline in the same
   commit, with the frame diff shown in the report as the evidence of the
   change. An *unintended* change fails the run with an attached inline diff.

**Volatile fields** (timestamps, elapsed timers from `countdown-timer.ts`,
usage token counts, session ids) are masked before diffing so they never
produce false regressions. Everything else — every glyph, colour, and column —
is significant.

This is the sense in which **UI element changes are the outcome**: the test does
not assert "the footer is correct"; it asserts "the footer frame equals the
approved footer frame," and the *diff itself* is the pass/fail artifact.

---

## 5. Report format

Every product/visual run emits **both** a human report and machine metadata.

### 5.1 Markdown report (`report.md`)

Statuses are **PASS / FAIL / BLOCK** (BLOCK = could not run: environment,
missing binary, unavailable faux provider).

```markdown
# HooCode-Test — Product & Visual Run <run-id>
Geometry: 80×24 · Provider: faux · Binary: hoocode-test.sh --no-env

| # | Feature | Status | Evidence |
|---|---------|--------|----------|
| 3.1 | Mode switching | PASS | frames/01-mode-plan.txt |
| 3.2 | Permission gate | PASS | frames/03-gate-*.txt |
| 3.3 | File tools + diff | FAIL | diffs/diff-render.diff |
| ... | ... | ... | ... |

## 3.3 File tools + diff — FAIL
Expected the `replaceAll` edit to show two hunks; only one rendered.
<inline unified diff of baseline vs. current frame>
```

Failures attach the inline frame diff (the terminal-capture equivalent of a
screenshot). Visual snapshot regressions attach the masked before/after frames.

### 5.2 JSON metadata (`report.json`) for CI

```json
{
  "runId": "2026-07-22T00-00Z-abc",
  "geometry": { "cols": 80, "rows": 24 },
  "provider": "faux",
  "summary": { "pass": 10, "fail": 1, "block": 0 },
  "features": [
    {
      "id": "3.2",
      "name": "Permission gate",
      "status": "PASS",
      "evidence": ["frames/03-gate-prompt.txt", "frames/04-gate-blocked.txt"],
      "uiElements": ["permission-prompt", "tool-execution-row"]
    },
    {
      "id": "3.3",
      "name": "File tools + diff",
      "status": "FAIL",
      "evidence": ["frames/05-diff.txt"],
      "diff": "diffs/diff-render.diff",
      "uiElements": ["diff.ts"]
    }
  ]
}
```

CI gates on `summary.fail == 0 && summary.block == 0`.

---

## 6. Scope & guardrails

- **Discussion/spec only.** This document defines methodology; it does not add
  or wire up a test runner. Implementing the `evidence/` harness, baseline
  store, and CI job is follow-up work.
- **No paid calls, ever.** Product and visual runs use the faux provider or
  recorded fixtures. `--no-env` is the belt-and-braces guarantee.
- **Deterministic geometry.** All captures are 80×24 unless a feature
  explicitly tests reflow; geometry is recorded in every report.
- **Isolation.** Every run executes in a fresh temp dir and a dedicated tmux
  session that is always torn down.
- **Reuse the fast path.** When a component exposes `render(width)`, snapshot it
  directly (§2.2) before reaching for a full binary capture.

## References

- [`AGENTS.md`](../AGENTS.md) — testing rules, faux provider, tmux capture flow.
- [`ui-map.md`](./ui-map.md) — where every UI component lives.
- [`product.md`](./product.md) — the feature set under test (modes, tools,
  permission gates, extensibility).
- [`npm-packages.md`](./npm-packages.md) — build/test mechanics and the
  src-vs-dist split.
