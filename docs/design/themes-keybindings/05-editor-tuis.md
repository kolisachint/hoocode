# Research 05: editor, shell and general TUI keybindings (2026-11-05)

Web research from official docs. Facts only; no code copied. "unverified" means the fetch
failed or the page did not cover the item. Not verified at all: Kakoune, Emacs (404),
Vim/Neovim (text not returned), htop, micro, tig, ranger, Midnight Commander, btop, screen, Zed.

## Sources

- Helix keymap: https://docs.helix-editor.com/keymap.html
- Yazi keymap: https://yazi-rs.github.io/docs/configuration/keymap/
- lazygit keybindings: https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings/Keybindings_en.md
- Readline manual: https://tiswww.case.edu/php/chet/readline/readline.html
- zsh zle: https://zsh.sourceforge.io/Doc/Release/Zsh-Line-Editor.html
- fish bind: https://fishshell.com/docs/current/cmds/bind.html
- prompt_toolkit keys: https://python-prompt-toolkit.readthedocs.io/en/master/pages/advanced_topics/key_bindings.html
- tmux: https://github.com/tmux/tmux/wiki/Getting-Started
- zellij: https://zellij.dev/documentation/keybindings.html
- Emacs keys: https://www.gnu.org/software/emacs/manual/html_node/emacs/Keys.html
- k9s: https://k9scli.io/topics/commands/
- fzf: https://github.com/junegunn/fzf/blob/master/README.md
- nano: https://www.nano-editor.org/dist/latest/cheatsheet.html
- kitty keyboard protocol: https://sw.kovidgoyal.net/kitty/keyboard-protocol/

## A. Line / prompt editing

| Action | readline (bash emacs mode) | Helix insert | others |
|---|---|---|---|
| start / end of line | C-a / C-e | none (normal mode g h / g l) | zsh, fish, prompt_toolkit C-a/C-e (unverified) |
| word left / right | M-b / M-f | — | unverified |
| delete word back / forward | M-DEL / M-d | Ctrl-w or Alt-Backspace / Alt-d or Alt-Delete | |
| kill to end / start | C-k / C-u | Ctrl-k / Ctrl-u | |
| yank | C-y | registers (Ctrl-r inserts), Space p in normal | |
| undo / redo | C-_ or C-x C-u / none | u / U in normal (unverified) | |
| history prev / next | C-p / C-n | — | arrows standard in fish |
| reverse search | C-r | — | fish Ctrl-r, zsh C-r (unverified) |
| external editor | C-x C-e in bash (unverified on page) | none | zsh C-x C-e (unverified) |
| cancel | C-g | Esc to normal | prompt_toolkit Ctrl-C |
| submit | Return (C-j sends LF) | — | prompt_toolkit enter = c-m |
| newline in multi-line | none | Ctrl-j or Enter | |

## B. Lists and panes

| Action | lazygit | k9s | fzf | Helix | yazi |
|---|---|---|---|---|---|
| up / down | j/k, arrows | unverified | Ctrl-k/j, Ctrl-p/n | — | arrow actions |
| page | PgUp/PgDn, K/J, Ctrl-u/d (main) | unverified | — | — | unverified |
| top / bottom | < or Home / > or End | unverified | — | g g / g e | top / bot actions |
| filter / search | / | / (regex, /! inverts) | typing filters | Space / (global) | search, filter actions |
| help | ? | ? | — | Space ? is command palette | help action |
| quit | q or Ctrl-c | Ctrl-c or :q | Ctrl-c, Ctrl-g, Esc | — | quit action |
| switch pane / tab | Tab, [ ], 0 | — | — | Ctrl-w h/j/k/l, Space w | tab actions |
| palette | ":" runs shell | ":" command mode | — | Space ? | unverified |

nano quits with Ctrl-X.

## C. Prefix and leader keys

- tmux: prefix C-b (twice sends a literal); key tables root, prefix, copy-mode, copy-mode-vi; `bind-key -T`, `-n` = root.
- screen: Ctrl-a (unverified).
- zellij: modes normal, locked (passes all keys), pane, tab, resize, move, scroll, search, session. Ctrl-g to and from locked, Ctrl-p pane mode, Alt-n new pane in normal. The status bar shows mode hints. Full default list: `zellij setup --dump-keybinds default` (not fetched). The "unlock-first" preset was not verified.
- Vim leader: backslash by default, Space common (unverified).
- Helix Space mode: f file picker, b buffers, s symbols, / global search, ? command palette, w window, c comment, y/p clipboard. The menu pops up after a pause.
- Emacs: C-x and C-c prefixes; C-x C-c quit, C-x C-s save (partly unverified).

## D. Alt / Meta

- Uses Alt: readline (M-b, M-f, M-d, M-DEL), Helix insert (Alt-d, Alt-Backspace), tmux (M- binds), fzf (Alt-c, Alt-r), nano (Alt menus), prompt_toolkit (Alt arrives as Esc prefix).
- macOS: Terminal and iTerm2 need "Use Option as Meta" (or Esc+), otherwise Option types special characters (unverified on a fetched page, widely known).
- zellij moved from Alt bindings toward Ctrl modes with a locked mode (reason not stated on the page).

## E. Config systems

- Helix: TOML `[keys.<mode>]` tables; `"no_op"` disables.
- Yazi: TOML `keymap.toml`; 8 layers (mgr, tasks, spot, pick, input, confirm, cmp, help); `prepend_keymap` / `append_keymap` / `keymap` (replace); rules `on` / `run` / `desc`, optional `for` (per OS); chords as arrays `on = ["g","d"]`; modifiers `<C-…>`, `<S-…>`, `<A-…>`, `<D-…>`; disable with `run = "noop"`.
- Readline: inputrc; `keyname: function` or `"keyseq": function`; `\C-`, `\M-`; `set keymap`; `$if mode=emacs`; `$include`.
- fish: `bind [-M mode] [-m newmode] [--preset|--user] KEYS COMMAND`; sequences `ctrl-x,ctrl-e`; `-e` erases; `fish_key_reader` shows key names.
- prompt_toolkit: chords as tuples; `ttimeoutlen` (lone Esc) and `timeoutlen` (chord wait).
- tmux: `set -g prefix`, `bind-key` / `unbind-key`, `list-keys`.
- zellij: KDL; `keybinds` grouped by mode; `clear-defaults=true` replaces the preset.
- fzf: `--bind key:action` pairs.
- lazygit (YAML), k9s, nano (nanorc bind/unbind), Emacs, Kakoune, Neovim: details unverified.

## F. Terminal constraints

- Ctrl-i = Tab, Ctrl-m = Enter, Ctrl-h = Backspace, Ctrl-[ = Esc (legacy bytes collide; prompt_toolkit aliases confirm).
- Ctrl-j = LF.
- Ctrl-s / Ctrl-q = XON/XOFF (disable with `stty -ixon`).
- Ctrl-z = suspend (standard job control).
- Ctrl-space = NUL (`c-@`).
- Kitty keyboard protocol flags: 1 disambiguate (ctrl-i/m/h/[ become CSI u), 2 event types, 4 alternate keys, 8 all keys as escapes, 16 associated text. Push/pop stack, separate for main and alternate screens. Supported by kitty, foot, WezTerm, Ghostty, iTerm2, Alacritty and others. Used by Vim, Neovim, Emacs, Helix, fish. Detect by querying flags then sending a primary device attributes request.

## Cross-tool conventions (4+ tools)

- Esc leaves a mode or cancels: Helix, Kakoune, zellij, k9s, lazygit, fzf, nano.
- j/k or arrows in lists: lazygit, Vim family, yazi, k9s (unverified).
- "/" filter or search: lazygit, k9s, Helix, Vim, yazi.
- "?" help: lazygit, k9s.
- "q" quits: lazygit, k9s (:q), yazi, htop (unverified).
- Ctrl-c quits or cancels: lazygit, k9s, fzf, prompt_toolkit.
- Ctrl-a / Ctrl-e line start/end, Ctrl-k / Ctrl-u kill, Ctrl-r reverse search: readline, shells, Helix (kill).
- Enter accepts: fzf, readline, prompt_toolkit, Helix.
- Ctrl-n / Ctrl-p next/prev: fzf, readline history, Emacs.
- Ctrl-g cancel: readline, Emacs (Helix uses Esc).

## Lessons for a coding-agent TUI (no Alt defaults, configurable leader)

- Avoid Alt as a default: Option-as-Meta is off by default on macOS, and Alt arrives as Esc-prefixed bytes (ambiguous and slow to detect).
- One explicit leader the user can change. In a text box a printable leader (Space) must type, so the leader must be a Ctrl key, or active only outside the prompt (Helix uses Space only in normal mode).
- Avoid Ctrl-i, Ctrl-m, Ctrl-h, Ctrl-[, Ctrl-s, Ctrl-q, Ctrl-z as defaults. Use the kitty protocol with a fallback.
- Follow the conventions: j/k and arrows, / filter, ? help, q or Esc close, Enter accept, Ctrl-c cancel, readline line editing, Ctrl-r search, Ctrl-x Ctrl-e editor.
- Config: per-context tables, a value that disables a binding, chords as sequences, a which-key popup after a short delay with a configurable timeout.
- Offer a key-reader command (like `fish_key_reader`).
- Document terminal constraints in user docs.
