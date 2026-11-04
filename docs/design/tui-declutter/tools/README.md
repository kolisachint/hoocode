# TUI declutter mockup tools

Regenerate the mockups in `../mockups/` and the comparison page `../compare.html`.
Needs `python3` only (standard library). Run from anywhere:

    docs/design/tui-declutter/tools/build.sh      # or: python3 .../tools/build.py

The build stops at the first failing step: gens, then light and mono converts, then all checks, then render.

| Script | What it does |
|---|---|
| `gen_screens.py` | Writes `mockups/dark/screens/*.txt` (reads `tests/golden/hoocode-code-tui-app`). |
| `gen_dialogs.py` | Writes `mockups/dark/dialogs/*.txt`. |
| `gen_notices.py` | Reuses the helpers of `gen_screens.py`; writes `mockups/dark/notices/b_*.txt` (status-line option; option A is not kept). |
| `light_convert.py`, `light_palette.py` | Maps dark to the light palette: `mockups/light/**`. Applies the hand fix for the theme screen. |
| `mono_convert.py` | NO_COLOR variants of a subset: `mockups/mono/**`. |
| `check_*.py`, `light_check.py`, `mono_check.py` | Width, palette, contrast and layout checks for each set. |
| `render.py` | Builds `compare.html` (Before vs redesign with dark/light toggle, dialogs, notices, first run, no colour). Before columns come from `tests/golden/**`. `--png` also writes `compare.png` with headless Chrome (optional; set `CHROME_BIN` if Chrome is not at the macOS default path). |

Mockup text uses the golden inline format: `«fg=#rrggbb;bg=#rrggbb;bold»text«»`.
