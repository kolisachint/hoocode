# Terminal Setup

HooCode uses the [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) for reliable modifier key detection. Most modern terminals support this protocol, but some require configuration.

## Kitty, iTerm2

Work out of the box.

## Alt keys

43 actions are reachable only with `Alt` — five of the six dials (`Alt+A` mode,
`Alt+M` model, `Alt+O` output, `Alt+L` task ledger, `Alt+C` color), settings
(`Alt+S`), the shortcut list (`Alt+K`), every picker verb, and the tree filters.
A terminal that speaks the Kitty keyboard protocol reports `Alt` correctly and
needs nothing here.

Interrupt (`Esc`), clear (`Ctrl+C`), exit (`Ctrl+D`), expand (`Ctrl+O`) and
submit (`Enter`) are deliberately never on `Alt`, so a misconfigured terminal is
recoverable. So is every dial that changes what the agent does: the thinking
level also steps on `Shift+Tab`, and `/mode`, `/model` and `/color` reach the
rest. Only the two that change what is drawn — the tool output dial and the task
ledger's lens — need `Alt`, and `Ctrl+O` still reaches full output and back.

### macOS

Outside the Kitty protocol, macOS treats `Option` as a compose key rather than
`Alt`. `Option+M` produces `µ`, `Option+S` produces `ß`, and `Option+E`,
`Option+U` and `Option+N` are dead keys that emit nothing at all. HooCode never
sees a key chord, so it types the character into the prompt instead — or, for
the dead keys, appears to ignore the press entirely.

Ghostty, Kitty, WezTerm and iTerm2 negotiate the Kitty protocol and are
unaffected. Terminal.app does not support it, so configure it explicitly:

| Terminal | Setting |
|---|---|
| Terminal.app | Settings → Profiles → Keyboard → **Use Option as Meta Key** |
| iTerm2 (protocol disabled) | Settings → Profiles → Keys → Left Option key → **Esc+** |
| Ghostty | `macos-option-as-alt = true` |
| Kitty | `macos_option_as_alt yes` |
| VS Code | `"terminal.integrated.macOptionIsMeta": true` |

### Linux

GNOME Terminal and Konsole can claim `Alt+<letter>` for menu mnemonics before
the application sees it — `Alt+F` for File, `Alt+E` for Edit, `Alt+S` for Search,
`Alt+T` for Terminal, `Alt+H` for Help. Where those overlap, hoocode gets
nothing.

In GNOME Terminal, turn off Preferences → General → **Enable mnemonics**. In
Konsole, Settings → Configure Keyboard Shortcuts.

### Rebinding instead

Any of these can be moved rather than configured around. See
[keybindings.md](keybindings.md); for example, in `~/.hoocode/keybindings.json`:

```json
{
  "app.hotkeys.open": "ctrl+q",
  "app.settings.open": ["alt+s", "f2"]
}
```

## Ghostty

Add to your Ghostty config (`~/Library/Application Support/com.mitchellh.ghostty/config` on macOS, `~/.config/ghostty/config` on Linux):

```
keybind = alt+backspace=text:\x1b\x7f
```

Older Claude Code versions may have added this Ghostty mapping:

```
keybind = shift+enter=text:\n
```

That mapping sends a raw linefeed byte. Inside hoocode, that is indistinguishable from `Ctrl+J`, so tmux and hoocode no longer see a real `shift+enter` key event.

If Claude Code 2.x or newer is the only reason you added that mapping, you can remove it, unless you want to use Claude Code in tmux, where it still requires that Ghostty mapping.

If you want `Shift+Enter` to keep working in tmux via that remap, add `ctrl+j` to your hoocode `newLine` keybinding in `~/.hoocode/keybindings.json`:

```json
{
  "newLine": ["shift+enter", "ctrl+j"]
}
```

## WezTerm

Create `~/.wezterm.lua`:

```lua
local wezterm = require 'wezterm'
local config = wezterm.config_builder()
config.enable_kitty_keyboard = true
return config
```

## VS Code (Integrated Terminal)

`keybindings.json` locations:
- macOS: `~/Library/Application Support/Code/User/keybindings.json`
- Linux: `~/.config/Code/User/keybindings.json`
- Windows: `%APPDATA%\\Code\\User\\keybindings.json`

Add to `keybindings.json` to enable `Shift+Enter` for multi-line input:

```json
{
  "key": "shift+enter",
  "command": "workbench.action.terminal.sendSequence",
  "args": { "text": "\u001b[13;2u" },
  "when": "terminalFocus"
}
```

## Windows Terminal

Add to `settings.json` (Ctrl+Shift+, or Settings → Open JSON file) to forward the modified Enter keys hoocode uses:

```json
{
  "actions": [
    {
      "command": { "action": "sendInput", "input": "\u001b[13;2u" },
      "keys": "shift+enter"
    },
    {
      "command": { "action": "sendInput", "input": "\u001b[13;3u" },
      "keys": "alt+enter"
    }
  ]
}
```

- `Shift+Enter` inserts a new line.
- Windows Terminal binds `Alt+Enter` to fullscreen by default. That prevents hoocode from receiving `Alt+Enter` for follow-up queueing.
- Remapping `Alt+Enter` to `sendInput` forwards the real key chord to hoocode instead.

If you already have an `actions` array, add the objects to it. If the old fullscreen behavior persists, fully close and reopen Windows Terminal.

## xfce4-terminal, terminator

These terminals have limited escape sequence support. Modified Enter keys like `Ctrl+Enter` and `Shift+Enter` cannot be distinguished from plain `Enter`, preventing custom keybindings such as `submit: ["ctrl+enter"]` from working.

For the best experience, use a terminal that supports the Kitty keyboard protocol:
- [Kitty](https://sw.kovidgoyal.net/kitty/)
- [Ghostty](https://ghostty.org/)
- [WezTerm](https://wezfurlong.org/wezterm/)
- [iTerm2](https://iterm2.com/)
- [Alacritty](https://github.com/alacritty/alacritty) (requires compilation with Kitty protocol support)

## IntelliJ IDEA (Integrated Terminal)

The built-in terminal has limited escape sequence support. Shift+Enter cannot be distinguished from Enter in IntelliJ's terminal.

If you want the hardware cursor visible, set `HOOCODE_HARDWARE_CURSOR=1` before running hoocode (disabled by default for compatibility).

Consider using a dedicated terminal emulator for the best experience.
