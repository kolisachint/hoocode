# Keybindings

Keys are set in **`~/.hoocode/keybindings.json`** (or in the folder set by
`HOOCODE_CODING_AGENT_DIR`). The file is optional. Without it you get the
defaults. Edit the file, then run `/reload` to apply it.

Run `hoocode keybindings --defaults` to print the defaults in the same form the
file uses. hoocode also writes that text to `~/.hoocode/keybindings.defaults.json`
after each upgrade. Treat that copy as a reference. hoocode never reads it as
your file.

## Two forms

The flat form maps an action to a key, or to a list of keys:

```json
{"app.model.cycleForward": "ctrl+x m", "app.hotkeys.open": ["ctrl+x ?", "ctrl+x k"]}
```

The grouped form lists keys by context. Single keys and leader sequences both
work in it. A `null` or `"none"` unbinds a key in that context:

```json
{
  "leader": "ctrl+x",
  "leaderTimeoutMs": 1500,
  "bindings": [
    {"context": "Prompt", "bindings": {"ctrl+x m": "app.model.cycleForward", "ctrl+x g": null}}
  ]
}
```

Contexts are `Global`, `Prompt`, `Leader`, `Scroll`, `Picker`, `Panel`, `Dialog`,
`Tree` and `Models`. A key is written as `"shift+enter"`, and a space means a
sequence, as in `"ctrl+x m"`. A single capital letter is Shift plus that letter:
`"G"` is the same key as `"shift+g"`, the one that types a capital G.

A binding applies where its action runs. `"u": "app.scroll.lineDown"` is a
`Scroll` binding, and it is skipped with a message if you write it under
`Prompt`. Each action's context is the one `hoocode keybindings --defaults`
groups it under. `Global` applies everywhere. In a context, a key bound to an
action takes that key from the other actions of the same context, so
`{"context": "Dialog", "bindings": {"y": "app.dialog.no"}}` makes `y` answer no
and not yes. A `null` takes the key away from every action of its context: the
`Scroll` `j` above no longer scrolls, and the picker's `j` still moves down.

`Global` applies everywhere, with one difference. A `Global` binding to an action
takes the key only from the other `Global` actions (today, only `app.interrupt`).
So `{"context": "Global", "bindings": {"escape": "app.interrupt"}}` keeps Esc in the
pickers, the pinned view and the task panel. A `Global` `null` still removes the
key in every context.

## Which binding wins

- The flat form sets an action's keys. Its list replaces the action's defaults.
  A flat key has no context, so it works in every context the action runs in.
- The grouped form is applied after the flat form. In a context, a grouped key
  wins over a flat key for the same key: it moves the key to its action and takes
  it from the other actions of that context. A grouped `null` removes it there.
- A flat key that another app action uses by default, in the same context (or in
  `Global`), is kept by both, and hoocode warns and names both. To give the key to
  one action only, bind it in the grouped form, or set the other action's keys in
  the flat form.

## The leader

Leader keys start with the leader (`ctrl+x` by default), then one key. Set another
leader with `"leader"`. Its timeout is `"leaderTimeoutMs"`. In the `Leader`
context, write the key after the leader alone: `{"context": "Leader", "bindings": {"m": "..."}}`.

The leader is one key. A sequence, or Esc, Enter, Tab or Backspace, is refused and
`ctrl+x` is used instead. A leader that is also the default key of an action
starts the leader first, so that key can no longer reach the action, and hoocode warns. The key after
the leader is checked against the reserved keys too: `ctrl+x ctrl+c` is refused.
A flat binding of the leader key by itself is skipped with a message: bind a
sequence after the leader instead, such as `"ctrl+x m"`.

## Keys hoocode keeps

These keys cannot be rebound. A binding to one is skipped with an error:
`ctrl+c`, `ctrl+d`, `ctrl+z`, `ctrl+m`, `ctrl+i`, `ctrl+[`, `ctrl+h`.

These keys are often taken by the terminal or the shell (flow control, or the
NUL that Ctrl+Space sends). They can be bound, with a caution that they may never
reach hoocode: `ctrl+s`, `ctrl+q`, `ctrl+b`, `ctrl+space`.

## Problems

hoocode checks the file at startup and on `/reload`. It never stops because of
it: a bad part is skipped, and the rest applies. The problems it reports:

- a file that is not JSON, or a value that is not a key or an action;
- an action id that does not exist;
- a key name hoocode does not know, such as `"ctrl+foo"` or `"shfit+enter"`
  (a misspelled modifier is not read as the plain key). In a flat list, the other
  keys still apply, and when every key of an action is refused, its default keys stay;
- a binding whose action runs in another context (see above);
- a key that is bound twice in the same context with different actions (the first is skipped);
- a key that is listed for two actions in the flat form (both are kept, and the first id in sorted order runs);
- a flat key that another app action uses by default, in the same context (both keep it);
- a leader that is refused, or that is also a default key;
- a reserved key, as above, also after the leader.

`/hotkeys` lists the problems under the key groups. Startup and `/reload` show a
one-line count.

## Terminals

Some keys depend on the terminal. These are the usual cases.

- **Shift+Enter** (the newline key, `tui.input.newLine`) reaches hoocode only in a
  terminal that uses the kitty keyboard protocol or xterm's modifyOtherKeys. In any
  other terminal Shift+Enter sends the same bytes as Enter, which sends the message.
  `ctrl+x enter` is a newline everywhere.
- **tmux** passes Shift+Enter and other modified keys only with `set -g extended-keys on`
  in `~/.tmux.conf`. Reload the file with `tmux source-file ~/.tmux.conf`.
- **macOS Terminal.app** does not send Shift+Enter. Use `ctrl+x enter` for a newline.
- **VS Code's terminal** may take Ctrl+X for its own commands, so the leader does
  nothing there. Set another leader, for example `{"leader": "ctrl+g"}`.
- **Option as Meta** (Terminal.app's and iTerm2's setting that makes Option send
  Alt): hoocode's defaults no longer use Alt keys, so Alt word motions are gone.
  Word motions are Ctrl+Left and Ctrl+Right, delete a word backward with Ctrl+W, and
  forward with Ctrl+Delete. To get the Alt keys back, add them to the flat form. An
  action keeps its defaults and gains the Alt keys:

```json
{
  "tui.editor.cursorWordLeft": ["ctrl+left", "alt+b"],
  "tui.editor.cursorWordRight": ["ctrl+right", "alt+f"],
  "tui.editor.deleteWordBackward": ["ctrl+w", "alt+backspace"]
}
```

## Commands

- `hoocode keybindings --defaults` prints the defaults.
- `hoocode keybindings check` reads your file and prints each error and caution.
  It exits with 1 when there is an error, so you can use it in a script.
- `hoocode keybindings --read` puts the terminal in raw mode and prints the bytes
  each key press sends, with the key id they parse to. Press `q`, or Ctrl+C twice,
  to exit. Keys are read as a legacy terminal sends them. Shift+Enter, for example,
  shows only where the terminal sends it without the kitty protocol.

## Alt keys

Earlier builds used Alt keys as defaults. Alt keys in your file still work. The
defaults no longer use them. The first start of each version shows one notice that
names the Alt keys your file binds. hoocode does not rewrite your file.
