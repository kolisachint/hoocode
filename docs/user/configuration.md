# Configuration

Everything hoocode keeps lives in **`~/.hoocode`**. Set `HOOCODE_CODING_AGENT_DIR`
to move it.

## Files

| File or folder | What it holds |
|---|---|
| `~/.hoocode/settings.json` | Global settings (`/settings` edits it) |
| `~/.hoocode/auth.json` | Logins and keys saved by `/login` |
| `~/.hoocode/models.json` | Your own providers and models, for example a local OpenAI-compatible server |
| `~/.hoocode/hoo-config.json` | Agent modes and tool approval rules (`auto_allow`) |
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
