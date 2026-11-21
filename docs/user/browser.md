# Browser

Web pages your agents can click, fill in, and read. Install it with:

```text
/plugin install hoo-browser@hoo-plugins
```

The same works from the shell without a session: `hoocode install hoo-browser@hoo-plugins`.
See [Commands](commands.md) for the other plugin commands.

## What it does

One shared Chromium serves the whole session. Each agent gets its own tabs,
and each agent's first tab opens its own window. An agent never sees another
agent's tabs. Full isolation (popups, downloads) needs the hoo fork of
agent-browser (versions with a `-hoo.N` suffix, pinned by the plugin); with
upstream agent-browser the helper runs in a safe mode (`browser_tabs` lists
and closes only the lease's own tab) and popups/downloads isolation is not
guaranteed. Subagents get the tools too: the plugin
marks its helper shared, so root and subagents browse side by side.

The loop is the same every time: `browser_open` a URL, `browser_snapshot` for
the page's refs, `browser_act` on a ref, `browser_screenshot` to look. Act on
refs from the snapshot, never on coordinates from memory.

## Tools

| Tool | What it does | Asks |
|---|---|---|
| `browser_open` | Open a URL in your tab | No (`read`) |
| `browser_snapshot` | Accessibility tree: refs for acting, text for reading | No (`read`) |
| `browser_act` | Click, fill, select, check, uncheck, hover, or press a ref | Yes (`act`) |
| `browser_screenshot` | Screenshot of your tab, returned as an image | No (`read`) |
| `browser_tabs` | List or close your tabs; root can adopt a finished agent's tabs | Yes (`act`) |
| `browser_wait` | Wait for time, text, a ref, or page load | No (`read`) |
| `browser_nav` | Back, forward, reload | Yes (`act`) |
| `browser_eval` | Run JavaScript in your tab | Always (`dangerous`) |

`act` asks like an edit. `dangerous` always asks, even with
`skip_permissions`. Prefer snapshot, act, and wait; use eval only to read
state the snapshot cannot show.

Limits: 4 tabs per agent, 24 per helper. A tab idle for 10 minutes closes.

## Settings

Defaults fit most setups. These environment variables change them:

| Variable | What it does |
|---|---|
| `HOO_BROWSER_BACKEND` | `chromium-headless` (default) or `chromium-headed`. `chrome` and `lightpanda` answer not yet supported |
| `HOO_BROWSER_HEADED=1` | Same as the headed backend |
| `HOO_BROWSER_CHROME` | Path to Chromium or Chrome. Else agent-browser's own install, else common install paths |
| `HOO_BROWSER_AGENT_BROWSER` | Path to the agent-browser CLI. Else the copy in the data dir, else `PATH` |
| `HOO_BROWSER_DATA` | Where the helper keeps its profile and logs. hoocode sets it from the plugin data dir. The daemon sockets live under the per-session runtime dir, not here |

## Long tail

Cookies, network, recording, and the rest the eight tools do not cover: run
the helper's CLI through Shell. It calls the helper scoped to your own tabs.
The binary is not on `PATH`: use the variable, which is set from your first
browser tool call on (a subagent's Shell learns it on its first shared tool
call).

```bash
"$HOOCODE_HELPER_BIN_HOO_BROWSER" cli snapshot
```

## Data

Two homes. Under the plugin data dir (`HOO_BROWSER_DATA`): the Chromium
profile, the pinned `agent-browser` binary, and the logs (`hoo-browser.log`
with per-call timings, `chromium-stderr.log`). Under the per-session runtime
dir (the helper socket's parent, removed when the session ends): the
agent-browser daemon sockets (`ab/`) and the helper-owned
`agent-browser-config.json` (`{}`).
