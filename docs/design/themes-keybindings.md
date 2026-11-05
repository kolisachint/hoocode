# Themes and keybindings (design, 2026-11-05)

Status: **decided with the user 2026-11-05. Built 2026-11-05: T-1 to T-6 and K-0 to K-10.**
The deviations from this plan, as built, are listed under "Built deviations" below.
Two parts that can be built on their own: **T** (themes) and **K** (keybindings). This is the third plan,
next to the core plan and the TUI plan ([README](README.md)). TUI work is done by
Haiku subagents. The main session orchestrates, briefs and merges.

## Goal

- Themes: a user can add a theme with a few lines, see the list of themes, and never
  lose the screen to a bad file.
- Keybindings: fewer Ctrl keys, no Alt keys in the defaults, one leader key for most
  actions, and key hints that always match the real keys.

## Part T: themes

### What exists

| Area | Where |
|---|---|
| Loader, auto, 50 ms poll | `crates/hoocode-code-tui-theme/src/registry.rs` (poll `:597`, debounce `:598`) |
| Built-in files, embedded | `registry.rs:18-23` (`dark.json`, `light.json`, `theme-schema.json` with `include_str!`) |
| Required tokens: 51; schema total: 86 | `schema.rs:12`, `schema.rs:77` |
| Colour tokens: 68 fg, 19 bg | `theme.rs` (lookup panics on an unknown token: `theme.rs:323`, `:330`) |
| Colour mode detection | `color.rs` (`detect_color_mode_with` before `:82`; `detect_color_mode` at `:82`) |
| Custom themes folder | `~/.hoocode/themes/` (`hoocode-code-paths/src/lib.rs:113`, `custom_themes_dir`). Documented in `docs/user/configuration.md:15` |
| Theme picker | `crates/hoocode-code-tui-selectors/src/small_selectors.rs`: `picker_themes` `:48-49` (`auto`, `dark`, `light`), `theme_selector` `:56` |
| Theme diagnostics | `crates/hoocode-code-tui-app/src/resource_display.rs:58`, `:654` |
| Unused setting | `theme_paths` in `crates/hoocode-code-settings/src/manager.rs:954-962`. The loader does not read it |
| Contrast test | `crates/hoocode-code-tui-theme/tests/it/theme_contrast.rs` |

Setting `theme` takes a name, `auto`, or `auto:<light>:<dark>`.

### Gaps

- The picker lists only `auto`, `dark` and `light`.
- There is no inheritance. Every custom theme needs all 51 required tokens.
- An unknown name silently becomes `auto`. A bad file silently becomes `dark`
  (`registry.rs:503-530`).
- A token name typed wrong in code panics at runtime.
- `theme_paths` is saved but not used.

### Decisions

| # | Decision |
|---|---|
| T1 | Format stays JSON. It matches settings, needs no new dependency, and editors can autocomplete with `"$schema"`. Token names stay as they are. |
| T2 | Built-in themes live in the binary and always win. A user cannot override a built-in name. A user adds new names. |
| T3 | Reference copies on disk. On startup, when the version changed, hoocode writes the built-in themes and `theme-schema.json` to `~/.hoocode/themes/defaults/`. The loader ignores that folder. The copies are refreshed on every upgrade, so they never go stale. A user copies one up a level and renames it, or uses `inherits`. |
| T4 | Add `"inherits": "<theme name>"`. Missing tokens come from the base. A short user theme is about 5 lines. |
| T5 | The theme picker lists the built-in themes and every user theme, with the file path. |
| T6 | Errors: a bad file or unknown name shows a warning in theme diagnostics and keeps the last good theme. Never crash. Never switch silently. |
| T7 | A test checks that every token literal used in code exists in the schema. This removes the runtime panic risk. |
| T8 | New built-in themes: a high-contrast theme and two more. Original palettes only, so no licence questions. Each one passes `theme_contrast.rs`. |
| T9 | Hot reload keeps the existing poll. No `notify` crate. |

| T10 | Remove the unused `theme_paths` setting (user, 2026-11-05). Done in T-3. |
| T11 | Persistence (user, 2026-11-05): choosing a theme in the picker saves `theme` to the global settings (`~/.hoocode`). `auto` (follow the system light or dark setting) is a picker entry and is saved the same way. Done in T-1. |
| T12 | `ansi:<name>` colours that follow the terminal palette: later, not this round. |

### What we build (T)

| Step | Size | Files | Done when |
|---|---|---|---|
| T-1 Picker lists user themes; picker saves the choice | S | `small_selectors.rs`, `registry.rs` (list function), settings | The picker shows `auto`, the built-ins and user themes, with paths. Choosing one saves `theme` to the global settings and survives a restart. Tests list a user theme and check the saved value. |
| T-2 `inherits` | M | `schema.rs`, `registry.rs` (`create_theme`), `themes/theme-schema.json`, tests in `tests/it/` | A 5-line theme that inherits from `dark` loads with every token set. A cycle is an error. |
| T-3 Warnings and keep-last-good; remove `theme_paths` | S | `registry.rs`, `resource_display.rs`, `manager.rs`, `config_selector.rs` | A bad file shows a warning in diagnostics. The last good theme stays on screen. No silent switch. Tests cover both cases. `theme_paths` is gone. |
| T-4 Defaults folder on version change | S | `registry.rs` or the startup code, `docs/user/configuration.md` | After an upgrade, `themes/defaults/` holds the current built-ins and schema. The docs page describes the folder. |
| T-5 Token-literal test | S | `theme.rs` tests | The test fails when code uses a token that is not in the schema. |
| T-6 New built-in themes | S each | `themes/*.json`, `theme_contrast.rs` | Each theme passes the contrast test. The picker shows it. |

Order: T-1, T-2, T-3, T-4, T-5, then T-6 (one theme per step).

## Part K: keybindings

### Principles

The user's words: "don't use ctrl much, use leader, use ctrl only where absolutely
necessary, keep it simple and easy to follow."

- **K-P1** No Alt or Option key in any default. Alt needs "Option as Meta" on macOS and
  arrives as ESC-prefixed bytes. Alt keys a user writes in `keybindings.json` still
  work (the ESC-prefix parser stays).
- **K-P2** One leader key: Ctrl+X by default. It is set as `"leader"` in
  `~/.hoocode/keybindings.json`. Most actions live behind it.
- **K-P3** Ctrl only where it is universal and needed: interrupt or clear, exit,
  suspend, paste, history search, line editing, and the leader itself.
- **K-P4** Lists, pickers, panes and the transcript scroll mode use bare letters
  (lazygit, k9s and fzf style). They are active only when no text field has focus, or
  when the query is empty. `/` focuses search. `?` shows the keys. `q` or Esc closes.
  `j` and `k` and the arrows move. Enter accepts.
- **K-P5** Every key hint on screen is generated from the keymap, so hints stay right
  after a user remaps a key.

### Research summary (2026-11-05)

Conventions only. No code was copied. All sources are docs.

| Action | Common default | Who |
|---|---|---|
| Submit | Enter | all |
| Newline | Ctrl+J, Shift+Enter | Claude Code, Gemini CLI, opencode, Goose, Helix |
| Interrupt | Esc or Ctrl+C | Claude Code and opencode (Esc). Gemini, aider and Goose (Ctrl+C) |
| Exit | Ctrl+D on empty | Claude Code, Gemini, opencode |
| History search | Ctrl+R | Claude Code, Gemini, aider, Goose, readline, fish, zsh |
| External editor | Ctrl+G, or Ctrl+X Ctrl+E | Claude Code, Gemini. Claude Code, aider, bash |
| Mode cycle | Shift+Tab | Claude Code, Gemini |
| Queue while running | Enter | Claude Code |
| Leader or prefix | Ctrl+X | opencode (leader, 2 s timeout), Claude Code chords, readline, Emacs |
| Unbind | `null` or `"none"` | Claude Code, opencode |
| Lists | j/k, `/`, `?`, `q`, Esc, Enter | lazygit, k9s, fzf, Helix |

No surveyed tool puts core actions on Alt. Zellij moved off Alt to Ctrl modes.

Keys that no tool should use as defaults:

- Ctrl+I, Ctrl+M, Ctrl+H, Ctrl+[: same bytes as Tab, Enter, Backspace and Esc.
- Ctrl+S and Ctrl+Q: XON/XOFF flow control.
- Ctrl+Z: suspend.
- Ctrl+Space: IME.
- Ctrl+B: tmux prefix.
- Ctrl+A as a prefix: screen.

Sources: code.claude.com/docs/en/keybindings, geminicli.com/docs/reference/keyboard-shortcuts,
opencode.ai/docs/keybinds, aider.chat/docs/usage/commands.html, block.github.io/goose
CLI docs, docs.helix-editor.com/keymap.html, lazygit docs/keybindings, the readline
manual (tiswww.case.edu), and sw.kovidgoyal.net/kitty/keyboard-protocol. Codex, Cursor and
Amp were not checked against official docs.

### What exists (code)

| Area | Where |
|---|---|
| Library key table | `crates/hoocode-tui-keys/src/keybindings.rs` `TUI_KEYBINDINGS` (`:158`) |
| App key table | `crates/hoocode-code-tui-keybindings/src/keybindings.rs` `app_keybindings()` |
| Prompt actions | `crates/hoocode-code-tui-app/src/mode/input.rs` `EDITOR_ACTIONS` (`:103`) |
| Hints | `crates/hoocode-code-tui-keybindings/src/hints.rs` |
| Parse and matching | `crates/hoocode-tui-keys/src/parse.rs`, `matching.rs`. Single keys only, no chords |
| Overrides | `~/.hoocode/keybindings.json`, a flat `{"id": key or [keys]}`. Loaded by `AppKeybindingsManager` (`keybindings.rs` about `:478-510`). Re-read on `/reload`. Unknown ids are dropped silently |
| Terminal input | Kitty protocol, with a modifyOtherKeys fallback (`crates/hoocode-tui-terminal/src/stdin_hub.rs:133-135`, `lib.rs` about `:430-436`) |
| Tests | `crates/hoocode-code-tui-keybindings/tests/it/keybinding_layout.rs`, `tests/fixtures/keybindings-gold.json` |

Counts measured on 2026-11-05: about 42 lines in the app table use `alt+`. Eight Alt keys
mean one thing on one surface and another thing on another (`alt+s`, `alt+x`, `alt+a`,
`alt+o`, `alt+t`, `alt+l`, `alt+r`, `alt+up`).

### Default keymap

**Direct keys** (prompt, always on):

| Key | Action | Note |
|---|---|---|
| Enter | Submit. While a turn runs, steer the agent with the message (user, 2026-11-05) | Queue was Alt+Enter (K-7 had made it a queue; reverted) |
| Shift+Enter | Newline | Leader Enter is the fallback where Shift+Enter cannot be sent. Ctrl+J is not changed (see "Ctrl+J") |
| Esc | Interrupt the running turn, or close a popup | |
| Esc Esc | Clear a non-empty prompt (kept in history) | |
| Up, Down | History. Up on an empty prompt with queued messages takes the last one back | Replaces Alt+Up |
| Shift+Tab | Cycle mode | |
| Tab | Completion | |
| PgUp, PgDn | Scroll the transcript (enters scroll mode) | |
| Ctrl+C | Clear the prompt. When empty, interrupt. Double press exits | Reserved |
| Ctrl+D | Exit on an empty prompt | Reserved |
| Ctrl+Z | Suspend | Reserved |
| Ctrl+V | Paste image (Windows: leader `v`) | |
| Ctrl+R | Search prompt history | Today Ctrl+R searches the transcript. That moves to leader `/` |
| Ctrl+A/E, Ctrl+K/U, Ctrl+W, Ctrl+Y, Ctrl+Left/Right, Ctrl+Delete | Line editing (readline) | Ctrl+Delete needs a legacy parser entry |
| Ctrl+X | Leader | Configurable |

**Leader keys** (Ctrl+X, then):

| Key | Action | Was |
|---|---|---|
| `e` (also Ctrl+E) | External editor | Alt+E |
| `m` / `M` | Model next / previous | Alt+M |
| `t` / `T` | Thinking level next / previous | Alt+T. Ctrl+T (toggle thinking blocks) moves to leader `b` |
| `b` | Show or hide thinking blocks | Ctrl+T |
| `o` / `O` | Tool output view next / previous | Alt+O |
| `p` | Step tool output | Ctrl+O |
| `a` | Mode previous | Shift+Alt+A |
| `g` | Task panel show or hide | Ctrl+G |
| `l` / `L` | Task panel tab next / previous | Alt+L |
| `n` | Attach subagent | Ctrl+N |
| `f` | Queue follow-up, sent after the turn (Enter steers while running) | Alt+Enter |
| `d` | Take back a queued message | Alt+Up |
| `/` | Search transcript | Ctrl+R |
| `r` | Voice input | Alt+R |
| `c` | Copy last reply | Alt+Q |
| `h` | Resume session | Alt+H |
| `w` | Change directory | Alt+W |
| `s` | Settings | Alt+S |
| `?` or `k` | Show all keys (`/hotkeys`) | Alt+K |
| `y` | Yank-pop | Alt+Y |
| `z` | Redo | Alt+U |
| Enter | Newline (works in every terminal) | |
| `v` | Paste image (Windows) | Alt+V |
| Esc or Ctrl+X | Cancel the leader | |

**Scroll mode** (after PgUp, no text focus):

| Key | Action | Was |
|---|---|---|
| `j` / `k` | Line down / up | |
| PgUp, PgDn | Page | |
| `[` / `]` | Previous / next message | Ctrl+Up/Down |
| `g` / `G` | Top / bottom | Ctrl+Home/End |
| `/` | Search | |
| `q` or Esc | Back to the prompt | |

**Pickers and panes** (no text focus, or empty query): `j`/`k` or arrows move, Enter
accepts, `/` searches, `?` shows keys, `q` or Esc closes.

- Session picker: `p` path, `o` sort, `n` named filter, `r` rename, `x` delete.
- Model picker: `s` save, `a` enable all, `x` clear all, `g` provider, Shift+Up/Down reorder (was Alt+Up/Down).
- Tree: `h`/`l` or left/right fold, `e` edit label (was Alt+L), `t` timestamps, `1`-`5` filters, `c` filter cycle.

**Approval and confirm dialogs** (listed 2026-11-05 from the code; line numbers as of that date).
Target keys: `y` yes, `n` no, `a` always, Enter default, Esc deny. Bare letters fire only
when no text field holds text (K-P4). Where a reply field is always open, the letter fires
only while the reply is empty, the same gate the clearance card uses for `e` today (decided by the
user, 2026-11-05).

| Surface | Current key | Target key | file:line |
|---|---|---|---|
| Tool clearance card (yes, always, no) | Enter picks the highlighted row (yes at the start). `1` `2` `3` pick. `↑` `↓` move. Esc = no. `e` edits the command first. Letters type into the reply | `y` yes, `a` always, `n` no. Enter default, Esc no. `1` `2` `3` stay as aliases. `e` stays | `clearance_card.rs:125-136` (labels), `:193-202` (`choose`), `:210` (handler), `:250-267` (keys) |
| Tool clearance card, reply typed | Enter sends the reply as the denial. Esc clears the reply (a second Esc denies). Letters type | No change. Letters type while the reply holds text | `clearance_card.rs:231-243` |
| End-of-shift card (quit) | Enter ends the shift. Esc stays | No change (Enter default, Esc stay) | `end_shift.rs:81-87` (keys `:83-84`) |
| End-of-shift jobs card | Enter or `k` kills all. `l` leaves them running. Esc stays | Enter kills all (default). `k` and `l` stay, since they name the action. Open question 4 | `end_shift.rs:229-236` (keys `:232-234`) |
| Yes/No lists on the generic selector: import confirm, missing-cwd confirm, extension and permission gate prompts | `↑` `↓` and `j` `k` move. Enter picks. Esc or Ctrl+C cancels. No letter answers | `y` = first "Yes", `n` = "No", `a` = "Always" when offered. Enter default, Esc deny | `extension_selector.rs:179-192` (shared selector); `mode/dialogs.rs:127-146` (gate); `mode/session_ops.rs:352-359` (import), `:690` (missing cwd); `dialog_bridge.rs:142` (permission gate select) |
| MCP trust prompt ("Trust and start", "Not now") | Generic selector: Enter picks row 1. Esc cancels | `y` = Trust and start, `n` = Not now. Enter default, Esc deny | `mode/mcp.rs:8-10` (labels), `:81` (`ui.select`) |
| MCP elicitation (URL: Accept, Decline, Cancel; form: Answer, Decline, Cancel) | Generic selector: Enter picks row 1. Esc cancels | `y` = Accept or Answer, `n` = Decline, Esc = Cancel. Enter default | `mcp_elicitation.rs:50-83` (choices at `:56`, `:67`) |
| Ask-user question card (AskUserQuestion) | Enter or Tab confirms. `→` next. Esc skips. `1`-`9` answer at once. Letters type into the custom answer when the row allows it | No `y` or `n`: the card has a text row, and letters are its text. Enter default, Esc skip, digits stay | `ask_options.rs:273` (handler), `:292-293` (confirm), `:298-299` (cancel), `:313` (letters), `:324-330` (digits) |

Not an approval, noted in passing: the session picker's delete (`app.session.delete`,
`alt+x`, `keybindings.rs:291`) has no confirm step in the code.

**Mission Control keys** (`docs/design/mission-control.md`, keys table at `:420-430`, plus keys named
elsewhere in that doc). Review against K-P3 (Ctrl only for interrupt or clear, exit, suspend,
paste, history search, line editing, leader):

| Key | Today (Mission Control) | K-P3 check | Target | Where |
|---|---|---|---|---|
| PgUp, PgDn | Page the transcript, pinning at the prompt | Not Ctrl. Keep | Keep | `:424` |
| Shift+↑, Shift+↓ | Aliases of PgUp and PgDn (M1d) | Not Ctrl. Keep | Keep | `:437` |
| Mouse wheel, scrollbar click | Scroll, pin, return to live | Not a key. Keep | Keep | `tui.rs:141`, `:1068`, `:527` (mission-control.md Keys, scroll rows) |
| Ctrl+Home, Ctrl+End | Top, and live | Ctrl not on the list | Scroll mode `g` / `G` (plan, Scroll mode) | `:425` |
| Ctrl+R | Transcript search (SCAN), pins at the bottom | Ctrl+R is for prompt history only | Leader `/` (plan: Ctrl+R moves to leader `/`) | `:78`, `:93`, `:426` |
| Ctrl+Up, Ctrl+Down | Previous and next user message | Ctrl not on the list | Scroll mode `[` / `]` | `:427` |
| ↑, ↓ (pinned) | One line. At the live prompt: input history | Arrows allowed (K-P4) | Keep | `:206-213` in code, `:428` |
| Esc | Leave the pinned view. Clears search first. Aborts a run | Esc allowed | Keep | `:429` |
| `/`, `n`, `p` | Search in the pinned view; next and previous match | Bare letters, scroll mode (K-P4) | Keep | `:430` |
| Ctrl+G | Flight plan console (task panel) | Ctrl not on the list | Leader `g` (task panel show or hide). Assumes the flight plan console is the task panel | `:38`, `:79` |
| Ctrl+T | Expand thinking text | Ctrl not on the list | Leader `b` (show or hide thinking blocks) | `:39`, `:133` |
| Shift+Tab | BUILD and PLAN toggle | Keep (plan: Shift+Tab cycles mode) | Keep | `:80` |
| Ctrl+C, twice | End of shift (`/quit` too) | Clear or exit. Keep | Keep | `:81` |
| Ctrl+Z | Suspend (M1e check) | Suspend. Keep | Keep | `:285`, `:460` |
| Alt+e | External editor (M1e check) | Alt is out of defaults (K-P1) | Leader `e` (ctrl+x, then e) | `:285` (fixed in this commit) |

The Alt mentions at `mission-control.md:210` and `:285` now name the leader keys.

### Leader behaviour

- Ctrl+X enters pending. The footer shows "Ctrl+X …". The next key runs its action.
- Esc, a second Ctrl+X, an unbound key (footer: "no binding"), or the timeout cancels.
  The timeout is 1500 ms by default, set by `leaderTimeoutMs` in the config.
- After 400 ms pending, a which-key popup lists the leader keys. It is built from the
  keymap, so user changes show.
- The pending key is swallowed. It never reaches the editor.
- A printable leader key (a letter, digit or symbol) is rejected by config.

### Config file (`~/.hoocode/keybindings.json`)

Both forms work.

Flat form (kept):

```json
{"app.model.cycleForward": "ctrl+x m"}
```

Grouped form (added):

```json
{
  "leader": "ctrl+x",
  "leaderTimeoutMs": 1500,
  "bindings": [
    {"context": "Prompt", "bindings": {"ctrl+x m": "app.model.cycleForward", "ctrl+r": null}}
  ]
}
```

- Contexts: Global, Prompt, Leader, Scroll, Picker, Panel, Dialog, Tree, Models.
- Key syntax: `"ctrl+x m"` (a space means a sequence). `"shift+enter"`. `null` or `"none"` unbinds.
- Reserved, cannot be rebound: ctrl+c, ctrl+d, ctrl+z, ctrl+m, ctrl+i, ctrl+[, ctrl+h.
- Warned about, can be rebound: ctrl+s, ctrl+q, ctrl+b, ctrl+space.
- Defaults on disk: like themes, hoocode writes `~/.hoocode/keybindings.defaults.json`
  (read-only reference, refreshed on a version change).
- Commands: `hoocode keybindings --defaults` prints the defaults. `hoocode keybindings check`
  validates the user file. `hoocode keybindings --read` prints what the terminal sends
  (like fish_key_reader).
- Validation runs at startup and on `/reload`. It checks parse errors, unknown ids,
  duplicate keys in one context, and reserved keys. Results show in `/hotkeys` and in
  `check`. Never crash.
- Migration: Alt keys in a user's file keep working. hoocode shows one notice per version
  that lists them. Never rewrite the user's file.

### Terminal notes

- Shift+Enter needs the kitty protocol or modifyOtherKeys. tmux needs extended-keys on.
- macOS Terminal.app has neither. Use Ctrl+J or leader Enter there.
- Check that VS Code's integrated terminal does not take Ctrl+X.

### Ctrl+J

Finding from the code (2026-11-05, checked by reading the code):

**Ctrl+J can be a portable newline: yes, with two small code changes, because a legacy
terminal sends Ctrl+J as byte 0x0A and Enter as byte 0x0D, and hoocode's raw mode keeps
the two bytes apart.** Today hoocode maps 0x0A to Enter in legacy mode, so Ctrl+J acts
as Enter. Details:

- Raw mode: `crates/hoocode-tui-terminal/src/lib.rs:384` and `:506` call
  `crossterm::terminal::enable_raw_mode()`. On Unix, crossterm 0.29.0 calls `cfmakeraw`
  (`crossterm-0.29.0/src/terminal/sys/unix.rs:302`). `cfmakeraw` clears ICRNL, so the
  tty does not turn 0x0D into 0x0A. No code in `hoocode-tui-terminal` sets ICRNL or
  touches termios flags on its own (grep for icrnl, ICANON, ECHO, IXON found nothing).
- Parser, legacy mode: `crates/hoocode-tui-keys/src/parse.rs:133` returns `enter` for
  `"\n"` when the kitty protocol is not active. The generic Ctrl+letter rule at `parse.rs:176`
  would return `ctrl+j` for 0x0A, but `:133` returns first, so `ctrl+j` never comes out.
- Matcher, legacy mode: `matches_enter` in `crates/hoocode-tui-keys/src/matching.rs:307-308`
  accepts `"\n"` as plain Enter. `matches_key(data, "ctrl+j")` would match 0x0A
  (`raw_ctrl_char` in `matching.rs:71-81`). But `parse_key` returns `enter` for 0x0A
  before any ctrl+j check, so the key id a binding sees is `enter`.
- Kitty mode: `parse.rs:100` maps `"\n"` to `shift+enter`. This is a custom mapping
  for terminals that send a bare LF for Shift+Enter. Ctrl+J in kitty mode arrives as a
  CSI u sequence and parses as `ctrl+j` already.

**Decision (user, 2026-11-05): no change.** Newline is Shift+Enter, with leader Enter as
the fallback. Legacy Enter and 0x0A parsing stay as they are. The change below is kept
for reference only and is not built.

Change that was considered:

1. `parse.rs:133`: stop mapping `"\n"` to `enter` in legacy mode. Then `parse.rs:176`
   returns `ctrl+j`.
2. `matching.rs:308`: remove the `"\n"` case from the plain-Enter arm.
3. Keep `parse.rs:100` as it is. A terminal that sends LF for Shift+Enter then gets
   `shift+enter` in kitty mode and `ctrl+j` in legacy mode. The keymap binds both to newline.
4. Tests: add a legacy-mode case for `"\n"` to `crates/hoocode-tui-keys/tests/it/keys.rs`.
   Check that `"\r"` still gives Enter.

Caveat: this is a change to Enter-handling in every mode without the kitty protocol. Test on
macOS Terminal.app, tmux and a plain Linux tty before merge.

### What we build (K)

| Step | Size | Files | Done when |
|---|---|---|---|
| K-0 Enumerate dialog keys and Mission Control keys; finalise their rows | S | this page, `mission-control.md` | Every dialog and Mission Control key is in a table with its target key. |
| K-1 Sequences and the leader state machine | M | `crates/hoocode-tui-keys/src/` (step parsing, pending, timeout, Esc, swallow) | Tests in `crates/hoocode-tui-keys/tests/it/` cover pending, timeout, Esc, Ctrl+X again, an unbound key, and a swallowed key. |
| K-2 Leader config, grouped form, contexts | S | `AppKeybindingsManager` (`keybindings.rs`) | The grouped form loads. The flat form still works. A printable leader is rejected. |
| K-3 Footer pending text and which-key popup | M | `hints.rs`, footer code | The footer shows "Ctrl+X …" while pending. The popup appears after 400 ms and lists keys from the keymap. |
| K-4 New defaults in the app table | S | app key table, `keybinding_layout.rs`, `keybindings-gold.json` | The layout tests and the gold file match the table in this page. |
| K-5 Remove Alt from the library table; Ctrl+Delete parser entry | S | `keys` library table, `parse.rs` | No Alt in the defaults. Ctrl+Delete parses. Enter and 0x0A parsing unchanged. |
| K-6 Bare-letter pickers, scroll mode keys, empty-query gate, `/` focus | M | picker and scroll-mode code in `hoocode-code-tui-app` | Each list in "Pickers and panes" and "Scroll mode" works with its keys. Typing in a query field still types. |
| K-7 Enter queues while running; Up takes back a queued message | S | `mode/input.rs` | While a turn runs, Enter queues. Up on an empty prompt takes back the last queued message. |
| K-8 Validation, `check`, `--defaults`, `--read`, defaults file, Alt migration notice | S | `AppKeybindingsManager`, CLI | Each command works. Errors show in `/hotkeys`. A bad file never crashes. The Alt notice shows once per version. |
| K-9 Hints generated everywhere; lint against literal `ctrl+` and `alt+` in UI strings | S | `hints.rs`, a CI script in `scripts/ci/` | No UI string has a literal key name. The lint fails on a new literal. |
| K-10 Docs: user keybindings page, `docs/maps/ui.md`, `mission-control.md:210` and `:285` | S | `docs/user/`, `docs/maps/ui.md`, `mission-control.md` | Docs match the build. Pre-release: `python3 scripts/tui/goldens.py check all`. |

Order: K-0 first. Then K-1 and K-2 (they do not touch the UI). Then K-4 and K-5 together,
then K-3, K-6, K-7, K-8, K-9, K-10.

Every step runs `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo nextest run -p <touched crates>` (CLAUDE.md). TUI steps are done by Haiku subagents.

## Built deviations (2026-11-05)

What was built differs from the text above in these places. The user pages
(`docs/user/keybindings.md`) and `docs/maps/ui.md` say what the build does.

- **Leader only at the prompt.** Ctrl+X starts a sequence only when the prompt has
  focus. A picker, pane or the task panel above it takes the key instead.
- **`q` closes lists, not dialogs.** In a picker or list with an empty query, `q`
  closes it, as Esc does, and it leaves the pinned transcript view. The dialog
  answers are `y`, `a` and `n` (while no reply is typed); `q` does not answer a dialog.
- **Picker help is `f1` first.** `app.list.help` lists `f1` before `?`. `?` types
  while a query is typed, so the "more" hint shows `f1`. The gold test records it.
- **A grouped binding applies where its action runs.** `"u": "app.scroll.lineDown"`
  is a Scroll binding. Written under another context, it is skipped with a warning.
  A `null` in a context takes the key away from that context's actions only.
- **Some hint keys are fixed, not keymap ids.** Space (the second toggle in
  settings and config), `k` and `l` on the end-of-shift card, `v` and `r` on the
  interrupted-run row, and `e` on the approval card are written as they are typed.
  They are not in the keymap, so the K-9 lint does not cover them as key names.
- **Scroll indicator names.** The keymap's own names show: `pageUp/pageDown`, not
  `PgUp/PgDn`, and `↑↓` is kept for the bare arrows.

## Not doing

- No Alt key in any default.
- No `notify` crate. The poll stays.
- No copying code or schemas from other tools. Conventions only.
- No rewrite of the user's `keybindings.json` or theme files.
- No built-in theme override by name.
- No chords beyond the leader. No second leader.
- No in-app key editor. No Ctrl+J newline.
- No ANSI-palette themes in this round (later, T12).

## Decided questions (user, 2026-11-05)

1. `theme_paths`: removed (T10).
2. `ansi:<name>` colours: later (T12).
3. Ctrl+J: not changed. Newline is Shift+Enter and leader Enter.
4. Dialog keys: `y`, `n`, `a`, Enter, Esc. A bare letter fires only while the text field is
   empty. On the clearance card, whose reply field is always open, y/n/a work while the reply is empty.
5. Leader timeout 1500 ms and which-key delay 400 ms are the defaults.
6. VS Code: check whether its terminal takes Ctrl+X before K-2. If it does, the default
   leader stays and the docs say so.
7. Persistence: the theme picker saves to the global settings, `auto` included (T11).
   Keys change only by editing `~/.hoocode/keybindings.json`. No in-app key editor.
8. Enter while running steers; Ctrl+X f queues a follow-up (user, 2026-11-05).

## Details

### Research and code checks

The raw research notes (code facts, every current binding, the surveys of coding-agent and
editor TUIs, with sources) are in [themes-keybindings/](themes-keybindings/README.md).

The research table and the code facts above were checked on 2026-11-05. The line numbers
are from that date. They can move.

### Measured on 2026-11-05

- `REQUIRED_COLOR_TOKENS` has 51 entries (`schema.rs:12`). `COLOR_PROPERTIES` has 86 (`schema.rs:77`).
- Call sites: 78 lines call `theme().fg(` or `theme().bg(` in one line. About 327 lines mention `theme()`. The earlier figure of about 222 was not re-counted. Use these numbers for T-5.
- `theme_paths` is written by `crates/hoocode-code-tui-selectors/src/config_selector.rs:475-479` and read by nothing in the loader.
