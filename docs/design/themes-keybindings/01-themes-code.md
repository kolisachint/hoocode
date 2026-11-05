# Research 01: themes in hoocode today (2026-11-05)

Read-only code research. The decisions are in [../themes-keybindings.md](../themes-keybindings.md).

## Key finding

User themes already exist. Themes are JSON, loaded from `~/.hoocode/themes/*.json`, and
documented in `docs/user/configuration.md:15`. The gap is discoverability, inheritance,
validation and the picker, not the basic mechanism.

## 1. Where the theme code lives

- Crate `crates/hoocode-code-tui-theme/` (named in `docs/maps/packages.md:75, 175` and `docs/maps/ui.md:288`).
  - `registry.rs` (776 lines): loading, auto, watcher.
  - `schema.rs` (410): JSON schema and validation.
  - `theme.rs` (354): `Theme`, tokens to ANSI.
  - `color.rs` (479): `detect_color_mode_with` at :56-79 (`detect_color_mode` at :82), hex/256 math.
  - `chalk.rs`, `tui.rs` (414): hooks for select, markdown, editor.
- Built-ins: `themes/dark.json`, `themes/light.json`, `themes/theme-schema.json` in the crate, embedded with `include_str!` at `registry.rs:18-23`.
- Dir path: `hoocode-code-paths/src/lib.rs:113` `custom_themes_dir()` = `agent_dir()/themes`; `agent_dir()` at :78 is `~/.hoocode`.
- Picker: `small_selectors.rs:48-49` `picker_themes` (auto, dark, light); `:56` `theme_selector`. `ui.md:103-106` confirms custom themes are not offered in the UI.
- The `theme` setting: `settings_selector.rs:1579, :1874`; `hoocode-code-settings/src/manager.rs:523-528`. `/theme` is a pane under /settings, not a separate command.
- Init and fallback: `registry.rs:503-530` (`init_theme`). Unknown name silently becomes auto (:507-509). Load error silently becomes dark (:518-524). No warning.
- Hot reload: `WATCH_POLL` 50 ms at `registry.rs:597`, debounce 100 ms at :598. Only the active custom file is watched. Built-ins are not watched. No `notify` crate.
- Auto: `registry.rs:338-420`. `auto` and `auto:<light>:<dark>`. OS dark mode is read on macOS only (`defaults read`), otherwise `COLORFGBG`.
- `theme_paths` setting exists (`manager.rs:954-962`, `types.rs:179`). It is written by `config_selector.rs:475-479` but the loader never reads it. `get_available_themes` reads only `custom_themes_dir` and registered themes (`registry.rs:72-80`).

## 2. How colors are defined today

- Not a typed palette. A string-keyed token map: 68 fg tokens (`theme.rs` `THEME_COLORS`) and 19 bg tokens (`THEME_BGS`).
- Roles include `accent`, `error`, `success`, `dim`, `mdCode`, `toolDiffAdded`, `syntaxKeyword`, `thinkingHigh`, `bashMode`, `agent1-6`, and Mission Control tokens `ink/acc/ok/err/warn/pane/paneSel/chip`.
- Consumers call `theme().fg("token", text)`. A typo panics at runtime (`theme.rs:323`, `:330`). No compile-time check.
- Call-site counts (re-measured): 78 lines call `theme().fg(`/`theme().bg(` in one line; about 327 lines mention `theme()`. An earlier estimate of about 222 was not confirmed.
- Raw colors outside the theme are few: ANSI parsing of tool output (`tui-ratatui/src/ansi.rs`, `lines.rs`), a hex-to-Color helper (`widgets/src/pane.rs:98`), ratatui palette glue (`tui-ratatui/src/palette.rs`). About 26 `Color::Rgb/Indexed` lines outside tests, in about 9 files. A refactor to roles is small.

## 3. Config and file formats

- JSON everywhere: `settings.json`, `models.json`, `hoo-config.json` (written on first run, a seeding precedent), `auth.json`. Parsing via `serde_json`.
- No `toml` crate in `Cargo.lock`. No `notify`, `termbg` or `terminal-colorsaurus`.
- Versions in `Cargo.lock`: serde 1.0.229, serde_json 1.0.154, crossterm 0.29.0, ratatui-core 0.1.2.

## 4. Docs already written

- `docs/user/configuration.md:15` documents `~/.hoocode/themes/`, dark, light and auto.
- `docs/design/README.md` had no theme card. `tui-declutter.md:52` records dark/light only as the declutter decision.

## 5. Schema facts (`schema.rs`)

- Keys: `name`, `description`, `$schema`, `vars`, `colors`, `export`.
- 51 required color tokens (`REQUIRED_COLOR_TOKENS`, `schema.rs:12`), 86 declared color properties (`COLOR_PROPERTIES`, `schema.rs:77`).
- Color values: hex, `vars` reference, `""` (terminal default), 256 index.
- No `inherits`. A missing required token fails the whole file and dark is used silently.
- Not confirmed: whether unknown keys are rejected.

## 6. First proposal (input to the decisions)

- Keep JSON. Keep the token names. Optional `"ansi:<name>"` colors later.
- Add `"inherits"`. Builtins embedded and read-only; user files add new names.
- Picker lists custom themes. Bad file: warning, keep last good. Keep the 50 ms poll.
- Ship high-contrast and 2-3 more. Prefer original palettes. Well-known palettes (Solarized, Gruvbox, Nord, Catppuccin, Dracula) were not license-checked.
- Build order: picker (S), inherits (M), warnings (S), seed files (S), new themes (S each), token-literal test (S), `theme_paths` wire-or-remove (S/M).
