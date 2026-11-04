# hoocode TUI declutter spec v2 (lighter)

v1 still looked heavy. Research (clig.dev, Textual design guide, WCAG, Nielsen, and the
Claude Code / Codex / opencode / Gemini CLI / Crush comparisons) points to: too many
tones, too many glyphs, too many filled surfaces, facts shown while idle, chrome on
every tool call. v2 fixes exactly those. Same content as before; only presentation.

## Palette: 2 text tones, 1 surface, 1 accent, 3 state colours
- background: terminal default (render as `#14141c`)
- text `#e5e5e7`; dim `#989898` (the only secondary tone; `#a8a8a8` is gone)
- surface `#2a2a36`: the ONLY fill. Used for: user messages, the prompt, the selected
  row in a dialog. Nothing else has a background. (No top bar fill, no dialog tint.)
- accent teal `#5cc8bb`: the `›` marker (user messages, prompt, selected row) and inline
  code. Nothing else.
- state colours, only on glyphs and state words: green `#b5bd68` done, red `#d58181`
  failed, yellow `#f0c674` working / needs you.
- violet, blue and session colours are not used. The session name is dim text.
- NO_COLOR: shapes, bold and dim only; the surface becomes nothing (the `›` still marks
  user text and the prompt).

## Glyphs: 4 shapes + 1 marker
- `●` a tool call (green when done; dim while it runs)
- `⊗` failed (red); a stopped turn or a denied call is a dim `⊗` with the word
  `stopped` / `denied`
- `◐` working, only in the status line and the tasks list (yellow)
- `○` not started / queued (dim)
- `›` user text, prompt, selected row (teal)
- Gone: `◉ ⊘ ✻ ▸ ◈ ◆ ▲ ┊ ╰ └ ━ ┃` and step numbers.

## Layout (top to bottom)
- **No top bar.** The first row is the transcript.
- **Transcript.**
- **Dialog** (when open), directly above the prompt.
- **Prompt**: 1 row on the surface, ` › ` then the input (empty when idle). Grows with
  the input.
- **Status line**: 1 row, no fill, under the prompt. Left: live state, only while
  something happens: `◐ working · 4s · esc stop` (◐ yellow, rest dim). Right, always,
  dim: `amber-harbor · claude-haiku-5-5 · 42% context · $1.23`. In plan mode a bold
  `plan` word comes first on the right. Narrow terminal drops, in order: cost, session
  name, `context` word (keep `42%`), model effort.
- **Start screen** (empty transcript), two dim-and-bold lines at the top, nothing else:
  ` hoocode v0.2.31` (bold) and ` /work/hoocode · 1 skill · 5 agents` (dim).

## Transcript
- **User message**: surface rows full width, ` › ` teal, text normal weight `#e5e5e7`;
  wrapped lines indent 3. One blank line before it (none at the top of the screen).
- **Thinking**: peek: one dim italic line, indent 1, first sentence cut with `…`. full:
  the whole text, dim italic. outline: hidden. No glyph, no header.
- **Tool call row**: ` ● Read src/config.rs  30 lines` — glyph, tool name bold, subject,
  then two spaces and dim facts directly after the subject (NOT right-aligned; no
  column jumping). Edit facts `+2 −2` (+ green, − red). Shell facts `exit 0 · 8.2s`.
  Failed: ` ⊗ Shell cargo test -p hoocode-parser escaped_quote  exit 1 · 3.1s` with
  `⊗` and `exit 1` red.
- **Consecutive tool rows have no blank line between them.** One blank line separates
  a group of tool rows from prose.
- **Tool body** (indent 4, dim, no rail, no line-number gutter on Read):
  - peek: Read / Grep / search / successful agent calls: no body at all.
    Edit: up to 4 diff lines (− red sign, + green sign), then `… N more lines`.
    Shell success: last 2 output lines. Failed anything: last 4 lines (the error).
    Bodies of 4 lines or fewer are shown whole, never a fold.
  - full: every line.
  - outline: no bodies.
  - Fold row: `    … 27 more lines` dim (no italic).
- **Prose**: indent 1, normal text.
- **End of turn**: nothing on success. Failed: ` ⊗ failed · 5.0s` (⊗ and `failed` red,
  rest dim). Stopped: dim ` ⊗ stopped · 2.0s`. Tokens and cost live in the status
  line only.
- **Outline view**: user message; one row per batch of calls listing subjects, e.g.
  ` ● Read src/config.rs · Edit src/config.rs · Shell cargo test` (no count); a failed
  call gets its own red row ` ⊗ Grep \\"  regex parse error: unclosed group`; then the
  prose; the failed/stopped end line as above.
- **Errors** (provider): ` ⊗ connection reset by peer` red glyph, dim text.
- **Queued messages**: dim rows directly above the prompt:
  ` ○ steer this way · sends after this step`.

## Dialogs (all of them)
- Sit directly above the prompt. No fill, no border. One blank row above the title.
- **Title row**: bold text, then dim `/command` if a command opens it, then dim facts
  directly after (not right-aligned). A dialog that blocks the agent has its title in
  yellow bold (permission, question); no glyph.
- **Rows**: indent 3. Selected row: surface fill full width, ` › ` teal, text normal.
  Secondary columns dim. No column-header rows unless the columns would be unclear.
- **Filter**: no empty filter row. Typed text appears after the title: `filter: son`.
- **Hint row** (last, dim): at most 4 items `key action` with ` · ` between, keys in
  normal text, then `? more` if there are more keys. Lowercase key names.
- **Permission**: title `Allow Shell?` (yellow bold); row ` $ printf 'alpha\nbeta' | sort -r`
  (normal text, `$` dim); an `access:` row only if it writes or uses the network;
  options as rows: `1 yes` / `2 yes, always for printf this session` / `3 no, tell it
  why`; hint `1–3 pick · e edit first · or type a reply`.
- **Question**: title = the question text (yellow bold); numbered option rows with a
  dim description after; hint `1–3 pick · tab next question · esc pick 1`.
- **Tasks** (unfocused): rows only, no title, no tab row when one tab has items;
  `○` todo dim, `◐` working yellow, `●` done green with text dim, `⊗` failed red; agent
  rows ` ◐ explore  Trace the auth flow  12s`. With several tabs, one dim first row
  `plan 1/3 · agents 1/3 · mcp 1 · ctrl+g`.
- **Quit with jobs**: title `Quit? 2 background jobs running`; job rows; hint
  `k stop jobs and quit · l leave them running · esc stay`.
- **Pickers** (models, settings, sessions, theme, tree, fork, login, scoped models,
  keys) and completion dropdowns (slash, files): same rules. Plain titles: Models,
  Settings, Sessions, Theme, Tree, Fork, Log in, Scoped models, Keys. Show details
  (price, context) only on the selected row, dim after its name.

## v2.1 amendments
1. **Blank line after a tool row that shows a body.** Consecutive body-less tool rows stay
   tight (no blank between them); a row with a body is followed by one blank line.
2. **Status line left slot, idle after a turn:** the last turn's result, dim, until the next
   prompt is sent.
   - success: `● done · 11s · 23k tokens` (● green)
   - failed turn: `⊗ failed · 5.0s · 14k tokens` (⊗ red)
   - finished with a failed call: `● done · 8.5s · 7.1k tokens · 1 failed call`
     ("1 failed call" red)
   - while working, `◐ working · 4s · esc stop` replaces it.
   The rest of the status line's width rules still apply: when the left and right sides do
   not fit with a 2-column gap, the right side drops parts in the narrow-terminal order
   (cost, session name, `context` word, model effort).
3. **Calls made inside a subagent** are indented 3 under the Agent row
   (`   ⊗ Grep \\"  regex parse error: unclosed group`).
4. **Status line right side always starts with the view name**, dim: `peek · `,
   `outline · `, `full · `, then the session facts.
5. **One blank row at the very top** of every transcript screen, so the screen does not
   open with a surface band at the edge. The start screen is unchanged (its text is on row 1).

## v2.1 amendments (dialogs)
- **Comparable fact per row**: every picker row shows one short fact that lets rows be
  compared (models: price; scoped models: `on` green / `off` dim; sessions: age).
  Full details (provider, context, effort, category, counts) only on the selected row.
- **Aligned columns**: left-aligned columns are allowed whenever every row has the
  same fields (question option descriptions, sessions, settings values, theme
  descriptions, tree roles, login states, quit job commands, keys). Facts that follow
  a single subject still come directly after it with two spaces.
- **Hint overflow**: the last hint item is `f1 more` (not `?`, which types into filters).
  Still at most 4 items before it.
- **Permission access row**: shown when the command writes, uses the network, or runs
  more than one program, e.g. `access: pipes 2 programs` (dim, under the `$` row).
- **Question**: hint `1–3 pick · tab next question · esc skip`. On skip the agent states
  the default it uses (behaviour change from `esc pick 1`).
- **Selected row**: its cells and facts use normal text, not dim (dim on the surface
  reads too weak). The ` · ` separators stay dim.
- **`◐` in live job lists**: allowed in any list of running jobs (Quit dialog job rows,
  yellow), in addition to the status line and the tasks list.
- **Screen frame**: one blank row at the very top. The status line's right side starts
  with the view name: `peek · amber-harbor · claude-haiku-5-5 · 42% context · $1.23`.
  Status left slot: `◐ working · 4s · esc stop` while running. When idle, the last turn
  result is dim with the glyph coloured: `● done · 11s · 23k tokens` (or `⊗ failed …`).
  Empty while a permission or question is open (the yellow title already says it needs
  you).

## v2.2: dialogs take the prompt's place

Cards and consoles open in the prompt's slot (`show_in_editor_slot` clears the editor
container), so while they are open the prompt row is not shown. Applies to permission,
question, quit, models, settings, sessions, theme, tree, fork, login, scoped_models and
keys: the dialog is the last content row, followed only by the status line (chrome, like
the HUD). Task panels sit in a separate slot above the prompt, and the slash and file
dropdowns need the typed text, so tasks (all variants), slash and files keep the prompt
row. In the before screens the comms frame is likewise dropped for the 12 replacing
dialogs.

## v2.3 notices (today's "transmission toasts")

No box, no `● TRANSMISSION` title, no `▔` drain row, no glyph for info; the timeout is not
drawn (Info 3 s, Warning 8 s; newest shows, the queue waits). Keys after the text with two
spaces: key normal, action dim (`chrome is now compact  shift+alt+z undo`). A notice is
parts (message, hint): one row if they fit, else each part wraps at spaces; max 3 rows, the
3rd ends in `…` (fixes the `/color` usage overflow). Mockups: `after-notices-v2/`.
- **Option A, float:** rows on the surface, 1-space pad, top right under the blank top row,
  right edge at col 99, width = longest row (max 46). Warning: first row yellow bold.
- **Option B, status line (recommended):** a one-row notice takes the status line's left
  slot for its timeout (the last-turn result returns after; the right side drops parts in
  the narrow order to fit). Multi-row or too-long notices are rows directly above the
  prompt (indent 1, no fill, one blank row above, above any queued rows): first row yellow
  for a warning (bold if it is a title), the rest dim. While a turn runs, `◐ working` keeps
  the slot and notices use the above-prompt rows.
- Why B: no overlap with transcript text (A's borderless float merges with user-message
  bands and is unreadable under NO_COLOR), the answer appears next to the prompt where the
  command was typed, and the common one-row case causes no layout shift.

## v2.4: panes get their own colours (nvim-inspired)
User (2026-11-03): dialogs, settings and pickers are panes within the screen and should
read as separate surfaces, easy on the eyes; take inspiration from nvim for complex layouts.

- **Elevation scale** (supersedes "one surface" for panes only): base (transcript, prompt
  area) → pane bg (every dialog, picker, tasks panel, completion menu) → pane selection →
  pane input field. Steps must be visible (research: 1.07:1 bands disappear) but calm.
  Pick hexes from how well-liked nvim themes (tokyonight, catppuccin, kanagawa, rose-pine,
  gruvbox-material) set Normal / NormalFloat / FloatBorder / FloatTitle / Pmenu / PmenuSel /
  TelescopePromptNormal / TelescopeResultsNormal / TelescopePreviewNormal, in our hue
  family (cool grey-violet, base #14141c). No borders; the colour step is the edge (the
  "borderless telescope" style).
- **Title chip** (telescope-style): the pane title as a small chip with accent bg and dark
  fg, bold (` Models `), then dim `/command` and facts on the pane's title row. Blocking
  panes (permission, question) use a yellow chip.
- **Pane input row at the bottom** of a filterable pane, where the prompt was (the pane
  takes the prompt's place, so the eye stays put): input shade, ` › ` teal, dim
  placeholder `filter` until typing. Non-filterable panes have no input row.
- **Hint row**: inside the pane, its last row (below the input row), dim, ≤4 items +
  `f1 more`.
- **Split layouts for complex panes** (telescope results | preview; lazy.nvim / mason
  floats): at ≥100 cols, a list column on the left and a detail column on the right,
  separated by a different shade (no line). Models: list | details of the selected model.
  Sessions: list | first messages of the selected session. Settings: list | description +
  current value. Theme: list | a live preview of a few transcript rows in that theme.
  Tree: tree | the selected message. Keys: which-key style grid (3 columns of `key
  action`, grouped). Below 100 cols: stacked (detail under the list).
- **Inner padding**: 1 column left/right inside a pane; no blank padding rows.
- **Completion menu** (slash, files): pmenu style: a pane-coloured block directly above
  the prompt, its width the longest row (not full width), left edge aligned under the
  typed token; selected row in pane selection colour.
- **Tasks panel**: pane colour, no title chip when unfocused; focused it gets the chip
  ` Tasks ` and the hint row.
- **Notices** stay option B (status line / rows above the prompt), not panes.

## v2.4 palette
Chosen in `after-dialogs-v3/` (reasons and nvim sources in its NOTES.md). Values are WCAG
contrast of each text colour on that surface; every text/surface pair is ≥4.5:1.

| role | hex | text | dim | teal | green | red | yellow | vs base |
|---|---|---|---|---|---|---|---|---|
| base (transcript, status line) | `#14141c` | 14.56 | 6.35 | 9.10 | 9.11 | 6.35 | 11.37 | 1.00 |
| pane bg (dialogs, pickers, tasks, completion) | `#1e1e2a` | 13.10 | 5.71 | 8.19 | 8.20 | 5.72 | 10.23 | 1.11 |
| detail column (split panes) | `#252532` | 12.02 | 5.24 | 7.51 | 7.52 | 5.24 | 9.38 | 1.21 |
| pane selection (pane + 13% teal) | `#233239` | 10.52 | 4.59 | 6.58 | 6.58 | 4.59 | 8.21 | 1.38 |
| pane input row = prompt surface | `#2a2a36` | 11.26 | 4.91 | 7.04 | 7.04 | 4.91 | 8.79 | 1.29 |
| accent chip | bg `#5cc8bb`, fg `#14141c` bold | 9.10 | | | | | | |
| blocking chip (permission, question) | bg `#f0c674`, fg `#14141c` bold | 11.37 | | | | | | |

Steps: pane→selection 1.25, pane→detail 1.09, pane→input 1.16, selection→input 1.07 (hue
differs: teal tint vs neutral). Pane goes lighter than base (rose-pine style) because base is
too dark for a darker float to show; selection hue from kanagawa/tokyonight; chips from
catppuccin `float.solid`; pmenu selection adds bold (catppuccin `PmenuSel`).

## v2.5 decisions (user, 2026-11-03)
- Working glyph is `◉` (not `◐`, which JetBrainsMono Nerd Font lacks). `◐` is not used
  anywhere. "Needs you" has no glyph: the yellow title chip says it.
- Pane colours: tune the `dark` theme only for now; other themes map the new pane tokens
  to their existing `selectedBg`.
- `/color` and session colours are removed (command, chip, hull paint).
- Permission keys: `1/2/3` only; letters type a reply. No y/a/n aliases.
- `/chrome` and `alt+z`/`shift+alt+z` are removed; a saved `chromeDensity` is ignored.
- Scrolled-back and search bars live in the status line's left slot:
  `scrolled 43–61 of 78 · ↓ 3 new` and `search  query  1 match` (keys after, dim).
- Defaults taken: notices option B; question esc = skip; `radar` → `outline` (old value
  accepted); failed turns keep the transcript end line and the status-line result; no logo,
  no step numbers.

## v2.6 themes (user, 2026-11-03)
Only `dark` and `light` remain, both tuned with the pane tokens. `daylight`, `deep-space`,
`colorsafe-dark` and `colorsafe-light` are deleted. Their names and aliases
(`high-contrast-*`, `warm-*`, `solarized-*`) map to `dark` / `light`; `auto` follows
`dark` / `light`. This replaces v2.5's "tune dark only".
