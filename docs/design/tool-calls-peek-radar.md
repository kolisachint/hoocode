# Tool calls in peek and radar

Date: 2026-10-10
Status: **design agreed, implementation in progress.**
Scope: transcript tool blocks. Ctrl+O toggles radar / peek (setting `toolOutputView`).

## Current state (facts, with file refs)

- Every tool block is a `ToolExecutionComponent`:
  `crates/hoocode-code-tui-widgets/src/tool_execution.rs`.
- `leading_spacer = Spacer::new(1)` gives one plain blank row above each block in peek
  (`set_lines(1)` in `rebuild_display`; 0 in radar).
- Block left pad: 1 space (`BoxComponent::new(1, 0, None)`), then `● ` on the call line.
  Result lines get 1 space only, not under the dot.
  `indent_under_signal_row` indents only in radar.
- Extra blank row between call and output, per tool:
  - Shell: `build_result` pushes `""` first (`tools/bash.rs`).
  - CodeSearch: `format_search_result` prefixes `"\n"` (`tools/search.rs`).
  - Edit: `Spacer(1)` inside its banded box, between call and diff (`tools/edit.rs`, `EditCall::render`).
  - Read: `format!("\n{body}")` (`tools/read.rs:185`).
  - Write: success shows nothing (`tools/write.rs:236` returns an empty `Container`).
    Errors prefix `"\n"` (`tools/write.rs:233`). The call preview already shows the
    content after `"\n\n"` (`tools/write.rs:182`).
- Edit is the only banded tool: `BoxComponent` with bg (`toolPendingBg` / `toolSuccessBg` /
  `toolErrorBg`), padding_y = 0. No band rows above or below.
- Golden `tests/golden/tui/tool-edit/after-tool.txt` L12-L22: plain blank (leading spacer),
  banded call line, banded blank (the `Spacer(1)`), diff, hint, plain blank (next block's
  spacer). Spacing between blocks is already even. The only extra row is the banded `Spacer(1)`.
- Shell call line: `format_bash_call` writes `$ {cmd}` (`tools/bash.rs`).
  Other tools start with their name in bold `toolTitle`.
- Peek trimming: `tool_output_view.rs` `peek_block` (5 lines, then `... (N more lines)`).
  Shell uses `truncate_to_visual_lines`.
- Radar: one row per chain (`tool_chain.rs` `rows()`), drawn by
  `tool_signal.rs` `render_tool_signal_line`. `VERB_WIDTH = 14`, `VERB_GAP = 2`.
  The subject is cut only at the screen edge, with `...`. Single calls and groups render the same way.
- Goldens: only `tests/golden/tui/tool-edit/` and
  `tests/golden/hoocode-code-tui-widgets/shell_output_collapsed_80.txt`.
  No Read, Write or CodeSearch golden. No radar tool-call golden.

## Decisions

| Topic | Decision |
|---|---|
| **Peek** | |
| Call to output gap | **No blank row** between a tool call and its output, all tools. Remove Shell `""`, CodeSearch `"\n"`, Edit `Spacer(1)`, Read `"\n"`. |
| Between blocks | **Keep one plain blank row** (`leading_spacer`). Spacing stays even. Edit has no extra top row to remove. |
| Left pad | **2 spaces** before the dot (was 1). |
| Output indent | Output starts **under the tool name** (after `● `), i.e. 4 columns from the left edge. |
| Shell call line | `● Shell  <cmd>`, like other tools. Drop the `$` prefix. |
| Name gap | **Two spaces** between tool name and target. No padding of names to a common width. |
| Band | **File tools only: Read, Write, Edit.** Shell and CodeSearch stay plain. |
| Band extent | Covers the call line and the output (as Edit does today). |
| Band tint | Very light, base2-like on Solarized Light. The band is never the only separator: the blank row between blocks and the `●` still mark blocks. |
| Write preview | On success, show the first 5 lines of the written file, banded, with `peek_block`'s `... (N more lines)`. Today it shows nothing. |
| **Radar** | |
| Single call | A single tool call is always shown, trimmed. |
| Target trim | **40 characters**, all tools, ending in `…` (one char) instead of `...`. |
| Multi-line Shell | First line only, then trim to 40. |
| Stats | Stats (e.g. `12 lines`) stay visible after the trimmed target. |

## From / To (peek)

Reconstructed from code and the Edit golden. **Not live captures.**

From:

```
 Editing.

 ● Edit src/app.ts
▒
▒  1 export function greet…
▒ -2     return `hello`;
▒ +2     return `Hello!`;
▒  ... (3 more lines, ctrl+o to expand)

 ● $ cargo test -p hoocode-code-tui-widgets

 running 12 tests
 test ok
```

To:

```
  Editing.

  ● Edit  src/app.ts
▒     1 export function greet…
▒    -2     return `hello`;
▒    +2     return `Hello!`;
▒     ... (3 more lines, ctrl+o to expand)

  ● Write  src/new.rs
▒     1 fn main() {
▒     2     println!("hi");
▒     3 }

  ● Shell  cargo test -p hoocode-code-tui-widgets
    running 12 tests
    test ok

  ● CodeSearch  footer density
    crates/…/footer.rs:498
```

`▒` marks the band. In the real UI the band also covers the call line.

## From / To (radar)

From:

```
 ● ran           cargo test -p hoocode-code-tui-widgets --no-fail-fast -- --nocapture …
```

To:

```
 ● ran           cargo test -p hoocode-code-tui-widgets…       12 lines
```

## Resolved (2026-10-10)

- **Band tint:** new theme token `toolBandBg`. It is base2-like on light themes and a matching subtle tint on dark themes. The band runs full width.
- **Write preview:** shows line numbers, like Read.
- **40-char trim:** counts display width, not bytes.
- **Live captures:** done through the screen goldens during implementation.

## Affected areas (for later implementation; no code now)

- `tool_execution.rs`: left pad, result indent.
- `tools/bash.rs`: call prefix (`$` removed), leading `""`.
- `tools/search.rs`: leading `"\n"`.
- `tools/edit.rs`: `Spacer(1)`, band.
- `tools/read.rs:185`: leading `"\n"`, band.
- `tools/write.rs`: success preview, band, error `"\n"` (`:233`).
- `tool_output_view.rs`: `peek_block` use for Write.
- `tool_signal.rs`: 40-char trim, `…`.
- Theme tokens: band tint.
- Goldens: `tool-edit`, `shell_output_collapsed_80.txt`, new Read, Write, CodeSearch and radar goldens.
- `docs/maps/ui.md`: update in the same commit.
