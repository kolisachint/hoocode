# Decisions 2026-11-03: TUI declutter

Card: [tui-declutter.md](tui-declutter.md). Process: four audits (screens, components,
code map, design history), web research (TUI principles; coding-agent TUIs), three mockup
rounds reviewed side by side with the real goldens.

| # | Question | Answer |
|---|---|---|
| 1 | How far does the space metaphor reach? | Go plain: remove it everywhere. |
| 2 | Who owns each status fact? | The most minimal option: status line under the prompt; no HUD. |
| 3 | Startup banner | Text only (`hoocode v…` + one dim line). |
| 4 | Idle prompt placeholder | Empty. |
| 5 | Recap line under a turn | Only when steps are hidden (outline). |
| 6 | How to run | All tiers in one go, in worktree `agent/tui-declutter`. |
| 7 | View names | `radar` → `outline`; `peek`, `full` stay; `radar` still parses. |
| 8 | Separation | Background and text colour, not borders (user). |
| 9 | Dialogs and the prompt | A dialog takes the prompt's place (as the code does); the task panel and completion menus keep it. |
| 10 | Panes | Own colours, nvim-inspired, split list \| detail layouts (user). |
| 11 | Notices | Option B: status line slot, rows above the prompt for longer ones. |
| 12 | Working glyph | `◉` (`◐` is missing from JetBrainsMono Nerd Font). |
| 13 | Themes | Keep only `dark` and `light`, both tuned. Delete `daylight`, `deep-space`, `colorsafe-dark`, `colorsafe-light`; their names and aliases map to `dark`/`light`; `auto` follows dark/light. |
| 14 | `/color` | Remove. |
| 15 | Permission keys | `1/2/3` only; letters type a reply. |
| 16 | `/chrome`, `alt+z` | Remove; a saved `chromeDensity` is ignored. |
| 17 | Scroll and search bars | Status line left slot. |
| 18 | Question `esc` | Skip (was: pick option 1); the agent states the default it uses. |
