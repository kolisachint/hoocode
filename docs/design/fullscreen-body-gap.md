# Fullscreen gap between transcript and task panel: diagnostic

Status: investigated, no code change. The empty rows are the body's
top-aligned padding for a transcript shorter than its viewport — the layout
the M1 spec mandates — not rows leaked or reserved by the panel.

## Symptom

Fresh session, tall screen: chat messages end, a block of empty rows follows,
then the FLIGHT PLAN / task panel (e.g. `0/4`), then the prompt.

## Repro (committed)

`crates/hoocode-tui-render/tests/it/fullscreen_body_gap.rs` builds the
app-like tree (HUD, gap, body of header + 5 chat rows, panel, 2-row prompt)
at 40x20 and asserts the exact screen:

- rows 3–9: whole transcript, top-aligned;
- rows 10–14: 5 blank body rows;
- rows 15–18: panel, rows 19–20: prompt, contiguous, no gap between them.

The second test grows the panel 0 → 4 rows and shows the body shrinking by
exactly 4 rows: an appearing panel already takes its rows from the former
padding while the transcript stays whole.

## Row accounting (before → after panel appears, 40x20, 7-row transcript)

`split_rows` (`crates/hoocode-tui-render/src/fullscreen.rs:39`) gives the
body every row the top and bottom do not take: body 16 → 12 as the bottom
goes 2 → 6. Live, `body_start` (`fullscreen.rs:77`) returns
`total.saturating_sub(rows)` — 0 for a short transcript — and the composer
pads the short window with `""` (`crates/hoocode-tui-render/src/tui.rs:2263`).
Blank rows exist if and only if transcript lines < body rows.

## Suspects cleared

- Fixed-height viewport: no. The body is the leftover (`tui.rs:2192`,
  `fullscreen.rs:39`); only a 1-row minimum is kept.
- Spacer/filler: no. `screen_fill` (`mode/mod.rs:893`) is added only in the
  inline `ScreenFill` arm (`mode/mod.rs:940`); fullscreen sets only the body
  (`mode/mod.rs:948`). M1 spec: "No flex spacer."
- Bottom-anchoring: no, the opposite. The body is top-aligned per spec
  (`docs/design/mission-control.md:407`).
- Reserved panel slot: no. `resolve_chrome`
  (`chrome_layout.rs:51-64`) never hides tasks, but a visible `Slot` with an
  empty panel renders zero rows (`component.rs:178-184`; panel collapses at
  `task_panel.rs:1150-1153`).
- Panel fixed block: no. Unfocused rail rows, focused console, and summary
  track all render exactly their content rows (`task_panel.rs:1183-1216`,
  `render_flight_plan`, `render_summary_track`).
- Scroll offset: no. Live (`None`) follows the tail (`fullscreen.rs:77-80`);
  a long transcript fills the body (covered by
  `fullscreen_frame_100x30_composes_regions`).
- Bottom chrome blanks: no. `FooterTransientLines` renders only transient
  lines (`mode/mod.rs:258-265`), `NotificationPanel` renders nothing in
  fullscreen overlay, `CommsFrame` renders chrome + queue + editor tightly
  (`comms_frame.rs:484-524`).

## Why no change

A 7-line transcript cannot "use" 12 rows. The alternatives — bottom-anchor
short transcripts (moves the gap under the HUD, contradicts the agreed
top-aligned spec, changes the no-panel frame too), float the panel over the
transcript (hides the newest messages), or shrink the panel (hides
information) — all violate the spec or the no-regression constraint. The
recovery the symptom asks for already happens: the panel's rows come out of
the former padding, never out of the transcript.
