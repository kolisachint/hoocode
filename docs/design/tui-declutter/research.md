# TUI declutter: audits and research (2026-11-03)

These are the findings behind [../tui-declutter.md](../tui-declutter.md). They are summaries of
read-only audits and web research done by subagents. File:line references are to `main @ f9092282`.

## 1. Audits of the current TUI

### Repetition on one screen (from the full-screen and component goldens)

| Fact | Where it appears today |
|---|---|
| Turn state (LANDED, time, steps) | The landing line, the comms frame title, and the recap line (`Ran 2 commands`) |
| Cost | HUD `CR $0.000` and the landing line |
| Context % | HUD `FUEL ▱▱ 0%` and the comms foot `0% of 128k` (plus the footer in inline mode) |
| Tokens | Landing line `120 tokens` and the comms foot `↑100 ↓20` |
| Model, cwd | HUD and the start banner |
| Exit code | `HIT exit 3`, then `Command exited with code 3`, then `exit 3` in the row |
| Thinking text | Printed twice: once on the `·· ● Think` line and again as `✻ …` |
| Esc hint | Attach view: `Esc back` in the header and `Escape back to the main ship` in the prompt |
| Picker scope | Hangar ×3, Mission Log ×3. Systems repeats `4,140` ×3 |

### Glyphs with more than one meaning

| Glyph | Meanings it carries today |
|---|---|
| `●` | done, running, failed (red), queued, full view, toast |
| `◉` | running, and a selected radio button |
| `◐` and `◉` | both mean in flight |
| `◆` | waiting, and attached agent |
| `⊗` | failure, and a retry tick |
| `›` | selection, and the chain separator |

About 45 distinct symbols are in use, and there are three frame styles: heavy `┏━`, light `┌─` and rounded `╭─`.

### Decoration

- The starfield `✦ · ⋆`; the spec itself says `⋆` is unclear.
- Blank `┊` rail rows.
- `≋≋≋`, `✂ ╌╌╌`, the `⊗·⊗··⊗` sparkline, and the `▔` drain bar.
- Zeros: `0 hits`, `0.0s`, `$0.000`, `↑0 ↓0`.

### Bugs seen in the goldens

- The `/color` usage text overflows the toast: a raw `[0m` shows and the right border is lost.
- An agents-list row runs into the frame border.
- The toast borders are misaligned.
- The controls list cuts `(read-on` mid-word.

### Code map (where things are drawn)

| Element | Where |
|---|---|
| HUD | `code-tui-app/src/hud.rs` (`hud_runs`, `tree_plan`) |
| Banner | `mode/mod.rs` `startup_header`, `wordmark.rs` |
| Resource line | `resource_display.rs` |
| Transcript rows | `code-tui-widgets/src/trajectory.rs` (`MissionHeader`, `step_number`, `hit_chip`, `RAIL`, `landing_line`), `tool_chain.rs`, `tool_execution.rs`, `tools/bash.rs` (`Took`), `fold.rs` |
| Radar | `radar.rs` (`FlightMark`) |
| Comms frame | `comms_frame.rs` |
| Footer | `footer.rs` and `chrome_layout.rs` (inline only) |
| Toasts | `transmission.rs`, `notification_panel.rs`, float in `tui-render/src/tui.rs` |
| Cards | `clearance_card.rs`, `end_shift.rs`, `ask_options.rs` |
| Consoles | `code-tui-selectors/*`, `console.rs`, `card_frame.rs`, `controls.rs` |
| Task panel | `task_panel.rs` |
| Glyphs | `tui-glyphs/src/lib.rs` |

- The space vocabulary is hard-coded in about 14 files. There is no plain-words setting.
- Cards and consoles replace the prompt: `mode/dialogs.rs` `show_in_editor_slot` clears the editor container. The task panel is in its own slot.
- The theme came from `mission-control.md` (the user's own design, 2026-10-17). It contains the rule "the metaphor lives in the chrome only". The user chose to go plain on 2026-11-03.

## 2. TUI design principles (web)

Sources:
- clig.dev
- no-color.org
- bixense.com/clicolors
- Textual design guide (textual.textualize.io/guide/design)
- Nielsen's 10 heuristics (nngroup.com)
- WCAG 1.4.1, 1.4.3, 1.4.6, 1.4.11
- lipgloss and bubbletea READMEs
- Unicode TR11

Principles:
- **Say just enough.** clig: too much output drowns what matters. Nielsen #8: every extra unit of information competes with the relevant ones.
- **Colour sparingly and by role.** clig: "the eye will be drawn to red". Textual uses about 8 semantic roles, and makes shades by blending rather than adding hues.
- **Weight and tone before colour.** Use muted text for lower importance.
- **Never colour alone** (WCAG 1.4.1). The glyph shape must carry the state.
- **NO_COLOR disables colour only.** Bold, reverse and glyphs stay allowed (no-color.org FAQ).
- **Non-text contrast of 3:1** for boundaries (WCAG 1.4.11) is a ceiling, not a hard rule, for grouping. Text needs 4.5:1 (AA).
- **Don't log-label.** No `ERR`/`WARN` labels and no extra context unless verbose (clig).
- **Liveness without decoration.** Show elapsed time; no animation in non-TTY output (clig).
- **Glyph width risk.** East Asian Ambiguous-width glyphs can render 1 or 2 cells wide, and font coverage varies.

Numbers measured on the first draft:
- The surface bands were only 1.07–1.11:1, which reads as invisible.
- Two dim tones differed by 1.2:1, so one was redundant.
- The draft used about 9 glyphs and 5 accent colours. These were the main reasons it looked heavy.

## 3. Coding-agent TUIs (web)

Sources:
- code.claude.com docs: fullscreen, interactive-mode, statusline, permissions, terminal-config
- `openai/codex` `codex-rs/tui/styles.md`, `style.rs` and onboarding sources
- opencode.ai/docs/tui
- charmbracelet/crush README and PR #4219
- google-gemini/gemini-cli `AuthDialog.tsx`, PR #772, issue #25879
- aider.chat docs and `onboarding.py`
- HN threads: 47033622, 47036307, 47793411, 45684134, 47495527, 48696585, 47597085, 45986781, 48848274, 48830674

Patterns the well-liked agents share:
- One glyph and one line per tool call, with the result hung under it, folded by default and expandable.
- A global detail toggle (Ctrl+O).
- Status shows only while working (elapsed time, `esc to interrupt`). Idle shows nothing, or one dim line. Cost is rarely prominent.
- Colour is sparse and semantic. Codex `styles.md`: "most of the time, just use the default foreground"; it avoids blue and yellow.
- A fixed prompt at the bottom. Its stability matters more than its shape.
- Numbered permission options that name the exact command, with one hint row.
- Thinking as one dim line.

Complaints users voice:
- Flicker and scrollback jumps.
- Over-folding: "Read 3 files" hid the file names, and users pushed back.
- Hidden actions.
- Approval fatigue.
- Box-heavy UIs (Gemini, Crush, opencode) read as dense.

## 4. Open questions researched (decisions 19–22)

1. **NO_COLOR.**
   - Findings:
     - nvim's monochrome fallback uses `reverse` for Pmenu, PmenuSel and StatusLine.
     - fzf switches to its `bw` theme.
     - Gemini CLI issue #25879: padding was lost when the user-message band vanished.
     - Claude Code issue #82432: state colour leaked past NO_COLOR.
   - Decision: in monochrome, panes get light borders.
2. **First run and sign-in.**
   - Findings:
     - Claude Code, Codex (Welcome → Auth → Trust), Gemini CLI and Crush all block on an auth flow.
     - Aider prints a persistent warning.
     - NN/g: action-required notices should be intrusive.
     - GNOME HIG: toasts are for events, banners are for states.
     - WCAG 2.2.1: avoid time limits on essential information.
   - Decision: Log in opens on its own, and a persistent `⊗ not signed in  /login` stays in the status line.
3. **Split panes.**
   - Findings:
     - nvim float vs Normal is 1.11–1.14:1, and Pmenu is 1.3:1.
     - catppuccin's WinSeparator is 1.17:1.
     - NvChad's borderless telescope keeps a 1-column border cell painted as padding.
     - lazygit and yazi draw rules.
   - Decision: a detail step of about 1.15–1.2:1 plus a 1-column gutter, with a `│` in monochrome.
4. **Light theme.**
   - Findings:
     - catppuccin latte, tokyonight day, github light and solarized light make floats darker than the base.
     - Yellow text on light backgrounds fails: latte yellow is 2.31:1 and solarized yellow 2.98:1.
     - Codex detects the terminal background and avoids yellow on light.
     - Codex issues #16411, #54159 and #45163 show painted backgrounds paired with a default foreground, and stale palettes after an appearance change.
   - Decision: panes darker than the page, dark amber for yellow text, OSC 11 detection with re-checks, and every painted background also sets its foreground.

## 5. Mockup rounds

| Round | What changed | Why |
|---|---|---|
| v1 | Plain words; no borders or rail; light frames | First cut |
| v2 | No HUD (a status line under the prompt instead); one surface colour, one dim tone, 4 glyphs; no right-aligned facts; no body for successful reads | The user said v1 still looked heavy; research findings |
| v2.1 | Blank line after bodies; last-turn result in the status line; view name shown; nesting inside subagents | Review notes on v2 |
| v2.2 | Dialogs take the prompt's place | The user's correction, confirmed in code |
| v2.3 | Notices option B | Notices were added to the review |
| v2.4 / v3 | Panes with their own colours, nvim-inspired: chips, input row, split and grid layouts, pmenu-style completion | The user asked for this |
| v2.5–v2.9 | `◉` glyph, two themes only, light palette, no-colour borders, first run, stronger split | User decisions and research |
