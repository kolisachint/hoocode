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
| `~/.hoocode/themes/` | Custom themes |
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

`hoocode config` opens a list for turning skills, prompts and other resources on
or off.

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
- **Always** on an approval card: for Shell it lasts for the session and covers that
  program; for other tools it adds the tool to the mode's `auto_allow` in this file.
- A project can add rules in `.hoocode/hoo-config.json`: its `denied_tools` and
  `denied_bash_commands` add to yours. It cannot make anything run without asking:
  `auto_allow`, `safe_shell` and `skip_permissions` come from your file only, and so
  does `allowed_write_paths` when your file sets it.
- A write to hoocode's own project config (`.hoocode/*.json`, such as
  `.hoocode/hoo-config.json` or `.hoocode/settings.json`) always asks, in every mode and
  even with `skip_permissions`.
