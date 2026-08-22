# Canvas extensions

A canvas extension is a separate process that serves an interactive surface —
a web UI the agent can also drive — while the session keeps running. Canvases
are how a task that needs a real interface (a diff explorer, a chart, a form)
gets one without leaving the terminal.

hoocode implements GitHub's canvas wire protocol, so extensions written for
GitHub Copilot work unchanged.

## Using canvases

```
/canvas list                              # what the loaded extensions provide
/canvas open <extension>[:<canvas>]       # open one
/canvas reload [extension]                # pick up code changes without restarting
/canvas close <instanceId>                # close a running one
/canvas rename <extension> <new-name>     # rename everywhere the name appears
/canvas remove <extension>                # delete it, after confirming
```

Esc during an open cancels it, and the extension is told to release any port it
had already bound — the spinner disappearing is not the whole story.

`rename` matters more than it looks: a canvas's name lives in four places — the
directory (which *is* the extension id), the canvas's own `id`, its
`displayName`, and its header comment. Getting the `id` wrong by hand drops the
canvas you are looking at on the next reload. `rename` does all four at once,
closes what was open first, and prints every line it rewrote; it only touches a
string that is *entirely* the old name, so a sentence merely mentioning the
canvas is reported rather than rewritten.

`rename` and `remove` both refuse a canvas that came from a plugin, and point at
`/plugin` instead.

## Reloading

Editing an extension's code while it is open used to do nothing: the running
process was forked from the old code, so neither the open page nor a newly
opened second instance saw the change, and only restarting the session helped.

`/canvas reload` (and the `reload_canvas` tool) forks the new code and asks it
for its declarations **before** stopping the old process, so an edit that does
not run leaves the canvas you are looking at exactly as it was and reports the
error instead.

Instances keep their ids and the input they were opened with, but each gets a
**new url** — the extension binds a new port and mints a new token on every
open — so the previous browser tab is dead and the replacement url is printed.

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

These register on the **first successful open** and stay for the session:

| Tool | Purpose |
|------|---------|
| `list_canvas_capabilities` | What is open and which actions each instance declares |
| `invoke_canvas_action` | Call an action on an instance, with input matching its declared schema |
| `reload_canvas` | Re-fork an extension after its code changed, keeping instance ids |

A session that never opens a canvas pays nothing for them, and they answer
honestly when nothing is open.

## Authoring

```
/new-canvas <what it should do>          # scaffold, open, and build it
/new-canvas <name>                       # scaffold the template only
/new-canvas <name>: <what it should do>  # name it yourself, then build
```

Given a description, hoocode scaffolds the extension, derives and reports a
directory name, opens the canvas, and hands the agent a brief to build it —
which you then steer like any other turn, with `reload_canvas` picking up each
edit. Given a bare name, you get the template to edit by hand and no build
starts.

Scaffolding into the project grants workspace trust, since you are demonstrably
working in the directory on purpose.

hoocode ships one canvas of its own at `.agents/extensions/arrow-key-games/`
(`/canvas open arrow-key-games`), built this way.

## Related

- [Extensions](extensions.md) — in-process TypeScript extensions, which canvases are not
- [Plugins](plugins.md) — distribution and the shared trust model
