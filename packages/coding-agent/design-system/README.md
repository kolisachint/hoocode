# HooCode Design System

> Deterministic terminal coding agent. The product **is** a TUI — so this design
> system describes a *text-grid* interface, not a graphical one. Every "component"
> is monospaced text painted with ANSI colors and Unicode glyphs. There are no
> overlays, modals, drop-shadows, gradients, or rounded cards. The user is king:
> tokens, tasks, subagents and cost are always shown deterministically on screen.

---

## 1. Product context

**HooCode** (`hoocode` / `hoo` on the CLI) is an interactive coding agent that runs
entirely in the terminal. It is a fork of the upstream **pi-mono** project by Mario
Zechner (`@badlogicgames`), MIT-licensed. The monorepo ships four packages:

| Package | What it is |
|---------|-----------|
| `@kolisachint/hoocode-agent` | The interactive CLI (`hoocode` / `hoo`) |
| `@kolisachint/hoocode-agent-core` | Agent runtime: tool calling + state |
| `@kolisachint/hoocode-ai` | Unified multi-provider LLM API |
| `@kolisachint/hoocode-tui` | Terminal UI library with differential rendering |

The screen is a single scrolling transcript. Top→bottom: assistant text, tool
executions (each in a tinted block), user messages (tinted block), then a pinned
**task panel** + **editor prompt** + **two-line footer** at the bottom. State is
selected with inline list selectors (model picker, session picker, settings,
resource config) that render *in the flow* — never as floating windows.

### Design intent (from the maintainer)
- **Minimal. No bloat, no overlays.** Keep it simple, effective.
- **Deterministic by default.** Always surface tokens, tasks, subagents, cost,
  context % and the auto-compact trip point as plain, legible numbers/glyphs.
- **The user should be aware of everything.** Nothing important is hidden.

### Sources used to build this system
All values were lifted directly from code (not screenshots), at commit `111a592`:

- **Repo:** https://github.com/kolisachint/hoocode  *(explore further to build
  higher-fidelity designs — the TUI components are the source of truth)*
- Upstream: https://github.com/earendil-works/pi-mono (and `badlogic/pi-mono`)
- Themes: `packages/coding-agent/src/modes/interactive/theme/{dark,light}.json`
- HTML export theme: `packages/coding-agent/src/core/export-html/template.css`
- Rendering components: `…/modes/interactive/components/*.ts`
  (footer, tool-execution, diff, task-panel, assistant-message, config-selector…)
- Tool glyphs: `packages/coding-agent/src/core/tools/{read,bash,…}.ts`
- Conventions / tone: `AGENTS.md`, `CONTRIBUTING.md`

---

## 2. Content fundamentals (voice & tone)

The agent and its project culture share one voice, codified in `AGENTS.md`:

- **Short and concise.** Technical prose only. "No fluff or cheerful filler text."
- **Kind but direct.** e.g. `Thanks @user` — never `Thanks so much @user!`.
- **No emoji. Anywhere.** Not in UI, commits, issues, PRs, or code. (This is a
  hard rule and a defining trait — see Iconography.)
- **Lowercase, mechanical labels.** Tool names render lowercase: `read`, `bash`,
  `edit`, `write`, `find`, `grep`, `ls`. The bash tool literally prints `$ <cmd>`.
- **"You" address, imperative voice** in hints: `ctrl+r to expand`, `esc close`,
  `space toggle`, `Type to filter resources`, `Ctrl+P to cycle`.
- **Numbers are first-class and exact.** The footer shows
  `↑12.4k ↓3.2k R45k W1.1k $0.084 (sub) 45.2%/200k (auto @ 90%)` — no rounding
  away of meaning, no "~". Durations: `Took 1.2s` / `Elapsed 4.8s`.
- **Status is stated plainly:** `[Truncated: showing 250 of 1.2k lines]`,
  `Command exited with code 1`, `Operation aborted`, `(3/15)`.
- **Punctuation as structure:** ` • ` joins session metadata, ` · ` separates
  inline hints, `:12-48` appends a line range to a path.

**Vibe:** a precise, unhurried senior engineer pair-programming over SSH. Calm,
legible, never performative. Information density over decoration.

---

## 3. Visual foundations

### Palette & mood
A **Tomorrow-Night-derived** dark palette (the default) plus a clean light theme.
Cool, low-saturation, slightly desaturated — easy on the eyes for long sessions.
The signature color is **aqua `#8abeb7`** (accent): file paths, list bullets, the
active mode tag, the input cursor. Structure is **blue `#5f87ff`** rules and
**cyan `#00d7ff`** headings. Semantics never drift: green = success/added,
red = error/removed, yellow = warning/in-progress/line-numbers, gray/dim = chrome.
See `colors_and_type.css` for every token; both themes are defined there.

### Type
**One family, one size.** Monospaced — the brand ships two self-hosted faces:
**JetBrains Mono** (primary UI/body) and **MesloLGS NF** (a Menlo-derived Nerd
Font used for the wordmark/banner and the classic-console look), falling back to
`ui-monospace`, Cascadia Code, Menlo, Consolas. Body is **12px / 18px line-height (1.5)**; chrome is 11px;
timestamps/counters 10px. **Hierarchy is created with color, bold, and italic
only — never font size.** Bold = tool titles, headings, active list items, labels.
Italic = reasoning/"thinking" traces. The grid is everything: each glyph occupies
one cell, and layouts are built by padding/truncating to the terminal width.

### Backgrounds, surfaces & "cards"
- The canvas is a flat dark fill (`#18181e`); panels/sidebars are `#1e1e24`.
- **No images, gradients, textures, or illustrations. Ever.**
- The closest thing to a "card" is a **tinted block with internal padding (1 row /
  1 col) and NO border, NO shadow, NO radius**:
  - tool pending → `#282832`, success → `#283228`, error → `#3c2828`
  - user message → `#343541`; skill/hook/compaction → `#2d2838`
  - selected list row → `#3a3a4a`
  These tints are *barely* above the canvas — just enough to group, never to pop.
- The HTML export (a web artifact) adds 4px radius and uses the same tints; treat
  4px as the maximum radius anywhere, and prefer 0 inside the TUI itself.

### Borders & rules
- The only border is a **full-width horizontal rule** drawn with `─` (U+2500) in
  the blue `border` color (`DynamicBorder`). Used above/below selectors and the
  loader. No box-drawing rectangles in the main flow.
- The multi-line **editor** is the exception: framed in the muted `border-muted`
  color, and the frame color shifts to encode state — the **thinking-level ramp**
  (`off #505050 → minimal → low → medium → high → xhigh #d183e8`) or **bash-mode
  green** when the prompt starts with `!`.
- Focused single-field input outline → `accent`.

### Spacing & layout
- Unit of vertical rhythm = **one row (18px / one `\n`)**. `Spacer(1)` between
  blocks. Tool blocks pad 1 row top+bottom, 1 col left+right.
- Layouts are **width-aware**: content is right-aligned by computing
  `width - visibleWidth(text)` padding (footer mode tag, model name, list hints),
  and truncated with `…` when it won't fit. Everything reflows to terminal columns.
- Fixed regions pinned to the bottom, in order: **task panel → editor → footer**.

### Motion
Terminals don't animate; the system is almost entirely static. The *only* motion:
- **Braille spinner** `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` cycling at **80ms**, accent-colored, beside a
  muted message ("Working…", "Thinking…").
- A 1s `setInterval` re-render to tick the live `Elapsed Ns` bash timer.
- The HTML export adds one subtle 2s `highlight-pulse` box-shadow on deep-linked
  messages and 0.15s opacity fades on hover affordances — web only.
No easing curves, no bounces, no slide/fade transitions in the TUI.

### Interaction states
- **Selected / focused:** `> ` cursor prefix on the active row + **bold** label;
  or the `--selected-bg` tint behind a row. (Web export: hover = `--selected-bg`.)
- **Toggles:** `[x]` (success) enabled / `[ ]` (dim) disabled.
- **Pressed/hover** don't exist in a terminal — there is no pointer. In the web
  export, hover lightens to `--selected-bg` and shows a copy-link affordance.
- **Disabled / inactive context** is conveyed by `dim`/`muted` color and reduced
  opacity (the export dims off-path tree nodes to 0.5).
- **No transparency or blur** in the TUI (terminals can't). The web export uses a
  flat `rgba(0,0,0,0.5)` scrim only for its mobile sidebar — not part of the TUI.

### Imagery
None native. If a tool returns an image and the terminal supports it (kitty/iTerm),
it's drawn inline capped at 60 cells / 500px; otherwise a text fallback like
`[image/png 1024×768]` is shown. Imagery is functional output, never decoration.

---

## 4. Iconography

HooCode has **no icon font, no SVG set, and no PNG icons**, and — by hard project
rule — **no emoji**. Its entire "icon system" is a small, deterministic set of
**Unicode glyphs** rendered in the monospace grid and colored by semantic token.
This is the authentic, faithful approach — do not introduce icon libraries or
emoji when designing for HooCode. The complete vocabulary:

| Glyph | Codepoint | Meaning | Color token |
|------|-----------|---------|-------------|
| `●` | U+25CF | task pending | `dim` |
| `◐` | U+25D0 | task in-progress | `warning` |
| `✓` | U+2713 | task done | `success` |
| `✗` | U+2717 | task failed | `error` |
| `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | braille | loading spinner (80ms) | `accent` |
| `>` | U+003E | input prompt / selected-row cursor | default / `accent` |
| `$` | U+0024 | bash tool command prefix | `toolTitle` bold |
| `[x]` `[ ]` | ASCII | toggle on / off | `success` / `dim` |
| `─` | U+2500 | full-width rule / divider | `border` |
| `↑ ↓` | U+2191/2193 | input / output tokens (footer) | `dim` |
| `R W` | ASCII | cache read / write tokens | `dim` |
| `+ -` (space) | ASCII | diff added / removed / context | `success`/`error`/`dim` |
| `•` | U+2022 | metadata joiner | `dim` |
| `·` | U+00B7 | hint separator | `muted` |
| `…` | U+2026 | truncation ellipsis | inherits |
| `[skill]` `[invalid arg]` | bracket-tags | inline labels | `customMessageLabel` / `error` |

Because there are no raster/vector assets, the **`assets/` folder holds only an
ASCII wordmark** (`assets/wordmark.txt`) — the brand "logo" of a TUI is its
monospace name. Use the glyph table above as the canonical icon reference.

---

## 5. Index / manifest

Root files:
- **`README.md`** — this file.
- **`colors_and_type.css`** — all color tokens (dark + light) and the type system.
- **`SKILL.md`** — Agent-Skill front-matter so this folder works as a Claude skill.
- **`assets/wordmark.txt`** — ASCII wordmark (the only "logo").

Folders:
- **`preview/`** — small Design-System cards (colors, type, glyphs, components).
  Each is registered to the Design System tab.
- **`ui_kits/hoocode/`** — high-fidelity, interactive HTML/JSX recreation of the
  HooCode terminal: transcript, tool blocks, diff, task panel, footer, editor,
  and an inline model/session selector. Start at `ui_kits/hoocode/index.html`.

Imported reference code (read-only, kept for provenance) lives under
`packages/coding-agent/src/…` — the theme JSON and component `.ts` files this
system was derived from. Explore the full repo at the link in §1 for deeper work.

---

## 6. Caveats
- This is a **terminal** design language. When asked for "a HooCode screen", default
  to the monospace transcript aesthetic — resist web-app instincts (no hero
  sections, cards-with-shadows, gradient CTAs, or emoji).
- The two themes (`dark`, `light`) are the only built-ins; users can supply custom
  theme JSON, but design against `dark` first.
- `JetBrains Mono` and `MesloLGS NF` are self-hosted brand fonts (`fonts/`,
  wired via `@font-face` in `colors_and_type.css`); real terminals use whatever
  monospace font the user has configured.
