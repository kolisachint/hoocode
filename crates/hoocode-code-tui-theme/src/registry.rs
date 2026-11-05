//! Theme loading and the process-wide current theme: built-ins (embedded),
//! the user's `themes/` directory, extension-registered themes, retired
//! names, the custom-theme file watcher, and the export-color helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crate::color::{ansi256_to_hex, detect_color_mode, relative_luminance, ColorMode, RawColor};
use serde_json::{Map, Value};

use crate::schema::{
    parse_theme_json, parse_theme_json_content, theme_json_value, ColorValue, ThemeJson,
};
use crate::theme::{Theme, ThemeOptions, AGENT_COLOR_TOKENS, PANE_BG_TOKENS, THEME_BGS};

/// The bundled theme files, by file name (the `readdirSync().sort()` order).
pub(crate) const BUILTIN_THEME_FILES: [(&str, &str); 2] = [
    ("dark.json", include_str!("../themes/dark.json")),
    ("light.json", include_str!("../themes/light.json")),
];

/// The bundled JSON schema for theme files (`theme-schema.json`).
pub const THEME_SCHEMA_JSON: &str = include_str!("../themes/theme-schema.json");

/// The bundled theme file text for `name`, if it is a built-in.
pub fn builtin_theme_source(name: &str) -> Option<&'static str> {
    BUILTIN_THEME_FILES
        .iter()
        .find(|(file, _)| file.strip_suffix(".json") == Some(name))
        .map(|(_, content)| *content)
}

fn builtin_themes() -> &'static BTreeMap<String, ThemeJson> {
    static BUILTINS: OnceLock<BTreeMap<String, ThemeJson>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        BUILTIN_THEME_FILES
            .iter()
            .map(|(file, content)| {
                let name = file.trim_end_matches(".json").to_string();
                let json = parse_theme_json_content(file, content)
                    .unwrap_or_else(|e| panic!("bundled theme {file} is invalid: {e}"));
                (name, json)
            })
            .collect()
    })
}

/// True for themes shipped with the package.
fn is_builtin_theme(name: &str) -> bool {
    builtin_themes().contains_key(name)
}

fn custom_themes_dir() -> PathBuf {
    hoocode_code_paths::custom_themes_dir()
}

fn json_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| f.ends_with(".json"))
        .collect();
    names.sort();
    names
}

/// `getAvailableThemes`: built-ins, custom files and registered themes,
/// sorted.
pub fn get_available_themes() -> Vec<String> {
    let mut themes: BTreeSet<String> = builtin_themes().keys().cloned().collect();
    for file in json_files(&custom_themes_dir()) {
        themes.insert(file.trim_end_matches(".json").to_string());
    }
    for (name, _) in &state().registered {
        themes.insert(name.clone());
    }
    themes.into_iter().collect()
}

/// `getThemeDescription`: the one-line description from a theme file.
pub fn get_theme_description(name: &str) -> Option<String> {
    load_theme_json(name).ok()?.description
}

/// `ThemeInfo`. Built-ins are embedded, so their `path` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeInfo {
    pub name: String,
    pub path: Option<PathBuf>,
}

/// `getAvailableThemesWithPaths`.
pub fn get_available_themes_with_paths() -> Vec<ThemeInfo> {
    let mut result: Vec<ThemeInfo> = builtin_themes()
        .keys()
        .map(|name| ThemeInfo {
            name: name.clone(),
            path: None,
        })
        .collect();
    let dir = custom_themes_dir();
    for file in json_files(&dir) {
        let name = file.trim_end_matches(".json").to_string();
        if !result.iter().any(|t| t.name == name) {
            result.push(ThemeInfo {
                name,
                path: Some(dir.join(&file)),
            });
        }
    }
    for (name, theme) in &state().registered {
        if !result.iter().any(|t| &t.name == name) {
            result.push(ThemeInfo {
                name: name.clone(),
                path: theme.source_path.clone(),
            });
        }
    }
    // `localeCompare`: case-insensitive first.
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    result
}

/// The theme picker's list, read from the user's themes folder: `auto`,
/// the built-in themes, then every user theme in `dir` that loads, by name.
/// Only the folder's top level counts, so `defaults/` is never listed. A
/// user file that reuses a built-in or `auto` name is left out (built-ins
/// win), and so is a file that does not load (its warning comes from the
/// theme diagnostics).
pub fn picker_theme_list_in(dir: &Path) -> Vec<ThemeInfo> {
    let mut list = vec![ThemeInfo {
        name: "auto".to_string(),
        path: None,
    }];
    list.extend(builtin_themes().keys().map(|name| ThemeInfo {
        name: name.clone(),
        path: None,
    }));
    let mut user: Vec<ThemeInfo> = Vec::new();
    for file in json_files(dir) {
        let name = file.trim_end_matches(".json").to_string();
        if is_builtin_theme(&name) || is_auto_theme(&name) {
            continue;
        }
        let path = dir.join(&file);
        if load_theme_from_path(&path, None).is_ok() {
            user.push(ThemeInfo {
                name,
                path: Some(path),
            });
        }
    }
    user.sort_by(|a, b| a.name.cmp(&b.name));
    list.extend(user);
    list
}

/// [`picker_theme_list_in`] for the user's own themes folder.
pub fn picker_theme_list() -> Vec<ThemeInfo> {
    picker_theme_list_in(&custom_themes_dir())
}

/// The file of user theme `name` in the themes folder, when there is one.
/// Built-ins, `auto` and names without a file give `None`.
pub fn user_theme_path(name: &str) -> Option<PathBuf> {
    if is_builtin_theme(name) || is_auto_theme(name) {
        return None;
    }
    let path = custom_themes_dir().join(format!("{name}.json"));
    path.is_file().then_some(path)
}

fn read_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!(
            "ENOENT: no such file or directory, open '{}'",
            path.display()
        ),
        _ => e.to_string(),
    })
}

/// `loadThemeJson`: a theme's JSON by name.
pub fn load_theme_json(name: &str) -> Result<ThemeJson, String> {
    if let Some(json) = builtin_themes().get(name) {
        return Ok(json.clone());
    }
    let registered = registered_theme(name);
    if let Some(theme) = &registered {
        return match &theme.source_path {
            Some(path) => {
                let content = read_file(path)?;
                let (stem, dir) = theme_file_identity(path);
                parse_user_theme(&stem, &path.display().to_string(), &content, &dir)
            }
            None => Err(format!(
                "Theme \"{name}\" does not have a source path for export"
            )),
        };
    }
    let dir = custom_themes_dir();
    let theme_path = dir.join(format!("{name}.json"));
    if !theme_path.exists() {
        return Err(format!("Theme not found: {name}"));
    }
    let content = read_file(&theme_path)?;
    parse_user_theme(name, name, &content, &dir)
}

/// A theme file's name without `.json` (the name other themes inherit it by)
/// and the folder it sits in (where its base theme is looked up).
fn theme_file_identity(path: &Path) -> (String, PathBuf) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    (stem, dir)
}

/// Parses a user theme file: `inherits` is resolved against `dir` (the
/// folder the file is in), then the merged theme is validated as a whole.
fn parse_user_theme(
    name: &str,
    label: &str,
    content: &str,
    dir: &Path,
) -> Result<ThemeJson, String> {
    let raw = theme_json_value(label, content)?;
    let merged = resolve_inherits(label, raw, dir, vec![name.to_string()])?;
    parse_theme_json(label, &merged)
}

/// Resolves `inherits` for the theme at the end of `chain`, the names being
/// resolved outermost first. Each level is checked on its own first. A base
/// is a built-in theme or a file in `dir`, and its own base resolves the same
/// way. A name already in the chain is a cycle, so this always ends.
fn resolve_inherits(
    label: &str,
    raw: Value,
    dir: &Path,
    chain: Vec<String>,
) -> Result<Value, String> {
    // Required tokens wait for the merged theme (see `validate`).
    parse_theme_json(label, &raw)?;
    let Some(base_name) = raw.get("inherits").and_then(Value::as_str) else {
        return Ok(raw);
    };
    let base_name = base_name.to_string();
    if chain.contains(&base_name) {
        let mut cycle = chain;
        cycle.push(base_name);
        return Err(format!("Theme inheritance cycle: {}", cycle.join(" -> ")));
    }
    let child = chain.last().cloned().unwrap_or_default();
    let base_raw = base_theme_value(&base_name, &child, dir)?;
    let mut next = chain;
    next.push(base_name.clone());
    let base = resolve_inherits(&base_name, base_raw, dir, next)?;
    Ok(merge_over_base(raw, &base))
}

/// The JSON of base theme `base`, which theme `child` inherits: a built-in,
/// or a file in `dir`. Anything else is an error.
fn base_theme_value(base: &str, child: &str, dir: &Path) -> Result<Value, String> {
    if let Some(text) = builtin_theme_source(base) {
        return theme_json_value(base, text);
    }
    let path = dir.join(format!("{base}.json"));
    if !path.is_file() {
        return Err(format!(
            "Theme \"{child}\" inherits \"{base}\", which is neither a built-in theme nor a file in the themes folder"
        ));
    }
    let content = read_file(&path)?;
    theme_json_value(base, &content)
}

/// `raw` over `base`: the theme's own `colors` and `vars` entries win over
/// the base's, its `export` wins when it has one, and `inherits` is dropped.
/// The name and description stay the theme's own.
fn merge_over_base(mut raw: Value, base: &Value) -> Value {
    if let (Value::Object(own), Value::Object(base)) = (&mut raw, base) {
        own.remove("inherits");
        for section in ["colors", "vars"] {
            if !base.contains_key(section) && !own.contains_key(section) {
                continue;
            }
            let mut merged: Map<String, Value> = match base.get(section) {
                Some(Value::Object(entries)) => entries.clone(),
                _ => Map::new(),
            };
            if let Some(Value::Object(mine)) = own.get(section) {
                merged.extend(mine.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            own.insert(section.to_string(), Value::Object(merged));
        }
        if !own.contains_key("export") {
            if let Some(export) = base.get("export") {
                own.insert("export".to_string(), export.clone());
            }
        }
    }
    raw
}

/// `resolveVarRefs`.
pub fn resolve_var_refs(
    value: &ColorValue,
    vars: &[(String, ColorValue)],
) -> Result<RawColor, String> {
    let mut visited: Vec<String> = Vec::new();
    let mut current = value.clone();
    loop {
        let name = match &current {
            RawColor::Index(_) => return Ok(current),
            RawColor::Str(s) if s.is_empty() || s.starts_with('#') => return Ok(current),
            RawColor::Str(s) => s.clone(),
        };
        if visited.contains(&name) {
            return Err(format!("Circular variable reference detected: {name}"));
        }
        let Some((_, next)) = vars.iter().find(|(k, _)| *k == name) else {
            return Err(format!("Variable reference not found: {name}"));
        };
        visited.push(name);
        current = next.clone();
    }
}

/// `resolveThemeColors`: every color with its `vars` references resolved.
pub fn resolve_theme_colors(
    colors: &[(String, ColorValue)],
    vars: Option<&[(String, ColorValue)]>,
) -> Result<Vec<(String, RawColor)>, String> {
    let vars = vars.unwrap_or(&[]);
    colors
        .iter()
        .map(|(key, value)| Ok((key.clone(), resolve_var_refs(value, vars)?)))
        .collect()
}

/// `createTheme`.
pub fn create_theme(
    theme_json: &ThemeJson,
    mode: Option<ColorMode>,
    source_path: Option<PathBuf>,
) -> Result<Theme, String> {
    let color_mode = mode.unwrap_or_else(detect_color_mode);
    let resolved = resolve_theme_colors(&theme_json.colors, theme_json.vars.as_deref())?;
    let mut fg: Vec<(String, RawColor)> = Vec::new();
    let mut bg: Vec<(String, RawColor)> = Vec::new();
    for (key, value) in resolved {
        if THEME_BGS.contains(&key.as_str()) {
            bg.push((key, value));
        } else {
            fg.push((key, value));
        }
    }
    // The agent palette is optional; missing entries fall back to accent.
    let accent = fg
        .iter()
        .find(|(k, _)| k == "accent")
        .map(|(_, v)| v.clone());
    for token in AGENT_COLOR_TOKENS {
        if !fg.iter().any(|(k, _)| k == token) {
            if let Some(accent) = &accent {
                fg.push((token.to_string(), accent.clone()));
            }
        }
    }
    // The MCP tag color is optional too (the task panel reads it on every MCP row).
    if let Some(accent) = &accent {
        if !fg.iter().any(|(k, _)| k == "mcp") {
            fg.push(("mcp".to_string(), accent.clone()));
        }
    }
    // Chip text falls back to the theme's own text colour.
    if !fg.iter().any(|(k, _)| k == "chipFg") {
        if let Some((_, v)) = fg.iter().find(|(k, _)| k == "text").cloned() {
            fg.push(("chipFg".to_string(), v));
        }
    }
    // Custom themes predate the warning surface: fall back to customMessageBg.
    if !bg.iter().any(|(k, _)| k == "warningBg") {
        if let Some((_, v)) = bg.iter().find(|(k, _)| k == "customMessageBg").cloned() {
            bg.push(("warningBg".to_string(), v));
        }
    }
    // Pane surfaces predate the declutter: they fall back to selectedBg.
    for token in PANE_BG_TOKENS {
        if !bg.iter().any(|(k, _)| k == token) {
            if let Some((_, v)) = bg.iter().find(|(k, _)| k == "selectedBg").cloned() {
                bg.push((token.to_string(), v));
            }
        }
    }
    // The tool peek band falls back to the pending tint when a theme omits it.
    if !bg.iter().any(|(k, _)| k == "toolBandBg") {
        if let Some((_, v)) = bg.iter().find(|(k, _)| k == "toolPendingBg").cloned() {
            bg.push(("toolBandBg".to_string(), v));
        }
    }
    Theme::new(
        fg,
        bg,
        color_mode,
        ThemeOptions {
            name: Some(theme_json.name.clone()),
            source_path,
        },
    )
}

/// `loadThemeFromPath`.
pub fn load_theme_from_path(theme_path: &Path, mode: Option<ColorMode>) -> Result<Theme, String> {
    let content = read_file(theme_path)?;
    let (stem, dir) = theme_file_identity(theme_path);
    let theme_json = parse_user_theme(&stem, &theme_path.display().to_string(), &content, &dir)?;
    create_theme(&theme_json, mode, Some(theme_path.to_path_buf()))
}

fn load_theme(name: &str, mode: Option<ColorMode>) -> Result<Arc<Theme>, String> {
    if let Some(theme) = registered_theme(name) {
        return Ok(theme);
    }
    let json = load_theme_json(name)?;
    Ok(Arc::new(create_theme(&json, mode, None)?))
}

/// `getThemeByName`.
pub fn get_theme_by_name(name: &str) -> Option<Arc<Theme>> {
    load_theme(name, None).ok()
}

/// `detectTerminalBackground` from `COLORFGBG`.
pub fn detect_terminal_background_with(colorfgbg: Option<&str>) -> &'static str {
    if let Some(value) = colorfgbg.filter(|v| !v.is_empty()) {
        let parts: Vec<&str> = value.split(';').collect();
        if parts.len() >= 2 {
            let digits: String = {
                let s = parts[1].trim_start();
                let (sign, rest) = match s.strip_prefix('-') {
                    Some(r) => ("-", r),
                    None => ("", s.strip_prefix('+').unwrap_or(s)),
                };
                let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if d.is_empty() {
                    String::new()
                } else {
                    format!("{sign}{d}")
                }
            };
            if let Ok(bg) = digits.parse::<i64>() {
                return if bg < 8 { "dark" } else { "light" };
            }
        }
    }
    "dark"
}

fn default_theme() -> &'static str {
    detect_terminal_background_with(std::env::var("COLORFGBG").ok().as_deref())
}

/// The concrete themes `"auto"` follows: Light maps to `light`, Dark maps
/// to `dark` (both built in, so the pair always loads).
pub const AUTO_LIGHT_THEME: &str = "light";
pub const AUTO_DARK_THEME: &str = "dark";

/// How often the interactive mode re-checks the OS appearance while the
/// settings theme is an auto value.
pub const AUTO_THEME_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// True when the settings value asks the theme to follow the OS appearance:
/// `"auto"`, or `"auto:<light>:<dark>"` for a custom pair (the day side
/// first). The raw value stays in settings; only the resolved concrete
/// name reaches the loader.
pub fn is_auto_theme(value: &str) -> bool {
    value == "auto" || value.starts_with("auto:")
}

/// The concrete pair an auto value stands for, as `(light, dark)`. Blank
/// sides fall back to the built-in pair, so `"auto"` means
/// `(light, dark)`.
fn auto_pair(value: &str) -> (String, String) {
    let mut parts = value.splitn(3, ':');
    parts.next();
    let light = parts.next().unwrap_or("").trim();
    let dark = parts.next().unwrap_or("").trim();
    (
        if light.is_empty() {
            AUTO_LIGHT_THEME.to_string()
        } else {
            light.to_string()
        },
        if dark.is_empty() {
            AUTO_DARK_THEME.to_string()
        } else {
            dark.to_string()
        },
    )
}

/// The OS appearance, as `Some(dark)`: `None` when it cannot be read.
/// On macOS `defaults read -g AppleInterfaceStyle` prints `Dark` only in
/// dark mode (Light removes the key, so any other outcome reads as light);
/// anywhere else there is nothing to read. A failure to even run `defaults`
/// is `None` too, and the caller falls back to the `COLORFGBG` heuristic.
#[cfg(target_os = "macos")]
fn os_dark_mode() -> Option<bool> {
    let output = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()?;
    Some(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .trim()
                .eq_ignore_ascii_case("dark"),
    )
}

/// No OS-appearance API outside macOS: the caller falls back to the
/// `COLORFGBG` heuristic.
#[cfg(not(target_os = "macos"))]
fn os_dark_mode() -> Option<bool> {
    None
}

/// `resolveAutoTheme`: the concrete theme an auto settings value means right
/// now. Non-auto values pass through unchanged. The OS reading decides the
/// side; when it cannot be read the `COLORFGBG` heuristic does (which itself
/// falls back to dark), so a total failure resolves to the dark side. An
/// unknown custom side falls back to its built-in default and records a
/// theme warning (see [`theme_warnings`]), which clears once the side loads.
pub fn resolve_auto_theme(value: &str) -> String {
    if !is_auto_theme(value) {
        return value.to_string();
    }
    let (light, dark) = auto_pair(value);
    let dark_mode = os_dark_mode().unwrap_or_else(|| default_theme() == "dark");
    let (side, fallback) = if dark_mode {
        (dark, AUTO_DARK_THEME)
    } else {
        (light, AUTO_LIGHT_THEME)
    };
    let name = side;
    if get_available_themes().iter().any(|t| t == &name) {
        clear_theme_warning(&name);
        name
    } else {
        record_theme_warning(
            &name,
            "Theme not found",
            "Using the built-in theme for that side.",
        );
        fallback.to_string()
    }
}

// ============================================================================
// Global theme
// ============================================================================

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    theme: Option<Arc<Theme>>,
    current_name: Option<String>,
    registered: Vec<(String, Arc<Theme>)>,
    on_change: Option<ChangeCallback>,
    watcher: Option<Watcher>,
    /// Problems with themes the user asked for, by the name they asked for.
    warnings: BTreeMap<String, String>,
    /// Problems that are not one theme's load failure (see [`record_notice`]).
    notices: BTreeMap<String, String>,
}

/// Counts new theme warnings, so the screen can show each one once.
static WARNING_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The theme warnings to show: a bad file, or a name that is not a theme.
/// Each says what the screen does instead (keeps the last good theme, or
/// uses the built-in dark for now). A warning clears when its name loads.
pub fn theme_warnings() -> Vec<String> {
    let s = state();
    s.warnings
        .values()
        .chain(s.notices.values())
        .cloned()
        .collect()
}

/// Records a problem that is not a theme's failure to load, such as the
/// defaults folder that could not be written. It is listed with the theme
/// warnings until [`clear_notice`] removes it.
pub(crate) fn record_notice(key: &str, message: String) {
    {
        let mut s = state();
        if s.notices.get(key) == Some(&message) {
            return;
        }
        s.notices.insert(key.to_string(), message);
    }
    WARNING_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Removes the notice `key`, once the problem is gone.
pub(crate) fn clear_notice(key: &str) {
    state().notices.remove(key);
}

/// Bumps with every new theme warning (a repeat of the same one does not).
pub fn theme_warning_generation() -> u64 {
    WARNING_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Records that theme `name` did not load. The error text is put on one line.
fn record_theme_warning(name: &str, error: &str, consequence: &str) {
    let error = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let message = format!("Theme \"{name}\" did not load: {error}. {consequence}");
    {
        let mut s = state();
        if s.warnings.get(name) == Some(&message) {
            return;
        }
        s.warnings.insert(name.to_string(), message);
    }
    WARNING_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Clears the warning for theme `name`, once it loads.
fn clear_theme_warning(name: &str) {
    state().warnings.remove(name);
}

/// The running custom-theme watcher: told to stop by `stop`, and joined when
/// it is stopped, so no reload or change callback runs after
/// [`stop_theme_watcher`] returns.
struct Watcher {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn registered_theme(name: &str) -> Option<Arc<Theme>> {
    state()
        .registered
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t.clone())
}

/// The current theme (`theme` in hoocode). Panics before [`init_theme`].
pub fn theme() -> Arc<Theme> {
    try_theme().expect("Theme not initialized. Call initTheme() first.")
}

/// The current theme, if one has been set.
pub fn try_theme() -> Option<Arc<Theme>> {
    state().theme.clone()
}

/// The name of the current theme (`"<in-memory>"` after
/// [`set_theme_instance`]).
pub fn current_theme_name() -> Option<String> {
    state().current_name.clone()
}

/// Counts theme changes: every [`init_theme`], [`set_theme`],
/// [`unset_theme`] and watcher reload bumps it (see [`notify_change`]).
/// Render caches key on `(current_theme_name(), theme_generation())`, so
/// a reload under the same name still invalidates them.
static THEME_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The theme generation: higher means a newer theme.
pub fn theme_generation() -> u64 {
    THEME_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// `setRegisteredThemes`: themes registered by extensions, by name.
pub fn set_registered_themes(themes: Vec<Theme>) {
    let mut s = state();
    s.registered.clear();
    for theme in themes {
        if let Some(name) = theme.name.clone() {
            let theme = Arc::new(theme);
            match s.registered.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = theme,
                None => s.registered.push((name, theme)),
            }
        }
    }
}

fn fallback_dark() -> Arc<Theme> {
    load_theme("dark", None).expect("the bundled dark theme loads")
}

/// `initTheme`: load the requested (or detected default) theme; `"auto"`
/// resolves to the OS side first. A theme that does not load (a bad file, or
/// a name that is not a theme) records a warning and the built-in dark stands
/// in, since nothing loaded yet. The requested setting is left as it is.
pub fn init_theme(theme_name: Option<&str>, enable_watcher: bool) {
    let requested = theme_name.map_or_else(|| default_theme().to_string(), str::to_string);
    let name = resolve_auto_theme(&requested);
    match load_theme(&name, None) {
        Ok(theme) => {
            clear_theme_warning(&requested);
            {
                let mut s = state();
                s.current_name = Some(name.clone());
                s.theme = Some(theme);
            }
            if enable_watcher {
                start_theme_watcher();
            }
        }
        Err(error) => {
            record_theme_warning(&requested, &error, "Using the built-in dark theme for now.");
            let dark = fallback_dark();
            let mut s = state();
            s.current_name = Some("dark".to_string());
            s.theme = Some(dark);
        }
    }
    // A fresh load is a new theme even under the same name.
    bump_theme_generation();
}

/// The theme changed: caches keyed on its generation, and the lines containers
/// keep for their children, are stale.
fn bump_theme_generation() {
    THEME_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    hoocode_tui_render::bump_render_epoch();
}

fn notify_change() {
    bump_theme_generation();
    let callback = state().on_change.clone();
    if let Some(callback) = callback {
        callback();
    }
}

/// `setTheme`: switch themes. `"auto"` resolves to the OS side first, so the
/// current name is always concrete. On failure nothing changes: the last good
/// theme stays on screen and the error is returned for the caller to show.
pub fn set_theme(requested: &str, enable_watcher: bool) -> Result<(), String> {
    // A new choice replaces the warnings about the old one. A side of an
    // auto pair that is still missing warns again in the resolve below.
    state().warnings.clear();
    let name = resolve_auto_theme(requested);
    let theme = match load_theme(&name, None) {
        Ok(theme) => theme,
        Err(error) => {
            record_theme_warning(requested, &error, "Kept the current theme.");
            return Err(error);
        }
    };
    {
        let mut s = state();
        s.current_name = Some(name.clone());
        s.theme = Some(theme);
    }
    if enable_watcher {
        start_theme_watcher();
    }
    notify_change();
    Ok(())
}

/// `setThemeInstance`: use a theme object directly (not watchable).
pub fn set_theme_instance(theme: Theme) {
    {
        let mut s = state();
        s.theme = Some(Arc::new(theme));
        s.current_name = Some("<in-memory>".to_string());
    }
    stop_theme_watcher();
    notify_change();
}

/// `onThemeChange`: the callback run after the theme changes (it may run on
/// the watcher's thread).
pub fn on_theme_change(callback: impl Fn() + Send + Sync + 'static) {
    state().on_change = Some(Arc::new(callback));
}

/// File identity used to notice edits (the watcher polls; see
/// [`start_theme_watcher`]).
fn file_signature(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// How often the watcher looks at the file, and how long it waits for edits
/// to settle before reloading (hoocode's reload debounce).
const WATCH_POLL: Duration = Duration::from_millis(50);
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// `startThemeWatcher`: reload the current custom theme when its file
/// changes. Built-ins are not watched. Polls the file rather than using OS
/// notifications, so it needs no platform watcher.
fn start_theme_watcher() {
    stop_theme_watcher();
    let Some(watched_name) = current_theme_name() else {
        return;
    };
    if is_builtin_theme(&watched_name) {
        return;
    }
    let theme_file = custom_themes_dir().join(format!("{watched_name}.json"));
    if !theme_file.exists() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = hoocode_runtime::spawn_thread("hoocode-theme-watch", move || {
        let mut files = theme_watch_files(&theme_file);
        let mut last = signatures(&files);
        let mut pending: Option<Instant> = None;
        while !thread_stop.load(Ordering::Relaxed) {
            std::thread::sleep(WATCH_POLL);
            let signature = signatures(&files);
            if signature != last {
                last = signature;
                pending = Some(Instant::now());
            }
            if pending.is_some_and(|at| at.elapsed() >= RELOAD_DEBOUNCE) {
                pending = None;
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                // An edit may have changed `inherits`, so the chain is read
                // again. The snapshot comes before the reload, so an edit made
                // during the reload triggers another one.
                files = theme_watch_files(&theme_file);
                last = signatures(&files);
                reload_watched(&watched_name, &theme_file);
            }
        }
    });
    state().watcher = Some(Watcher { stop, thread });
}

/// The file signatures of `files`, in order.
fn signatures(files: &[PathBuf]) -> Vec<Option<(SystemTime, u64)>> {
    files.iter().map(|f| file_signature(f)).collect()
}

/// The files the watcher polls for a custom theme: the file and the user
/// files its `inherits` chain reads, so
/// an edit to a base reloads the theme too. Built-in bases never change and
/// are not listed. The chain stops at a missing file or a repeated one.
pub fn theme_watch_files(theme_file: &Path) -> Vec<PathBuf> {
    let dir = theme_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut files = vec![theme_file.to_path_buf()];
    let mut content = std::fs::read_to_string(theme_file).ok();
    while let Some(raw) = content.take() {
        let Ok(Value::Object(json)) = serde_json::from_str::<Value>(&raw) else {
            break;
        };
        let Some(base) = json.get("inherits").and_then(Value::as_str) else {
            break;
        };
        if is_builtin_theme(base) {
            break;
        }
        let path = dir.join(format!("{base}.json"));
        if files.contains(&path) || !path.is_file() {
            break;
        }
        content = std::fs::read_to_string(&path).ok();
        files.push(path);
    }
    files
}

fn reload_watched(watched_name: &str, theme_file: &Path) {
    // Ignore stale reloads after switching themes.
    if current_theme_name().as_deref() != Some(watched_name) {
        return;
    }
    // Keep the last good theme while the file is temporarily missing.
    if !theme_file.exists() {
        return;
    }
    // A file mid-edit may be invalid: keep the current theme and say why.
    let reloaded = match load_theme_from_path(theme_file, None) {
        Ok(theme) => theme,
        Err(error) => {
            record_theme_warning(watched_name, &error, "Keeping the last good theme.");
            return;
        }
    };
    clear_theme_warning(watched_name);
    {
        let reloaded = Arc::new(reloaded);
        let mut s = state();
        match s.registered.iter_mut().find(|(n, _)| n == watched_name) {
            Some(slot) => slot.1 = reloaded.clone(),
            None => s
                .registered
                .push((watched_name.to_string(), reloaded.clone())),
        }
        s.theme = Some(reloaded);
    }
    notify_change();
}

/// `stopThemeWatcher`. Waits for the watcher thread to exit (it polls every
/// [`WATCH_POLL`]), so a reload cannot land after this returns. A watcher
/// stopping itself (from a change callback) is not joined.
pub fn stop_theme_watcher() {
    // Take the handle out first: the watcher needs the state lock to reload.
    let watcher = state().watcher.take();
    if let Some(Watcher { stop, thread }) = watcher {
        stop.store(true, Ordering::Relaxed);
        if thread.thread().id() != std::thread::current().id() {
            let _ = thread.join();
        }
    }
}

// ============================================================================
// HTML export helpers
// ============================================================================

/// `hasLightBackdrop`: judged by the theme's export page, then
/// `userMessageBg`. A bad `export.pageBg` reference is an error, as in hoocode.
fn has_light_backdrop(
    theme_json: &ThemeJson,
    resolved: &[(String, RawColor)],
) -> Result<bool, String> {
    let vars = theme_json.vars.as_deref().unwrap_or(&[]);
    let page_bg = match theme_json.export.as_ref().and_then(|e| e.page_bg.as_ref()) {
        Some(v) => Some(resolve_var_refs(v, vars)?),
        None => None,
    };
    let user_bg = resolved
        .iter()
        .find(|(k, _)| k == "userMessageBg")
        .map(|(_, v)| v.clone());
    for candidate in [page_bg, user_bg].into_iter().flatten() {
        if candidate.is_default() {
            continue;
        }
        if let Some(luminance) = relative_luminance(&candidate) {
            return Ok(luminance > 0.5);
        }
    }
    Ok(false)
}

fn name_or_current(theme_name: Option<&str>) -> String {
    theme_name
        .map(str::to_string)
        .or_else(current_theme_name)
        .unwrap_or_else(|| default_theme().to_string())
}

/// `getResolvedThemeColors`: every token as a CSS hex string (256-color
/// indices converted, `""` replaced by a default text color).
pub fn get_resolved_theme_colors(
    theme_name: Option<&str>,
) -> Result<BTreeMap<String, String>, String> {
    let name = name_or_current(theme_name);
    let theme_json = load_theme_json(&name)?;
    let resolved = resolve_theme_colors(&theme_json.colors, theme_json.vars.as_deref())?;
    let default_text = if has_light_backdrop(&theme_json, &resolved)? {
        "#000000"
    } else {
        "#e5e5e7"
    };
    Ok(resolved
        .into_iter()
        .map(|(key, value)| {
            let css = match value {
                RawColor::Index(n) => ansi256_to_hex(n as i64),
                RawColor::Str(s) if s.is_empty() => default_text.to_string(),
                RawColor::Str(s) => s,
            };
            (key, css)
        })
        .collect())
}

/// Explicit export colors (`getThemeExportColors`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeExportColors {
    pub page_bg: Option<String>,
    pub card_bg: Option<String>,
    pub info_bg: Option<String>,
}

/// `getThemeExportColors`: the theme's `export` section resolved to hex;
/// each is `None` when unset (or `""`). Errors yield all `None`.
pub fn get_theme_export_colors(theme_name: Option<&str>) -> ThemeExportColors {
    let name = name_or_current(theme_name);
    let resolve_all = || -> Result<ThemeExportColors, String> {
        let theme_json = load_theme_json(&name)?;
        let Some(export) = &theme_json.export else {
            return Ok(ThemeExportColors::default());
        };
        let vars = theme_json.vars.as_deref().unwrap_or(&[]);
        let resolve = |value: &Option<ColorValue>| -> Result<Option<String>, String> {
            let Some(value) = value else { return Ok(None) };
            Ok(match resolve_var_refs(value, vars)? {
                RawColor::Index(n) => Some(ansi256_to_hex(n as i64)),
                RawColor::Str(s) if s.is_empty() => None,
                RawColor::Str(s) => Some(s),
            })
        };
        Ok(ThemeExportColors {
            page_bg: resolve(&export.page_bg)?,
            card_bg: resolve(&export.card_bg)?,
            info_bg: resolve(&export.info_bg)?,
        })
    };
    resolve_all().unwrap_or_default()
}
