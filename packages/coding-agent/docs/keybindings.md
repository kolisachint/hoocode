# Keybindings

All keyboard shortcuts can be customized via `~/.hoocode/keybindings.json`. Each action can be bound to one or more keys.

The config file uses the same namespaced keybinding ids that hoocode uses internally and that extension authors use in `keyHint()` and injected `keybindings` managers.

Older configs using pre-namespaced ids such as `cursorUp` or `expandTools` are migrated automatically to the namespaced ids on startup, as are `app.thinking.cycle` and `app.mode.cycle`, which became `…cycleForward` when each gained a backward half.

After editing `keybindings.json`, run `/reload` in hoocode to apply the changes without restarting the session.

## The dials

Six things on screen are dials — an ordered set of stops you step through, with
the current stop painted where you can see it. They are the most-pressed keys in
the app, and they follow one rule:

> **`Alt+<letter>` steps a dial forward, `Shift+Alt+<letter>` steps it back. The
> letter names the dial. The slash command picks a stop outright.**

| Dial | Where you see it | Forward | Back | Steps through |
|---|---|---|---|---|
| **A**gent mode | footer, bold, top left | `Alt+A` | `Shift+Alt+A` | ask → plan → build → debug |
| **M**odel | footer, bottom right | `Alt+M` | `Shift+Alt+M` | your enabled models |
| **T**hinking level | footer, bottom right | `Alt+T` | `Shift+Alt+T` | off → … → high |
| Tool **o**utput | footer, top right | `Alt+O` | `Shift+Alt+O` | radar → peek → full |
| Task **l**edger | task panel header | `Alt+L` | `Shift+Alt+L` | tasks → subagents → teams |
| Session **c**olor | the session chip | `Alt+C` | `Shift+Alt+C` | six chip slots |

One modifier, one shape, six letters that each name their dial. Nothing else in
the app steps a dial and no dial is anywhere else, which is what makes that
table worth learning once instead of looking up.

`/mode`, `/model` and `/color` are the pickers behind three of them: **the key
steps, the command chooses.** That is why `app.model.select` ships unbound —
`Alt+M` stepping the model is worth more than `Alt+M` opening a list of them —
and why `app.session.tree` gave `Alt+T` to the thinking dial and kept `/tree`.

The thinking level is the one dial with a second key: `Shift+Tab` still steps it.
Every other dial is either reachable through a slash command or is about what is
drawn rather than what the agent does, so on a terminal that does not send `Alt`
(see below) this is the one setting that would otherwise be stranded.

## The rings

Which ring a key is in is decided by its modifier, so the modifier tells you
what kind of thing the key does before you remember the key itself.

- **`Ctrl` is the view** — what is on screen right now: expand a tool block,
  show thinking, step the task panel. Pressed many times a minute, so they
  never need a second modifier.
- **`Alt` is the cockpit** — what the agent *is* and where it works: mode,
  model, working directory, settings, sessions. Pressed a few times a session.
- **`Shift` reverses** whatever the unshifted key does. `Shift+Alt+O` cycles
  the tool view backward, `Shift+Alt+U` re-folds what `Alt+U` opened.
- **Inside a picker, `Ctrl` belongs to the query you are typing** — `Ctrl+A` is
  start of line, `Ctrl+U` kills the line, `Ctrl+W` kills a word. A picker's own
  verbs are therefore all on `Alt`, mnemonic to that picker. The picker
  captures keys while it is open, so a verb may reuse a letter the global set
  already has.

Where the same letter appears on both rings it is the same subject seen twice —
`Ctrl` acts on what is already drawn, `Alt` on how much is ever drawn:

- `Alt+O` sets how much tool output there ever is and saves where it lands;
  `Ctrl+O` jumps to all of it and back, leaving the dial where it was.
- `Alt+T` sets how much thinking there ever is; `Ctrl+T` shows or hides the
  thinking you have.

Two consequences worth knowing before rebinding:

- No default takes a key the editor or the terminal already owns —
  `Ctrl+A/E/B/F/K/U/W/Y/D/L/R/G`, `Ctrl+S` (XOFF), `Ctrl+M` (enter), `Ctrl+I`
  (tab).
- No verb sits on a bare `Shift+<letter>`. Outside the Kitty keyboard protocol
  that arrives as the plain uppercase letter, which is indistinguishable from
  typing in any scope that has a query line.

`test/keybinding-layout.test.ts` holds all of this.

## Key Format

`modifier+key` where modifiers are `ctrl`, `shift`, `alt` (combinable) and keys are:

- **Letters:** `a-z`
- **Digits:** `0-9`
- **Special:** `escape`, `esc`, `enter`, `return`, `tab`, `space`, `backspace`, `delete`, `insert`, `clear`, `home`, `end`, `pageUp`, `pageDown`, `up`, `down`, `left`, `right`
- **Function:** `f1`-`f12`
- **Symbols:** `` ` ``, `-`, `=`, `[`, `]`, `\`, `;`, `'`, `,`, `.`, `/`, `!`, `@`, `#`, `$`, `%`, `^`, `&`, `*`, `(`, `)`, `_`, `+`, `|`, `~`, `{`, `}`, `:`, `<`, `>`, `?`

Modifier combinations: `ctrl+shift+x`, `alt+ctrl+x`, `ctrl+shift+alt+x`, `ctrl+1`, etc.

> **`alt` needs a cooperating terminal.** On macOS outside the Kitty keyboard
> protocol, `Option` composes characters instead of sending `Alt` — `Option+M`
> types `µ`. Some Linux terminals claim `Alt+<letter>` for menu mnemonics. See
> [terminal-setup.md](terminal-setup.md#alt-keys) for the per-terminal setting.
>
> Interrupt, clear, exit, expand and submit are deliberately never on `Alt`, so
> a terminal that eats `Option` is still recoverable.

## All Actions

### Flow — the keys you hit without thinking

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `tui.input.submit` | `enter` | Send message |
| `tui.input.newLine` | `shift+enter` | Insert new line |
| `tui.input.tab` | `tab` | Path completion / accept autocomplete |
| `app.interrupt` | `escape` | Cancel autocomplete / abort streaming |
| `app.clear` | `ctrl+c` | Clear editor (press twice to exit) |
| `app.exit` | `ctrl+d` | Exit when the editor is empty |
| `app.suspend` | `ctrl+z` (none on Windows) | Suspend to background |
| `app.message.followUp` | `alt+enter` | Queue a follow-up message |
| `app.message.dequeue` | `alt+up` | Restore queued messages to the editor |
| `app.clipboard.pasteImage` | `ctrl+v` (`alt+v` on Windows) | Paste image from clipboard |

### The dials — step forward, `shift` steps back

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.mode.cycleForward` | `alt+a` | Step agent mode: ask → plan → build → debug |
| `app.mode.cycleBackward` | `shift+alt+a` | Step agent mode backward |
| `app.model.cycleForward` | `alt+m` | Step to the next model |
| `app.model.cycleBackward` | `shift+alt+m` | Step to the previous model |
| `app.thinking.cycleForward` | `alt+t`, `shift+tab` | Step thinking level: off → … → high |
| `app.thinking.cycleBackward` | `shift+alt+t` | Step thinking level backward |
| `app.view.cycleForward` | `alt+o` | Step tool output: radar → peek → full |
| `app.view.cycleBackward` | `shift+alt+o` | Step tool output backward |
| `app.session.color.cycleForward` | `alt+c` | Step the session chip's color |
| `app.session.color.cycleBackward` | `shift+alt+c` | Step the session chip's color backward |
| `app.tasks.cycleForward` | `alt+l` | Step task panel view: tasks → subagents → teams |
| `app.tasks.cycleBackward` | `shift+alt+l` | Step task panel view backward |

### View — what is on screen right now (`Ctrl`)

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.tools.expand` | `ctrl+o` | Jump to the full tool-output view and back, without moving the dial |
| `app.thinking.toggle` | `ctrl+t` | Show or hide thinking blocks |
| `app.team.focus` | `alt+n` | Focus the team roster (`--team`) |

### Cockpit — what the agent is, and where it works (`Alt`)

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.model.select` | *(none)* | Open the model selector (`/model`) |
| `app.session.changeDirectory` | `alt+w` | Change working directory (`/cd`) |
| `app.session.tree` | *(none)* | Open the session tree (`/tree`) |
| `app.session.resume` | `alt+h` | Resume a session from history |
| `app.settings.open` | `alt+s` | Open settings |
| `app.hotkeys.open` | `alt+k` | Show the shortcut list (`/hotkeys`) |
| `app.editor.external` | `alt+e` | Edit the message in `$VISUAL` / `$EDITOR` |
| `app.input.voiceTranscribe` | `alt+r` | Record voice and transcribe into the editor |
| `app.session.new` | *(none)* | Start a new session (`/new`) |
| `app.session.fork` | *(none)* | Fork the current session (`/fork`) |

`app.session.new` and `app.session.fork` ship unbound: one replaces the
transcript and the other needs a message picked out of it, so neither wants to
be one stray chord away. `app.model.select` and `app.session.tree` are unbound
for a different reason — each gave its letter to the dial that shares it, and
`/model` and `/tree` open them. All four are bindable by hand.

### Prompt editor

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `tui.editor.cursorUp` | `up` | Move cursor up / browse history when empty |
| `tui.editor.cursorDown` | `down` | Move cursor down |
| `tui.editor.cursorLeft` | `left`, `ctrl+b` | Move cursor left |
| `tui.editor.cursorRight` | `right`, `ctrl+f` | Move cursor right |
| `tui.editor.cursorWordLeft` | `alt+left`, `ctrl+left`, `alt+b` | Move cursor word left |
| `tui.editor.cursorWordRight` | `alt+right`, `ctrl+right`, `alt+f` | Move cursor word right |
| `tui.editor.cursorLineStart` | `home`, `ctrl+a` | Move to line start |
| `tui.editor.cursorLineEnd` | `end`, `ctrl+e` | Move to line end |
| `tui.editor.jumpForward` | `ctrl+]` | Jump forward to character |
| `tui.editor.jumpBackward` | `ctrl+alt+]` | Jump backward to character |
| `tui.editor.pageUp` | `pageUp` | Scroll up by page |
| `tui.editor.pageDown` | `pageDown` | Scroll down by page |
| `tui.editor.deleteCharBackward` | `backspace` | Delete character backward |
| `tui.editor.deleteCharForward` | `delete`, `ctrl+d` | Delete character forward |
| `tui.editor.deleteWordBackward` | `ctrl+w`, `alt+backspace` | Delete word backward |
| `tui.editor.deleteWordForward` | `alt+d`, `alt+delete` | Delete word forward |
| `tui.editor.deleteToLineStart` | `ctrl+u` | Delete to line start |
| `tui.editor.deleteToLineEnd` | `ctrl+k` | Delete to line end |
| `tui.editor.yank` | `ctrl+y` | Paste the most recently deleted text |
| `tui.editor.yankPop` | `alt+y` | Cycle through deleted text after a yank |
| `tui.editor.undo` | `ctrl+-` | Undo last edit |
| `tui.input.copy` | `ctrl+c` | Copy the selection |

`ctrl+d` and `ctrl+c` each carry two ids. They are one key with a
state-dependent meaning, not a conflict: `ctrl+d` exits only on an empty
editor and deletes a character otherwise, and `ctrl+c` copies when there is a
selection and clears the editor when there is not.

### Lists and pickers, everywhere

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `tui.select.up` | `up` | Move selection up |
| `tui.select.down` | `down` | Move selection down |
| `tui.select.pageUp` | `pageUp` | Page up in list |
| `tui.select.pageDown` | `pageDown` | Page down in list |
| `tui.select.confirm` | `enter` | Confirm selection |
| `tui.select.cancel` | `escape`, `ctrl+c` | Cancel selection |

### Session picker (`/resume`)

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.session.togglePath` | `alt+p` | Toggle path display |
| `app.session.toggleSort` | `alt+o` | Toggle sort order |
| `app.session.toggleNamedFilter` | `alt+n` | Toggle the named-only filter |
| `app.session.rename` | `alt+r` | Rename the selected session |
| `app.session.delete` | `alt+x` | Delete the selected session |
| `app.session.deleteNoninvasive` | `ctrl+backspace` | Delete, but only while the query is empty |

### Session tree (`/tree`)

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.tree.foldOrUp` | `ctrl+left`, `alt+left` | Fold the branch segment, or jump to the previous one |
| `app.tree.unfoldOrDown` | `ctrl+right`, `alt+right` | Unfold the branch segment, or jump to the next one |
| `app.tree.editLabel` | `alt+l` | Edit the label on the selected node |
| `app.tree.toggleLabelTimestamp` | `alt+t` | Show or hide label timestamps |
| `app.tree.filter.default` | `alt+1` | Filter: default view |
| `app.tree.filter.noTools` | `alt+2` | Filter: hide tool results |
| `app.tree.filter.userOnly` | `alt+3` | Filter: user messages only |
| `app.tree.filter.labeledOnly` | `alt+4` | Filter: labeled entries only |
| `app.tree.filter.all` | `alt+5` | Filter: show everything |
| `app.tree.filter.cycleForward` | `alt+c` | Cycle the filter forward |
| `app.tree.filter.cycleBackward` | `shift+alt+c` | Cycle the filter backward |

The five lenses are numbered rather than lettered because they are an ordered
set: `alt+1`-`alt+5` needs no mnemonic. Everything else the tree types goes
into its search query.

### Scoped models picker (`/models`)

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.models.save` | `alt+s` | Save the selection to settings |
| `app.models.enableAll` | `alt+a` | Enable all models (or all matching the query) |
| `app.models.clearAll` | `alt+x` | Clear all models (or all matching the query) |
| `app.models.toggleProvider` | `alt+g` | Toggle every model for the current provider |
| `app.models.reorderUp` | `alt+up` | Move the selected model up in the cycle order |
| `app.models.reorderDown` | `alt+down` | Move the selected model down in the cycle order |

`alt+g` ("group") rather than the obvious `alt+p`: without the Kitty protocol
`alt+p` arrives as `ESC-p`, which the parser also reads as `alt+up` — this
picker's reorder key.

### Team roster (`--team`)

Live only while the task panel holds focus, which is why they are plain
letters: you are never typing there.

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.team.nudge` | `n` | Nudge the selected role |
| `app.team.attach` | `a` | Attach to the selected role |

### Options pane

The pane the agent raises to ask you a question.

| Keybinding id | Default | Description |
|--------|---------|-------------|
| `app.options.next` | `right` | Confirm the highlighted answer and advance |
| `app.options.back` | `left` | Go back to the previous question |

On the free-text row the arrows are the text cursor's first, and only act on
the step at the ends of what you have typed. `enter` always commits.

## Custom Configuration

Create `~/.hoocode/keybindings.json`:

```json
{
  "tui.editor.cursorUp": ["up", "ctrl+p"],
  "tui.editor.cursorDown": ["down", "ctrl+n"],
  "tui.editor.deleteWordBackward": ["ctrl+w", "alt+backspace"]
}
```

Each action can have a single key or an array of keys. User config overrides defaults.

On native Windows, `app.suspend` has no default binding because Windows terminals do not support Unix job control. If you bind it manually, hoocode shows a status message instead of suspending. In WSL, the normal Linux `ctrl+z`/`fg` behavior still applies.

### Emacs Example

```json
{
  "tui.editor.cursorUp": ["up", "ctrl+p"],
  "tui.editor.cursorDown": ["down", "ctrl+n"],
  "tui.editor.cursorLeft": ["left", "ctrl+b"],
  "tui.editor.cursorRight": ["right", "ctrl+f"],
  "tui.editor.cursorWordLeft": ["alt+left", "alt+b"],
  "tui.editor.cursorWordRight": ["alt+right", "alt+f"],
  "tui.editor.deleteCharForward": ["delete", "ctrl+d"],
  "tui.editor.deleteCharBackward": ["backspace", "ctrl+h"],
  "tui.input.newLine": ["shift+enter", "ctrl+j"]
}
```

`ctrl+p` and `ctrl+n` are both free of app bindings by default — the model and
task-ledger dials that used to hold them are on `alt+m` and `alt+l` now — so
this config collides with nothing.

### Vim Example

```json
{
  "tui.editor.cursorUp": ["up", "alt+k"],
  "tui.editor.cursorDown": ["down", "alt+j"],
  "tui.editor.cursorLeft": ["left", "alt+h"],
  "tui.editor.cursorRight": ["right", "alt+l"],
  "tui.editor.cursorWordLeft": ["alt+left", "alt+b"],
  "tui.editor.cursorWordRight": ["alt+right", "alt+w"]
}
```

This one takes `alt+h` from the session picker, `alt+k` from the shortcut list
and `alt+w` from `/cd`; rebind those too if you use them.
