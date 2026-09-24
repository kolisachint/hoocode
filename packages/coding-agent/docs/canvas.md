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
| `list_canvas_capabilities` | What is open: each canvas's own description, and the actions it declares with their schemas |
| `invoke_canvas_action` | Call an action on an instance, with input matching its declared schema |
| `reload_canvas` | Re-fork an extension after its code changed, keeping instance ids |

A session that never opens a canvas pays nothing for them, and they answer
honestly when nothing is open.

**The canvas's `description` is how the model learns to work with it.** It is
shown verbatim in `list_canvas_capabilities`, so a canvas author can use it for
the working loop and what is fast or slow, not just a one-line summary.

**Each action reports how fast it really is.** Once an action has run in the
session, the listing carries `observed_ms`, the median of its last 20 calls
measured by hoocode. A read that takes 2 ms and a round trip through the
person's browser look alike in a schema, and a model plans differently around
them.

**Inputs sent as JSON strings are decoded.** Some models (Qwen through
OpenAI-compatible gateways) send `input` as a JSON-encoded string. A string
that starts with `{` or `[` and parses is passed to the canvas as the value it
encodes; any other string is passed through as is.

## Working with an open canvas

For the agent driving a canvas a person has opened. The person is usually
looking at the same surface in their browser while you work.

1. **Discover.** Call `list_canvas_capabilities`. Read each canvas's
   `description` first: it is the canvas author's guide to working with it
   (the loop, what is fast, what to avoid). Then the actions, with their
   `inputSchema`s. Take `instanceId` from here; it stays the same across a
   reload.
2. **Read, then act.** Call the canvas's read action before changing anything
   the person may have touched. A canvas that tracks the person may refuse
   edits over work you have not seen.
3. **Batch.** Put many changes into one call when the action takes a list.
   The action itself is usually milliseconds; each extra call costs a whole
   model turn.
4. **Read the refusal.** A failing action comes back as `code: message`, where
   `code` is the canvas's own error code (for example `invalid_input`). The
   message says what to do next. A good canvas puts
   everything you need to retry into it, so resend straight away instead of
   re-reading.
5. **Point the person at your work.** If the canvas has a `focus`-style action,
   use it after a change they should look at.

`input` must match the action's `inputSchema`, as an object. hoocode decodes
an `input` sent as a JSON string, but an object is what the schema asks for.

### How long canvas actions take

The action runs in the extension's own process, reached over a local pipe, so
host overhead is a millisecond or two. What remains is the action itself:

| Kind of action | Typical time |
|---|---|
| Reads and edits of the canvas's own state | 1–5 ms |
| A change appearing in the person's browser | 10–50 ms |
| Anything done *in* the person's browser (screenshot, focus, layout, export) | 10 ms–0.5 s |
| A model turn, for comparison | seconds |

`observed_ms` in `list_canvas_capabilities` gives the median hoocode measured
for each action in this session. Results over 8,000 characters are cut at that
point and say so. Prefer the canvas's filtered reads (by id, by page) over
reading everything.

## Talking back

A canvas can also start the conversation. The canvas SDK's session object has
`send` and `on`, and hoocode implements the part of them a canvas needs:

```js
const session = await joinSession({ canvases: [canvas] });

// Ask the agent for something. Resolves with a message id.
await session.send({ prompt: "The person asked: tidy the selected shapes", mode: "enqueue" });

// Watch what the agent is doing. Returns a function that stops listening.
session.on("assistant.intent", (event) => showHint(event.data.intent));
session.on("session.idle", () => showIdle());
```

**What the agent sees.** A sent message arrives as a user message labelled
`[canvas <id>]`, so it shows in the transcript like anything the person types.
When it reaches the agent is hoocode's decision, not the canvas's:

- **Agent idle:** a turn starts.
- **Agent busy:** the message waits for the turn to end. A later message from the
  same canvas *replaces* the waiting one, so send your current state each time
  ("3 open requests"), not a delta.
- **`mode: "immediate"`** steers the running turn, at most once per canvas every
  10 seconds; otherwise it waits like `enqueue`.
- **No loops without a person.** A canvas can start at most 3 turns in a row
  without the person saying anything. After that its message waits for the
  person's next message, and the person is told once.

Send only when the person asked for something, or when something they did
clearly needs the agent. Sending on every edit costs the person a model turn
each time.

**What a canvas can hear.** `on(type, handler)` for one type, `on(handler)` for
all of them. Names and fields are GitHub's session events. hoocode sends:

| Event | `data` |
|---|---|
| `assistant.turn_start` | `turnId` |
| `assistant.intent` | `intent`: one line on what the agent is doing, derived from the tool it is running; sent when it changes |
| `tool.execution_start` | `toolCallId`, `toolName`, `toolTitle` (one line, never the arguments) |
| `tool.execution_complete` | `toolCallId`, `success`, `toolTitle` |
| `session.idle` | nothing |
| `session.todos_changed` | `todos`: the agent's todo list as `{id, title, status}`; upstream sends this event empty |

Events cost nothing unless a canvas subscribes to them. Other event types are
never sent. A canvas that also runs in a host without `send` or `on` should
check they exist before calling them.

## Making a canvas agents can use well

For canvas authors. The agent learns everything from the declaration, so
write it for the model:

- **Make `description` the playbook, not a tagline.** Give the loop in one
  sentence, the collaboration rule (build on the person's work, never over
  it), and measured speed classes. hoocode shows it verbatim in
  `list_canvas_capabilities`.
- **Say what really happens.** If a batch applies partly, say so. "All or
  nothing" that is not true makes a model resend what already worked.
- **Validate input against your own `inputSchema`** and refuse with
  `CanvasError("invalid_input", …)`, naming the field and what would have
  worked. hoocode checks the three outer tool fields, not the contents of
  `input`. An uncaught `TypeError` reaches the model as `internal_error`,
  which it cannot act on.
- **Put the fix in the refusal.** When an edit is refused because the person
  changed something, include what it is now, so the retry needs no re-read.
- **Never hang.** Anything that waits on the person's browser needs a timeout,
  and should fail at once when no browser is connected.
- **Keep results small.** Return ids and summaries; let the agent ask for
  detail. Results over 8,000 characters are cut.

## Testing a canvas end to end

Unit tests of the handlers catch most bugs; these catch the rest:

- **A sweep.** Send every action right, wrong and strange input (wrong types,
  `null`, unknown fields, missing pages or ids, broken markup, path escapes,
  large batches), and fail on anything but a success or a coded refusal with a
  usable message.
- **Through real hoocode.** Run `hoocode --mode rpc` with a scripted model: a
  small HTTP server speaking OpenAI chat completions, registered as a custom
  provider in `~/.hoocode/models.json` (see [Custom models](models.md)).
  Install the canvas with `/plugin marketplace add <path>` and
  `/plugin install`, open it with `/canvas open`, and drive a browser with
  Playwright as the person.

## Troubleshooting canvases

| Symptom | Cause and fix |
|---|---|
| `/canvas list` shows nothing after `/plugin install` | The plugin has no `extension.mjs` where hoocode looks: the plugin's root (the canvas id is then the plugin name), or `extensions/<id>/` (or the directory its manifest's `extensions` key names). |
| `[withheld: untrusted workspace]` | The canvas comes from repository content. `/plugin trust` if you trust this checkout. |
| `/canvas open` says "cancelled" from an IDE or RPC host | Fixed. Upgrade hoocode; older builds read RPC's non-drawing UI as a cancel. |
| `/plugin marketplace add /abs/path` says "Path not found" | Fixed. Upgrade hoocode; older builds joined an absolute path onto the workspace. |
| The browser tab stopped updating after `reload_canvas` | Every reload binds a new port. Open the new url it printed. |
| An action that runs in the person's browser fails at once | No tab is open on the canvas. Ask them to open the url pinned above the prompt. A good canvas waits while a tab is still loading, and says which of the two it is. |
| A `[canvas <id>]` message appeared that you did not type | The canvas sent it with `session.send`, usually because you asked for help from the canvas itself. See [Talking back](#talking-back). |
| The model sends `input` as a string | Handled: hoocode decodes JSON strings before validation. |

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
