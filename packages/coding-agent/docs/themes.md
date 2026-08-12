> hoocode can create themes. Ask it to build one for your setup.

# Themes

Themes are JSON files that define colors for the TUI.

## Table of Contents

- [Locations](#locations)
- [Built-in Themes](#built-in-themes)
- [Selecting a Theme](#selecting-a-theme)
- [Creating a Custom Theme](#creating-a-custom-theme)
- [Theme Format](#theme-format)
- [Color Tokens](#color-tokens)
- [Color Values](#color-values)
- [Tips](#tips)

## Locations

HooCode loads themes from:

- Built-in: every `*.json` in the shipped theme directory (see [Built-in Themes](#built-in-themes))
- Global: `~/.hoocode/agent/themes/*.json`
- Project: `.hoocode/themes/*.json`
- Packages: `themes/` directories or `pi.themes` entries in `package.json`
- Settings: `themes` array with files or directories
- CLI: `--theme <path>` (repeatable)

Disable discovery with `--no-themes`.

## Built-in Themes

| Theme | Background | Notes |
|-------|------------|-------|
| `dark` | dark | Default dark theme |
| `light` | light | Default light theme |
| `high-contrast-dark` | black | Maximum contrast, bright saturated hues |
| `high-contrast-light` | white | Maximum contrast, deep saturated hues |
| `warm-dark` | warm near-black | Low-glare amber/sand palette, no harsh blues |
| `warm-light` | warm paper | Dark ink on cream, less glare than pure white |
| `colorsafe-dark` | dark | Okabe-Ito hues, no red/green pairs |
| `colorsafe-light` | white | Okabe-Ito hues, no red/green pairs |
| `vox-dark` | newsprint black | Explanatory-journalism yellow; `warning` is orange |
| `vox-light` | cream | Ink on paper; yellow survives as highlight and brand chip |

The six accessible themes are `high-contrast-*`, `warm-*`, and `colorsafe-*`; the
`vox-*` pair is a style theme and is described separately below. The six are built
for low vision. Every color they
draw clears **WCAG AAA (7:1)** against every surface the TUI paints behind it —
the page, selected rows, user messages, and all three tool-box states — and none
of them defer to the terminal's default foreground, so contrast does not depend
on your terminal's own palette. `test/theme-contrast.test.ts` enforces this, so
a future edit cannot quietly wash one of them out.

Picking between them:

- **Contrast above all** → `high-contrast-dark` / `high-contrast-light`.
- **Bright screens hurt, or you read for long stretches** → `warm-dark` /
  `warm-light`. Same 7:1 floor, warmer and less glaring.
- **Red/green are hard to tell apart** → `colorsafe-dark` / `colorsafe-light`.
  Success and error are teal and orange, and no pair of tokens relies on a
  red-versus-green distinction.

Set the matching terminal background for the theme you pick (black-ish for the
dark themes, white-ish for the light ones) — hoocode colors the text, but the
canvas behind it belongs to your terminal.

### The `vox-*` pair

Explanatory-journalism styling: one loud yellow used sparingly, everything else
newsprint-quiet. The yellow is reserved for four roles — `accent`, `mdHeading`,
`selectedBg`, and `borderAccent` — and it never carries a status. That forces one
departure from the other themes: **`warning` is orange, not yellow**, because a
yellow `◐` or a yellow context gauge stops reading as a signal when yellow is
already the accent.

In `vox-light` the accent drops to a deep amber, since a yellow bright enough to
read as a highlight is not legible as text on cream. The brand yellow survives in
two places: the selected-row background, and the footer's brand chip (see
`brandBg`/`brandText` below).

These two are **AA, not AAA** — they are not covered by
`test/theme-contrast.test.ts`. Every text token clears 4.5:1 on every surface it
paints, rules and inactive chrome clear 2.8:1, no two tokens that mean different
things are closer than ΔE 11 (CIEDE2000), and the selection tint separates from
the page. If you need a guaranteed 7:1 floor, use one of the six accessible
themes.

Two deliberate departures from the standard the default `light` theme is held to:

- **Rules are neutral, not saturated.** `border`, `mdHr`, and the code/quote
  borders are warm grays rather than a saturated hue. A rule carries no meaning
  through color, and newsprint separators are the point of the theme. They still
  have to be *visible* — all of them clear the 2.8:1 decorative floor.
- **`brandText` is exempt from the surface sweep.** It only ever renders on
  `brandBg`, never on a page surface, so measuring it against the page is
  meaningless. On its own chip it sits at 14.8:1.

## Selecting a Theme

Select a theme via `/settings` or in `settings.json`:

```json
{
  "theme": "my-theme"
}
```

On first run, hoocode detects your terminal background and defaults to `dark` or `light`.

## Creating a Custom Theme

1. Create a theme file:

```bash
mkdir -p ~/.hoocode/agent/themes
vim ~/.hoocode/agent/themes/my-theme.json
```

2. Define the theme with all required colors (see [Color Tokens](#color-tokens)):

```json
{
  "$schema": "https://raw.githubusercontent.com/kolisachint/hoocode/main/packages/coding-agent/src/modes/interactive/theme/theme-schema.json",
  "name": "my-theme",
  "vars": {
    "primary": "#00aaff",
    "secondary": 242
  },
  "colors": {
    "accent": "primary",
    "border": "primary",
    "borderAccent": "#00ffff",
    "borderMuted": "secondary",
    "success": "#00ff00",
    "error": "#ff0000",
    "warning": "#ffff00",
    "muted": "secondary",
    "dim": 240,
    "text": "",
    "thinkingText": "secondary",
    "selectedBg": "#2d2d30",
    "userMessageBg": "#2d2d30",
    "userMessageText": "",
    "customMessageBg": "#2d2d30",
    "customMessageText": "",
    "customMessageLabel": "primary",
    "toolPendingBg": "#1e1e2e",
    "toolSuccessBg": "#1e2e1e",
    "toolErrorBg": "#2e1e1e",
    "toolTitle": "primary",
    "toolOutput": "",
    "mdHeading": "#ffaa00",
    "mdLink": "primary",
    "mdLinkUrl": "secondary",
    "mdCode": "#00ffff",
    "mdCodeBlock": "",
    "mdCodeBlockBorder": "secondary",
    "mdQuote": "secondary",
    "mdQuoteBorder": "secondary",
    "mdHr": "secondary",
    "mdListBullet": "#00ffff",
    "toolDiffAdded": "#00ff00",
    "toolDiffRemoved": "#ff0000",
    "toolDiffContext": "secondary",
    "syntaxComment": "secondary",
    "syntaxKeyword": "primary",
    "syntaxFunction": "#00aaff",
    "syntaxVariable": "#ffaa00",
    "syntaxString": "#00ff00",
    "syntaxNumber": "#ff00ff",
    "syntaxType": "#00aaff",
    "syntaxOperator": "primary",
    "syntaxPunctuation": "secondary",
    "thinkingOff": "secondary",
    "thinkingMinimal": "primary",
    "thinkingLow": "#00aaff",
    "thinkingMedium": "#00ffff",
    "thinkingHigh": "#ff00ff",
    "thinkingXhigh": "#ff0000",
    "bashMode": "#ffaa00"
  }
}
```

3. Select the theme via `/settings`.

**Hot reload:** When you edit the currently active custom theme file, hoocode reloads it automatically for immediate visual feedback.

## Theme Format

```json
{
  "$schema": "https://raw.githubusercontent.com/kolisachint/hoocode/main/packages/coding-agent/src/modes/interactive/theme/theme-schema.json",
  "name": "my-theme",
  "description": "One line shown next to the name in the theme picker",
  "vars": {
    "blue": "#0066cc",
    "gray": 242
  },
  "colors": {
    "accent": "blue",
    "muted": "gray",
    "text": "",
    ...
  }
}
```

- `name` is required and must be unique.
- `description` is optional. `/settings` shows it beside the theme name.
- `vars` is optional. Define reusable colors here, then reference them in `colors`.
- `colors` must define all 51 required tokens.

The `$schema` field enables editor auto-completion and validation.

## Color Tokens

Every theme must define all 51 required color tokens. A few further tokens are
optional — see [Optional Tokens](#optional-tokens) — and a theme that omits them
falls back to the behavior described there.

### Core UI (11 colors)

| Token | Purpose |
|-------|---------|
| `accent` | Primary accent (logo, selected items, cursor) |
| `border` | Normal borders |
| `borderAccent` | Highlighted borders |
| `borderMuted` | Subtle borders (editor) |
| `success` | Success states |
| `error` | Error states |
| `warning` | Warning states |
| `muted` | Secondary text |
| `dim` | Tertiary text |
| `text` | Default text (usually `""`) |
| `thinkingText` | Thinking block text |

### Backgrounds & Content (11 colors)

| Token | Purpose |
|-------|---------|
| `selectedBg` | Selected line background |
| `userMessageBg` | User message background |
| `userMessageText` | User message text |
| `customMessageBg` | Extension message background |
| `customMessageText` | Extension message text |
| `customMessageLabel` | Extension message label |
| `toolPendingBg` | Tool box (pending) |
| `toolSuccessBg` | Tool box (success) |
| `toolErrorBg` | Tool box (error) |
| `toolTitle` | Tool title |
| `toolOutput` | Tool output text |

### Markdown (10 colors)

| Token | Purpose |
|-------|---------|
| `mdHeading` | Headings |
| `mdLink` | Link text |
| `mdLinkUrl` | Link URL |
| `mdCode` | Inline code |
| `mdCodeBlock` | Code block content |
| `mdCodeBlockBorder` | Code block fences |
| `mdQuote` | Blockquote text |
| `mdQuoteBorder` | Blockquote border |
| `mdHr` | Horizontal rule |
| `mdListBullet` | List bullets |

### Tool Diffs (3 colors)

| Token | Purpose |
|-------|---------|
| `toolDiffAdded` | Added lines |
| `toolDiffRemoved` | Removed lines |
| `toolDiffContext` | Context lines |

### Syntax Highlighting (9 colors)

| Token | Purpose |
|-------|---------|
| `syntaxComment` | Comments |
| `syntaxKeyword` | Keywords |
| `syntaxFunction` | Function names |
| `syntaxVariable` | Variables |
| `syntaxString` | Strings |
| `syntaxNumber` | Numbers |
| `syntaxType` | Types |
| `syntaxOperator` | Operators |
| `syntaxPunctuation` | Punctuation |

### Thinking Level Borders (6 colors)

Editor border colors indicating thinking level (visual hierarchy from subtle to prominent):

| Token | Purpose |
|-------|---------|
| `thinkingOff` | Thinking off |
| `thinkingMinimal` | Minimal thinking |
| `thinkingLow` | Low thinking |
| `thinkingMedium` | Medium thinking |
| `thinkingHigh` | High thinking |
| `thinkingXhigh` | Extra high thinking |

### Bash Mode (1 color)

| Token | Purpose |
|-------|---------|
| `bashMode` | Editor border in bash mode (`!` prefix) |

### Optional Tokens

Omitting any of these is valid — each has a defined fallback, so older custom
themes keep working.

| Token | Purpose | If omitted |
|-------|---------|------------|
| `agent1`–`agent6` | Subagent identity palette; agent types hash into these slots | Falls back to `accent` |
| `mcp` | MCP server identity color | Falls back to `accent` |
| `brandBg` | Background of the footer brand chip | Brand mark renders as `accent`-colored text |
| `brandText` | Text color of the footer brand chip | Brand mark renders as `accent`-colored text |

`brandBg` and `brandText` are honored **only as a pair** — set both, or neither.
They exist for light themes, where a brand hue vivid enough to be recognizable is
usually illegible as text on a light canvas but reads well inside a filled chip.
`vox-light` is the only built-in that sets them.

```json
{
  "colors": {
    "brandBg": "#141209",
    "brandText": "#ffe600"
  }
}
```

### HTML Export (optional)

The `export` section controls colors for `/export` HTML output. If omitted, colors are derived from `userMessageBg`.

```json
{
  "export": {
    "pageBg": "#18181e",
    "cardBg": "#1e1e24",
    "infoBg": "#3c3728"
  }
}
```

## Color Values

Four formats are supported:

| Format | Example | Description |
|--------|---------|-------------|
| Hex | `"#ff0000"` | 6-digit hex RGB |
| 256-color | `39` | xterm 256-color palette index (0-255) |
| Variable | `"primary"` | Reference to a `vars` entry |
| Default | `""` | Terminal's default color |

### 256-Color Palette

- `0-15`: Basic ANSI colors (terminal-dependent)
- `16-231`: 6×6×6 RGB cube (`16 + 36×R + 6×G + B` where R,G,B are 0-5)
- `232-255`: Grayscale ramp

### Terminal Compatibility

HooCode uses 24-bit RGB colors. Most modern terminals support this (iTerm2, Kitty, WezTerm, Windows Terminal, VS Code). For older terminals with only 256-color support, hoocode falls back to the nearest approximation.

Check truecolor support:

```bash
echo $COLORTERM  # Should output "truecolor" or "24bit"
```

## Tips

**Dark terminals:** Use bright, saturated colors with higher contrast.

**Light terminals:** Use dark, saturated colors. Muting them costs contrast; if
you want a softer look, warm the background rather than lightening the text.

**Contrast:** Aim for 4.5:1 against your terminal background, or 7:1 if the
theme has to stay readable for low vision. Check every token against the
selected-row and tool-box backgrounds too, not just the page — those are where
themes usually lose contrast. `test/theme-contrast.test.ts` shows how the
built-in accessible themes are verified.

**Color harmony:** Start with a base palette (Nord, Gruvbox, Tokyo Night), define it in `vars`, and reference consistently.

**Testing:** Check your theme with different message types, tool states, markdown content, and long wrapped text.

**VS Code:** Set `terminal.integrated.minimumContrastRatio` to `1` for accurate colors.

## Examples

See the built-in themes:
- [dark.json](../src/modes/interactive/theme/dark.json)
- [light.json](../src/modes/interactive/theme/light.json)
- [high-contrast-dark.json](../src/modes/interactive/theme/high-contrast-dark.json)
- [high-contrast-light.json](../src/modes/interactive/theme/high-contrast-light.json)
- [warm-dark.json](../src/modes/interactive/theme/warm-dark.json)
- [warm-light.json](../src/modes/interactive/theme/warm-light.json)
- [colorsafe-dark.json](../src/modes/interactive/theme/colorsafe-dark.json)
- [colorsafe-light.json](../src/modes/interactive/theme/colorsafe-light.json)

Preview any of them, with per-token contrast ratios:

```bash
npx tsx test/test-theme-colors.ts theme warm-dark
```
