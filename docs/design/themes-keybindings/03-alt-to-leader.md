# Research 03: removing Alt, adding a leader (2026-11-05)

User decision: no Alt/Option defaults; use a configurable leader. Read-only code research.
The final keymap is in [../themes-keybindings.md](../themes-keybindings.md) (it uses fewer
Ctrl keys than this note, per the user's later "use Ctrl only where needed").

Correction to the brief: Mission Control has no Alt bindings (Ctrl and PgUp only,
`mission-control.md:420-430`). Alt is mentioned only at `mission-control.md:210` ("Letter keys
work with ctrl or alt") and :285 (Alt+e in the M1e checks). The alt+v entry is Windows-only.

## 1. Current Alt bindings

Library (`crates/hoocode-tui-keys/src/keybindings.rs`), context Editor:

| id | key | line |
|---|---|---|
| tui.editor.cursorWordLeft / Right | alt+left, alt+b / alt+right, alt+f | :183, :188 |
| tui.editor.deleteWordBackward | alt+backspace | :226 |
| tui.editor.deleteWordForward | alt+d, alt+delete | :231 |
| tui.editor.yankPop | alt+y | :245 |
| tui.editor.redo | alt+u | :249 |
| tui.editor.jumpBackward | ctrl+alt+] | :208 |

App (`crates/hoocode-code-tui-keybindings/src/keybindings.rs`), about 42 lines with `alt+`:

| id | key | context | line |
|---|---|---|---|
| app.editor.external | alt+e | Prompt | :49 |
| app.input.voiceTranscribe | alt+r | Prompt | :52 |
| app.clipboard.pasteImage | alt+v (Windows) | Prompt | :57 |
| app.clipboard.copyMessage | alt+q | Prompt | :62 |
| app.message.followUp | alt+enter | Prompt | :67 |
| app.message.dequeue | alt+up | Prompt | :72 |
| app.mode.cycleForward / Backward | alt+a / shift+alt+a | Prompt | :78, :83 |
| app.model.cycleForward / Backward | alt+m / shift+alt+m | Prompt | :86, :89 |
| app.thinking.cycleForward / Backward | alt+t / shift+alt+t | Prompt | :95, :100 |
| app.view.cycleForward / Backward | alt+o / shift+alt+o | Prompt | :106, :111 |
| app.tasks.cycleForward / Backward | alt+l / shift+alt+l | Prompt | :122, :127 |
| app.session.resume | alt+h | Prompt | :180 |
| app.session.changeDirectory | alt+w | Prompt | :188 |
| app.settings.open | alt+s | Prompt | :191 |
| app.hotkeys.open | alt+k | Prompt | :192 |
| app.session.togglePath / toggleSort / toggleNamedFilter | alt+p / alt+o / alt+n | Session picker | :277, :282, :287 |
| app.session.rename / delete | alt+r / alt+x | Session picker | :290, :291 |
| app.models.save / enableAll / clearAll | alt+s / alt+a / alt+x | Model picker | :297-299 |
| app.models.toggleProvider | alt+g | Model picker | :302 |
| app.models.reorderUp / Down | alt+up / alt+down | Model picker | :307, :312 |
| app.tree.foldOrUp / unfoldOrDown | alt+left / alt+right (ctrl too) | Tree | :317, :322 |
| app.tree.editLabel | alt+l | Tree | :325 |
| app.tree.toggleLabelTimestamp | alt+t | Tree | :328 |
| app.tree.filter.default..all | alt+1 to alt+5 | Tree | :333-353 |
| app.tree.filter.cycleForward / Backward | alt+c / shift+alt+c | Tree | :358, :363 |

Overlap already in the table: alt+s, alt+x, alt+a, alt+o, alt+r, alt+t, alt+l and alt+up each
mean two things on different surfaces.

## 2. Leader choice

Default leader: Ctrl+X (0x18). Nothing in the app table uses it; the only ctrl+x is a test
fixture (tui-keys `keybindings.rs:424`). Emacs, readline and opencode use it as a prefix.

Rejected:
- Ctrl+Space: `parse.rs:118` maps NUL to ctrl+space, but macOS input-source switching and many Linux IMEs take it first.
- Ctrl+]: already the editor's jumpForward (`keybindings.rs:203`).
- Ctrl+B, Ctrl+A: tmux and screen prefixes, and readline cursor keys.
- Ctrl+S, Ctrl+Q: XON/XOFF.
- Ctrl+J: thought unusable at this point (legacy `\n` parses as Enter, `parse.rs:133`; under kitty `\n` is shift+enter, `parse.rs:100`). Later finding: it can be the newline key; see the design doc.

State machine (in tui-keys, before the prompt sees input):
- Idle → leader → Pending; the footer shows the leader.
- Pending: the next key is looked up in the leader map. Match → run, back to Idle. Esc, an unbound key (footer flashes "no binding"), leader again, or the timeout (about 1.5 s, configurable) → Idle.
- Which-key popup after about 400 ms, rows read from the keymap.
- Config `"leader": "ctrl+x"`; a printable leader is rejected.
- Bindings written `"leader m"`. The parser splits on `+` only today (`matching.rs:27`), so it needs a step list.

## 3. Proposed defaults (this note's version)

Leader: e editor, r voice, q copy reply, f queue follow-up, d restore queued, m/M model,
t/T thinking, o/O tool view, l/L task tab, h resume, w cwd, s settings, k shortcuts, a/A mode,
y yank-pop, z redo, [ jump backward, Enter newline, v paste image (Windows).

This note kept Ctrl+R, Ctrl+O, Ctrl+T, Ctrl+G, Ctrl+N as direct keys. The final design
moves most of them behind the leader.

Pickers, bare letters while no text field has focus / the query is empty, "/" focuses the
query (an existing gate for ctrl+backspace at :293):
- Session picker: p, o, n, r, x.
- Model picker: s, a, x, g; reorder with ctrl+up/down (final design: shift+up/down).
- Tree: l label, t timestamps, c filter cycle, 1-5 filters, left/right fold.

Editing without Alt: ctrl+left/right word moves (already bound); ctrl+w delete word back;
ctrl+delete delete word forward (needs a legacy parser entry, only `\x1b[3~` exists);
ctrl+backspace is unreliable in legacy terminals (0x08 parses as backspace, `parse.rs:136-141`,
except Windows Terminal). Keep the ESC-prefix parser (`parse.rs:162-170`) so user-written Alt
keys still work; no Alt in defaults.

## 4. Risks

- tmux: shift+enter and ctrl+delete need extended-keys (CSI u).
- macOS Terminal.app: no kitty protocol, so shift+enter does not arrive. Option already sends unicode by default, which is why the Alt bindings fail there now.
- VS Code integrated terminal: check whether it takes Ctrl+X.
- Leader inside text input: it is a control key so it never types; the pending state must swallow the next key.
- Discoverability: which-key popup and footer hints (`hints.rs` labels alt as Option at :8-10). Screen goldens change.
- Migration: Alt keys in `keybindings.json` keep working; one notice per version; never rewrite the user's file.

## 5. Build order (this note's version)

1. M tui-keys chord and leader state. 2. S leader config in `AppKeybindingsManager`
(`keybindings.rs:478-493`). 3. M footer pending text and which-key popup. 4. S app table
moves; update `keybinding_layout` tests (lines 216-530), `keybindings-gold.json`, `gold.rs:79`.
5. S remove Alt from the library table; ctrl+delete parser entry. 6. M picker bare letters.
7. S newline fallbacks; queue on Enter. 8. S Alt migration notice. 9. S docs. 10. Pre-release goldens.
