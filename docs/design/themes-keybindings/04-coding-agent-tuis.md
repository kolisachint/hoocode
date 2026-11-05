# Research 04: coding-agent TUI keybindings (2026-11-05)

Web research from official docs. Facts only; no code copied.

Key: CC = Claude Code, Gem = Gemini CLI, oc = opencode, Aid = aider, Gse = Goose,
Qwn = Qwen Code, Cop = GitHub Copilot CLI, Crs = Crush, Cdx = Codex, Cur = Cursor CLI,
Amp = Amp. "n/d" = not documented on the page read. "unv" = only third-party or search
snippets. "n/v" = official page not reachable.

## 1. Table

| Action | CC | Gem | oc | Aid | Gse | Qwn | Cop | Crs | Cdx | Cur | Amp |
|---|---|---|---|---|---|---|---|---|---|---|---|
| submit | Enter | Enter | return | Enter (implied) | n/d | n/d | n/d | n/d | unv | unv | n/v |
| newline | Ctrl+J (Option+Enter with Option-as-Meta) | Ctrl+J, Shift+Enter, Ctrl+Enter, Alt+Enter, \ + Enter | shift+return, ctrl+return, alt+return, ctrl+j | Meta+Enter | Ctrl+J (rebindable) | n/d | n/d | n/d | unv Shift+Enter | unv Shift+Enter, Ctrl+J in tmux | n/v |
| interrupt / cancel | Esc (cancel input); Ctrl+C (app:interrupt) | Ctrl+C | Esc (session_interrupt) | Ctrl+C | Ctrl+C | Esc only for /btw | n/d | n/d | unv | unv | n/v |
| exit | Ctrl+D twice (800 ms) | Ctrl+C or Ctrl+D on empty | ctrl+c, ctrl+d, leader q | n/d | Ctrl+C on empty line | Ctrl+C twice | Ctrl+C twice (unv) | n/d | n/d | Ctrl+D (unv) | n/v |
| clear input | Ctrl+C when idle; Esc Esc saves draft | Ctrl+C | ctrl+c (input_clear) | n/d | Ctrl+C with text | n/d | n/d | n/d | n/d | n/d | n/v |
| history prev/next | Up/Down (also Ctrl+P/N) | Up/Down at edge; Ctrl+P/N | up/down | Up/Down | Cmd+Up/Down | n/d | n/d | n/d | unv | n/d | n/v |
| reverse search | Ctrl+R | Ctrl+R | n/d | Ctrl+R | Ctrl+R (Ctrl+S forward) | n/d | n/d | n/d | n/d | n/d | n/v |
| external editor | Ctrl+G, Ctrl+X Ctrl+E | Ctrl+G, Ctrl+Shift+G (Ctrl+X deprecated) | leader e | Ctrl+X Ctrl+E | none (GOOSE_PROMPT_EDITOR) | n/d | n/d | n/d | n/d | n/d | n/v |
| paste image | Ctrl+V (Alt+V on Windows/WSL; Cmd+V iTerm2) | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/v |
| mode cycle | Shift+Tab (Meta+M on old Windows) | Shift+Tab (default, auto_edit, plan) | tab / shift+tab (agent cycle) | n/d (/ask, /code) | n/d | n/d | n/d | n/d | unv Shift+Tab | unv Shift+Tab | n/v |
| model switch | Option/Alt+P | n/d | ctrl+a (providers), leader m (list), f2 (recent) | n/d | n/d | n/d | n/d | Ctrl+L | n/d | n/d | n/v |
| transcript / verbose | Ctrl+O | Ctrl+O (expand/collapse) | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/v |
| todo list | Ctrl+T | Ctrl+T | n/d (ctrl+t = variant cycle) | n/d | n/d | Ctrl+T (MCP tool descriptions) | n/d | n/d | n/d | n/d | n/v |
| permission dialog | Enter yes, Esc no, Up/Down; y/n only if bound | number keys + Enter; Up/K, Down/J | n/d | n/d | n/d | n/d | n/d | n/d | n/d | unv y/n | n/v |
| @ / slash / ! | @ files, / commands, ! bash | n/d (? shortcuts panel) | n/d | /commands | n/d | n/d | n/d | n/d | unv @ | unv | n/v |
| double Esc | clears non-empty draft, else rewind menu | clears draft, else rewind | n/d | n/d | n/d | n/d | n/d | n/d | unv | n/d | n/v |
| queue while running | Enter queues; Ctrl+X Enter queue-submit; Ctrl+Enter / Ctrl+X Ctrl+S send queued now; Up takes queue back | Tab queues | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/d | n/v |
| leader / prefix | none; Ctrl+X is a chord prefix | none | Ctrl+X leader, 2000 ms timeout | Ctrl+X prefix (readline) | none | none | n/d | n/d | n/d | n/d | n/v |
| config file | `~/.claude/keybindings.json` (/keybindings creates it) | `~/.gemini/keybindings.json` | `tui.json` "keybinds" | none (--vim, --multiline) | GOOSE_CLI_NEWLINE_KEY only | none found | none found | none found | unv `config.toml [tui.keymap]` | none found | n/v |

## 2. Patterns

Agreement (3+ tools, verified):
- Enter submits: CC, oc, Gem (Aid implied).
- Ctrl+J newline: CC, oc, Gem, Gse (4).
- Alt/Meta+Enter newline: oc, Gem, Aid (3). Shift+Enter also in oc and Gem.
- Ctrl+R reverse search: CC, Gem, Aid, Gse (4).
- Up/Down history: CC, Gem, oc, Aid (4). Ctrl+P/N in CC and Gem; oc uses ctrl+p for its command palette.
- Ctrl+C interrupts or cancels: CC, Gem, Aid, Gse (4). Esc interrupts in CC (input cancel) and oc (session).
- Ctrl+D exit: CC, Gem, oc (3). Gse, Gem, Qwn also exit with Ctrl+C on empty input.
- Ctrl+L clear/redraw: CC (redraw), Gem, Qwn (clear screen).
- Shift+Tab cycles a mode: CC, Gem, plus oc's agent cycle.
- Two verified: Ctrl+O expand (CC, Gem); double Esc (CC, Gem); Ctrl+T todo (CC, Gem); Ctrl+G editor (CC, Gem); Ctrl+X Ctrl+E (CC, Aid).

Disagreements: Ctrl+C meaning (interrupt / quit on empty / clear input); Ctrl+T (todo in CC
and Gem, variant cycle in oc, MCP tool descriptions in Qwn); Ctrl+P (history vs command
palette); queue key (Enter in CC, Tab in Gem).

## 3. Alt / Option

- CC: Alt+P (model), Alt+T (thinking), Alt+O, and word motion (Alt+B/F/D/Y) need Option-as-Meta on macOS. Option+Enter newline only with that setting. Alt+V pastes images on Windows/WSL.
- Gem: Alt is a synonym for word movement and newline. On macOS, Option+B/F/M act as Cmd+B/F/M without Meta setup.
- oc: alt+return newline, ctrl+alt+b/f message paging.
- Aid: Meta+Enter newline; Esc+Enter works in some terminals.
- Gse: newline is Ctrl+J; Alt+Enter not documented.
- Cursor, Codex: Alt+Delete / Alt+Enter only in third-party lists (unv).
- Pattern: no tool puts core actions (submit, cancel, mode cycle) on Alt. Alt is used for word motion and extra newline keys. Ctrl+J is the portable newline.

## 4. Config scoping

- CC: JSON `{"bindings":[{"context":"Chat","bindings":{"ctrl+e":"chat:externalEditor","ctrl+s":null}}]}`. Namespaced actions. About 30 contexts (Global, Chat, Autocomplete, Confirmation, HistorySearch, Transcript, Select, Settings, Pane, ...). Modifiers ctrl, shift, alt/opt/option/meta, cmd. Unbind with null. Chords space-separated with a 3 s timeout; a chord keeps its prefix reserved in its context. Reserved: Ctrl+C, Ctrl+D, Ctrl+M, Ctrl+[, Ctrl+I, Ctrl+H, Caps Lock. Hot-reloads. Warnings under --debug.
- oc: `keybinds` in `tui.json`. Strings or arrays; commas separate alternatives. `<leader>` + key with `leader_timeout` (2000 ms); leader = ctrl+x. Unbind with "none" or false. No chords beyond the leader.
- Gem: `~/.gemini/keybindings.json`, an array of `{command, key}`. Unbind a default with `"-command"`. Same key may serve different contexts. No documented chords.
- Aid: no keybinding file; `--vim` and `--multiline` only.
- Gse: no keybinding file; `GOOSE_CLI_NEWLINE_KEY` only.
- Qwn, Crs, Cop: no keybinding config found.
- Cdx: third-party (unv) says `config.toml [tui.keymap.global|composer|approval]` plus a `/keymap` picker.
- Cur, Amp: no official data retrieved.

## Sources

Verified:
- CC: https://code.claude.com/docs/en/keybindings, https://code.claude.com/docs/en/interactive-mode
- Gem: https://geminicli.com/docs/reference/keyboard-shortcuts/
- oc: https://opencode.ai/docs/keybinds/
- Aid: https://aider.chat/docs/usage/commands.html
- Gse: https://block.github.io/goose/docs/guides/goose-cli-commands
- Qwn: https://github.com/QwenLM/qwen-code/blob/main/docs/users/features/commands.md
- Cop: https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli (index level only)
- Crs: https://raw.githubusercontent.com/charmbracelet/crush/main/README.md (partial)

Unverified or unreachable:
- Cdx: https://learn.chatgpt.com/docs/codex/cli (no keys); third-party https://codex.danielvaughan.com/2026/05/06/codex-cli-configurable-tui-keymaps-custom-keyboard-shortcuts/
- Cur: third-party https://www.developersdigest.tech/reference/cursor-cli/keyboard-shortcuts
- Amp: https://ampcode.com/manual (sign-in only)

Not covered: Kilo, Cline CLI, Warp agent mode.
