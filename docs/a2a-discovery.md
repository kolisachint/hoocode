# A2A Discovery

HooCode can advertise itself to other agents using the **A2A (Agent2Agent)
Protocol** discovery mechanism. It publishes an `AgentCard` — a JSON metadata
document describing which tools and skills the running instance offers — so a
peer agent (or a HooTeams orchestrator) can discover what this HooCode can do
before delegating work to it.

HooCode implements the **discovery** half of A2A only. It does not (yet)
implement the JSON-RPC task-execution half (`message/send`, streaming, push
notifications); the card advertises those capabilities honestly as `false`.

## Quick start

Print the card for the current working directory:

```bash
hoocode a2a
# or, explicitly
hoocode a2a --print
```

Serve it over HTTP so other agents can fetch it:

```bash
hoocode a2a --serve                 # http://127.0.0.1:41411
hoocode a2a --port 8080             # implies --serve
hoocode a2a --host 0.0.0.0 --port 0 # bind all interfaces, OS-assigned port
```

Once serving, the card is available at:

- `http://<host>:<port>/.well-known/agent.json` — the AgentCard (per the A2A spec)
- `http://<host>:<port>/.well-known/agent-card.json` — alias for newer clients
- `http://<host>:<port>/` — a short human-readable index

The subcommand makes no LLM call and never modifies files. `--serve` runs until
interrupted (Ctrl+C).

## What gets advertised

The card is assembled **dynamically** from the instance's active capabilities,
so it always reflects what this HooCode can actually do:

- **Built-in tools** are grouped into A2A skills. The default coding bundle
  (`read`, `bash`, `edit`, `write`, `grep`, `find`, `ls`) maps to the
  `shell-execution`, `file-editing`, and `code-navigation` skills. Opt-in
  bundles appear only when enabled:
  - `web-retrieval` — when web tools are enabled (`enableWebTools`)
  - `browser-automation` — when browser tools are enabled (`enableBrowserTools`)
  - `document-editing` — when document tools are enabled (`enableFileTools`)
- **Discovered skills** — every `SKILL.md` found for the current working
  directory (user, project, `.hoocode/`, and `.claude/` locations) is added as
  an A2A skill, mirroring `hoocode resources`.

Enabling browser tools or dropping a project skill changes the published card
with no extra configuration.

## Example card

```json
{
  "protocolVersion": "0.2.5",
  "name": "HooCode",
  "description": "Deterministic terminal coding agent. Exposes its active tools and skills for A2A discovery.",
  "url": "http://127.0.0.1:41411",
  "version": "0.4.140",
  "capabilities": {
    "streaming": false,
    "pushNotifications": false,
    "stateTransitionHistory": false
  },
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain"],
  "skills": [
    {
      "id": "shell-execution",
      "name": "Shell execution",
      "description": "Run shell commands in the workspace through a permission-gated bash tool.",
      "tags": ["bash", "shell", "exec"],
      "examples": ["Run the test suite", "Show the git status of this repo"]
    }
  ]
}
```

## Security

The discovery endpoint is unauthenticated by design — an `AgentCard` is public
metadata. It responds only to `GET`/`HEAD` and sets
`Access-Control-Allow-Origin: *` so browser-based agents can read it. If you
front HooCode behind authentication, add the corresponding `securitySchemes` to
the card before publishing it.

By default the server binds to loopback (`127.0.0.1`). Use `--host 0.0.0.0` to
expose it on other interfaces only when you intend to.
