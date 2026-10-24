# Quickstart

```bash
export ANTHROPIC_API_KEY=...   # or another provider's key
cd your-project
hoocode                        # full-screen session
```

`hoocode --help` lists the provider keys it reads. No API key? Start `hoocode` and run `/login` to sign in with a subscription
(Anthropic, GitHub Copilot, OpenAI Codex or Google) instead.

## In a session

Shell, Edit and Write ask before they run: **Yes**, **No**, or **Always**.

| Type | What it does |
|---|---|
| `/model` | Pick a model (Ctrl+P cycles through your favourites) |
| `/mode` | Switch between ask, plan, build and debug |
| `/resume` | Reopen an earlier session in this folder |
| `@path/to/file` | Attach a file to your message |
| `/hotkeys` | Show the keyboard shortcuts |
| `/settings` | Change settings |

## Modes

| Mode | What the agent may do |
|---|---|
| **ask** | Read and answer. No edits, no state-changing commands |
| **plan** | Explore and write a plan. No other file changes |
| **build** | Implement and verify, one change at a time |
| **debug** | Find the root cause. No edits; switch to build to fix |

## Sessions

Sessions are saved as JSONL under `~/.hoocode`. `--continue` picks up the last one,
`/resume` lists them, and `/fork` and `/tree` branch from an earlier message. Long
conversations are compacted automatically. `--no-session` keeps a run ephemeral.

## Scripted use

```bash
hoocode -p "Summarise this repo"                  # answer and exit
hoocode -p --mode json "List the TODOs in src/"   # JSON events
hoocode --tools Read,CodeSearch -p "Review src/"  # read-only
hoocode --mode rpc                                # JSON-RPC
```

Next: [where configuration lives](configuration.md), or the
[commands and flags](commands.md).
