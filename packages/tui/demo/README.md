# hoocode-tui demos

Runnable demos built **entirely from the library's real internals** — every
demo imports the actual components from `../src` (and shared themes from
`../test/test-themes.ts`). Nothing is copied, forked, or reimplemented; the
demos only compose the public API and mutate component state.

Each file is self-contained and exits on `Ctrl+C`.

| Demo | Feature showcased | Run |
| --- | --- | --- |
| `live-dashboard.ts` | Differential rendering & flicker-free updates (`Box`, `Text`, `Loader`, 10 fps in-place redraws) | `npm run demo:dashboard` |
| `command-palette.ts` | Overlays + `SelectList` + focus capture/restore (`tui.showOverlay`) | `npm run demo:palette` |
| `settings-menu.ts` | `SettingsList` — cycling values, fuzzy search, change events | `npm run demo:settings` |
| `markdown-viewer.ts` | `Markdown` rendering + live theme switching | `npm run demo:markdown` |

Or run any of them directly:

```bash
npx tsx packages/tui/demo/live-dashboard.ts
npx tsx packages/tui/demo/command-palette.ts   # Ctrl+K opens the palette
npx tsx packages/tui/demo/settings-menu.ts
npx tsx packages/tui/demo/markdown-viewer.ts    # press T to switch themes
```

> Tip: capture the raw ANSI stream with `PI_TUI_WRITE_LOG=/tmp/tui.log npx tsx demo/live-dashboard.ts`.
