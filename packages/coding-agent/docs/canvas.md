# Canvas extensions

A canvas extension is a separate process that serves an interactive surface —
a web UI the agent can also drive — while the session keeps running. Canvases
are how a task that needs a real interface (a diff explorer, a chart, a form)
gets one without leaving the terminal.

hoocode implements GitHub's canvas wire protocol, so extensions written for
GitHub Copilot work unchanged.

## Using canvases

```
/canvas list                            # what the loaded extensions provide
/canvas open <extension>[:<canvas>]     # open one
/canvas close <instanceId>              # close a running one
```

Esc during an open cancels it, and the extension is told to release any port it
had already bound — the spinner disappearing is not the whole story.

## Discovery

An extension is a directory containing an `extension.mjs` entry file. That file
is the entire detection contract: no `package.json` is read, and the directory
name is the extension id.

Search roots, in precedence order:

| Directory | Scope |
|-----------|-------|
| `./.agents/extensions/` | Project (hoocode convention) |
| `./.github/extensions/` | Project (Copilot convention) |
| `~/.copilot/extensions/` | User |

Only ES modules are supported.

Plugins can also ship canvases; see [Plugins](plugins.md).

## Trust

`.github/extensions/` travels with a clone, so a canvas extension found there is
repository-supplied code. A canvas extension is a process **that also opens a
listening socket**, so it sits behind the same workspace-trust record as plugin
hooks and MCP servers — granted with `/plugin trust`, revoked with
`/plugin untrust`, and stored outside the repository so repository content
cannot forge it. See [Plugins → Trust](plugins.md#trust).

Discovery itself is read-only and always runs; the gate applies before an
extension is forked.

## Agent-facing tools

Two tools register on the **first successful open** and stay for the session:

| Tool | Purpose |
|------|---------|
| `list_canvas_capabilities` | What is open and which actions each instance declares |
| `invoke_canvas_action` | Call an action on an instance, with input matching its declared schema |

A session that never opens a canvas pays nothing for them. After the first open
they cost roughly 235 tokens, and they answer honestly when nothing is open.

## Authoring

```
/new-canvas <name>
```

This scaffolds the directory and `extension.mjs`. Scaffolding into the project
grants workspace trust, since you are demonstrably working in the directory on
purpose.

## Related

- [Extensions](extensions.md) — in-process TypeScript extensions, which canvases are not
- [Plugins](plugins.md) — distribution and the shared trust model
