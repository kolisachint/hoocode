//! U22 P6(a) theme slice: yellow is blocking only (tui.md principle 17, mock
//! §§3-4). No new tokens: the blocking chip is `chipWarn`, selection is
//! `paneSel`, and the state glyphs read `accent` (`◉` running), `success`
//! (`●` done), `error` (`⊗` failed) and `dim` (stopped, `○` queued).
//!
//! These tests lock the theme side of the contract. Where yellow appears on
//! screen is decided at the call sites, which this slice does not touch.

use std::collections::BTreeMap;
use std::path::Path;

use hoocode_code_tui_theme::*;

/// Every theme the crate ships.
const BUILTINS: [&str; 5] = ["dark", "dusk", "high-contrast", "light", "paper"];

/// The pane surfaces plus the chip pair: the tokens the pane grammar reads.
const PANE_SET: [&str; 9] = [
    "surface",
    "pane",
    "paneSel",
    "paneInput",
    "paneDetail",
    "chip",
    "chipWarn",
    "chipFg",
    "selectedBg",
];

/// The border tokens monochrome mode draws its light borders from.
const BORDER_SET: [&str; 3] = ["border", "borderAccent", "borderMuted"];

/// Every token resolved to a CSS hex string (built-ins only, so this reads
/// no global state and no user directory).
fn resolved(name: &str) -> BTreeMap<String, String> {
    get_resolved_theme_colors(Some(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn channels(hex: &str) -> [f64; 3] {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    [0, 2, 4].map(|o| i64::from_str_radix(&hex[o..o + 2], 16).unwrap() as f64)
}

fn luminance(hex: &str) -> f64 {
    let to_linear = |c: f64| {
        let s = c / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = channels(hex).map(to_linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn hue(hex: &str) -> f64 {
    let [r, g, b] = channels(hex).map(|c| c / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    if delta == 0.0 {
        return 0.0;
    }
    let raw = if max == r {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    raw * 60.0
}

fn saturation(hex: &str) -> f64 {
    let [r, g, b] = channels(hex).map(|c| c / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max == min {
        return 0.0;
    }
    (max - min) / (1.0 - (max + min - 1.0).abs())
}

/// The yellow family both blocking tokens live in (dark `#f0c674` is 40°,
/// high-contrast `#ffe600` is 54°).
const YELLOW_HUE: (f64, f64) = (30.0, 65.0);

fn is_yellow(hex: &str) -> bool {
    (YELLOW_HUE.0..=YELLOW_HUE.1).contains(&hue(hex)) && saturation(hex) >= 0.2
}

#[test]
fn pane_chip_and_border_tokens_exist_in_every_builtin() {
    for name in BUILTINS {
        let theme = get_theme_by_name(name).unwrap_or_else(|| panic!("{name} loads"));
        for token in PANE_SET {
            assert!(
                theme.has(token) || theme.has_bg(token),
                "{name}: {token} is defined"
            );
        }
        for token in BORDER_SET {
            assert!(theme.has(token), "{name}: {token} is defined");
        }
        // Each token renders without panicking, on either channel it owns.
        for token in PANE_SET.into_iter().chain(BORDER_SET) {
            if theme.has_bg(token) {
                theme.bg(token, "x");
            } else {
                theme.fg(token, "x");
            }
        }
    }
}

#[test]
fn pane_selection_stays_a_distinct_step_above_the_pane() {
    for name in BUILTINS {
        let colors = resolved(name);
        assert_ne!(
            colors["paneSel"], colors["pane"],
            "{name}: selection reads apart from the pane"
        );
    }
}

/// Mock §3 "Tokens and contrast", recomputed from the theme files: the exact
/// values, so dim stays as it is and `paneSel` gets no darker.
#[test]
fn mock_section_3_locks_the_dark_and_light_token_values() {
    let dark = resolved("dark");
    let expected_dark = [
        ("pane", "#303043", "pane"),
        ("paneSel", "#2c3f47", "paneSel"),
        ("surface", "#393949", "surface"),
        ("detail", "#363854", "paneDetail"),
        ("accent chip", "#5cc8bb", "chip"),
        ("blocking chip", "#f0c674", "chipWarn"),
        ("ink", "#e5e5e7", "text"),
        ("dim", "#a7a7a7", "dim"),
        ("accent", "#5cc8bb", "accent"),
        ("ok", "#b5bd68", "success"),
        ("err", "#dc9494", "error"),
        ("yellow text", "#f0c674", "warning"),
    ];
    let page_bg = get_theme_export_colors(Some("dark"))
        .page_bg
        .expect("dark export.pageBg");
    assert_eq!(page_bg, "#14141c", "dark base");
    for (role, hex, token) in expected_dark {
        assert_eq!(&dark[token], hex, "dark {role}");
    }

    let light = resolved("light");
    let expected_light = [
        ("base", "#f8f8fb"),
        ("pane", "#dfdfea"),
        ("paneSel", "#b6d5d5"),
        ("surface", "#cfd1df"),
        ("detail", "#cdd2e6"),
        ("accent chip", "#00625c"),
        ("blocking chip", "#f0c674"),
        ("ink", "#2b2f36"),
        ("dim", "#555763"),
        ("accent", "#00625c"),
        ("ok", "#0a6625"),
        ("err", "#b0101d"),
        ("yellow text", "#785000"),
    ];
    let tokens = [
        "pane",
        "paneSel",
        "surface",
        "paneDetail",
        "chip",
        "chipWarn",
        "text",
        "dim",
        "accent",
        "success",
        "error",
        "warning",
    ];
    let page_bg = get_theme_export_colors(Some("light"))
        .page_bg
        .expect("light export.pageBg");
    assert_eq!(page_bg, expected_light[0].1);
    for ((role, hex), token) in expected_light.into_iter().skip(1).zip(tokens) {
        assert_eq!(&light[token], hex, "light {role}");
    }
}

/// Mock §3: every text role on every pane surface is at least 4.5:1. The
/// tightest after the 2026-11-12 pane lift are red and dim on dark `paneSel`
/// (4.56, 4.57) and warning and green on light `paneSel` (4.57, 4.58).
#[test]
fn mock_section_3_pairs_hold_on_base_pane_and_panesel() {
    for name in ["dark", "light"] {
        let colors = resolved(name);
        let page_bg = get_theme_export_colors(Some(name))
            .page_bg
            .unwrap_or_else(|| panic!("{name} export.pageBg"));
        for surface in [&page_bg, &colors["pane"], &colors["paneSel"]] {
            for role in ["text", "dim", "accent", "success", "error", "warning"] {
                let ratio = contrast(&colors[role], surface);
                assert!(
                    ratio >= 4.5,
                    "{name} {role} ({}) on {surface}: {ratio:.2}:1",
                    colors[role]
                );
            }
        }
    }
}

/// The blocking yellow in every built-in: `chipWarn` is a yellow fill with
/// a dark ink at 4.5:1 or better (`chipFg` on dark themes, `text` on light
/// ones — the pane keeps the dark ink, so no new token), and the blocking
/// yellow text (`warning`) reads on the base.
#[test]
fn blocking_yellow_reads_in_every_builtin() {
    for name in BUILTINS {
        let colors = resolved(name);
        assert!(is_yellow(&colors["chipWarn"]), "{name} chipWarn");
        assert!(is_yellow(&colors["warning"]), "{name} warning");
        let chip_ink = contrast(&colors["chipWarn"], &colors["chipFg"]);
        let text_ink = contrast(&colors["chipWarn"], &colors["text"]);
        let (ink, ratio) = if chip_ink >= text_ink {
            ("chipFg", chip_ink)
        } else {
            ("text", text_ink)
        };
        assert!(
            ratio >= 4.5,
            "{name} chipWarn ({}) on {ink}: {ratio:.2}:1",
            colors["chipWarn"]
        );
        assert!(
            luminance(&colors[if ink == "chipFg" { "chipFg" } else { "text" }]) < 0.5,
            "{name}: the blocking chip keeps dark text"
        );
        let page_bg = get_theme_export_colors(Some(name))
            .page_bg
            .unwrap_or_else(|| panic!("{name} export.pageBg"));
        let ratio = contrast(&colors["warning"], &page_bg);
        assert!(
            ratio >= 4.5,
            "{name} warning ({}) on base: {ratio:.2}:1",
            colors["warning"]
        );
    }
}

/// State marks read apart in every built-in: done is green, failed is red,
/// running is the saturated accent. Mark legibility on the base, and the
/// selected row's ink and secondary cells on `paneSel`, all hold 4.5:1.
#[test]
fn state_marks_read_green_red_and_accent_in_every_builtin() {
    for name in BUILTINS {
        let colors = resolved(name);
        assert!(
            (55.0..=170.0).contains(&hue(&colors["success"]))
                && saturation(&colors["success"]) >= 0.2,
            "{name} success is green: {}",
            colors["success"]
        );
        let err_hue = hue(&colors["error"]);
        assert!(
            (err_hue <= 25.0 || err_hue >= 335.0) && saturation(&colors["error"]) >= 0.2,
            "{name} error is red: {}",
            colors["error"]
        );
        assert!(
            saturation(&colors["accent"]) >= 0.2,
            "{name} accent carries the running mark: {}",
            colors["accent"]
        );
        let page_bg = get_theme_export_colors(Some(name))
            .page_bg
            .unwrap_or_else(|| panic!("{name} export.pageBg"));
        for role in ["accent", "success", "error"] {
            let ratio = contrast(&colors[role], &page_bg);
            assert!(
                ratio >= 4.5,
                "{name} {role} ({}) on base: {ratio:.2}:1",
                colors[role]
            );
        }
        for role in ["text", "dim"] {
            let ratio = contrast(&colors[role], &colors["paneSel"]);
            assert!(
                ratio >= 4.5,
                "{name} {role} ({}) on paneSel: {ratio:.2}:1",
                colors[role]
            );
        }
    }
}

/// Pane surfaces a theme omits fall back to `selectedBg`, so an older
/// custom theme still paints the pane grammar (including `chipWarn`).
#[test]
fn omitted_pane_surfaces_fall_back_to_selected_bg() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/dark.json");
    let content = std::fs::read_to_string(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&content).unwrap();
    let colors = json
        .get_mut("colors")
        .and_then(|c| c.as_object_mut())
        .expect("colors object");
    for token in PANE_BG_TOKENS {
        colors.remove(token);
    }
    let theme_json = parse_theme_json_content("no-panes", &json.to_string()).unwrap();
    let theme = create_theme(&theme_json, Some(ColorMode::Truecolor), None).unwrap();
    let selected = theme.get_bg_ansi("selectedBg").to_string();
    for token in PANE_BG_TOKENS {
        assert!(theme.has_bg(token), "{token} falls back");
        assert_eq!(theme.get_bg_ansi(token), selected, "{token} falls back");
    }
}
