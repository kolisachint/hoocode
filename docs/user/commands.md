# Commands

```text
hoocode [options] [@files...] [messages...]
```

`hoocode --help` lists every flag and the provider environment variables. This page
covers the ones most people need.

## Commands

| Command | What it does |
|---|---|
| `hoocode config` | Turn skills, prompts and other resources on or off |
| `hoocode update` | Print the command that updates hoocode |
| `hoocode migrate [--dry-run]` | Merge data from an older hoocode home folder into `~/.hoocode` |
| `hoocode app-server [--listen URL]` | Serve this folder's sessions over the app-server protocol |

For managing plugins without starting a session:

| Command | What it does |
|---|---|
| `hoocode install <name[@marketplace]>` | Install a plugin; `--scope user\|project` picks where |
| `hoocode remove <name>` | Remove a plugin (alias: `uninstall`) |
| `hoocode list` | List marketplaces and installed plugins |
| `hoocode plugin <command>` | Every `/plugin` command: `marketplace add`, `update`, `trust` and the rest |

`hoocode <command> --help` shows the options for `install`, `remove`, `list`,
`plugin` and `app-server`.

## Common flags

| Flag | What it does |
|---|---|
| `-p`, `--print` | Answer and exit, no interactive screen |
| `--mode json` / `--mode rpc` | JSON event stream, or JSON-RPC over stdin/stdout |
| `--model <pattern>` | Pick a model, e.g. `anthropic/claude-sonnet-4-5` or `sonnet:high` |
| `--provider <name>` | Pick a provider |
| `-c`, `--continue` | Continue the last session |
| `-r`, `--resume` | Choose a session to resume |
| `--no-session` | Don't save this run |
| `-t`, `--tools <list>` | Only these tools, e.g. `Read,CodeSearch` for a read-only run |
| `--thinking <level>` | `off`, `minimal`, `low`, `medium`, `high` or `xhigh` |
| `--light` | Minimal low-token preset for small or local models |
| `--list-models [search]` | List the models hoocode knows |
| `--offline` | No startup network calls, such as the version check |

Built-in tools: **Read**, **Shell**, **Edit**, **Write** and **CodeSearch**.

## More

The [README](../../README.md) and the
[CHANGELOG](../../CHANGELOG.md) in the
repository have the full detail, and `/changelog` shows it inside hoocode.
