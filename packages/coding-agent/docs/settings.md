# Settings

HooCode uses JSON settings files with project settings overriding global settings.

| Location | Scope |
|----------|-------|
| `~/.hoocode/settings.json` | Global (all projects) |
| `.hoocode/settings.json` | Project (current directory) |

Edit directly or use `/settings` for common options.

## All Settings

### Model & Thinking

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `defaultProvider` | string | - | Default provider (e.g., `"anthropic"`, `"openai"`) |
| `defaultModel` | string | - | Default model ID |
| `defaultThinkingLevel` | string | - | `"off"`, `"minimal"`, `"low"`, `"medium"`, `"high"`, `"xhigh"` |
| `hideThinkingBlock` | boolean | `false` | Hide thinking blocks in output |
| `thinkingBudgets` | object | - | Custom token budgets per thinking level |
| `thinkingDisplay` | string | - | `"summarized"` or `"omitted"`. Controls how adaptive-thinking models return thinking content. Opus 4.8 defaults to `"omitted"` (faster tool-use turns, reasoning effort unchanged); set `"summarized"` to surface thinking text. |

#### thinkingDisplay

`"omitted"` keeps full reasoning effort but skips streaming the thinking summary,
lowering time-to-first-token (faster tool use) at the cost of not showing thinking
text. `"summarized"` returns visible thinking. When unset, Opus 4.8 defaults to
`"omitted"`; other models default to `"summarized"`.

```json
{
  "thinkingDisplay": "summarized"
}
```

#### thinkingBudgets

```json
{
  "thinkingBudgets": {
    "minimal": 1024,
    "low": 4096,
    "medium": 10240,
    "high": 32768
  }
}
```

### UI & Display

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `theme` | string | `"dark"` | Theme name (`"dark"`, `"light"`, or custom) |
| `quietStartup` | boolean | `false` | Hide startup header |
| `collapseChangelog` | boolean | `false` | Show condensed changelog after updates |
| `enableInstallTelemetry` | boolean | `true` | Send an anonymous install/update version ping after first install or changelog-detected updates. This does not control update checks |
| `doubleEscapeAction` | string | `"tree"` | Action for double-escape: `"tree"`, `"fork"`, or `"none"` |
| `treeFilterMode` | string | `"default"` | Default filter for `/tree`: `"default"`, `"no-tools"`, `"user-only"`, `"labeled-only"`, `"all"` |
| `editorBorder` | string | `"box"` | Input editor border: `"box"` (side borders and corners) or `"rule"` (horizontal lines only) |
| `editorPaddingX` | number | `1` | Horizontal padding for input editor (0-3) |
| `autocompleteMaxVisible` | number | `5` | Max visible items in autocomplete dropdown (3-20) |
| `showHardwareCursor` | boolean | `false` | Show terminal cursor |

### Telemetry and update checks

`enableInstallTelemetry` only controls the anonymous install/update ping to `https://hoocode.dev/api/report-install`. Opting out of telemetry does not disable update checks; HooCode can still fetch `https://hoocode.dev/api/latest-version` to look for the latest version.

Set `HOOCODE_SKIP_VERSION_CHECK=1` to disable the HooCode version update check. Use `--offline` or `HOOCODE_OFFLINE=1` to disable all startup network operations described here, including update checks, package update checks, and install/update telemetry.

### Warnings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `warnings.anthropicExtraUsage` | boolean | `true` | Show a warning when Anthropic subscription auth may use paid extra usage |
| `warnings.websearchApiKey` | boolean | `true` | Show a warning when `websearch` is enabled with no search API key, so it falls back to keyless DuckDuckGo |

```json
{
  "warnings": {
    "anthropicExtraUsage": false,
    "websearchApiKey": false
  }
}
```

Both warnings are shown once per session in the TUI and can also be toggled from
`/settings` → Warnings.

### Compaction

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `compaction.enabled` | boolean | `true` | Enable auto-compaction |
| `compaction.reserveTokens` | number | `16384` | Tokens reserved for LLM response |
| `compaction.keepRecentTokens` | number | `20000` | Recent tokens to keep (not summarized) |

```json
{
  "compaction": {
    "enabled": true,
    "reserveTokens": 16384,
    "keepRecentTokens": 20000
  }
}
```

### Context budget

Everything in the system prompt and in an active tool's schema is re-sent on
every request. These three settings decide how large that is, and `/settings`
groups them under **Context** with the live per-turn cost printed under the
list.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `light` | boolean | `false` | Low-token preset: `read`/`write`/`edit`/`bash` only with stripped schemas, a terse system prompt, and no subagents/TodoWrite/skills/context files |
| `contextGc` | boolean | `true` | Stub superseded read results out of the outgoing context |

`light` is read at startup to pick the tool set and the system prompt, so
toggling it in `/settings` applies on the next session. `--light` turns it on
for a single run. Auto-compaction lives in the same category and is documented
under [Compaction](#compaction).

`/settings` also prices each tool by what its schema costs per turn, on the
switch itself, so it is clear which one is worth turning off. `hoocode
--print-token-surface` prints the same measurement as text.

### Branch Summary

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `branchSummary.reserveTokens` | number | `16384` | Tokens reserved for branch summarization |
| `branchSummary.skipPrompt` | boolean | `false` | Skip "Summarize branch?" prompt on `/tree` navigation (defaults to no summary) |

### Retry

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `retry.enabled` | boolean | `true` | Enable automatic agent-level retry on transient errors |
| `retry.maxRetries` | number | `3` | Maximum agent-level retry attempts |
| `retry.baseDelayMs` | number | `2000` | Base delay for agent-level exponential backoff (2s, 4s, 8s) |
| `retry.provider.timeoutMs` | number | SDK default | Provider/SDK request timeout in milliseconds |
| `retry.provider.maxRetries` | number | SDK default | Provider/SDK retry attempts |
| `retry.provider.maxRetryDelayMs` | number | `60000` | Max server-requested delay before failing (60s) |

When a provider requests a retry delay longer than `retry.provider.maxRetryDelayMs` (e.g., Google's "quota will reset after 5h"), the request fails immediately with an informative error instead of waiting silently. Set to `0` to disable the cap.

```json
{
  "retry": {
    "enabled": true,
    "maxRetries": 3,
    "baseDelayMs": 2000,
    "provider": {
      "timeoutMs": 3600000,
      "maxRetries": 0,
      "maxRetryDelayMs": 60000
    }
  }
}
```

### Message Delivery

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `steeringMode` | string | `"one-at-a-time"` | How steering messages are sent: `"all"` or `"one-at-a-time"` |
| `followUpMode` | string | `"one-at-a-time"` | How follow-up messages are sent: `"all"` or `"one-at-a-time"` |
| `transport` | string | `"sse"` | Preferred transport for providers that support multiple transports: `"sse"`, `"websocket"`, or `"auto"` |

### Terminal & Images

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `terminal.showImages` | boolean | `true` | Show images in terminal (if supported) |
| `terminal.imageWidthCells` | number | `60` | Preferred inline image width in terminal cells |
| `terminal.clearOnShrink` | boolean | `false` | Clear empty rows when content shrinks (can cause flicker) |
| `images.autoResize` | boolean | `true` | Resize images to 2000x2000 max |
| `images.blockImages` | boolean | `false` | Block all images from being sent to LLM |

### Shell

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `shellPath` | string | - | Custom shell path (e.g., for Cygwin on Windows) |
| `shellCommandPrefix` | string | - | Prefix for every bash command (e.g., `"shopt -s expand_aliases"`) |
| `npmCommand` | string[] | - | Command argv used for npm package lookup/install operations (e.g., `["mise", "exec", "node@20", "--", "npm"]`) |

```json
{
  "npmCommand": ["mise", "exec", "node@20", "--", "npm"]
}
```

`npmCommand` is used for all npm package-manager operations, including installs, uninstalls, and dependency installs inside git packages. Use argv-style entries exactly as the process should be launched. When `npmCommand` is configured, git package dependency installs use plain `install` to avoid npm-specific flags in wrappers or alternate package managers.

Normally the package manager's global modules location is queried using `root -g`. As a special case, if the first element of `npmCommand` is `"bun"`, the modules location will instead be queried with `pm bin -g`.

### Sessions

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `sessionDir` | string | - | Directory where session files are stored. Accepts absolute or relative paths, plus `~`. |

```json
{ "sessionDir": ".hoocode/sessions" }
```

When multiple sources specify a session directory, precedence is `--session-dir`, `HOOCODE_CODING_AGENT_SESSION_DIR`, then `sessionDir` in settings.json.

### Learning from sessions

The window `/learn` mines when looking for repeated directives, fixes, and workflows.

All five are editable from `/settings` under **Learning**. `/learn settings`
prints the same values as text, alongside the paths of both settings files and
the session directory being read.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `learnMaxSessions` | number | `20` | Recent sessions in this directory to scan |
| `learnMaxAgeDays` | number | `30` | Ignore sessions older than this |
| `learnMinRepeats` | number | `2` | Separate sessions a directive must recur in before it is proposed |
| `learnMinRequestRepeats` | number | `3` | Separate sessions a piece of work must be asked for before it is proposed as a slash command |
| `learnMaxProposals` | number | `8` | Cap on each list in the digest |

```json
{ "learnMaxSessions": 50, "learnMaxAgeDays": 90, "learnMinRepeats": 3 }
```

Widen the window on a repo you touch rarely, so a habit spread over months still
reaches the repeat threshold. Narrow it on one you work in daily, where the last
few weeks are the only relevant history. `learnMinRepeats` is the signal/noise
dial: raise it for fewer, better-evidenced proposals.

Both thresholds count *distinct sessions*, not occurrences: saying a thing twice
in one sitting usually means it was ignored the first time, which is evidence
about that afternoon rather than about how you work.
`learnMinRequestRepeats` is higher than `learnMinRepeats` on purpose — a rule
stated twice is a rule, but a job asked for twice may just be a job that came up
twice. `learnMaxProposals` bounds what a single run can ask you to review; every
proposal costs the model context.

Non-numeric or non-positive values fall back to the default rather than
narrowing the window to nothing. As with all settings, a project
`.hoocode/settings.json` overrides the global one, so a repo can carry its own
window.

### Plugins and artifacts

Where hoocode writes what it authors. Both are editable from `/settings` under
**Plugins**, which writes the global settings file — the layout a machine
targets is an environment-level choice, so it is set once rather than passed on
every run.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `enablePluginTools` | boolean | `false` | Master switch for the autonomous plugin system: the lifecycle tools, `ProposePlugin`, and the plugin-reuse nudge |
| `platform` | string \| string[] | - | Vendor layout(s) to write artifacts in: `claude`, `github` (aliases `copilot`, `gh`), `agents` (alias `native`) |
| `pluginInstallScope` | string | `"user"` | Where an autonomous plugin install lands: `user` (`~/.agents/plugins`) or `project` (`<cwd>/.agents/plugins`, shared once committed) |

```json
{ "enablePluginTools": true, "platform": ["claude", "github"], "pluginInstallScope": "user" }
```

`/settings` writes the global file. If a project `.hoocode/settings.json` sets
one of these keys it is merged over that on the next session, so the pane marks
the row as overridden rather than letting the change look permanent.

`enablePluginTools` is off by default and is the one gate for the whole
autonomous system — see [Plugins](plugins.md). Toggling it in `/settings`
attaches the tools on the next session, since they are wired up when the session
is built; the reuse nudge re-reads the setting and follows immediately. The
`--enable-plugintools` flag turns it on for a single run. Slash-command plugin
management (`/plugin`) is unaffected either way.

`platform` governs the two things hoocode *writes*: authored plugins, and the
`/new-skill`, `/new-agent` and `/new-command` scaffolds. Reading is unaffected —
every vendor convention is read regardless of this setting.

It takes a list because emitting for two platforms at once is a supported shape;
in `/settings` the three tokens are independent toggles for that reason.
Unsetting it (all toggles off) is not the same as targeting nothing: the
defaults come back, which are `claude` for an authored plugin and `.hoocode/`
for a scaffold. `agents` is a scaffold-only target — a plugin is a distribution
unit and the native layout belongs to no marketplace — so plugin authoring
filters it out rather than failing.

The `--platform` CLI flag sets the same targets for a single run and overrides
this setting.

### Model Cycling

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `enabledModels` | string[] | - | Model patterns for Ctrl+P cycling (same format as `--models` CLI flag) |

```json
{
  "enabledModels": ["claude-*", "gpt-4o", "gemini-2*"]
}
```

### Markdown

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `markdown.codeBlockIndent` | string | `"  "` | Indentation for code blocks |

### External tools

hoocode is self-sufficient without any of these. Five optional Rust binaries
expand what it can do: two make an existing tool faster, three add a capability
hoocode otherwise does not have. Each resolves from
`HOOCODE_<TOOL>_BINARY` → hoocode's bin directory → `PATH` → a published
release download.

Because hoocode degrades quietly without them, they are easy to miss. `/settings`
→ **External tools** lists all five with live status and what each one adds;
rows below that are inert without their binary are marked `needs <binary>`
there.

| Binary | Adds | Without it |
|---|---|---|
| `rg` | Fast path for `grep` and the lexical half of `search`. Fetched at startup. | A pure-JS scanner with identical output, slower on large trees. |
| `fd` | Fast path for `find`. Fetched at startup. | A JS directory walker, slower, with approximated glob/ignore handling. |
| `embsearch` | Semantic hits fused into `search`, and meaning-ranked capability/MCP lookup. Fetched on first use. Must be the ONNX build. | `search` runs lexical-only. Nothing errors; intent-phrased queries rank worse. |
| `webtools` | The `webfetch` and `websearch` tools. Fetched on first use. | Both tools error when called. |
| `voicetools` | Push-to-talk voice input in the TUI. Fetched on first use. | Voice capture reports an error and never starts. |

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `enableWebTools` | boolean | `false` | Enable `webfetch` + `websearch` (network access). Needs the `webtools` binary. |
| `webtools.timeoutSecs` | number | `15` | Per-request timeout in seconds, clamped 1–120. Wins over `HOOCODE_WEBTOOLS_TIMEOUT`. |
| `webtools.search` | object | — | Search backend and credentials, read by the `webtools` binary itself. See [Web search providers](#web-search-providers). |
| `enableEmbsearchTools` | boolean | `true` | Build and fuse the semantic index into `search`. Set false to run `search` lexical-only. `grep`/`find` are unaffected. |
| `embsearchBinaryPath` | string | — | Explicit path to the `embsearch` binary. Default: resolve from `PATH`. |
| `embsearchThresholdBytes` | number | `0` | Minimum indexable source bytes before a repo is embedded. `0` indexes every repo. |
| `voice.silenceMs` | number | `800` | Trailing silence before voice capture auto-stops, clamped 300–10000. `VOICETOOLS_SILENCE_MS` overrides this. |

Environment variables:

| Variable | Effect |
|---|---|
| `HOOCODE_<TOOL>_BINARY` | Point at a local build; authoritative when the path exists. `<TOOL>` is `RG`, `FD`, `EMBSEARCH`, `WEBTOOLS`, or `VOICETOOLS`. |
| `VOICETOOLS_BIN` | Same, checked before the generic override for voice. |
| `HOOCODE_OFFLINE=1` | Never download a missing binary. |
| `HOOCODE_NATIVE_SEARCH=1` | Force the JS search paths even when `rg`/`fd` are installed. |

On Android/Termux the published Linux builds do not run; install with
`pkg install <name>` instead.

#### Reading a long page

`webfetch` budgets its output in tokens (`maxTokens`, default 4000, hard cap
25000). When a page runs past that, the result says so — the budget it stopped
at, and the ways past it — so a long document reads as a first page rather than
a dead end. The TUI marks the same fetch `~4000 tokens (truncated at 4000)`.

Prefer a more specific URL or `#anchor` over re-fetching the same page at a
larger budget: extraction has already dropped nav and boilerplate, so the first
few thousand tokens are usually the article itself.

#### Web search providers

`websearch` needs no configuration: it defaults to keyless DuckDuckGo Lite.
That backend is scraped HTML, though — rate-limited hard, and it can fail
outright — so when search reliability matters, point it at a backend with an
actual API contract. With none configured, the TUI says so once per session
(see [`warnings.websearchApiKey`](#warnings)).

The `webtools` binary reads its own `webtools` key out of the user-level
`~/.hoocode/settings.json`, using its own (snake_case) key names. HooCode never
writes this block; it only reads it to tell whether a keyed backend exists. Put
it in the user-level file — a project `.hoocode/settings.json` is not read by the
binary, and a file holding credentials is worth keeping out of a working tree.

```json
{
  "webtools": {
    "search": {
      "provider": "brave",
      "fallback": "duckduckgo",
      "providers": {
        "brave": { "api_key": "..." },
        "tavily": { "api_key": "..." },
        "searxng": { "base_url": "https://searx.internal" }
      }
    }
  }
}
```

Environment variables win over the file, so a key can stay out of it entirely:

| Variable | Effect |
|---|---|
| `WEBTOOLS_SEARCH_PROVIDER` | Backend to use: `duckduckgo` (default), `brave`, `tavily`, `searxng`. Pin `duckduckgo` to accept the keyless backend and silence the warning. |
| `WEBTOOLS_SEARCH_FALLBACK` | Backend tried when the primary fails; `none` disables the fallback. |
| `WEBTOOLS_BRAVE_API_KEY`, `BRAVE_API_KEY` | Brave Search key. |
| `WEBTOOLS_TAVILY_API_KEY`, `TAVILY_API_KEY` | Tavily key. Returns cleaned page content, so a search often answers without a follow-up `webfetch`. |
| `WEBTOOLS_SEARXNG_URL`, `WEBTOOLS_SEARXNG_API_KEY` | Self-hosted SearXNG endpoint (and optional key), for networks where the public APIs are unreachable. |

Every result records the backend that answered it, so a fallback to the scraped
backend is never silent.

### Resources

These settings define where to load extensions, skills, prompts, and themes from.

Paths in `~/.hoocode/settings.json` resolve relative to `~/.hoocode`. Paths in `.hoocode/settings.json` resolve relative to `.hoocode`. Absolute paths and `~` are supported.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `packages` | array | `[]` | npm/git packages to load resources from |
| `extensions` | string[] | `[]` | Local extension file paths or directories |
| `skills` | string[] | `[]` | Local skill file paths or directories |
| `prompts` | string[] | `[]` | Local prompt template paths or directories |
| `themes` | string[] | `[]` | Local theme file paths or directories |
| `enableSkillCommands` | boolean | `true` | Register skills as `/skill:name` commands |

Arrays support glob patterns and exclusions. Use `!pattern` to exclude. Use `+path` to force-include an exact path and `-path` to force-exclude an exact path.

#### packages

String form loads all resources from a package:

```json
{
  "packages": ["hoocode-skills", "@org/my-extension"]
}
```

Object form filters which resources to load:

```json
{
  "packages": [
    {
      "source": "hoocode-skills",
      "skills": ["brave-search", "transcribe"],
      "extensions": []
    }
  ]
}
```

See [packages.md](packages.md) for package management details.

## Example

```json
{
  "defaultProvider": "anthropic",
  "defaultModel": "claude-sonnet-4-20250514",
  "defaultThinkingLevel": "medium",
  "theme": "dark",
  "compaction": {
    "enabled": true,
    "reserveTokens": 16384,
    "keepRecentTokens": 20000
  },
  "retry": {
    "enabled": true,
    "maxRetries": 3
  },
  "enabledModels": ["claude-*", "gpt-4o"],
  "warnings": {
    "anthropicExtraUsage": true,
    "websearchApiKey": true
  },
  "packages": ["hoocode-skills"]
}
```

## Project Overrides

Project settings (`.hoocode/settings.json`) override global settings. Nested objects are merged:

```json
// ~/.hoocode/settings.json (global)
{
  "theme": "dark",
  "compaction": { "enabled": true, "reserveTokens": 16384 }
}

// .hoocode/settings.json (project)
{
  "compaction": { "reserveTokens": 8192 }
}

// Result
{
  "theme": "dark",
  "compaction": { "enabled": true, "reserveTokens": 8192 }
}
```
