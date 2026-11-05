# Configuration

Everything hoocode keeps lives in **`~/.hoocode`**. Set `HOOCODE_CODING_AGENT_DIR`
to move it.

## Files

| File or folder | What it holds |
|---|---|
| `~/.hoocode/settings.json` | Global settings (`/settings` edits it) |
| `~/.hoocode/auth.json` | Logins and keys saved by `/login` |
| `~/.hoocode/models.json` | Your own providers and models, for example a local OpenAI-compatible server |
| `~/.hoocode/hoo-config.json` | Agent modes and tool approval rules, written on first run ([below](#modes-and-approvals)) |
| `~/.hoocode/skills/` | Skills |
| `~/.hoocode/themes/` | Custom themes, one JSON file each. `/theme` lists `auto`, the built-in themes `dark` and `light`, and your files here (by file name, with the path shown). A theme may set `"inherits": "<name>"` to take the colors it leaves out from a built-in theme or another file here. The `theme` setting also accepts `auto` (follows the terminal background). A file that does not load, or a name that is not a theme, shows a warning under theme diagnostics: a switch keeps the theme on screen, and at startup the built-in `dark` stands in until the setting is fixed. |
| `~/.hoocode/sessions/` | Saved sessions |

## Per project

| File or folder | What it holds |
|---|---|
| `.hoocode/settings.json` | Project settings, merged over the global ones |
| `.hoocode/skills/` | Project skills |
| `AGENTS.md` or `CLAUDE.md` | Project instructions, read from the working folder and its parents |

## MCP servers

Declared in `~/.agents/mcp.json` (yours) and `.agents/mcp.json` (the project's).
hoocode asks before it starts servers a project declares. `/mcp` shows each server's
state, and `/mcp login <server>` signs in to one that needs it.

## Plugins

Plugins add skills, subagents and MCP servers. They live in `~/.agents/plugins/`
(yours) and `.agents/plugins/` (the project's). Manage them with `/plugin` in a
session: add a marketplace, then install from it.

```text
/plugin marketplace add <git-url-or-owner/repo>
/plugin install <plugin>@<marketplace>
```

The same commands work from the shell, without a session: `hoocode install`,
`hoocode remove`, `hoocode list` and `hoocode plugin ...`. See
[Commands](commands.md).

`hoocode config` opens a list for turning extensions, skills and prompts on or
off. Themes are not in it: pick one with `/theme`.

## Herdr

Run hoocode in a [Herdr](https://herdr.dev) pane and it tells Herdr what it is
doing. Nothing to set up: it turns on by itself inside Herdr, and does nothing
anywhere else.

- The sidebar shows **working** during a turn, **idle** when hoocode waits for you,
  and **blocked** when it needs a decision (an approval card or a question).
- Herdr stores the command that reopens your session. After a Herdr restart, the
  pane opens the same session again (`hoocode --session <id>`).
- When you quit hoocode, it releases the pane.

To show the hoocode icon in Herdr desktop, save an icon (an SVG) as
`~/.config/herdr-app/my-icons/hoocode.svg` and add this to
`~/.config/herdr-app/programs.json`:

```json
{ "programs": {
    "hoocode": { "name": "hoocode", "icon": "my-icons/hoocode.svg",
                 "tint": false, "tile": "#09090B", "color": "#ffffff",
                 "match": ["hoo"], "run": ["hoocode"] } } }
```

## Screen

The screen has the transcript, one prompt row (` › `) and a status line under it that
holds the view, session, model, context use, cost and what hoocode is doing now. There
is no header, logo or frame; dialogs open as panes in the prompt's place.

- **Views** (`alt+o` cycles, `shift+alt+o` goes back): `outline` shows one recap row per
  turn, `peek` (the default) shows each call on one row with its facts, `full` shows
  every body. The old name `radar` still works as `outline` in `toolOutputView`.
- **Themes**: `dark` and `light`, and `auto` follows the terminal background and
  re-checks when it changes. Pick one with `/theme`, which also lists your own themes.
  A pick is saved to your global settings. If this repo's `.hoocode/settings.json` sets
  `theme`, that wins over the pick, and the picker says so.
  The older names (`daylight`, `deep-space`, `colorsafe-*`, `high-contrast-*`, `warm-*`,
  `solarized-*`) are gone; a saved value that no longer exists shows a warning under
  theme diagnostics and the built-in `dark` stands in until you pick another theme.
- **No colour**: with `NO_COLOR` set, `TERM=dumb` or fewer than 256 colours, panes get
  light borders and the selected row keeps its ` › ` marker.
- **Removed**: the `/chrome` and `/color` commands and the `alt+z` key. Notices no longer
  pop up as a toast: a short one takes the status line's left side for a few seconds,
  a longer one sits as dim rows above the prompt.
- **First run**: with no working provider the Log in pane opens by itself. Esc closes it
  and the status line shows `not signed in  /login` until a provider works.

## Modes and approvals

`~/.hoocode/hoo-config.json` is written the first time hoocode runs:

```json
{
  "active_mode": "build",
  "safe_shell": true,
  "skip_permissions": false,
  "modes": {
    "ask": { "denied_tools": ["Write", "Edit"] },
    "debug": { "denied_tools": ["Write", "Edit"] },
    "plan": { "allowed_write_paths": [".hoocode/*"], "auto_allow": ["Write", "Edit"] },
    "build": {}
  }
}
```

- **`safe_shell`**: read-only commands (`ls`, `cat`, `head`, `grep`, `rg`, `wc`,
  `git status`, `git diff`, `git log`, and similar, joined by pipes or `&&`) run in every
  mode without asking. A redirect to a file, `$(…)`, `$VAR`, a flag that writes or runs
  a program (`sort -o`, `rg --pre`, `git diff --output`), `tail -f`, or any other
  program asks as usual. Set it to `false` to be asked for every command.
- **`skip_permissions`**: `true` runs every tool without asking, like
  `hoocode --skip-permissions` or `HOOCODE_SKIP_PERMISSIONS=1`. The mode rules still
  apply (ask mode still blocks Edit). Only the global file can turn it on; a project's
  `.hoocode/hoo-config.json` cannot.
- **Per mode**: `denied_tools` blocks tools, `enabled_tools` allows only the listed ones,
  `allowed_write_paths` limits Edit and Write to matching paths, `auto_allow` runs
  tools without asking, and `allowed_bash_commands` / `denied_bash_commands` take regexes.
  A key you leave out takes the shipped value above; set it (even to `[]`) to override.
- **Approval card** (`Allow Shell?`): press `1` to allow once, `2` to always allow
  (when offered), `3` to refuse and tell the agent why. `e` edits the command first, and
  any other letter starts typing a reply that `enter` sends as the reason. `enter`
  takes the highlighted option and `esc` refuses. An `access:` line appears only when
  the call writes, uses the network or pipes.
- **Always** on an approval card (`2`): for Shell it lasts for the session and covers
  that program; for other tools it adds the tool to the mode's `auto_allow` in this file.
- **Questions**: when the agent asks a question, `1`-`9` pick an answer, `tab` moves to
  the next question and `esc` skips it. The agent is told it was skipped and states the
  default it will use.
- A project can add rules in `.hoocode/hoo-config.json`: its `denied_tools` and
  `denied_bash_commands` add to yours. It cannot make anything run without asking:
  `auto_allow`, `safe_shell` and `skip_permissions` come from your file only, and so
  does `allowed_write_paths` when your file sets it.
- A write to hoocode's own project config (`.hoocode/*.json`, such as
  `.hoocode/hoo-config.json` or `.hoocode/settings.json`) always asks, in every mode and
  even with `skip_permissions`.
