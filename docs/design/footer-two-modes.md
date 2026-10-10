# Footer: two modes

Date: 2026-10-10
Status: **design agreed, implementation in progress.**

The footer goes from three densities (full, compact, bare) to two: **detailed** and **short**.
Both are readable at a glance, and the left edge always holds the most useful visuals.

## Current state (facts, with file refs)

- Footer: `FooterComponent::render_lines`, `crates/hoocode-code-tui-app/src/footer.rs:546`.
  Densities `FooterDensity::Full` and `FooterDensity::Line`; `set_density` (~498).
- Alt+Z chrome dial: `cycle_density` at `src/chrome_layout.rs:114-129`.
  Three stops: full → compact → bare, then wraps. Shift+Alt+Z steps back.
  Stop → layout mapping: `resolve_chrome` at `src/chrome_layout.rs:50-69`.
- `/chrome <full|compact|bare>`: `src/mode/chrome.rs:234-262`.
- Saved in settings key `chromeDensity`: `crates/hoocode-code-settings/src/manager.rs:886`.
- Default when unset: compact if terminal rows < 25, else full (`src/mode/mod.rs:534-538`, `SMALL_TERMINAL_ROWS`).
- Ctrl+Z is suspend (`src/mode/input.rs:776`), not the footer.
- Mode chips: ASK, PLAN, BUILD, DEBUG (`hoocode-code-modes/src/extension.rs:40`).
  Longest is 5 chars.
- Dial: ◌ radar / ◍ peek, toggled with Ctrl+O; setting `toolOutputView`.
- Compact (`FooterDensity::Line`) is **2 rows** today (`footer.rs:546` returns two lines; `tests/it/footer.rs:209-215` asserts it). Short mode as one row is a real change.

## From (today)

```
hoocode  ⑂ main  ~/github/hoocode                       BUILD · ◌ radar
claude-opus-5-5 • medium   ▱▱▱▱▱▱▱▱ 2% of 1.0M   ↑38 ↓6.5k R247k W24k  $0.30 (sub)
```

Problems:

- The mode chip and the bar are not at the left.
- `R` and `W` are unlabeled.
- One space between the bar and the percent.
- Uneven gaps (2 or 3 spaces).
- `bare` is a third state, but the user wants exactly two.

## Decisions

| Topic | Decision |
|---|---|
| Count | **Exactly two footers.** Alt+Z toggles detailed ↔ short. |
| Remove `bare` | The dial has two stops: full = detailed, compact = short. |
| Persistence | The choice is saved across restarts (existing `chromeDensity`). |
| Left edge | Mode chip at the left of row 1; context bar at the left of row 2. |
| Mode chip | Padded to 5 chars (longest mode name), so columns line up. |
| Detailed rows | Fixed-width left column. Row 1 left column = `BUILD · ◌ radar`; the dial fills the gap. Row 2 left column = bar + percent + window. |
| Percent | Right-aligned in 3 chars: `  2%`, ` 88%`, `100%`. |
| Bar | 8 cells in both modes. Two spaces (or more, for alignment) between bar and percent. |
| Warning | `!` after the window (`88% of 1.0M!`; short: `88%!`). Shown at the auto-compact warning threshold, the same point where the color changes today. |
| Warning cue | The bar is also colored. The `!` is the cue that does not depend on color. Both modes. |
| Cache | Labeled `cache 247k/24k` instead of `R247k W24k`. |
| Subscription | Shown as `sub`, no parens. |
| Model name | Full in detailed. Short (e.g. `opus-5-5 · med`) in short mode only. |
| Detailed drop order | Row 2: provider → cache → tokens → cost → window (as today). Model and bar+% never drop. |
| Row 1 drop order | As today, but the mode chip and the dial at the left never drop. |
| Short mode | **One row.** |
| Short display order | mode, bar+%, branch, model, folder, cost, dial, subagents. |
| Short drop order | subagents → cost → folder → model → branch → dial. Separate from display order. Mode chip and bar+% never drop. |

## To: detailed

```
BUILD · ◌ radar        hoocode  ⑂ main  ~/github/hoocode                 ◇2 running
▱▱▱▱▱▱▱▱   2% of 1.0M  claude-opus-5-5 · medium   ↑38 ↓6.5k   cache 247k/24k   $0.30 sub
```

Other mode, with warning:

```
PLAN  · ◍ peek         hoocode  ⑂ main  ~/github/hoocode
▰▰▰▰▰▰▰▱  88% of 1.0M! claude-opus-5-5 · medium   ...
```

## To: short (drop steps as width shrinks)

```
BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  opus-5-5 · med  hoocode  $0.30  ◌ radar  ◇2
BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  opus-5-5 · med  hoocode  $0.30  ◌ radar
BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  opus-5-5 · med  hoocode  ◌ radar
BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  opus-5-5 · med  ◌ radar
BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  ◌ radar
BUILD  ▱▱▱▱▱▱▱▱   2%  ◌ radar
BUILD  ▱▱▱▱▱▱▱▱  88%!
```

## Resolved (2026-10-10)

- **Saved `bare` and `/chrome bare`:** a saved `chromeDensity = "bare"` is read as `compact`. `/chrome bare` gives a short error that names the two valid stops (full, compact).
- **Task ledger:** the dial never hides it. Accepted.
- **Autocomplete list:** keeps hiding the footer.
- **Short model name:** strip the provider prefix (`claude-`, `gpt-`, etc., the part before the first `-` when it is a known vendor). Shorten thinking levels: `medium` → `med`, `minimal` → `min`. `high`, `low` and `off` stay as they are.
- **Window size:** padded to 7 chars (`of 32k `) so the next column does not shift.

## Affected areas (for later implementation; no code now)

- `footer.rs`: `render_lines`, candidates, fit.
- `chrome_layout.rs`: stops.
- `mode/chrome.rs`: `/chrome` command.
- Settings types: `chromeDensity` values.
- `hotkeys.rs`: help text.
- Footer component goldens (`tests/it/footer.rs`, layout B) and screen goldens.
- `docs/maps/ui.md`: names the crate folder `code-tui-app`. The real folder is
  `hoocode-code-tui-app`. Fix it in the same commit.
