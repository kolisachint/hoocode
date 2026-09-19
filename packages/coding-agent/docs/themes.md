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
- Global: `~/.hoocode/themes/*.json`
- Project: `.hoocode/themes/*.json`
- Packages: `themes/` directories or `hoo.themes` entries in `package.json`
- Settings: `themes` array with files or directories
- CLI: `--theme <path>` (repeatable)

Disable discovery with `--no-themes`.

## Built-in Themes

Eight themes ship: four light, four dark, one pair per job.

| Theme | Background | Notes |
|-------|------------|-------|
| `dark` | dark | Default dark theme |
| `light` | light | Default light theme |
| `colorsafe-dark` | dark | Okabe-Ito hues, no red/green pairs; AAA |
| `colorsafe-light` | white | Okabe-Ito hues, no red/green pairs; AAA |
| `vox-cutout-dark` | Solarized Dark | Paper cut-outs on newsprint black: bold flat stocks, cut edges, AAA ink |
| `vox-cutout-light` | cream / Solarized Light | The same cut-outs on paper; AAA ink |
| `solarized-dark` | Solarized Dark | The canonical sixteen colors on `base03`; low, selective contrast |
| `solarized-light` | Solarized Light | The same sixteen on `base3`; the accents do not change |

Picking between them:

- **Just work** → `dark` / `light`. Every token that is read clears **WCAG AA
  (4.5:1)** on every surface the TUI paints behind it; rules and inactive chrome
  clear 2.8:1.
- **Low vision** → `colorsafe-dark` / `colorsafe-light`. Everything they draw
  clears **WCAG AAA (7:1)** on every surface, and nothing defers to the
  terminal's default foreground, so contrast does not depend on your terminal's
  own palette. Success and error are teal and orange, and no pair of tokens
  relies on a red-versus-green distinction — so they double as the colour-blind
  option.
- **Bold flat colour, still AAA** → `vox-cutout-dark` / `vox-cutout-light`.
- **Solarized, as designed** → `solarized-dark` / `solarized-light`.

`test/theme-contrast.test.ts` enforces each of those floors, theme by theme, so
a future edit cannot quietly wash one of them out.

Set the matching terminal background for the theme you pick (black-ish for the
dark themes, white-ish for the light ones) — hoocode colors the text, but the
canvas behind it belongs to your terminal.

### Retired themes

`high-contrast-dark`, `high-contrast-light`, `warm-dark`, `warm-light`,
`vox-dark` and `vox-light` no longer ship. A settings file that still names one
keeps working — the name loads as its nearest surviving relative rather than
dropping to the fallback theme, so a light terminal does not flip to a dark
theme on the next launch:

| Retired | Loads as |
|---------|----------|
| `high-contrast-dark`, `warm-dark` | `colorsafe-dark` |
| `high-contrast-light`, `warm-light` | `colorsafe-light` |
| `vox-dark` | `vox-cutout-dark` |
| `vox-light` | `vox-cutout-light` |

A theme of your own in `~/.hoocode/themes/` wins over the redirect, so keeping a
copy of a retired file under its old name brings it back exactly as it was.

### The `vox-cutout-*` pair

Explanatory-journalism styling cut for paper rather than newsprint: flat stocks
pasted onto the page, a scissor-cut rule around them, poster inks printed on top.
One loud yellow, used sparingly and never for a status — which forces one
departure from the other themes: **`warning` is orange, not yellow**, because a
yellow `◐` or a yellow context gauge stops reading as a signal when yellow is
already the accent. The pair is AAA, and `test/theme-contrast.test.ts` enforces
it.

`vox-cutout-light` is cut for a cream page (Solarized Light, or anything near
`#fdf6e3`); `vox-cutout-dark` is cut for Solarized Dark. Two things define them:

- **The surfaces are stock, not washes.** A background tinted a degree or two off
  white disappears on a cream terminal such as Solarized Light. Every surface
  here is a flat colored sheet held at least ΔE 10 clear of both Solarized
  grounds (`base3` `#fdf6e3` and `base2` `#eee8d5`), so a message block or tool
  box reads as a piece of paper laid on the page rather than a faint tint of it.
  The sheets carry the state: white for what you typed, blue for work in flight,
  mint for done, blush for failed, lilac for an extension, masking-tape tan for a
  warning notice, and flat vox yellow for the selected row.
- **The ink is capped by the loudest sheet.** Full-strength `#ffe600` under a
  selected row is the brightest surface any text has to survive, and AAA on it
  puts a hard ceiling on every foreground in the palette — each ink sits within a
  hair of that ceiling at ~7.2:1, as dark as a printed ink and no darker. That
  ceiling is why the accent is a deep gold rather than a bright amber: an amber
  light enough to read as yellow cannot clear 7:1 on its own highlight band. Bold
  flat color and AAA pull against each other, and this theme spends the whole
  budget on flat color.

Two deliberate departures from the standard the default `light` theme is held to:

- **Rules are neutral, not saturated.** `border`, `mdHr`, and the code/quote
  borders are warm grays rather than a saturated hue. A rule carries no meaning
  through color, and newsprint separators are the point of the theme. They still
  have to be *visible* — all of them clear the 2.8:1 decorative floor.
- **`brandText` is exempt from the surface sweep.** It only ever renders on
  `brandBg`, never on a page surface, so measuring it against the page is
  meaningless. On its own chip it sits at 14.8:1.

On the dark side the same rules run in reverse: AAA on a dark ground is a
*floor*, and Solarized's `base02` is the lighter of its two stops, so it is what
every ink has to clear. The stocks are dark coloured papers — navy in flight,
deep green done, maroon failed, aubergine for an extension, burnt umber for a
warning, olive-gold selected — and they all avoid the cyan the ground itself is
made of, because a stock in the ground's own hue cannot separate from it.

Point your terminal at the matching Solarized palette and the sheets land where
they were cut to land. On pure white or pure black the contrast floor still
holds; the sheets simply read a touch louder.

Both themes set the four cut-out token groups described under
[Optional Tokens](#optional-tokens).

### The `solarized-*` pair

Ethan Schoonover's Solarized, kept as it was designed rather than corrected. Two
things define it, and both are enforced by `test/theme-contrast.test.ts`.

**One palette, read from both ends.** Solarized is sixteen colors: eight
monotones on a symmetric CIELAB lightness ramp, and eight accents. The two
themes are the same ramp with the roles mirrored — `base03`↔`base3` for the
page, `base02`↔`base2` for the highlight, `base01`↔`base1` for comments and
tertiary chrome, `base00`↔`base0` for body text — and the eight accents are the
*same hex* on both grounds. Nothing is retuned per mode: `success` is `green`
`#859900` and `error` is `red` `#dc322f` whether you are on cream or on slate.
That is the palette's whole claim, so the pair is checked against each other
rather than only against their own surfaces.

**Low contrast, spent selectively.** Those tones sit well below maximum contrast
on purpose, and each mode has three content tones rather than a ramp of grays:
emphasized, body, comment. `text` and `toolTitle` take the emphasized tone (5.6:1
on `base03`, 5.0:1 on `base3`), `muted` and `toolOutput` the body tone (4.8:1 and
4.1:1), and `dim` — with quotes, rules and syntax comments — the comment tone, at
2.0-2.8:1, which is exactly where Solarized puts its comments. Holding this pair
to the 7:1 floor the accessible themes clear would mean shipping something that
is not Solarized, so what the tests enforce is the contrast Solarized actually
has, tier by tier:
**content inks 3.2:1, accents 2.3:1, the comment tone 1.9:1**, on every surface
either theme paints *and* on both Solarized terminal stops. If you want a
guaranteed floor rather than the original, use `colorsafe-dark` / `colorsafe-light`.

Two places where the TUI needs more than the palette ships:

- **A sheet per tool state.** Solarized has two background stops and the
  transcript needs one for each of pending, done, failed, extension and notice.
  Each is mixed from the accent that already means that thing and then pulled
  back to the highlight's own lightness, so a state reads through hue and not
  through getting louder — which is what keeps the ink on a tool box at the same
  contrast it has on the page. They are named in `vars` and are the only colors
  in either file that are not canonical.
- **A neutral for position.** `selectedBg` and the radar mark are neutrals on
  the `base03`→`base01` axis (`base3`→`base1` on the light side), one step off
  the page. Hue carries meaning in these themes; position does not get a hue.

`solarized-light` sets `brandBg`/`brandText` so the footer's brand mark reads as
a `base02` chip — a cyan bright enough to be the accent is not legible as text on
cream. Neither theme sets the cut-out tokens: paper shadows and headline chips
are the `vox-cutout-*` voice, and Solarized's is restraint.

Point your terminal at the matching Solarized background and the themes sit on
their own page; on pure white or pure black every floor above still holds.

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
mkdir -p ~/.hoocode/themes
vim ~/.hoocode/themes/my-theme.json
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
| `agent1`–`agent6` | Subagent and session identity palette; agent types and session colour slots hash into these | Falls back to `accent` |
| `mcp` | MCP server identity color | Falls back to `accent` |
| `brandBg` | Background of the footer brand chip | Brand mark renders as `accent`-colored text |
| `brandText` | Text color of the footer brand chip | Brand mark renders as `accent`-colored text |
| `warningBg` | Fill behind a warning notice (the billing caveat box) | Falls back to `customMessageBg` |
| `paperShadow` | Offset band under a filled message block | No shadow row is drawn |
| `halftone` | Unfilled remainder of a gauge or progress track | Falls back to `dim` |
| `headlineBg`/`headlineText` | Markdown heading chip | Headings stay `mdHeading`-coloured text |
| `tapeBg`/`tapeText` | Tape-strip message label | The label keeps `customMessageLabel` in brackets |

`warningBg` is a surface, not text: `warning` renders the title on it and `muted`
the body, so it wants a low-chroma tint of the theme's warning hue rather than the
warning color itself. Every built-in sets it, and the AAA themes keep every
foreground at AAA against it (`test/theme-contrast.test.ts` sweeps it with the
other surfaces).

`agent1`–`agent6` double as the session colour slots, which `/color` addresses by
name: slot 1 is `cyan`, then `purple`, `yellow`, `magenta`, `green`, `blue`. What
counts as a theme's cyan or its green is the theme's own business — a light
theme's slot 1 is deep enough to be ink, and an accessible theme may have no true
purple at all — but the *order* is not: put each hue in the slot whose name points
at it, or `/color purple` paints a red chip. `test/theme-contrast.test.ts` holds
the line by checking that no rearrangement of a theme's own six colours fits the
names better than the order it ships.

On a light theme these entries are ink — dark enough to read on paper — and a
session chip is not text but a filled block, so it is not painted with the token
directly: the fill keeps the token's hue and saturation and is lifted to a
lightness where that hue reads as itself. Set the token for the text role, at
whatever contrast the page needs, and pick its *hue* for the slot's name; the
chip takes care of itself. Dark palettes are already bright and are used as the
fill unchanged. The exception is magenta (slot 4): a deep rose still reads as
magenta where a dark yellow reads as brown, so a magenta fill is deepened —
never lifted — until white ink clears the chip bar, and one that already
carries white is used exactly as the theme wrote it.

### The cut-out tokens

Four groups that draw the paper-collage language rather than only colouring it.
Every one is optional with a defined fallback, so a theme that sets none of them
renders exactly as it did before they existed; `test/theme-cutout-tokens.test.ts`
exercises both sides of each. The `vox-cutout-*` pair is the only built-in that
sets them.

Optional means optional *per frame*, not per session. Every renderer that reads
one of these tokens asks whether the current theme defines it at the moment it
draws, never once when the component was built: the transcript on screen when
the user switches theme is full of blocks built under the old one, and a block
that had decided it was paper would go on asking a plain theme for
`paperShadow`, a token that theme never defined. `Box` takes the paper treatment
as a function it calls each render (`applyPaperSheet`), and `getMarkdownTheme`
checks for the headline pair inside its heading callbacks. So a switch in either
direction repaints what is already on screen — a cut-out theme gives the blocks
above the prompt their shadows, a plain one takes them away.

**`paperShadow`** draws an offset band under a filled message block — a user
message, an extension message, an error or warning notice — so the block reads
as a sheet laid on the page rather than a colour printed into it. A terminal has
no sub-pixel offsets and no blur, so the band is one extra row of `▔`: an upper
*one-eighth* block, a hairline along the top of its cells that hugs the block's
bottom edge, indented one column to give the offset. The block's right edge gets
the matching column of `▏` — a left one-eighth block, a hairline at the edge of
the cell nearest the sheet.

Eighths, not halves. `▀` and `▌` are half a cell of solid ink, which at a
terminal's resolution is not a shadow but a second band of colour wrapped around
two sides of every message.

The two legs meet as an L and neither one draws the corner. `▔` fills its cell
edge to edge, so a run of `bandWidth - 1` starting in column 1 already ends at
exactly the cell boundary the column paints its hairline on — the horizontal
leg's right end abuts the vertical leg's left edge, and the gutter cell on that
row stays empty. Putting a glyph in it can only overshoot: `▔` there runs seven
eighths of a cell past the corner, and `▏` there — which fills the *full height*
of its cell, against the run's top eighth — hangs a tick a whole row below the
shadow's bottom edge.

Setting it also gives the block the rest of the paper treatment, because the two
are one decision rather than two. A block that runs from margin to margin has no
right edge to show and nowhere to cast a shadow sideways — it is a band of colour
between two screen edges, not a sheet on a page. So a theme that sets
`paperShadow` also gets a one-column gutter of page at the block's right, which
is where that column of `▏` goes.

The edge is ruled, not cut. Nicking it a column on roughly every fifth row was
an earlier attempt at a hand-cut look, and a terminal cell is far too coarse a
step for it: the sheet read as damaged, and the shadow ink that backfilled the
gap put a tooth of shadow inside the sheet's own outline. Every row of a sheet
now ends in the same column.

It is a shape, not text, so it answers to the 2.8:1 decorative floor rather than
a text-contrast one — and it has to stay ΔE-clear of every `*Bg` token as well,
or the shadow sinks into the sheet it is hugging. On a dark theme nothing can be
darker than the page, so the band is set *lighter* than the ground: it reads as a
cut edge rather than a shadow, which is the closest a dark canvas gets.

**`halftone`** colours the unfilled remainder of a gauge or progress track. Those
tracks used `dim`, which is body-weight text: a track drawn in it reads as
writing rather than as the space the fill has yet to reach. The floor is
inverted from the usual one — the track wants to stay *below* the filled bar so
the fill keeps carrying the signal, and above roughly 2.8:1 so it is visible at
all — and it has to separate from `dim` by ΔE 11, or it has bought nothing.

**`headlineBg`/`headlineText`** render markdown headings as a filled chip instead
of coloured text. This is how a light theme keeps a vivid brand hue: inside a
chip a colour can be far brighter than it could ever be as text on the page.
`vox-cutout-light` is the case in point — its AAA ceiling pushes `accent` from
vox amber down to a deep gold, and the chip gets full-strength `#ffe600` back at
15:1.

**`tapeBg`/`tapeText`** render the `[branch]` / `[skill]` / `[extension]` tag that
heads a message block as a strip laid across it, rather than brackets set inside
it. Kept separate from `brandBg`/`brandText` so taping a label does not restyle
the footer's brand mark.

Both chip pairs are honored **only as a pair**, and both are exempt from the
surface sweep for the same reason `brandText` is: a chip ink never renders on a
page surface, so measuring it against one says nothing. Each is measured against
its own fill instead.

`brandBg` and `brandText` are honored **only as a pair** — set both, or neither.
They exist for light themes, where a brand hue vivid enough to be recognizable is
usually illegible as text on a light canvas but reads well inside a filled chip.
`solarized-light` and the `vox-cutout-*` pair set them.

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
- [colorsafe-dark.json](../src/modes/interactive/theme/colorsafe-dark.json)
- [colorsafe-light.json](../src/modes/interactive/theme/colorsafe-light.json)
- [vox-cutout-dark.json](../src/modes/interactive/theme/vox-cutout-dark.json)
- [vox-cutout-light.json](../src/modes/interactive/theme/vox-cutout-light.json)
- [solarized-dark.json](../src/modes/interactive/theme/solarized-dark.json)
- [solarized-light.json](../src/modes/interactive/theme/solarized-light.json)

Preview any of them, with per-token contrast ratios:

```bash
npx tsx test/test-theme-colors.ts theme colorsafe-dark
```
