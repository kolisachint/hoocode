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

The grouped form lists keys by context. Leader sequences in it work. A single
key in the grouped form is checked, but the screen does not apply it yet, so use
the flat form for single keys. A `null` or `"none"` unbinds a key:

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
sequence, as in `"ctrl+x m"`.

## The leader

Leader keys start with the leader (`ctrl+x` by default), then one key. Set another
leader with `"leader"`. Its timeout is `"leaderTimeoutMs"`. In the `Leader`
context, write the key after the leader alone: `{"context": "Leader", "bindings": {"m": "..."}}`.

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
- a key that is bound twice in the same context with different actions (the first is skipped);
- a key that is listed for two actions in the flat form (both are kept, and the first id in sorted order runs);
- a reserved key, as above.

`/hotkeys` lists the problems under the key groups. Startup and `/reload` show a
one-line count.

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
