# Research 02: keybindings in hoocode today (2026-11-05)

Read-only code research plus a first web pass. Decisions are in
[../themes-keybindings.md](../themes-keybindings.md). Note: the first proposal below kept
Alt keys; the user later decided no Alt defaults and a Ctrl+X leader (see 03).

## 1. Key handling

Crates: `hoocode-tui-keys` (matching, parse, kitty, registry), `hoocode-code-tui-keybindings`
(app map), `hoocode-code-tui-app` (dispatch), `hoocode-tui-terminal` (input/output).

- App table: `crates/hoocode-code-tui-keybindings/src/keybindings.rs`, `fn app_keybindings()`. Flat list of id → default keys → description, grouped Flow/Compose/Steer/Read/Scroll/Go/Overlays.
- Library table: `crates/hoocode-tui-keys/src/keybindings.rs`, `TUI_KEYBINDINGS` (`tui.editor.*` cursor/history/delete/yank, `tui.input.newLine` = shift+enter, `tui.input.submit` = enter, `tui.select.*`).
- Prompt dispatch: `crates/hoocode-code-tui-app/src/mode/input.rs`, `EDITOR_ACTIONS` (27 ids → Action). Dispatch is by id, so a default-key change needs no dispatch change.
- Hints: `crates/hoocode-code-tui-keybindings/src/hints.rs` (`key_text`, `key_hint`, `app_key_label` ~:101 shows only the first key).
- Matching: `crates/hoocode-tui-keys/src/matching.rs` (`matches_enter` 277-310, `matches_printable` ~342). Legacy parse: `parse.rs:96-135`. Single-key only; no chord state.

### Default bindings in effect (prompt), 2026-11-05

Enter submit; shift+enter newline; alt+enter follow-up queue; alt+up restore queue; ctrl+c
clear editor (double-press exit, `hoocode-tui-terminal/src/interrupt.rs:34, 118`); ctrl+d
exit when empty; ctrl+z suspend; ctrl+v paste image (alt+v on Windows); alt+e external
editor; alt+r voice; alt+q copy last message; shift+tab / alt+a mode forward, shift+alt+a
back; alt+m / shift+alt+m model; alt+t / shift+alt+t thinking; alt+o / shift+alt+o tool
output view; ctrl+o step tool output; ctrl+t thinking blocks; alt+l task tab; ctrl+g task
panel; ctrl+n attach subagent; ctrl+r transcript search; pageUp/pageDown, ctrl+home/end,
ctrl+up/down (message jump), shift+up/down (line scroll); alt+h resume, alt+w cwd, alt+s
settings, alt+k hotkeys. Escape is bound to interrupt, scroll exit, subagent detach and
panel exit (resolved by context). No default for session new/tree/fork or model select.

- No transcript toggle (ctrl+o is tool-output stepping). No reverse history search (ctrl+r is transcript search). Up/down history exists.
- Approval/confirm dialog keys were not enumerated (open gap, step K-0).

### Existing overlaps (same key, different ids, implicit context)

escape (4 ids); alt+s (settings.open / models.save); alt+r (voiceTranscribe / session.rename);
alt+o (view.cycleForward / session.toggleSort); alt+t (thinking.cycleForward /
tree.toggleLabelTimestamp); alt+l (tasks.cycleForward / tree.editLabel); alt+a
(mode.cycleForward / models.enableAll); alt+x (session.delete / models.clearAll); alt+up
(message.dequeue / models.reorderUp); shift+tab (mode.cycleForward / panel.prevTab);
up/down/enter/tab (panel vs lists). `get_conflicts` flags only user-config overlaps.

## 2. Settings

- Global `<agentDir>/settings.json` (agentDir = `~/.hoocode`, or `HOOCODE_` env override) and project `<cwd>/.hoocode/settings.json`; merged (`crates/hoocode-code-settings/src/storage.rs:87-102`, `lib.rs:3`). Paths: `crates/hoocode-code-paths/src/lib.rs:78-83`.
- Keybindings are a separate file: `<agentDir>/keybindings.json`, flat `{"app.x": "key" | ["k1","k2"]}`. Loaded at startup by `AppKeybindingsManager::create` then `install()` (`hoocode-code-tui-app/src/mode/mod.rs:623-624`; `keybindings.rs` ~478-510). `/reload` re-reads it (`mode/session_ops.rs:241-303`). Unknown ids are dropped silently (tui-keys `keybindings.rs` `rebuild()`).

## 3. Docs already written

- No user-facing keybinding doc; only `/hotkeys` (`docs/user/quickstart.md:25`).
- `docs/design/tui-activity.md`: X18 not built, the task-panel hint shows first key only (:43, :156); N2 kitty flag shared, so custom Shift+Enter acts as Alt+Enter (:122); L7 defers surfacing `get_conflicts` in /hotkeys (:168); panel focus is ctrl+g (:236); ctrl+n chosen for attach because "ctrl acts on what is drawn, alt letters are the dials'" (:225).
- `mission-control.md:420-430` lists its keys (Ctrl and PgUp only). Alt mentions at :210 and :285.
- The `keybindings.rs` header cites a stale TS rationale (`core/keybindings.ts`); replace it with the layout tests (`crates/hoocode-code-tui-keybindings/tests/it/keybinding_layout.rs`).

## 4. Terminal

- Kitty: a query is sent; on reply, push `CSI >7u` (`hoocode-tui-terminal/src/stdin_hub.rs:133-135`). If no reply, modifyOtherKeys `CSI >4;2m` (`lib.rs` ~430-436). Restore: `interrupt.rs:31`.
- Bracketed paste is parsed (`stdin_buffer.rs:22, 195`; Ctrl+C inside paste is text, `interrupt.rs:45`).
- Shift+Enter works only with kitty or modifyOtherKeys. Legacy `\x1b\r` is alt+enter (`matching.rs:277-310`).
- Legacy `\n` is parsed as Enter (`parse.rs:133`, `matching.rs:308`). Later check (in the design doc, "Ctrl+J"): raw mode clears ICRNL (crossterm 0.29.0 `cfmakeraw`), so the terminal does send Ctrl+J and Enter differently; the parser is what merges them.
- So in a plain terminal with neither protocol, there is no newline key today, since alt+enter is follow-up.

## 5. First web pass (2026-11-05)

- Claude Code (code.claude.com/docs/en/keybindings): `~/.claude/keybindings.json`, `{"bindings":[{"context":"Chat","bindings":{"key":"action"|null}}]}`, about 25 contexts, namespaced actions, chords space-separated with a 3 s timeout, unbind with null, reserved ctrl+c/d/m/[/i/h, a terminal-conflict table (ctrl+b tmux, ctrl+a screen, ctrl+z suspend), validation warnings.
- opencode (opencode.ai/docs/keybinds): `keybinds` object, leader ctrl+x, comma lists or arrays, `"none"` disables.
- Codex CLI: no keybinding file documented on the official page.
- lazygit: bindings grouped by context, `<disabled>` unbinds.
- Helix: TOML tables per mode, `no_op` unbinds, Space as leader-like mode.
- Readline/emacs and fzf: from memory, not fetched.

## 6. First proposal (superseded in part)

- Keep `<agentDir>/keybindings.json`; add a grouped context form; null/"none" unbind; chords in phase 2.
- `hoocode keybindings --defaults`, `--init`, `check`.
- Validation at load and /reload; show in /hotkeys.
- Hints generated from the keymap; show all keys (X18); lint against literal key names in UI strings.
- Open questions then: ctrl+c meaning, legacy newline key, alt on macOS, dialog keys. These were answered in 03, 04, 05 and the design doc.
