# Configuration

Everything hoocode keeps lives in **`~/.hoocode`**. Set `HOOCODE_CODING_AGENT_DIR`
to move it.

## Files

| File or folder | What it holds |
|---|---|
| `~/.hoocode/settings.json` | Global settings (`/settings` edits it) |
| `~/.hoocode/auth.json` | Logins and keys saved by `/login`. See [sign-in](login.md) |
| `~/.hoocode/models.json` | Your own providers and models, for example a local OpenAI-compatible server |
| `~/.hoocode/hoo-config.json` | Agent modes and tool approval rules, written on first run ([below](#modes-and-approvals)) |
| `~/.hoocode/skills/` | Skills |
| `~/.hoocode/themes/` | Custom themes, one JSON file each. `/theme` lists `auto`, the built-in themes `dark`, `dusk`, `high-contrast` (white text on black, for low vision), `light` and `paper`, and your files here (by file name, with the path shown). A theme may set `"inherits": "<name>"` to take the colors it leaves out from a built-in theme or another file here. Plugin and MCP tool names take the `toolExternal` color; a theme that leaves it out shows them in the tool name color. The `theme` setting also accepts `auto` (follows the terminal background). A file that does not load, or a name that is not a theme, shows a warning under theme diagnostics: a switch keeps the theme on screen, and at startup the built-in `dark` stands in until the setting is fixed. |
| `~/.hoocode/themes/defaults/` | Reference copies of the built-in themes (`dark.json`, `dusk.json`, `high-contrast.json`, `light.json`, `paper.json`) and `theme-schema.json`. hoocode writes them again after each upgrade, so they match the version you run; a copy you edit is replaced at the next upgrade. The folder is not a theme list: `/theme` never shows it. To start a custom theme, copy one up a level to `~/.hoocode/themes/`, rename it, and change what you need. A shorter theme can instead set `"inherits": "dark"` (or `"light"`, or another file in `~/.hoocode/themes/`) and keep only its own colors. For editor completion, set a custom theme's `"$schema"` to the local `theme-schema.json` path. |
| `~/.hoocode/sessions/` | Saved sessions |

## Per project

| File or folder | What it holds |
|---|---|
| `.hoocode/settings.json` | Project settings, merged over the global ones |
| `.hoocode/skills/` | Project skills |
| `AGENTS.md` or `CLAUDE.md` | Project instructions, read from the working folder and its parents |

## Environment variables

Each one is also listed by `hoocode --help`. Provider API keys (such as
`ANTHROPIC_API_KEY`) are not listed here. The variables for Google sign-in are in
[sign-in](login.md).

| Variable | What it does |
|---|---|
| `HOOCODE_OFFLINE` | Disable startup network operations when set to `1`, `true` or `yes`. |
| `HOOCODE_FULLSCREEN` | Set to `0` to run the interactive screen inline instead of full-screen. |
| `HOOCODE_USER_AGENTS_DIR` | Folder for user plugins and `mcp.json` (default: `~/.agents`). |
| `HOOCODE_PACKAGE_DIR` | Folder of hoocode's bundled files such as `CHANGELOG.md` (for Nix/Guix store paths). |
| `HOOCODE_CODING_AGENT_SESSION_DIR` | Session storage directory (overridden by `--session-dir`). |
| `HOOCODE_CA_CERT` | Path to an extra PEM CA bundle to trust for hoocode's own TLS traffic, including plugin (WASM) network requests, which connect directly and do not use `HTTPS_PROXY` yet (additive; verification stays on). Precedence: `--ca-cert` > this > `NODE_EXTRA_CA_CERTS`. |
| `HOOCODE_USE_SYSTEM_CA` | Also trust the OS/system CA store when set to `1`, `true` or `yes` (same as `--use-system-ca`). Applies to plugin (WASM) network requests too. |
| `NODE_EXTRA_CA_CERTS` | Path to an extra PEM CA bundle (the lowest-precedence CA source). |
| `HOOCODE_TELEMETRY` | Set to `1`, `true` or `yes`, or `0`, `false` or `no`, to override the `enableInstallTelemetry` setting (OpenRouter app attribution headers). |

## MCP servers

Declared in `~/.agents/mcp.json` (yours) and `.agents/mcp.json` (the project's).
hoocode asks before it starts servers a project declares. `/mcp` shows each server's
state, and `/mcp login <server>` signs in to one that needs it.

## Plugins

Plugins add skills, MCP servers and tools. They live in `~/.agents/plugins/`
(yours) and `.agents/plugins/` (the project's). Manage them with `/plugin` in a
session: add a marketplace, then install from it. A plugin's tools and MCP servers
are live as soon as you install, update or remove it, with no `/reload`. Its skills
load on `/reload`.

```text
/plugin marketplace add <git-url-or-owner/repo>
/plugin install <plugin>@<marketplace>
```

Two marketplaces are built in and trusted: `claude-plugins-official` and
`hoo-plugins` (`/plugin install webtools@hoo-plugins`). Installs from them need no
extra grant.

The same commands work from the shell, without a session: `hoocode install`,
`hoocode remove`, `hoocode list` and `hoocode plugin ...`. See
[Commands](commands.md).

A plugin may also ship a WASM component at `dev.hoocode/extension.wasm`. Its tools
appear to the model under the names the plugin gives them, with no prefix (`WebFetch`,
for example). They are live as soon as the plugin is installed, updated or removed, with
no `/reload`.

A plugin may also ship a native helper: a per-platform binary the plugin names in its `mcp.json` and describes in `extensions."dev.hoocode".native`. hoocode starts one helper per session on first use. Its tools appear under the names the plugin declares, with no prefix, and are live with no `/reload`. Only trusted plugins start helpers.

- The binary is downloaded once per version into `~/.hoocode/plugin-bin/<plugin>/<version>/`, checksum verified. To use an installed binary instead (offline or IT-approved installs), set an absolute path in your user `settings.json` (project settings are ignored: a cloned repo cannot redirect the helper):

```json
{ "nativeHelpers": { "<plugin>": { "binaryPath": "/abs/path" } } }
```

- Each declared tool carries a permission hint. `read` runs without asking. `act` asks like an edit, and is denied where your mode denies Write or Edit. `dangerous` always asks unless your mode's `auto_allow` names it, even with `skip_permissions` (where nobody can answer, it stays blocked).
- Subagents get a helper's tools only when the plugin marks it shared. Other helpers are root-only, as are plain MCP plugin servers.
- Native helpers are not available in `hoocode app-server`: its sessions register no native tools. Single-session paths (TUI, print, json, rpc) are unaffected.

- Tool names. An MCP server's tool appears as `mcp__<server>__<tool>`, for example
  `mcp__github__create_issue`. `--tools`, `--disallowed-tools` and an agent's `tools`
  and `disallowedTools` take a name or a selector: `plugin:<plugin>` (every tool of a
  plugin), `plugin:<plugin>/<Tool>`, `plugin:*`, `mcp:<server>`, `mcp:<server>/<tool>`
  and `mcp:*`. An entry that is neither is malformed: `--tools` and `--disallowed-tools`
  stop with an error, an agent definition with one is rejected, and a permission rule with
  one stops hoocode from starting. The old `ext_*` and `mcp_*` names are gone, with no alias.
  A bare name (`echo`, a plugin tool's name) is kept as written, and matches the tool of
  that name at start, case-insensitively. A name that matches no tool is ignored with a
  warning. Claude Code names with a hoocode alias (`Bash`, `Grep`) still map to the hoocode tool.
- Two plugin or MCP tools with the same name: the first one loaded is used, and the
  other is skipped with a diagnostic that names both. A plugin tool never replaces a
  built-in tool.

- A component runs only when its plugin is trusted, the same trust that lets the
  plugin's MCP servers start without a `/mcp` grant. An untrusted plugin's component
  is skipped, and the session says so under diagnostics.
- Web access (fetch and search) is the webtools plugin, not a built-in tool: install it
  with `/plugin install webtools@hoo-plugins`. Its tools are `WebFetch` and `WebSearch`.
  Its settings go under `pluginConfig.webtools.<key>`
  (`searchProvider`, `braveApiKey`, `tavilyApiKey`, `searxngUrl`, `searxngApiKey`,
  `searchFallback`). A call that reaches the network asks you like `WebFetch`. Later
  requests in the same call (for example redirects) do not ask again.
- Network access for any plugin's tool, not only webtools, is the `WebFetch` permission
  category. A mode whose `enabled_tools` does not list `WebFetch` blocks that request
  ("not enabled") even when the plugin's own call runs, so list `WebFetch` in `enabled_tools`
  as well as `plugin:<plugin>`. Add `WebFetch` to `auto_allow` to answer the request without
  asking. `plugin:<plugin>` alone does not allow a plugin's network access.
- A plugin reads its own settings under `pluginConfig.<plugin>.<key>` in your user
  `settings.json` (`~/.hoocode/settings.json`). It is read from user settings only, never
  from a project's `.hoocode/settings.json`, so a cloned repository cannot point a plugin
  at its own server. Only string values are read. After you edit them, run `/reload`
  (or start a new session) for the plugin's tools to use the new values:

```json
{ "pluginConfig": { "webtools": { "braveApiKey": "..." } } }
```

- hoocode compiles each component once into `~/.hoocode/cache/wasm`. The folder is
  safe to delete; the next start compiles again.

`hoocode config` opens a list for turning skills, prompts and other resources on or
off. Themes are not in it: pick one with `/theme`.

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

## Screen

The screen has the transcript, one prompt row (` › `) and a status line under it that
holds the session, model, context use, cost and what hoocode is doing now. There
is no header, logo or frame; dialogs open as panes in the prompt's place.

- **Transcript**: one row per tool call, collapsed (`● Edit src/config.rs  +2 −2`).
  Click a header, or focus it and press `enter`, to show the full output; again to hide.
  A running Shell shows one dim tail line until it settles; a failed call stays collapsed.
- **Pinned transcript keys**: `ctrl+x u` focuses the newest block from the prompt, and `ctrl+x o` opens or closes all tool blocks.
  With a block focused and the prompt empty, `{` and `}` move between blocks,
  `enter` opens the focused block, `y` copies its raw output, and `esc` clears the
  focus. `End` goes back to live. Letters always type at the prompt. See
  [Keybindings](keybindings.md).
- **Themes**: `dark`, `dusk`, `high-contrast`, `light` and `paper`, and `auto` follows the terminal background and
  re-checks when it changes. Pick one with `/theme`, which also lists your own themes.
  A pick is saved to your global settings. If this repo's `.hoocode/settings.json` sets
  `theme`, that wins over the pick, and the picker says so.
  The older names (`daylight`, `deep-space`, `colorsafe-*`, `high-contrast-*` (the built-in is `high-contrast`), `warm-*`,
  `solarized-*`) are gone; a saved value that no longer exists shows a warning under
  theme diagnostics and the built-in `dark` stands in until you pick another theme.
- **No colour**: with `NO_COLOR` set, `TERM=dumb` or fewer than 256 colours, panes get
  light borders and the selected row keeps its ` › ` marker.
- **Removed**: the `/chrome` and `/color` commands and the `alt+z` key. Notices no longer
  pop up as a toast: a short one takes the status line's left side for a few seconds,
  a longer one sits as dim rows above the prompt.
- **First run**: with no working provider the Log in pane opens by itself. Esc closes it
  and the status line shows `not signed in  /login` until a provider works.

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
- **Tool selectors in rules**: `denied_tools`, `enabled_tools` and `auto_allow` take the
  same selectors as `--tools`. A bare name (`WebFetch`) matches the tool of that name from
  any source, case-insensitively. `plugin:webtools` is every tool of that plugin,
  `plugin:webtools/WebFetch` one tool of it, `plugin:*` every plugin tool, `mcp:github`
  every tool of that MCP server and `mcp:github/create_issue` one of them. Plugin tools
  are not asked about one by one: their network access is asked as `WebFetch`, once per
  site. A malformed entry stops hoocode from starting, with a message that names the mode
  and the entry, until it is fixed.
- **Approval card** (`Allow Shell?`): press `1` to allow once, `2` to always allow
  (when offered), `3` to refuse and tell the agent why. `e` edits the command first, and
  any other letter starts typing a reply that `enter` sends as the reason. `enter`
  takes the highlighted option and `esc` refuses. An `access:` line appears only when
  the call writes, uses the network or pipes.
- **Always** on an approval card (`2`): for Shell it lasts for the session and covers
  that program; for other tools it adds the tool to the mode's `auto_allow` in this file.
- **Questions**: when the agent asks a question, `1`-`9` pick an answer, `tab` moves to
  the next question and `esc` skips it. The agent is told it was skipped and states the
  default it will use.
- A project can add rules in `.hoocode/hoo-config.json`: its `denied_tools` and
  `denied_bash_commands` add to yours. It cannot make anything run without asking:
  `auto_allow`, `safe_shell` and `skip_permissions` come from your file only, and so
  does `allowed_write_paths` when your file sets it.
- A write to hoocode's own project config (`.hoocode/*.json`, such as
  `.hoocode/hoo-config.json` or `.hoocode/settings.json`) always asks, in every mode and
  even with `skip_permissions`.
