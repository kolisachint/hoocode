# Project-local resources via `.agents/`

hoocode discovers project-local resources from four directory conventions. Each resource type (slash commands, subagents, MCP servers, skills) has its own precedence order across these roots.

## Discovery roots

| Root | Scope | Notes |
|------|-------|-------|
| `.hoocode/` | project + user (`~/.hoocode/`) | hoocode-native convention |
| `.agents/` | project (ancestor-walked) + user (`~/.agents/`) | cross-vendor convention |
| `.claude/` | project + user (`~/.claude/`) | Claude Code native import |
| `hoocode.*` manifest fields | installed packages | declared in `package.json` |

---

## Ancestor-walk-to-git-root rule

Source: `packages/coding-agent/src/utils/paths.ts` (`collectAgentsAncestorDirs`, `findGitRepoRoot`).

For `.agents/<subdir>` only, hoocode walks upward from the current working directory, collecting `<dir>/.agents/<subdir>` at each level, stopping at (and including) the nearest ancestor that contains a `.git` entry (the git repo root). If the cwd is not inside a git repo, the walk continues to the filesystem root.

Directories are collected **cwd-first**, so a closer `.agents/` directory beats an ancestor under first-match-wins semantics.

---

## Slash commands / prompt templates

Source: `packages/coding-agent/src/core/resource-loader.ts` (`defaultSlashCommandDirs` in `reload()`).

**First-match-wins**, highest precedence first:

1. `{cwd}/.hoocode/commands`
2. `{cwd}/.claude/commands`
3. `.agents/commands` — ancestor-walk from cwd up to git root, cwd-first
4. `{agentDir}/commands` (i.e. `~/.hoocode/commands`)
5. `~/.agents/commands`
6. `~/.claude/commands`

Pass `--no-slash-commands` or `--no-prompt-templates` to disable discovery. Explicit `--slash-command`/`--prompt-template` paths still load when discovery is off.

---

## Subagents

Source: `packages/coding-agent/src/core/agent-registry.ts` (`loadAgentRegistry`).

Registered **lowest precedence first**; later sources override earlier ones by name:

1. Built-in agents
2. Package-manifest agents (`hoocode.agents` in `package.json`)
3. `~/.claude/agents` (source `claude-user`)
4. `{agentDir}/agents` i.e. `~/.hoocode/agents` (source `user`)
5. `.agents/agents` — ancestor-walk from cwd up to git root, registered git-root-first so the cwd-level entry overrides ancestors (source `project`)
6. `{cwd}/.claude/agents` (source `claude-project`)
7. `{cwd}/.hoocode/agents` (source `project`)
8. CLI `--agent` paths (highest precedence)

---

## MCP servers

Source: `packages/coding-agent/src/extensions/core/mcp-loader.ts` (`setupMcpLoader`).

**First-wins by server name**, in this order:

1. `~/.agents/mcp.json` — user, standard format (`mcpServers` object)
2. `./.agents/mcp.json` — project, standard format
3. `~/.config/claude/mcp.json` — Claude Desktop, standard format
4. `~/.hoocode/mcp-servers/*.json` — user, one file per server (each with top-level `name`/`command`)
5. `./.hoocode/mcp-servers/*.json` — project, per-server format

Standard `mcp.json` format:

```json
{
  "mcpServers": {
    "my-server": {
      "command": "npx",
      "args": ["-y", "my-mcp-server"],
      "env": { "API_KEY": "..." },
      "promptSnippet": "Use these tools for acme-db queries instead of raw SQL over bash",
      "promptGuidelines": ["Prefer my-server tools over bash psql for database work"]
    }
  }
}
```

`promptSnippet` and `promptGuidelines` are optional server-level steering for the
system prompt. With eager schemas they ride on each of the server's tools; with
deferred schemas (the default) the snippet annotates the server's line in the
`ResolveMcpTools` catalog and the guidelines ride on the resolver, so the model
is steered toward the server before resolving any schema. Plugin-provided
servers (`mcpServers` in a plugin manifest or `.mcp.json`) accept the same
fields.

Per-server format (`.hoocode/mcp-servers/<name>.json`):

```json
{
  "name": "my-server",
  "command": "npx",
  "args": ["-y", "my-mcp-server"]
}
```

---

## Debugging discovery

Run `hoocode resources` to print everything that was discovered for the current cwd — skills, subagents, slash commands, and MCP servers — with source path and origin label.

Source: `packages/coding-agent/src/resources-cli.ts`.

```
$ hoocode resources
Resources discovered for /path/to/project

Skills (0)
  (none)

Subagents (1)
  docs - Maintain project documentation  [project (.hoocode)]
    /path/to/project/.hoocode/agents/docs.md

Slash commands (1)
  /review - Review a diff for risk  [project (.hoocode)]
    /path/to/project/.hoocode/commands/review.md

MCP servers (1)
  my-db-tools  [project (.agents)]
    /path/to/project/.agents/mcp.json
```

This is the fastest way to verify why a `.agents/` resource is or is not being loaded.
