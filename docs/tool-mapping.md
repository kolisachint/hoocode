# Tool Mapping: Legacy → Modern

Quick reference for tool alternatives. Hoocode auto-installs modern tools when available.

## File Operations

| Legacy | Modern | Speed | Hoocode Uses | Auto-Install |
|--------|--------|-------|--------------|--------------|
| `grep` | `rg` (ripgrep) | 10x faster | ✅ `grep` tool | ✅ |
| `find` | `fd` | 10x faster | ✅ `find` tool | ✅ |
| `ls` | `exa`/`eza` | 5x faster | ❌ (bash only) | ❌ |
| `cat` | `bat` | 5x faster | ❌ (read tool) | ❌ |
| `du` | `dust` | 5x faster | ❌ (bash only) | ❌ |
| `diff` | `delta` | better UX | ❌ (bash only) | ❌ |

## Binary File Operations

| Task | Fastest | Speed | Platform |
|------|---------|-------|----------|
| Text search | `rg -a pattern file` | fastest | all |
| ASCII extract | `strings file \| grep pattern` | fast | unix |
| Hex search | `xxd file \| rg pattern` | fast | all |
| Hex dump | `xxd file \| head -20` | instant | all |
| File info | `file file` | instant | unix |

### Platform Support

| Command | macOS | Linux | Windows (Git Bash) |
|---------|-------|-------|-------------------|
| `rg` | ✅ | ✅ | ✅ |
| `fd` | ✅ | ✅ | ✅ |
| `strings` | ✅ | ✅ | ✅ |
| `xxd` | ✅ | ✅ | ✅ |
| `hexdump` | ✅ | ✅ | ❌ |
| `file` | ✅ | ✅ | ❌ |

## Hoocode Built-in Tools

| Tool | Backend | Fallback | Speed |
|------|---------|----------|-------|
| `grep` | `rg` (ripgrep) | error | fastest |
| `find` | `fd` | error | fastest |
| `bash` | system shell | - | native |
| `read` | Node.js `readFile` | - | fast |
| `edit` | Node.js | - | fast |
| `write` | Node.js | - | fast |
| `ls` | Node.js `readdir` | - | fast |

## Settings

### Shell Configuration

```json
{
  "shellPath": "/opt/homebrew/bin/fish",
  "shellCommandPrefix": "shopt -s expand_aliases"
}
```

### Thinking Escalation

```json
{
  "thinking_escalation": {
    "enabled": true,
    "on_error": "high",
    "cooldown_turns": 1,
    "tools": ["bash", "mcp_*"]
  }
}
```

## Installation

### macOS (Homebrew)

```bash
brew install fd ripgrep
```

### Linux

```bash
# Debian/Ubuntu
sudo apt install fd-find ripgrep

# Arch
sudo pacman -S fd ripgrep
```

### Windows

```bash
# Install Git for Windows (provides bash, xxd, strings)
# https://git-scm.com/download/win

# Or use winget
winget install sharkdp.fd BurntSushi.ripgrep.MSVC
```

### Auto-Install

Hoocode auto-downloads to `~/.hoocode/bin/`:
- `fd` from [sharkdp/fd](https://github.com/sharkdp/fd)
- `rg` from [BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep)

Add to PATH:
```bash
export PATH="$HOME/.hoocode/bin:$PATH"
```

## Quick Reference

### Search in Code

```bash
# Fastest (ripgrep)
rg "pattern" src/

# With context
rg -C 2 "pattern" src/

# Case insensitive
rg -i "pattern" src/

# File type filter
rg -t ts "pattern" src/
```

### Search in Binaries

```bash
# Fastest (ripgrep with text mode)
rg -a "pattern" binary

# ASCII strings only
strings binary | grep "pattern"

# Hex pattern
xxd binary | rg "4142"  # search for "AB"
```

### Find Files

```bash
# Fastest (fd)
fd "pattern" src/

# By extension
fd -e ts "pattern" src/

# Include hidden
fd -H "pattern" src/

# Respect gitignore
fd --no-require-git "pattern" src/
```
