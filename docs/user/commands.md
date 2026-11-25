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
| `hoocode keybindings` | Print the default keys (`--defaults`), check `keybindings.json` (`check`), or show the bytes a key press sends (`--read`). See [Keybindings](keybindings.md) |
| `hoocode app-server [--listen URL]` | Serve this folder's sessions over the app-server protocol |

For managing plugins without starting a session:

| Command | What it does |
|---|---|
| `hoocode install <name[@marketplace]>` | Install a plugin; `--scope user\|project` picks where |
| `hoocode remove <name>` | Remove a plugin (alias: `uninstall`) |
| `hoocode list` | List marketplaces and installed plugins |
| `hoocode plugin <command>` | Every `/plugin` command: `marketplace add`, `update`, `trust` and the rest |

`hoocode <command> --help` shows the options for `install`, `remove`, `list`,
`plugin`, `app-server`, `update` and `config`. `config --help` prints its usage
and does not open the picker.

## Common flags

| Flag | What it does |
|---|---|
| `-p`, `--print` | Answer and exit, no interactive screen |
| `--mode json` / `--mode rpc` | JSON event stream, or JSON-RPC over stdin/stdout |
| `--model <pattern>` | Pick a model, e.g. `anthropic/claude-sonnet-4-5` or `sonnet:high` |
| `--provider <name>` | Pick a provider |
| `-c`, `--continue` | Continue the last session |
| `-r`, `--resume` | Choose a session to resume |
| `--session <path or id>` | Open that session. One process writes a session at a time: a session another process has open is refused with its process id |
| `--no-session` | Don't save this run |
| `-t`, `--tools <list>` | Only these tools, e.g. `Read,CodeSearch` for a read-only run |
| `--thinking <level>` | `off`, `minimal`, `low`, `medium`, `high` or `xhigh` |
| `--skip-permissions` | Run every tool without asking for approval (mode rules still apply). Same as `HOOCODE_SKIP_PERMISSIONS=1` |
| `--light` | Minimal low-token preset for small or local models |
| `--list-models [search]` | List the models hoocode knows |
| `--offline` | No startup network calls, such as the version check |
| `--ca-cert <path>` | Trust an extra PEM CA bundle for hoocode's HTTPS traffic, such as a TLS-intercepting proxy's CA. Verification stays on. Also applies to plugin (WASM) network requests, which connect directly and do not use `HTTPS_PROXY` yet. See [configuration](configuration.md) |
| `--use-system-ca` | Also trust the operating system's CA store. Also applies to plugin (WASM) network requests |

Built-in tools: **Read**, **Shell**, **Edit**, **Write** and **CodeSearch**.

## Detached sessions

A background run can keep writing its session while you are away. Open that
session with `hoocode --session <path>`, the command the run shows (`--resume`
and `--continue` open it too when they pick it). It opens read-only:

- The history shows, and new messages appear about once a second as the run writes them.
- Enter does not send. The text stays in the prompt and a note says the session is read-only.
  `/quit`, `/resume` and `/new` still work. Commands that change the session (`/name`,
  `/compact`, `/model`, `/tree`, `/fork`) are refused.
- Ctrl+C sends an interrupt (SIGINT) to the process that runs the session, and the
  status line says so. A second Ctrl+C within half a second quits hoocode. Quitting does
  not stop the run.
- When the run ends, the status line says so and the session opens as a normal one, so
  you can send messages to it. A tool call the run was waiting on (a permission) is
  asked then.

## More

The [README](../../README.md) and the
[CHANGELOG](../../CHANGELOG.md) in the
repository have the full detail, and `/changelog` shows it inside hoocode.

Sign in to a provider with `/login` and sign out with `/logout`. See [sign-in](login.md).
