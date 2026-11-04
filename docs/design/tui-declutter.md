# TUI declutter: plain, minimal, easy on the eyes

Status: **design agreed 2026-11-03** ([decisions-2026-11-03.md](decisions-2026-11-03.md)).
Supersedes the look (not the behaviour) of [mission-control.md](mission-control.md),
[radar-r3.md](radar-r3.md), [tui-surfaces.md](tui-surfaces.md),
[footer-two-modes.md](footer-two-modes.md) and [tool-calls-peek-radar.md](tool-calls-peek-radar.md).
Full spec: [tui-declutter/spec.md](tui-declutter/spec.md) (v2 plus amendments v2.1–v2.5;
the latest amendment wins). Mockups (styled text, `«fg=…;bg=…»` runs, 100 columns):
[tui-declutter/mockups/](tui-declutter/mockups/) (`screens/`, `dialogs/`, `notices/`).

## Goal

The TUI states the same fact two to four times per screen, needs a glossary (SHIP, FUEL,
CR, LANDED, HIT, M1, CLEARANCE, TRANSMISSION, HANGAR…), and frames everything in heavy
boxes. Make it plain and calm: each fact once, plain words, separation by colour weight
instead of borders, nvim-style panes for dialogs. Behaviour stays; only presentation and
a few keys change.

## Decisions

| Topic | Decision |
|---|---|
| Vocabulary | Plain words everywhere. No space metaphor. `radar` view is `outline` (`radar` still parses). |
| Fact ownership | Status line (under the prompt): view, session, model, context %, cost, and the live state or last-turn result. Transcript: per-call facts. Prompt: input only. |
| Top of screen | No HUD, no logo. Start screen is two text lines. One blank row at the top. |
| Prompt | One row on the surface colour, ` › `, empty placeholder. Grows with input. No frame. |
| Transcript | `›` user band; thinking one dim line (peek) / all (full) / hidden (outline); tool row `● Name subject  facts`; no step numbers, no rail, no `Took`, no "exited with code"; per-view body rules (spec); no success end line; failed/stopped end line. |
| Glyphs | `●` done/call, `⊗` failed/stopped/denied, `◉` working, `○` todo/queued, `›` marker. Nothing else. |
| Colours (dark) | Text `#e5e5e7`, one dim `#989898`, one accent teal, green/red/yellow for state only. Surface `#2a2a36` (user band, prompt). Panes: bg `#1e1e2a`, detail `#252532`, selection `#233239`, input `#2a2a36`, chips teal / yellow with dark text. Light gets its own tuned pane values. Only `dark` and `light` remain. |
| Dialogs | Panes take the prompt's place (task panel and completion menus keep it). Title chip, rows with a 3-col gutter, input row at the bottom for filterable panes, hint row last (≤4 + `f1 more`). Split list \| detail at ≥100 cols for models, sessions, settings, theme, tree; which-key grid for keys; pmenu-style completion. |
| Permission | `Allow Shell?` yellow chip, `$ command`, `access:` row when it writes, uses the network or pipes; options `1 yes · 2 always for <program> this session · 3 no, tell it why`; `e` edits; letters type a reply. No y/a/n. |
| Question | Question text in a yellow chip, numbered options, `esc` skips (the agent states the default it uses). |
| Notices | No toast. A one-row notice takes the status line's left slot for its timeout; longer ones are dim rows above the prompt (max 3). |
| Themes | Only `dark` and `light`. `daylight`, `deep-space`, `colorsafe-dark`, `colorsafe-light` are deleted. `auto` follows dark/light; the old names and their aliases (`high-contrast-*`, `warm-*`, `solarized-*`) map to `dark`/`light`. |
| Removed | HUD, comms frame, wordmark/starfield, session chip and `/color`, `/chrome` and `alt+z`, toast float, two-row footer layouts (inline mode uses the status line). |
| Scroll/search bars | In the status line's left slot: `scrolled 43–61 of 78 · ↓ 3 new`, `search  query  1 match`. |

## What we build

Four waves; packages in one wave own disjoint files and run in parallel worktrees.

| Wave | WP | Scope | Size |
|---|---|---|---|
| 1 | WP1 | Theme tokens (`surface`, `pane`, `paneSel`, `paneInput`, `paneDetail`, `chip`, `chipWarn`, `chipFg`) for dark and light; delete the four other themes (aliases map to dark/light); glyph set | M |
| 1 | WP2 | `radar` → `outline` (setting alias, enum, names, golden file names, user docs) | M |
| 1 | WP3 | `status_line.rs` component (not wired) | M |
| 1 | WP4 | `pane.rs` primitives; `console.rs`, `card_frame.rs` onto it | L |
| 2 | WP5 | Screen frame: remove HUD/comms/wordmark/footer layouts/`/chrome`/`/color`; prompt row; wire status line; scroll/search in status line | L |
| 2 | WP6a | Tool rows and bodies | M |
| 2 | WP7 | Pickers (models, scoped, settings, theme, login, sessions, tree, fork, config, keys) | L |
| 2 | WP8 | Permission, question (esc skip), quit, task panel | M |
| 3 | WP6b | Transcript structure, outline view, turn result to status line, `layers_*` goldens | L |
| 3 | WP9 | Notices (option B), completion menu (pmenu) | M |
| 4 | WP10 | Leftover words sweep + `scripts/ci/plain_words.sh`, docs (`maps/ui.md`, `docs/user/*`), CHANGELOG, screen goldens | M |

## Not doing

- Renaming internal identifiers beyond the code each WP rewrites anyway.
- New features. Behaviour of views, tasks, attach and queueing is unchanged.

## Open questions

- Sign-in notice: keep it until a model exists rather than 8 s?
- NO_COLOR: panes lose their edges (every edge is a colour step); needs a rule (e.g. a blank row above a pane, title in bold).
