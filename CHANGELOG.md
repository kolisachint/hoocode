# Changelog

All notable changes to hoocode are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed: focus-keys one-time notice (2026-11-08)
- **Focusing a tool block no longer shows the one-time key notice.** The pinned hint row (`enter open · y copy · esc clear`) already teaches the keys, so the prose notice and its per-version state key are gone.

### Fixed: transcript top/bottom move to Home/End (2026-11-08)
- **Bare `g` and `G` no longer teleport the transcript.** They typed nothing anywhere: pinned with an empty prompt, `g` jumped to the top and `G` back to live, eating the letter. `app.scroll.top` is now `Home` and `app.scroll.bottom` is `End`, so no typed letter ever moves the view. Bind `g`/`G` back in `keybindings.json` for the vim keys.

### Changed: session surfaces read as transcript facts (2026-11-08)
- **`/info` is one flat list.** The `Session Info`, `Messages`, `Tokens` and `Cost` titles are gone; every row is a dim label with its value, and the full session-file path still shows.
- **`/name` reads as a fact** (`Session name (auto)  ember-mesa · /name <name>`).
- **`/import` usage and empty `/copy` results are dim transcript rows** (`Usage: /import <path.jsonl>`, `nothing to copy yet`) instead of red errors.

### Changed: quieter startup notices (2026-11-08)
- **Persistent state lives in the status line.** Extra-usage and skip-permissions are dim status rows (no timeout); skill/prompt/theme conflicts collapse to one dim row with details behind the toggle. No more yellow chat blocks for non-blocking info.
- **One startup voice.** Dead tip entries are gone, the changelog starts as one line, and the update notice follows the 3 s info rule.

### Changed: quieter errors, truer hints (2026-11-08)
- **Red only on `⊗` and `exit N`.** The turn-end `failed` word, the `N failed call(s)` count, the failed Model error text and the error blocks are all dim now; the glyph (and `exit N`) keeps the red. Error rows read as dim band rows (at most 3) with the error and what to do about it. The failed Model block keeps every fact (`gave up · 6 attempts · 6.2s`) with the key that sends again.
- **Hint rows match the other panes.** The extension prompt reads `move · open · close`, like every other picker. Hint keys always come from the keymap, so a rebound key shows the new key.

### Changed: task-panel flight rows (2026-11-08)
- **Agent rows read type first.** A running subagent row is `◉ explore  Trace the auth flow  12s · muse · 3:12 left`: the type, then the title, then the facts. The live tool activity is gone, and the row carries the model's short scoped alias instead of the full `provider/id`.
- **Facts sit after the title.** Token counts, schedule firings, notes and `cancelled` follow the title after two spaces instead of pinning right. Narrow rows drop the deadline first, then the model, then the clock, and cut the title at a word boundary last.

### Changed: subagent-response nudge (2026-11-08)
- **A finished background subagent names its run.** The follow-up header reads `Background agent explore#4 (toolu_01Hx7) finished:` instead of `Background tool "Agent" (id toolu_01Hx7) finished:`, so the notice matches the run's handle. Old session files still read.
- **Notice heads use the glyph set.** A finish is a green `●`, a failure a red `⊗` with a dim reason, a cancel all dim; the `◦`, `✗` and `⊘` marks and the yellow `(partial)` are gone.
- **The attached-run header takes `›`, and a skill block is its bold name only.** `› explore · Find the footer code · running · esc back`. The `◆` mark is gone.

### Fixed: duplicate parallel tool calls (2026-11-05)
- **Parallel tool calls keep their own arguments.** Proxies may deliver one call's completion after the next call started; the Responses stream reader applied every completion to the open call, so the first call's arguments leaked into the last call of a batch (`Read` rows appearing twice, `Shell` calls carrying a `Read`'s `path`). Argument assembly is now routed per call, and a replayed add reuses its row instead of pushing a second one.

### Changed: themes and keys (2026-11-05)
- **Themes.** `/theme` lists `auto` (follows the system light or dark setting), the built-ins and your themes in `~/.hoocode/themes/`, and saves the choice to the global settings. A project's own `theme` setting wins, and the picker says so. New built-ins: `high-contrast`, `dusk`, `paper`.
- **`"inherits": "<theme>"`** in a theme file takes the missing colours from another theme, so a custom theme can be a few lines. Reference copies of the built-ins and the schema are written to `~/.hoocode/themes/defaults/` on each upgrade.
- **A bad theme file or unknown name** shows a warning and keeps the last good theme. The theme on/off toggles in `hoocode config` and the unused `theme_paths` setting are gone (old settings files still load).
- **No Alt keys in the defaults. Ctrl+X is a leader key:** press it, then a letter (`m` model, `t` thinking, `e` external editor, `/` search the transcript, `?` all keys). A popup lists the leader keys after a short pause. Newline is Shift+Enter or Ctrl+X Enter.
- **Enter while a turn runs steers it.** Ctrl+X `f` queues a follow-up, and Up on an empty prompt takes the last queued message back. Ctrl+R searches past prompts.
- **Pickers, the tree and scroll mode use plain letters** (`j`/`k`, `g`/`G`, `q`, `?`). `/` starts typing a search. The approval card: `y` yes and `n` no answer at once, `a` selects always and Enter confirms.
- **`~/.hoocode/keybindings.json`** takes a grouped form with `leader` and per-context bindings as well as the old flat form. Alt keys you set there still work. New `hoocode keybindings --defaults`, `check` and `--read`. See the keybindings page.

### Changed: a plainer, calmer screen (2026-11-04)
- **Plain words everywhere.** The space words (SHIP, FUEL, CR, LANDED, HIT, CLEARANCE, HANGAR, mission, comms, satellites) are gone from the screen, help and notices. `scripts/ci/plain_words.sh` keeps them out.
- **No header, logo or frames.** The start screen is two text lines. The prompt is one row (` › `). Under it a status line shows the view, session, model, context use, cost and what hoocode is doing now (`◉ working`, the last turn's result, `not signed in  /login`). Scroll and search position show there too.
- **Transcript** reads as: your message on a band, one dim line of thinking, one row per tool call (`● Edit src/wrap.rs  +4 −1`), then the answer. Step numbers, rails and "Took" lines are gone; only a failed or stopped turn gets an end line.
- **`outline` view** (the old `radar`, which still works as a setting value) shows one recap row per turn (` ● Read x · Edit y`). `alt+o` cycles outline, peek and full.
- **Dialogs are panes** that take the prompt's place: a title chip, rows, a filter row and a hint row, with a detail column beside the list at 100 columns or more. With `NO_COLOR` they get light borders instead.
- **Permission asks** `Allow Shell?` with `$ command` and an `access:` line when the call writes, uses the network or pipes. Keys: `1` yes, `2` always for that program this session, `3` no and say why, `e` edit first. Questions use numbered options and `esc` skips (the agent says which default it uses).
- **Notices** no longer pop up: a short one takes the left of the status line for a few seconds, a longer one is a dim row above the prompt.
- **First run** opens the Log in pane by itself when no provider works. Esc leaves `not signed in  /login` in the status line.
- **Themes** are `dark` and `light`, and `auto` follows the terminal background and re-checks when it changes. The other built-in themes and their old names are removed; an unknown saved theme falls back to `auto`.
- **Completion menus** (`/` commands, `@` files) use the same pane look.

### Removed: screen chrome (2026-11-04)
- The `/chrome` and `/color` commands and the `alt+z` key.
- The HUD, wordmark and starfield, session chip, toast float and the two-row footers.

### Added: defaults for a first run (2026-11-02)
- **`~/.hoocode/hoo-config.json` is written on first run** with the shipped mode rules: ask and debug block Edit and Write, plan writes only under `.hoocode/` without asking, build allows everything with the usual approvals. A mode key your file leaves out takes the shipped value.
- **Read-only shell commands run without asking, in every mode** (`safe_shell`, on by default): `ls`, `cat`, `head`, `grep`, `rg`, `git status`/`diff`/`log`/`show` and similar, alone or joined by pipes and `&&`. A file redirect, `$(…)`, or any other program asks as before.
- **`--skip-permissions`** (or `HOOCODE_SKIP_PERMISSIONS=1`, or `"skip_permissions": true` in the global `hoo-config.json`) runs every tool without asking; the mode rules still apply and the header shows SKIP PERMISSIONS. Subagents of such a session inherit it.
- **`hoocode --help`, the README, the install docs and the installer's closing message explain modes, approvals and `--skip-permissions`** up front.
- **Shift+Tab cycles the agent mode** (ask → plan → build → debug). Thinking level stays on Alt+T.
- **A project cannot loosen your approvals.** A project's `.hoocode/hoo-config.json` no longer adds to `auto_allow` or widens your `allowed_write_paths`, and a write to `.hoocode/*.json` (hoocode's own project config) always asks, in every mode and with `--skip-permissions`. A plan-mode write that a symlink would send out of `.hoocode/` is blocked.
- `/mode` shows in the first rows of the `/` menu, right after `/model`.

### Fixed: first-run rough edges (2026-11-02)
- The "no model selected" message no longer points at doc files that are not installed; it names `/login`, the API key variables and the online docs.
- A default start no longer warns that the semantic search index is unavailable; the notice shows only when you turn the index on.
- An allowed write path no longer matches a path that climbs out of it with `..`, and `>&file` / `>|` redirects no longer count as harmless for an Always-approved program.
- `--skip-permissions` now shows SKIP PERMISSIONS in the header from the first frame (it only appeared after a reload), with a line in the chat at startup.
- A Write or Edit blocked by a mode rule or denied no longer counts as "Edited" in the turn's summary.
- The install script creates the shell rc file when a fresh account has none (for the login shell: `.bashrc`, `.zshrc`, fish `config.fish`, else `.profile`), so `hoocode` is on `PATH` in the next terminal instead of a warning to add it by hand.
- **A first start with no model shows a "Sign in to start" transmission** pointing at `/login` and the API key variables, instead of failing only after the first message.
- The OpenCode Muse models (`muse-spark-*`, `muse-wonder-1.0` on `opencode` and `opencode-go`) no longer fail with thinking off: hoocode sends no reasoning effort instead of `none`, which they reject.
- **A default model your account cannot use no longer stops the first message.** When hoocode picked the model itself (no `--model` or `--models`, no saved default) and the provider refuses it (402, 403 other than a content refusal, 404, 410, a model it calls unavailable or deprecated, or a free tier used up for hours), it switches to the next provider default you have a key for, says so (`opencode/kimi-k2.6 is not available on this account (...). Switched to opencode-go/kimi-k3`), continues the turn, and saves the model that answered as your default. One OpenCode key serves both `opencode` (Zen) and `opencode-go` (Go), with different access. A model you chose is never switched. JSON and RPC clients see a `model_fallback` event.

### Removed: migration and old-format readers (2026-11-02)
- **hoocode is a new product, so nothing converts older data.** Gone: the one-time merge of the pre-1.2 (old product name) home and project folders into `~/.hoocode`, the `hoocode migrate` command, the v1/v2 session upgrade (a session file that is not the current version no longer loads), the settings key migrations (`queueMode`, `enableEmbsearchTools`, `websockets`, old `skills`, `retry.maxDelayMs`, `enabledModels`/`modelCategories` to `scopedModels`, `toolOutputDisplay` and `bare` values), the keybinding id renames, the `.hoocode/scheduled_tasks.json` fallback and owner-less scheduled tasks, and the `.hoocode/plan.md` plan fallback. Old values fall back to defaults.

### Fixed: the TUI froze after an interactive shell command (2026-11-01)
- **A command can no longer take over hoocode's terminal.** `Shell` commands, background jobs and subagent processes now run in a session of their own, with no controlling terminal. Before, an interactive program in a command (for example `zsh -i -c '…'`) made itself the terminal's foreground job, and the next key you pressed stopped hoocode ("suspended (tty input)"). Such a command now runs without a terminal, so a program that needs one fails instead of freezing hoocode.
- **The other helpers hoocode starts run without a terminal too.** MCP stdio servers, harness and extension commands, and `!cmd` config values (such as an API key read by a command) get the same treatment. The external editor still gets the terminal, as before. A `!cmd` credential helper that asks for a password on `/dev/tty` now fails instead of prompting. Use one that prompts through a desktop dialog or `pinentry`, or that needs no prompt.
- **The TUI comes back whole after an outside stop.** When hoocode is stopped by something other than Ctrl+Z ("suspended (tty input)", or a `kill -TSTP`) and you resume it with `fg`, it now turns raw mode back on, pushes its Kitty keyboard flags again and redraws the screen. Before, the screen stayed stale and keys echoed as plain text. While hoocode is stopped, the shell can still show key-release codes such as `;9:3u`, because the stop leaves the Kitty flags on.

### Fixed: startup logo and attached views (2026-11-01)
- **The owl logo keeps its three rows in a narrow terminal.** The banner re-lays itself out on resize (a vertical split no longer wraps the star dots onto their own rows), the dots are placed for the fullscreen body's one-narrower width, and a dot that would land past a long working-directory row is skipped instead of wrapping.
- **The startup logo is hidden while a subagent run or a shell job's output is attached.** It used to stay above the attached transcript. It comes back once on Esc, and a session switch leaves one logo, not two.
- **COMMS LOST stays with the session while you read a run.** A run's failed attempts no longer light the main session's COMMS LOST, and a re-read of the run keeps the session's flag as it was.

### Fixed: scheduled prompts (2026-11-01)
- **A scheduled task now fires only in the session that created it.** Every task records its owner (`ownerPid`, and `ownerSessionId` for `/loop`). Another idle hoocode open in the same folder used to claim the fires and run them as if typed, while the creating session saw nothing. Now it leaves the task alone. A task whose owner has exited is adopted by the first session that finds it due, with a warning: `Adopted scheduled task <id> from an exited session.` A task with no owner (written before this release) is not adopted by an idle session. It does not fire until it is re-created: run `CronDelete`, then `CronCreate`. `CronList` shows such a task as `owner: none — written by an older hoocode; it will not fire.` **Tasks created before this release must be re-created to fire.**
- **The Cron tools exist only in the interactive TUI.** Print, json, rpc and subagent runs never tick the scheduler, so a `CronCreate` there was accepted and never ran. They no longer offer `CronCreate`, `CronList` or `CronDelete`.
- **A fired prompt says it was scheduled.** The message starts with `⏰ Scheduled task <id> (<cron>, recurring|once)`, so you and the model can see where it came from. A prompt that starts with `/` (a slash command or prompt template) is sent unchanged so it still expands; a status line `⏰ Scheduled task <id> (<cron>, recurring) fired: <prompt>` shows the fire instead.
- **`CronList` shows the state of each task.** Per task: id, cron, recurring or once, owner (this session, a pid, or none), how many times it fired, last fire, next fire, and the prompt.
- **Prompts that fire in the same minute all run, in order, and none is lost.** A prompt is now "in flight" from the moment it is started, not from when its turn begins. The scheduler claims nothing while one is in flight (a claim moves the task's last-run minute or deletes a one-shot, so a fire it could not send was lost), and fires wait in a backlog that sends one per free moment as a normal prompt. A slash or extension command in the backlog works and no longer strands the prompts after it. Before, a job wake, a `/loop auto` step or a typed message could start a second prompt in that gap, fail with "Agent is already processing a prompt", and settle the first prompt's turn as failed. Text typed in the gap is queued behind the prompt. A `/new` or `/resume` drops the backlog with the warning `N scheduled prompt(s) dropped by the session switch`.
- **A Schedule row keeps its next fire after a run.** It reads `done · next in 4m` (or `⚠ run failed · next in 4m`) where the time used to vanish.

### Fixed: background shell jobs (2026-11-01)
- **A finished background job wakes an idle agent.** In the interactive app, when a job started with `Shell` and `run_in_background` ends while the agent is idle, hoocode sends the model a prompt headed `[Notice from hoocode, not typed by the user]` with the job's notice, so it can act without you typing. Before, the notice waited for your next message. A job you or the model killed does not wake it. While a turn runs the notice still rides on the next model call, and each completion reaches the model once. Print and rpc modes are unchanged.
- **`ShellOutput` can wait.** `wait: true` blocks until the job has ended (`timeout_ms`, default 300000, at most 1800000) and then reads as usual; if the job is still running at the timeout, the output ends with `[still running after Ns; call again with wait: true or keep working]`. Esc ends the wait at once. The model is told to use it instead of running `sleep` in a foreground `Shell`, which blocked the whole session.
- **Wrong-tool ids give a clear error.** `ShellOutput` and `ShellKill` given a subagent handle (`explore#1`) say to use `AgentOutput`; `AgentOutput` given a `job-N` id says to use `ShellOutput` or `ShellKill`. An unknown job id lists the running jobs.
- **The tool list is accurate.** It read `ShellOutput: ShellOutput: read or stop a background shell job` (the same text for `ShellKill`). Each tool now has its own line, and `AgentOutput`'s names its handle form.
- **`Shell` tells the truth about the notice.** The start message said "A notice arrives when it finishes"; it now says it comes with the next model call, or as a new message when the agent is idle.
- **The job wake is a short generic prompt and loses nothing.** It no longer carries the notices or takes them: the model call it starts adds them, as for any call, and a wake that fails before a model call (no model, no key) leaves them for the next call. It is not your last prompt (an empty Enter after COMMS LOST does not send it again), does not clear COMMS LOST, and waits while you type, `/loop auto` runs, or a prompt starts. A failed wake is not retried until the notices are taken or a new job finishes.
- **`ShellOutput` with `wait` polls every 100 ms** (was 10 ms) for up to 30 minutes; Esc still ends it at once.

### Changed: dependencies (2026-10-31)
- rustls 0.23.46 (was 0.23.45), in the main build and in `fuzz/`.

### Fixed: thinking on Opus 5 (2026-10-31)
- **Thinking text shows again on Opus 4.8 and Opus 5.** hoocode now asks every model for summarized thinking. Before, these models defaulted to `omitted`, which returns empty thinking blocks. Set `"thinkingDisplay": "omitted"` to turn the text off.
- **`thinkingDisplay` accepts `summarised`** as well as `summarized`. Any other value is ignored with a warning in the transcript at startup, where before it was ignored silently. A bad `scopedModels` entry and a settings file that fails to load now get the same warning.
- **Radar hides thinking.** Peek and full show it; radar no longer draws a `✻` line for it, in or out of a mission.

### Changed: task panel tabs (2026-10-31)
- **The hint foot names only what Enter and `x` do on the picked row.** An agent run reads `enter open its log` (and `x cancel` while it runs), a shell job `enter open output` (and `x stop` while it runs), a Login row `enter log in`. A plan item or a schedule job shows only the moves and `esc back`. The old `enter jump to its steps · x drop` named keys that did nothing.
- **The Plan tab counts its items only.** `Plan 0/3` with two plan items and a nested run now reads `Plan 0/2`, as the title does.
- **Live tabs count what runs.** Agents, Shell, Schedule and MCP show `Agents 2 running` or `Shell 1 running` while work runs, else `done/total` (`Agents 3/3`). The shown tab stays in brackets.
- **A shell row has one status glyph.** The leading `⌁` is gone (`◉ sleep 60`); a running job keeps its satellite mark on the right (`⌁ sat job-1 · in orbit 4s`).
- **The `alt+l` help text lists the tab order Plan → Agents → Shell → Schedule → MCP**, and no longer says "skips empty tabs".

### Changed: background work (2026-10-31)
- **No blank rows above the prompt while a mission runs.** The silent working indicator drew two empty rows between the transcript (or the REWIND bar) and the comms frame; it now draws none.
- **A prompt sent while a background subagent or shell job runs is answered at once.** The agent used to sit idle until the background work finished, and the prompt showed `queued · sends after this step`. Now it starts a turn right away; the background result arrives later as before.
- **Background shell jobs are on by default in the TUI** (`enableBackgroundShell`, set it to `false` to turn them off). The model is told to run long commands (full test suites, release builds, servers) with `run_in_background` and keep working, and to keep subagents in the background unless its next step needs the answer. Print, json, rpc and app-server runs and subagents never get background shell jobs.

### Changed: task panel and prompt keys (2026-10-31)
- **The task panel is shown by default in peek and full**, as the FLIGHT PLAN console, and hidden in radar. **Ctrl+G shows or hides it** in any view; it no longer moves the keyboard.
- **The prompt keeps the keyboard.** Up on the prompt's first line moves into the panel; Up and Down pick a row, Down past the last row comes back, Left and Right change tabs, Enter attaches to a subagent run or shell job's output, Esc goes back.
- **Prompt history is Shift+Up / Shift+Down.** Plain Up and Down move the cursor (and into the panel). PgUp pins the transcript.
- **The full view opens the startup keys and the Skills, Agents, Commands and MCP lists**; peek and radar fold them. Skill rows read `/skill:<name>`.
- `/chrome compact` no longer shrinks the task panel to one line; the panel shows at most a few rows on a short terminal.
- The comms frame foot reads `radar`, like `peek` and `full`.

### Fixed: radar answer, HUD effort, clickable links (2026-10-31)
- **A click on a link opens it in the browser.** Markdown links and bare `http(s)://` URLs in the transcript open on a plain left click (press and release on the same cell). Only http, https and mailto open. `HOOCODE_MOUSE=0` still hands the mouse back to the terminal.
- **The radar view renders a mission's final answer as Markdown**, as peek and full do: headings, lists, code and emphasis are drawn instead of flattened to plain text, and links stay clickable.
- **The top bar shows the thinking level** after the model (`SHIP claude-opus-5-5 · medium`), for reasoning models. On a narrow screen the folder drops first.

### Fixed: subagent handles (2026-10-30)
- **Each background subagent is named from the moment it starts.** An Agent call's first answer is now `explore#2 ("audit the db layer") started in the background` with its handle, instead of the same `Started "Agent" in the background` for every call. The model could not tell two runs of one agent apart, took one for a duplicate, cancelled the wrong one and started it again.
- **`AgentOutput` lists every run with its handle and description**, including one dispatched a moment ago, and the finish, failure and cancel messages name the run the same way. A cancel says which run it stopped, and only says so once the run has stopped (warm subagents included); a cancel on a finished run says nothing was cancelled.
- **Cancelling a subagent stops the commands it started** (they run in process groups of their own and used to keep running).
- A call refused after its handle was given (an unknown agent, a bad model) reports `explore#1 (…) failed ✗` with the reason.
- **Handles are per session and survive `--resume`:** a resumed session continues after the handles it already used, and an old handle says it ran in an earlier process. `/new` and other sessions no longer list each other's runs.
- OpenAI-compatible providers: a tool call with no id, or an id another call in the same reply already used, gets an id of its own.

### Changed: radar view reads like peek and full (2026-10-30)
- **Each mission is grouped under its prompt.** Radar shows a `▲ Mn  prompt` header, one line per batch of tool calls (a batch ends at thinking), failed calls as nested `└ … HIT` rows, thinking as one dim `✻` line, the full answer, and an end row that always closes the mission: `● LANDED`, `⊗ DID NOT LAND` (with its reason), `⊘ ABORTED`, `◐ IN FLIGHT` or `◆ WAITING` (naming the approval it waits for, including a subagent's). Older finished missions fold to one summary line plus the answer. Peek and full are unchanged.

### Changed: smaller binary, quieter when idle (2026-10-30)
- **Release builds use thin LTO, one codegen unit and stripped symbols.** The Linux x86_64 binary went from 57.8 MiB to 34.9 MiB. Panic backtraces from release builds no longer carry function names.
- **Fewer idle wakeups.** An idle TUI wakes about 7 times a second instead of about 17: the perf sampler thread runs only with `--perf-log` (`/perf` reads once), the watchdog ticks every 250 ms, the git branch poll runs every second, and the UI loop waits up to 1 s when nothing is due.

### Added: releases from a tag (2026-10-24)
- **Pushing a `vX.Y.Z` tag releases it.** `release-tag.yml` checks the tag against the workspace version and that the commit is on `main`, runs the CI gates, creates the GitHub release and builds the same archives, `SHA256SUMS`, npm and PyPI packages as a labelled-PR release.

### Changed: smoother TUI in long sessions (2026-10-23)
- **Streamed text shows up as it arrives.** The screen used to wake on a timer (up to 250 ms), so replies and tool output appeared in bursts. Repaints asked for only by the agent are capped at about 60 a second, so typing stays responsive.
- **Frame cost no longer grows with the transcript.** Messages that have not changed keep their drawn lines between frames, and full-screen mode copies only the rows on screen. In a 200-turn session the median frame went from about 43 ms to about 12 ms.
- Streaming no longer copies the whole reply several times per token.
- **Provider events share the reply so far.** `AssistantMessageEvent::partial` is an `Arc<AssistantMessage>`; providers append in place while the agent loop keeps up, instead of copying the whole reply for every token (a 57 KB reply through the faux provider: 22 ms of provider and stream CPU down to 10 ms). The `message_update` JSON of `--mode json` and RPC is byte-identical.
- **Idle frames skip the walk over the transcript.** A frame in which no component changed no longer visits every message in the transcript. In a 200-turn session the median frame went from about 3.7 ms to about 2.8 ms.
- **No freeze when a long reply ends.** Drawing a finished reply of about 57 KB took 1.8 s on the UI thread (the Markdown lexer tried each block rule against the rest of the text); it now takes about 50 ms.

### Changed: install with `npm install -g hoocode` (2026-10-23)
- The unscoped `hoocode` package is the install name in the README, docs and the update notice. `@kolisachint/hoocode` is still published as an alias.
- The README carries the hoocode logo.

## [0.2.0] - 2026-10-22

The first release for people outside the project: plugins can be managed from the shell, `--help` describes only what this build does, and the repo has a README, contributing guide, security policy and templates written for new users and contributors.

### Added: plugin commands in the shell (2026-10-22)
- **`hoocode install <name[@marketplace]>`, `hoocode remove <name>` (or `uninstall`) and `hoocode list`** install, remove and list plugins without starting a session. `--scope user|project` picks where a plugin goes; user scope (`~/.agents/plugins`) is the default.
- **`hoocode plugin <command>`** runs every `/plugin` command from the shell: `marketplace add <source> [--trust]`, `marketplace list`, `marketplace refresh`, `update`, `trust`. `hoocode plugin --help` lists them. A running session picks up a change on `/reload`.
- These replace the old `install`/`remove`/`list` entries in `--help`, which only printed "not yet supported".

### Changed: `--help` lists only what works (2026-10-22)
- **Config directory:** `HOOCODE_CODING_AGENT_DIR` defaults to `~/.hoocode`, as it always has in this build. The help said `~/.hoocode/agent`.
- **Commands:** adds `plugin`, `migrate` and `app-server`. `hoocode resources` is gone from the help and now says to use `hoocode config`.
- **Flags that are not implemented are no longer listed:** `--export`, `--extension`/`-e`, `--no-extensions`, `--agent`, `--theme`, `--no-themes`, `--verbose`, `--platform`, `--enable-webtools`, `--enable-todowrite`, `--enable-semantic-index` and `--print-token-surface`. Each still fails with a clear "not yet supported" error if you pass it.
- **Examples and variables:** examples use Rust files; `HOOCODE_USER_AGENTS_DIR`, `HOOCODE_FULLSCREEN` and `HF_TOKEN` are listed; `HOOCODE_SHARE_VIEWER_URL` and the two `HOOCODE_WEBTOOLS_*` variables, which nothing reads, are gone.

### Changed: in-app text matches what ships (2026-10-22)
- **Slash commands:** `/share` is no longer offered (it only said "not available yet"). `/export` says it writes JSONL. `/reload` no longer mentions extensions.
- **Tips:** the `/plugin` and `hoo` alias tips are back in the rotation; the export tip no longer promises HTML or `/share`. A test checks that every tip names only commands that exist.
- **Built-in skills:** `canvas-design` is removed, and `artifact-design` no longer points at `/canvas` or `/new-canvas`. hoocode has no canvases.
- **`DocSearch`:** its description now says what it searches, the loaded skills, subagents and plugins, instead of promising documentation it does not index and a `ResolveMcpTools` tool that does not exist.

### Added: a demo in the README (2026-10-22)
- **`docs/assets/demo.gif`** shows the real binary fixing a bug in a small Rust crate: Read, then Edit and `cargo test` after approval, then the slash-command menu. Only the model is scripted (the mock LLM of the screen goldens); `scripts/demo/record.sh` records it again for each release.

### Added: docs and templates for contributors (2026-10-22)
- **New:** `CONTRIBUTING.md` (setup, the dev loop, tests and goldens, code style, releases), `SECURITY.md` (private reporting, what counts), `CODE_OF_CONDUCT.md`, issue templates (bug, feature, question) and a pull request template.
- **README rewritten for users:** what hoocode is, every install route that exists (install script, npm, pip, release archives, source), quick start, where configuration lives, and links to the docs.

## [0.1.41] - 2026-10-21

Versions 0.1.1 to 0.1.41 were released from `main` as PRs merged. The entries below were written as the changes landed, newest first, and are not split by version. Per-version notes are on the [GitHub releases](https://github.com/kolisachint/hoocode/releases) page.

### Fixed: subagents ran gated tools without asking (2026-10-21)
- **A subagent's Shell, Edit, Write, WebFetch and WebSearch calls now go through clearance.** They used to run unasked because the child (`--mode json`) has no screen. The child now asks its parent over its pipes: `{"type":"clearance_request","id","tool","input"}` on stdout, `{"type":"clearance_response","id","allow","reason"?}` on stdin.
- **In the TUI the parent's own gate decides,** so config rules and a session `always` apply once for the main agent and every subagent. The card names the subagent; the Agent block shows `◉ … holding for clearance` while it waits. Requests that arrive together queue instead of denying the card that is up. A child waiting on a person is not reaped as stalled.
- **Without a UI it fails closed.** A subagent of a `--print`, `--mode json` or rpc parent is denied what needs approval, and so is one whose parent quits, whose stdin closes, or whose task is cancelled. Allow a tool with `auto_allow` in `hoo-config.json`. The stderr note of `--print` and `--mode json` says so.

### Fixed: subagent reliability (2026-10-11)
- **Background tasks survive their run.** A task that settles after its run ended is delivered by the next run of the same session. Managers are kept per session (cap 64, idle first evicted).
- **A tool-only final message no longer blanks the summary.** The summary falls back to the last text. It says `(from an earlier turn)` only when tool calls came after that text; an empty aborted wrap-up does not count. New RPC command `get_last_assistant_text_with_origin`.
- **Shell calls time out after 600 s by default, inside subagents only.** The main session has no default limit. `timeout: 0` means no limit; `HOOCODE_BASH_DEFAULT_TIMEOUT_SECS` overrides the 600 s.
- **A tool in flight is bounded** by the larger of 15 min × load and its declared timeout. A tool with timeout 0 has no bound.
- **Nested subagents are not swept.** A child's own subagents run in their own process groups, so the post-reap sweep does not reach them.
- **A run cut short at its deadline settles `partial`** in the pool, matching `result.json`. It used to show `complete`.
- **`Read` on a directory returns a listing.**
- **`resume_task_id` takes the label** from a result's `resume_task_id:` line (e.g. `explore#1`). A raw task id still works.
- **Stalled children get SIGUSR1 first,** then a grace period, then SIGKILL. After a reaped child exits, its process group is swept.
- **Resumed sessions survive the dispatch prune** while their task is running or queued.

### Removed: hoocode-ts pin, shim and parity checks (2026-10-10)
- **hoocode fully replaces hoocode-ts.** The pinned TS checkout, the `hoocode-ts` shim (installer and release archives), the parity harness, the CI fixture fetch and the TS-comparison tests are gone.
- **Compaction tests use a generated session** in the old v1 format instead of the TS `large-session.jsonl`.
- **Moved:** `replay.json` to `crates/hoocode-code-main/tests/fixtures/`, and the dependency firewall check to `scripts/ci/`.

### Changed: model catalog comes from models.dev (2026-10-10)
- **New crate `hoocode-models-sync`** (`cargo run -p hoocode-models-sync --bin models-sync`) refreshes `data/models.json` from models.dev. Curated fields are kept and models are never removed automatically. Rules live in `data/overrides.json`.
- **A weekly workflow** (`models-sync.yml`) opens a PR when the data changes. It needs a `MODELS_SYNC_TOKEN` secret for that PR to run CI.
- **First sync:** 31 models added and 8 prices updated (DeepSeek, Fireworks, Anthropic cache read, Kimi K3 cache write).

### Changed: smaller target/ (2026-10-10)
- **Dev builds have `debug = false`.** A full build is 5.5 GB instead of 61 GB, and there are no `.o` debug files. Panic messages still show file:line.
- **`scripts/dev/prune_target.sh`** removes build output unused for 3 days, old names that started with the previous project name, and an incremental cache over 5 GB. It is a dry run by default; `--apply` deletes and `--if-over GB` skips small dirs. `CLAUDE.md` runs it at session start.

### Fixed: subagents are no longer stopped in the middle of a long tool (2026-10-10)
- **A running tool counts as progress.** The watchdog stopped any subagent that went 150s without a turn or tool event, so a long `cargo` build killed the agent mid-tool ("Ran out of time before completing."). Now it only stops a child that has no tool running. The heartbeat check and the 2-hour deadline are unchanged.
- **The footer no longer misses a branch switch made right after startup.** The git HEAD watcher took its baseline on a background task, so a HEAD change that landed first was never seen.

### Changed: footer has two modes (2026-10-10)
- **Alt+Z switches between full and compact.** `bare` is gone; a saved `bare` loads as compact, and `/chrome bare` gives an error.
- **The full footer has a fixed 22-cell left column**, so mode, context bar, model and tokens line up.
- **The compact footer is one row.**

### Changed: tool calls in peek and radar (2026-10-10)
- **Peek view:** every tool block has a 2-space left margin, and its output starts under the tool name. Read, Write and Edit sit on a light band (new optional theme token `toolBandBg`, which falls back to `toolPendingBg`).
- **Shell reads `● Shell  <cmd>`**, like the other tools, instead of `$ <cmd>`. No blank row after the call line.
- **Write shows a 5-line, line-numbered preview** of what it wrote.
- **Radar trims targets to 40 columns** with `…`, and shows only the first line of a command.

### Changed: less process between an idea and a release (2026-10-10)
- **Done = fmt, clippy and the tests of the crates you touched.** CI runs the whole workspace.
- **Screen goldens cut to 10 scenarios, run before a release.** Review bundles and the `tui-review` skill are removed.
- **CI:** a release no longer re-runs the gates (label `release:run-gates` forces them). macOS tests and subagent evals are off PRs. No test retries. Fuzzing uses a prebuilt `cargo-fuzz` and a cache.
- **The finished TS→Rust migration is archived** under `archive/`.

### Changed: subagents may run for 2 hours; AgentOutput can cancel (2026-10-10)
- **One 2-hour deadline for every subagent type.** The per-type 5-20 minute table is gone. Load scaling adds at most 30 minutes.
- **Poll and cancel with `AgentOutput`.** Poll by `task_id`; stop a run you no longer need with `AgentOutput(task_id, cancel: true)`.
- **Stall detection still reaps hung children early**, so a child stuck in a provider call does not hold the full deadline.

### Changed: header cursor blinks; wording cleanup (2026-10-10)
- **The header cursor blinks** every 530 ms, driven by the app clock (Ghostty ignores SGR 5). It stops
  after the first prompt, or while the view is scrolled back, and then stays solid.
- **Warnings use `⚠`** in the notification band, not `●`. One `WARNING_GLYPH` serves every warning line.
- **Ellipses are `…`** in the status and dialog labels: reloading, summarizing, auto-compacting,
  waiting for browser login.
- **Key names are lowercase** in hints: `esc`, `tab`, `ctrl+enter`.
- **Compaction cancelled** is a status line, not an error.
- **The header hint** no longer shows `ctrl+o more`. The header expands only under `verbose`.
- **The `ctrl+o` hint** on the tool-output row reads `to toggle radar and peek`.

### Added: scroll back while a picker is open, and a scrollbar (2026-10-10)
- **Scroll with a picker open.** PgUp/PgDn, Ctrl+Home/End and the wheel move the transcript while a
  picker or question has focus. Arrows, Enter, Esc and typing still go to the picker and never un-pin.
- **Scrollbar** in the last column while scrolled back: `┃` is the thumb, `░` the track. Click above
  the thumb to page up, below it to page down.

### Changed: footer is always two rows, layout B; the folder name is never dropped (2026-10-09)

### Changed: the tool output dial is radar/peek only (2026-10-09)
- **Two stops, not three.** The dial is `radar` and `peek`. The `full` stop is gone; a saved `full`
  view reads as `peek`.
- **Ctrl+O toggles** between `radar` and `peek`. Alt+O still cycles the dial.
- **The `ctrl+o to expand` hints are gone.** Truncated tool output now ends with `... (N more lines)`
  and no longer names a key.

### Added: app-server clients choose the effort and see the scope (2026-10-09)
- `thread/start`, `thread/resume` and `turn/start` take an optional `effort`. An explicit effort the model
  does not support is rejected, not silently changed.
- Switching to a scoped model applies its effort. Scoped models are listed first in `model/list`, with
  their `category` and `alias`, and each model reports the efforts it supports.
- Thread results include `reasoningEffort`. Names outside the model scope are rejected.

### Fixed: expired OAuth login says how to sign in again (2026-10-09)
- When an OAuth login has expired, the error now names the provider and the `/login` command to run,
  instead of the raw "No API key found" message. API-key providers are unchanged.

### Added: Claude Haiku 5.5 (2026-10-09)
- Added Claude Haiku 5.5 (`claude-haiku-5-5`) to the Anthropic model catalog: 1M context, 128K max output,
  reasoning, text and image input, $0.10 / $0.50 per MTok (cache read $0.01, cache write $0.125; the
  over-100K-token tier of $0.50 / $2.50 is not modelled). Hoocode-ts v0.6.0 does not ship it yet, so
  `scripts/models_overrides.json` adds it on each regeneration until the pin catches up.
- Haiku 5.5 uses adaptive thinking (no `budget_tokens`) and never sends `temperature`, which the API rejects.
  Haiku 4.5 is unchanged.

### Added: scoped models carry an effort and a category (2026-10-09)
- **`scopedModels` setting** in `settings.json`: an ordered list of `{ model, effort?, category?, alias? }`.
  It replaces `enabledModels` and `modelCategories`. A project `scopedModels` replaces the global list.
- **Migration:** on the first load, when `scopedModels` is missing, it is built once from `enabledModels`
  (a `:level` suffix becomes `effort`) and `modelCategories` (model to category), and written to disk.
  The old keys stay in the file and are no longer read. hoocode-ts does not see the new scope.
- **`/scoped-models` picker** has an effort column (`tab` cycles the supported levels) and a category
  column (`alt+j` cycles none, cheap, fast, standard, capable). Save writes the full entries, not ids only.
  `/settings` has a Models row that opens it.
- **Agent tool** takes `model` (a category or a scoped model) and `effort`. Tool results start with
  `[model: ...]`, naming the model that ran. `--models` still works for one run and is not saved.

### Removed: the Agent tool `complexity` parameter (2026-10-09)
- `complexity` is removed, not aliased. `model` is the only model parameter. The dispatch log line
  still prints a heuristic complexity estimate.

### Changed: tool names renamed; the old names are no longer accepted (2026-10-08)
- **New tool names:** `read` → `Read`, `bash` → `Shell`, `edit` → `Edit`, `write` → `Write`,
  `SearchCodebase` → `CodeSearch`, `SearchHooCode` → `DocSearch`, `ask_options` →
  `AskUserQuestion`, `webfetch` → `WebFetch`, `websearch` → `WebSearch`, `AgentOut` →
  `AgentOutput`. `Agent` and `TodoWrite` are unchanged.
- **No aliases for the old names.** They are unknown tools now. The legacy `Task` and
  `TaskOutput` aliases are deleted. Settings, permission rules, `enabled_tools`/`denied_tools`
  and agent `tools:` lists must use the new names.
- The model sees the built-in tools in this order: `Read`, `Shell`, `Edit`, `Write`,
  `CodeSearch`, `AskUserQuestion`, `DocSearch`, and the rest.

### Changed: the subagent tools are `Agent` and `AgentOut` (2026-10-06)
> Superseded 2026-10-08: `AgentOut` is now `AgentOutput`, and the `Task` and `TaskOutput`
> aliases described below were deleted.

- **`Task` → `Agent`, `TaskOutput` → `AgentOut`.** `Task` read as a to-do item while the tool starts
  a subagent run. The old names stay registered for one release as deprecated aliases that run the
  same executor, and the prompt's tool list marks them `deprecated alias for Agent; prefer Agent`.
- The transcript line names the tool: `Agent explore` (`Agent resume plan · background`) and
  `AgentOut explore#1 (wait)`. Tool results, notifications and error hints say `Agent`/`AgentOut`.
- Every place that checked for the subagent tool by name accepts both spellings: the agent list in
  the prompt, the footer indicator, the startup listing, the nested-agent `--tools` grant (pool and
  warm pool) and the tool-chain summary.

### Added: subagent evals and a dispatch ledger (2026-10-05)
- **Subagent reliability is now measured, not remembered.** `<cwd>/.hoocode/dispatch/ledger.jsonl`
  is an append-only line per dispatch **attempt** - agent, requested and resolved model, mode, depth,
  status, verifier verdict, confidence, wall clock, generated tokens, peak context, exit code and cause.
  Every terminal path in the pool writes one, including the ones that used to vanish: the inherited-model
  retry, a kill at the deadline, a cancelled task, a spawn failure. The file is bounded (rewritten past
  20k lines, newest 10k kept) and strictly best-effort - an unwritable ledger logs and drops, and never
  fails a dispatch. `result.json` still decides correctness; the ledger only records what happened.
- **`/subagent-stats [24h|7d|all]`** renders it in the TUI: attempts, usable rate, statuses,
  median/p90/max wall clock, tokens, fallback count, per-agent breakdown and the last five failures
  with their causes.
- **`scripts/eval/subagent_evals.py`** drives the real binary end to end - real parent session, real
  `Agent` tool (then `Task`), real pool, real child process - against a routable scripted mock provider
  (`scripts/eval/mock_provider.py`). Twelve scenarios cover the happy paths, the turn limit, an invalid
  tool call, a stream that dies mid-answer, the deadline wrap-up, the stall reaper, a region rejection
  that must fall back, a bad complexity tier and eight concurrent dispatches against five slots.
  Exits non-zero when a scenario deviates from its declared expectation; `--min-success` gates a build,
  `--repeat` measures flakiness, `--include-slow` adds the two minute-long probes. Reports land in
  `target/subagent-evals/`.
- **Fixed: concurrent ledger appends interleaved.** The record and its newline were two `write_all`
  calls, so eight simultaneous settles produced seven lines and one unparseable one. One write per
  record now, with a 400-line concurrency test. Found by the eval suite on the code the suite needed.
- **Fixed: a background dispatch was recorded as `blocking`.** The Task tool's background flag never
  reached the pool task, so `mode` in the ledger was wrong for every background run.


### Fixed: subagents stopped failing on almost every dispatch (2026-10-03)
- Subagents ran to completion in none of ten recorded dispatches. Seven fixes, detailed in
  `docs/design/subagents.md`.
- **Every subagent was killed after 5 minutes.** The per-agent timeout table was keyed on
  `edit`, `test` and `review` — agents this project does not ship — so no entry ever matched
  and every agent fell through to the 5-minute default. `code-review` died at exactly 300s
  three times holding up to 297s of finished work. Real per-agent deadlines now exist
  (`code-review`/`security-review` 15 min, `general-purpose` 20, `explore`/`plan` 10), and
  the warm worker pool's flat 3-minute limit follows the same table.
- **A subagent killed at its deadline lost everything it had finished.** `result.json` is
  written only after the run returns cleanly, so the kill discarded it. A child is now told
  its deadline up front and asked to wrap up shortly before it, so a run cut short reports
  its findings as a usable partial result instead of vanishing.
- **A subagent that died because its model was unreachable never retried on a reachable one.**
  Timed-out and stalled runs skipped the inherited-model fallback entirely, and the error
  classifier did not recognise region rejections (`requires Global regions`) or
  `finish_reason: error`. Both fixed; a cancelled run is still never retried.
- **The inherited-model fallback could silently do nothing.** When a caller passed a
  `complexity` tier, the fallback re-resolved that same tier and retried the identical model.
- **Reported token usage was wildly wrong.** The subagent budget summed each turn's whole
  context size, so a 15-turn run reported 1.6M tokens against a 35k budget. It now counts what
  the subagent generated; context size is reported separately as `peak_context`.
- A failed dispatch wrote a 256 KB `output.json` that duplicated the transcript already in
  `session.jsonl`. It now holds the outcome, the cause, and a short tail of stderr.

### Release: npm publish is now verified after it runs (2026-10-02)
- The npm publish loop moved from an inline shell loop in `binaries.yml` to
  `scripts/npm/publish_packages.py`, which publishes, then checks every package
  against the registry packument and fails the job if any is missing. A fresh
  publish is readable only after registry propagation — after v0.1.5, `darwin-x64`
  took ~11 minutes to appear — so a partial publish used to look exactly like a
  successful one. Re-runs still skip published versions, so a failed release is
  recovered by re-running `binaries.yml` for the same tag.
- `--verify-only` checks a published release without publishing, and reads the
  packument with cache busting (npm's CDN otherwise serves stale versions).

### Fixed: compaction and branch summaries on OpenCode Go (2026-10-02)
- Compaction (`/compact`, automatic context compaction, branch summaries) builds its own
  summarization request, which did not carry the session id, so OpenCode Go rejected it with
  `400 Request is missing x-opencode-session and cannot be routed efficiently`. Normal turns
  were unaffected — they already sent the header. Summarization now passes the session id
  through on every request (chat completions, responses, Anthropic Messages), and it is still
  omitted when there is no session. Two tests cover both directions.

### Pin bumped to hoocode v0.6.0 (2026-10-01)
- Model catalog regenerated from hoocode v0.6.0 (1228 models). It adds
  `anthropic/claude-sonnet-5-5`, `opencode-go/longcat-2.5-preview-free`, `opencode/gpt-6.1-sol`
  and others, and drops models upstream removed (e.g. `opencode-go/kimi-k2.6`).
- OpenCode Go: every request sends `user-agent: hoocode` and, when a session exists,
  `x-opencode-session` (Chat Completions, Responses and Anthropic Messages), even with prompt
  caching off. Go rejected requests without it: `400 Request is missing x-opencode-session`.
- Default models for `opencode-go`, `fireworks` and `together` are now Kimi K3. A test now
  fails when any provider default drops out of the catalog.
- Bundled theme JSONs match the pin (a test checks them against the pinned checkout).
- `migration/pin_drift.py` reports when upstream has released past the pin and prints the
  port checklist for a bump. `setup_hoocode.sh` installs with the pin's declared bun version.

### Migration paused, `hoocode` ready for use (2026-10-01)
- Every remaining ledger task is deferred by user decision: MCP (9.1, 10.11),
  webfetch/websearch (10.2e), the TS test ledger close-out (13.4) and phase 12. See the
  README's "Migration status" and §0.3 of the migration plan for what works and what doesn't.
- Interactive agent modes: `/mode`, `/plan`, `/grill`, `/approve` now act in the TUI. They
  notify, reload with the new mode's prompt and tool filter, and update the footer badge. alt+a
  cycles ask → plan → build → debug, and mode commands appear in autocomplete with
  argument completions (10.5e).
- Earlier in this cycle: phase 11 (the interactive TUI) is complete. That covers the chat
  view, selectors, input features, task panel, key bindings, thinking toggle and the
  external editor.

### Migration re-baseline (2026-09-24)
- Pinned the port to hoocode v0.5.89 (`a6cd96e7`). An audit found the earlier
  "100% complete" status overstated, and progress is now tracked in
  `migration/ledger.json` (see `docs/design/ts-to-rust-migration.md` §0).
- Added two-level done gates: Level 1 = cargo checks, Level 2 = rendered-TUI parity
  against hoocode via `migration/tui-parity/`.
- Removed the stale status reports (`MIGRATION_*.md`, `TEST_RECORD.md`,
  `E2E_TESTING_SUMMARY.md`, `OPENCODE_E2E_TEST_REPORT.md`, `QUICKSTART.md`).
- Live OpenCode checks moved to `scripts/live/` and
  `crates/hoocode-ai-provider-openai/tests/opencode_live.rs` (`#[ignore]`d). The API
  key now comes only from `OPENCODE_API_KEY`. A key previously committed in these
  files must be rotated.

### Added
- OpenCode API provider support (opencode, opencode-go)
- mimo-v2.5-free as default model for OpenCode provider
- Comprehensive E2E testing framework
- Vertex AI Application Default Credentials (ADC) support
- Service account JWT authentication for Google Cloud
- GCE/GKE metadata server authentication
- Windows virtual terminal input tweaks
- Paste-marker compression in editor
- Vim-style character jump (f/F) in editor
- Interactive OAuth login (Anthropic, GitHub Copilot)
- Permission gates for tool call approval
- Auto-migration from legacy ~/.hoocode settings
- Explicit --config CLI override
- Reasoning-item ID pairing in Azure provider

### Changed
- Improved provider configuration and model selection
- Enhanced error handling and user feedback
- Optimized streaming performance
- Updated documentation and test coverage

### Fixed
- Cargo fmt formatting issues
- Clippy warnings resolved
- Windows console input handling
- Context preservation in multi-turn conversations

## [0.1.0] - 2026-07-23

### Added
- Initial release of HooCode
- Rust migration from HooCode TypeScript framework
- 43 crates in workspace structure
- 640 tests passing
- Support for 6 LLM providers:
  - Anthropic (Claude)
  - OpenAI (GPT)
  - OpenCode (MiMo, DeepSeek, etc.)
  - Google (Gemini)
  - Azure OpenAI
  - Faux (testing)
- CLI with print and interactive modes
- JSON-RPC server mode
- Subagent pool and Task tool
- Core tools (read, write, edit, bash, grep, find, ls)
- MCP (Model Context Protocol) support
- WASM plugin extensions
- Session persistence and management
- Context compaction
- Cross-platform binary builds (Linux, macOS, Windows)
- GitHub Actions CI/CD pipeline
- crates.io publishing workflow

### Technical Details
- **Build System:** Cargo workspace with lockstep versioning
- **Minimum Rust Version:** 1.78
- **Test Coverage:** 640 tests across 43 crates
- **Clippy Warnings:** 0
- **CI Status:** All checks passing

### Supported Platforms
- Linux (x86_64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64)

### Providers
| Provider | Models | Status |
|----------|--------|--------|
| Anthropic | Claude 3.5, Claude 4 | ✅ |
| OpenAI | GPT-4, GPT-4o | ✅ |
| OpenCode | MiMo, DeepSeek, Claude | ✅ |
| Google | Gemini 2.5 | ✅ |
| Azure | GPT-4, GPT-4o | ✅ |
| Faux | Test models | ✅ |
