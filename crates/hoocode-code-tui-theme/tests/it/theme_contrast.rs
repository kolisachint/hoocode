//! Port of the pin's `test/theme-contrast.test.ts`: contrast, separation,
//! roster and chip-fill sweeps over the bundled themes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Once;

use hoocode_code_tui_theme::*;
use serde_json::Value;

/// Point the agent dir at an empty directory so a developer's own custom
/// themes never join the roster under test.
fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("hoocode-theme-contrast-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", &dir);
    });
}

const ACCESSIBLE_THEMES: [&str; 2] = ["colorsafe-dark", "colorsafe-light"];
const LIGHT_THEMES: [&str; 3] = ["colorsafe-light", "daylight", "light"];
const DARK_THEMES: [&str; 3] = ["colorsafe-dark", "dark", "deep-space"];

fn shipped_themes() -> Vec<&'static str> {
    let mut all: Vec<&str> = LIGHT_THEMES
        .iter()
        .chain(DARK_THEMES.iter())
        .copied()
        .collect();
    all.sort();
    all
}

const RETIRED: [(&str, &str); 10] = [
    ("high-contrast-dark", "colorsafe-dark"),
    ("high-contrast-light", "colorsafe-light"),
    ("warm-dark", "colorsafe-dark"),
    ("warm-light", "colorsafe-light"),
    ("vox-dark", "dark"),
    ("vox-light", "light"),
    ("solarized-dark", "deep-space"),
    ("solarized-light", "daylight"),
    ("vox-cutout-dark", "dark"),
    ("vox-cutout-light", "light"),
];

/// AAA for body text.
const MIN_CONTRAST: f64 = 7.0;
/// CIEDE2000 floor for two tokens in the same group.
const MIN_DIFFERENCE: f64 = 11.0;

const BG_TOKENS: [&str; 8] = [
    "selectedBg",
    "userMessageBg",
    "customMessageBg",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
    "toolBandBg",
    "warningBg",
];

const AGENT_TOKENS: [&str; 6] = ["agent1", "agent2", "agent3", "agent4", "agent5", "agent6"];

fn meaning_groups() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "core UI",
            vec![
                "accent",
                "success",
                "error",
                "warning",
                "border",
                "customMessageLabel",
            ],
        ),
        (
            "agent identity",
            AGENT_TOKENS.iter().copied().chain(["mcp"]).collect(),
        ),
        (
            "syntax",
            vec![
                "syntaxComment",
                "syntaxKeyword",
                "syntaxFunction",
                "syntaxVariable",
                "syntaxString",
                "syntaxNumber",
                "syntaxType",
            ],
        ),
        (
            "thinking levels",
            vec![
                "thinkingOff",
                "thinkingMinimal",
                "thinkingLow",
                "thinkingMedium",
                "thinkingHigh",
                "thinkingXhigh",
            ],
        ),
        (
            "diff",
            vec!["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
        ),
    ]
}

/// The groups that also have to survive the 6x6x6 cube.
fn quantized_groups() -> Vec<(&'static str, Vec<&'static str>)> {
    meaning_groups()
        .into_iter()
        .filter(|(label, _)| matches!(*label, "core UI" | "agent identity" | "syntax"))
        .collect()
}

const DARK_DECORATIVE_TOKENS: [&str; 3] = ["borderMuted", "mdHr", "thinkingOff"];
const DARK_NEUTRAL_TOKENS: [&str; 1] = ["mdQuoteBorder"];
const LIGHT_DECORATIVE_TOKENS: [&str; 3] = ["borderMuted", "mdHr", "thinkingOff"];

/// Every token a theme must carry: the 51 the schema requires, plus the
/// agent palette and `mcp`.
fn all_tokens() -> Vec<String> {
    let schema: Value = serde_json::from_str(THEME_SCHEMA_JSON).unwrap();
    let mut tokens: Vec<String> = schema["properties"]["colors"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    tokens.extend(AGENT_TOKENS.iter().map(|s| s.to_string()));
    tokens.push("mcp".to_string());
    tokens
}

fn channels(hex: &str) -> [f64; 3] {
    let value = hex.replacen('#', "", 1);
    [0, 2, 4].map(|o| i64::from_str_radix(&value[o..o + 2], 16).unwrap() as f64)
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

fn srgb_linear(hex: &str) -> [f64; 3] {
    channels(hex).map(|c| {
        let c = c / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn to_lab(hex: &str) -> [f64; 3] {
    let [r, g, b] = srgb_linear(hex);
    let xyz = [
        (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047,
        0.2126 * r + 0.7152 * g + 0.0722 * b,
        (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883,
    ]
    .map(|c| {
        if c > 0.008856 {
            c.cbrt()
        } else {
            7.787 * c + 16.0 / 116.0
        }
    });
    [
        116.0 * xyz[1] - 16.0,
        500.0 * (xyz[0] - xyz[1]),
        200.0 * (xyz[1] - xyz[2]),
    ]
}

/// CIEDE2000 difference.
fn difference(hex_a: &str, hex_b: &str) -> f64 {
    let [l1, a1, b1] = to_lab(hex_a);
    let [l2, a2, b2] = to_lab(hex_b);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let mean_c = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (mean_c.powi(7) / (mean_c.powi(7) + 25f64.powi(7))).sqrt());
    let ap1 = (1.0 + g) * a1;
    let ap2 = (1.0 + g) * a2;
    let cp1 = ap1.hypot(b1);
    let cp2 = ap2.hypot(b2);
    let angle = |y: f64, x: f64| {
        if y == 0.0 && x == 0.0 {
            return 0.0;
        }
        let deg = y.atan2(x) * 180.0 / std::f64::consts::PI;
        if deg < 0.0 {
            deg + 360.0
        } else {
            deg
        }
    };
    let hp1 = angle(b1, ap1);
    let hp2 = angle(b2, ap2);
    let d_lp = l2 - l1;
    let d_cp = cp2 - cp1;
    let mut dhp = 0.0;
    if cp1 * cp2 != 0.0 {
        dhp = hp2 - hp1;
        if dhp > 180.0 {
            dhp -= 360.0;
        } else if dhp < -180.0 {
            dhp += 360.0;
        }
    }
    let d_hp = 2.0 * (cp1 * cp2).sqrt() * (dhp * std::f64::consts::PI / 360.0).sin();
    let mean_lp = (l1 + l2) / 2.0;
    let mean_cp = (cp1 + cp2) / 2.0;
    let mut mean_hp = hp1 + hp2;
    if cp1 * cp2 != 0.0 {
        if (hp1 - hp2).abs() > 180.0 {
            mean_hp = if hp1 + hp2 < 360.0 {
                mean_hp + 360.0
            } else {
                mean_hp - 360.0
            };
        }
        mean_hp /= 2.0;
    }
    let rad = |d: f64| d * std::f64::consts::PI / 180.0;
    let t = 1.0 - 0.17 * rad(mean_hp - 30.0).cos()
        + 0.24 * rad(2.0 * mean_hp).cos()
        + 0.32 * rad(3.0 * mean_hp + 6.0).cos()
        - 0.2 * rad(4.0 * mean_hp - 63.0).cos();
    let sl = 1.0 + (0.015 * (mean_lp - 50.0).powi(2)) / (20.0 + (mean_lp - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * mean_cp;
    let sh = 1.0 + 0.015 * mean_cp * t;
    let rt = -rad(2.0 * 30.0 * (-((mean_hp - 275.0) / 25.0).powi(2)).exp()).sin()
        * (2.0 * (mean_cp.powi(7) / (mean_cp.powi(7) + 25f64.powi(7))).sqrt());
    ((d_lp / sl).powi(2)
        + (d_cp / sc).powi(2)
        + (d_hp / sh).powi(2)
        + rt * (d_cp / sc) * (d_hp / sh))
        .sqrt()
}

#[derive(Clone, Copy, Debug)]
enum Cvd {
    Protan,
    Deutan,
    Tritan,
}

/// Viénot-Brettel-Mollon dichromat simulation.
fn simulate_cvd(hex: &str, kind: Cvd) -> String {
    let rgb = srgb_linear(hex);
    let apply =
        |m: [[f64; 3]; 3], v: [f64; 3]| m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
    let rgb_to_lms = [
        [0.31399, 0.63951, 0.04649],
        [0.15537, 0.75789, 0.0867],
        [0.01775, 0.10945, 0.87259],
    ];
    let lms_to_rgb = [
        [5.47221, -4.64196, 0.16963],
        [-1.12524, 2.29317, -0.16789],
        [0.0298, -0.19318, 1.16364],
    ];
    let collapse = match kind {
        Cvd::Protan => [
            [0.0, 1.05118294, -0.05116099],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        Cvd::Deutan => [
            [1.0, 0.0, 0.0],
            [0.9513092, 0.0, 0.04866992],
            [0.0, 0.0, 1.0],
        ],
        Cvd::Tritan => [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-0.86744736, 1.86727089, 0.0],
        ],
    };
    let out = apply(lms_to_rgb, apply(collapse, apply(rgb_to_lms, rgb)));
    let hex: Vec<String> = out
        .iter()
        .map(|&c| {
            let s = if c <= 0.0031308 {
                12.92 * c
            } else {
                1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055
            };
            format!("{:02x}", ((s.clamp(0.0, 1.0) * 255.0) + 0.5).floor() as i64)
        })
        .collect();
    format!("#{}", hex.join(""))
}

/// HSL saturation (0-1).
fn saturation(hex: &str) -> f64 {
    let [r, g, b] = channels(hex).map(|c| c / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max == min {
        return 0.0;
    }
    (max - min) / (1.0 - (max + min - 1.0).abs())
}

fn hue(hex: &str) -> f64 {
    let [r, g, b] = channels(hex).map(|c| c / 255.0);
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
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

fn lightness(hex: &str) -> f64 {
    let [r, g, b] = channels(hex).map(|c| c / 255.0);
    (r.max(g).max(b) + r.min(g).min(b)) / 2.0
}

fn hue_distance(a: f64, b: f64) -> f64 {
    let delta = (a - b).abs() % 360.0;
    if delta > 180.0 {
        360.0 - delta
    } else {
        delta
    }
}

fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for (i, &item) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(i);
        for mut tail in permutations(&rest) {
            tail.insert(0, item);
            out.push(tail);
        }
    }
    out
}

fn colors(theme: &str) -> BTreeMap<String, String> {
    isolate();
    get_resolved_theme_colors(Some(theme)).unwrap()
}

/// Every surface a theme paints text on: the backgrounds plus the export
/// canvases.
fn surfaces(theme: &str) -> Vec<(String, String)> {
    let c = colors(theme);
    let exported = get_theme_export_colors(Some(theme));
    let mut result: Vec<(String, String)> = BG_TOKENS
        .iter()
        .map(|t| (t.to_string(), c[*t].clone()))
        .collect();
    for (key, value) in [
        ("pageBg", exported.page_bg),
        ("cardBg", exported.card_bg),
        ("infoBg", exported.info_bg),
    ] {
        if let Some(v) = value {
            result.push((format!("export.{key}"), v));
        }
    }
    result
}

/// JS `toFixed` for the failure messages.
fn fixed(x: f64, digits: usize) -> String {
    format!("{x:.digits$}")
}

const LIGHT_HUE_TOKENS: [&str; 33] = [
    "accent",
    "border",
    "borderAccent",
    "success",
    "error",
    "warning",
    "customMessageLabel",
    "mdHeading",
    "mdLink",
    "mdCode",
    "mdQuoteBorder",
    "mdListBullet",
    "toolDiffAdded",
    "toolDiffRemoved",
    "syntaxComment",
    "syntaxKeyword",
    "syntaxFunction",
    "syntaxVariable",
    "syntaxString",
    "syntaxNumber",
    "syntaxType",
    "thinkingLow",
    "thinkingMedium",
    "thinkingHigh",
    "thinkingXhigh",
    "bashMode",
    "agent1",
    "agent2",
    "agent3",
    "agent4",
    "agent5",
    "agent6",
    "mcp",
];

fn light_hue_tokens() -> Vec<&'static str> {
    LIGHT_HUE_TOKENS.to_vec()
}

/// Chip fills, and the inks that only ever render inside them.
const CHIP_PAIRS: [(&str, &str); 3] = [
    ("headlineBg", "headlineText"),
    ("tapeBg", "tapeText"),
    ("brandBg", "brandText"),
];

fn is_chip_token(token: &str) -> bool {
    CHIP_PAIRS.iter().any(|(a, b)| *a == token || *b == token)
}

fn legibility_failures(
    theme: &str,
    backgrounds: &[(String, String)],
    minimum: impl Fn(&str) -> Option<f64>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for (token, value) in colors(theme) {
        if BG_TOKENS.contains(&token.as_str()) {
            continue;
        }
        let Some(min) = minimum(&token) else { continue };
        for (surface, background) in backgrounds {
            let ratio = contrast(&value, background);
            if ratio < min {
                failures.push(format!(
                    "{token} ({value}) on {surface} ({background}): {}:1",
                    fixed(ratio, 2)
                ));
            }
        }
    }
    failures
}

fn collisions(theme: &str, skip: impl Fn(&str) -> bool) -> Vec<String> {
    let c = colors(theme);
    let mut out = Vec::new();
    for (label, tokens) in meaning_groups() {
        if skip(label) {
            continue;
        }
        for i in 0..tokens.len() {
            for j in i + 1..tokens.len() {
                let delta = difference(&c[tokens[i]], &c[tokens[j]]);
                if delta < MIN_DIFFERENCE {
                    out.push(format!(
                        "[{label}] {} ({}) ~ {} ({}): ΔE {}",
                        tokens[i],
                        c[tokens[i]],
                        tokens[j],
                        c[tokens[j]],
                        fixed(delta, 1)
                    ));
                }
            }
        }
    }
    out
}

fn theme_path(name: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("themes")
        .join(format!("{name}.json"))
}

fn quantized_collisions(theme: &str, groups: Vec<(&'static str, Vec<&'static str>)>) {
    let quantized = load_theme_from_path(&theme_path(theme), Some(ColorMode::Color256)).unwrap();
    for (label, tokens) in groups {
        let mut by_index: Vec<(String, Vec<&str>)> = Vec::new();
        for token in tokens {
            let index = quantized.get_fg_ansi(token).to_string();
            match by_index.iter_mut().find(|(i, _)| *i == index) {
                Some((_, members)) => members.push(token),
                None => by_index.push((index, vec![token])),
            }
        }
        let collisions: Vec<String> = by_index
            .into_iter()
            .filter(|(_, m)| m.len() > 1)
            .map(|(_, m)| m.join(" = "))
            .collect();
        assert!(collisions.is_empty(), "{theme} {label}: {collisions:?}");
    }
}

fn wrong_side(theme: &str, is_light: bool) -> Vec<String> {
    surfaces(theme)
        .into_iter()
        .map(|(surface, background)| (surface, luminance(&background)))
        .filter(|(_, value)| {
            if is_light {
                *value <= 0.5
            } else {
                *value >= 0.5
            }
        })
        .map(|(surface, value)| format!("{surface}: {}", fixed(value, 3)))
        .collect()
}

fn raw_colors(theme: &str) -> (String, serde_json::Map<String, Value>) {
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(theme_path(theme)).unwrap()).unwrap();
    (
        raw["name"].as_str().unwrap().to_string(),
        raw["colors"].as_object().unwrap().clone(),
    )
}

fn renders_every_token(theme_name: &str) {
    let theme = get_theme_by_name(theme_name).expect(theme_name);
    for token in all_tokens() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if BG_TOKENS.contains(&token.as_str()) {
                theme.bg(&token, "x");
            } else {
                theme.fg(&token, "x");
            }
        }));
        assert!(result.is_ok(), "{theme_name}: {token} threw");
    }
}

mod default_dark_theme {
    use super::*;
    const MIN_TEXT_CONTRAST: f64 = 4.5;
    const MIN_DECORATIVE_CONTRAST: f64 = 2.8;
    const MIN_SATURATION: f64 = 0.2;

    #[test]
    fn keeps_every_foreground_legible_on_every_surface_it_paints() {
        let failures = legibility_failures("dark", &surfaces("dark"), |token| {
            if token == "activeToolBg" || is_chip_token(token) {
                return None;
            }
            Some(if DARK_DECORATIVE_TOKENS.contains(&token) {
                MIN_DECORATIVE_CONTRAST
            } else {
                MIN_TEXT_CONTRAST
            })
        });
        assert_eq!(failures, Vec::<String>::new());
    }

    #[test]
    fn keeps_meaning_carrying_tokens_saturated_enough_to_read_as_color() {
        let c = colors("dark");
        let washed_out: Vec<String> = light_hue_tokens()
            .into_iter()
            .filter(|t| !DARK_NEUTRAL_TOKENS.contains(t) && saturation(&c[*t]) < MIN_SATURATION)
            .map(|t| {
                format!(
                    "{t} ({}): {}% saturation",
                    c[t],
                    fixed(saturation(&c[t]) * 100.0, 0)
                )
            })
            .collect();
        assert_eq!(washed_out, Vec::<String>::new());
    }

    #[test]
    fn keeps_tokens_that_mean_different_things_apart() {
        assert_eq!(collisions("dark", |_| false), Vec::<String>::new());
    }

    #[test]
    fn keeps_colors_that_mean_different_things_apart_after_the_256_color_downgrade() {
        let groups = meaning_groups()
            .into_iter()
            .filter(|(label, _)| *label != "thinking levels" && *label != "diff")
            .collect();
        quantized_collisions("dark", groups);
    }

    #[test]
    fn marks_selection_visibly_against_the_page_without_lightening_it_into_paper() {
        let c = colors("dark");
        let page_bg = get_theme_export_colors(Some("dark"))
            .page_bg
            .expect("pageBg");
        assert!(contrast(&c["selectedBg"], &page_bg) > 1.2);
        assert!(luminance(&c["selectedBg"]) < 0.5);
    }
}

mod default_light_theme {
    use super::*;
    const MIN_TEXT_CONTRAST: f64 = 4.5;
    const MIN_DECORATIVE_CONTRAST: f64 = 2.8;
    const MIN_SATURATION: f64 = 0.35;

    #[test]
    fn keeps_every_foreground_legible_on_every_surface_it_paints() {
        let failures = legibility_failures("light", &surfaces("light"), |token| {
            Some(if LIGHT_DECORATIVE_TOKENS.contains(&token) {
                MIN_DECORATIVE_CONTRAST
            } else {
                MIN_TEXT_CONTRAST
            })
        });
        assert_eq!(failures, Vec::<String>::new());
    }

    #[test]
    fn keeps_meaning_carrying_tokens_saturated_enough_to_read_as_color() {
        let c = colors("light");
        let washed_out: Vec<String> = light_hue_tokens()
            .into_iter()
            .filter(|t| saturation(&c[*t]) < MIN_SATURATION)
            .map(|t| {
                format!(
                    "{t} ({}): {}% saturation",
                    c[t],
                    fixed(saturation(&c[t]) * 100.0, 0)
                )
            })
            .collect();
        assert_eq!(washed_out, Vec::<String>::new());
    }

    #[test]
    fn keeps_tokens_that_mean_different_things_apart() {
        assert_eq!(collisions("light", |_| false), Vec::<String>::new());
    }

    #[test]
    fn keeps_colors_that_mean_different_things_apart_after_the_256_color_downgrade() {
        quantized_collisions("light", quantized_groups());
    }

    #[test]
    fn marks_selection_visibly_against_the_page_without_darkening_it_into_text_territory() {
        let c = colors("light");
        let page_bg = get_theme_export_colors(Some("light"))
            .page_bg
            .expect("pageBg");
        assert!(contrast(&c["selectedBg"], &page_bg) > 1.2);
        assert!(luminance(&c["selectedBg"]) > 0.5);
    }
}

mod the_shipped_roster {
    use super::*;

    #[test]
    fn is_six_themes_three_light_and_three_dark() {
        isolate();
        assert_eq!(get_available_themes(), shipped_themes());
        assert_eq!(LIGHT_THEMES.len(), 3);
        assert_eq!(DARK_THEMES.len(), 3);
    }

    #[test]
    fn each_theme_sits_on_the_side_of_the_split_it_claims() {
        for theme in LIGHT_THEMES.iter().chain(DARK_THEMES.iter()) {
            let is_light = LIGHT_THEMES.contains(theme);
            assert_eq!(wrong_side(theme, is_light), Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn loads_each_retired_name_as_its_successor_rather_than_dropping_to_the_fallback() {
        isolate();
        for (retired, heir) in RETIRED {
            assert!(!get_available_themes().iter().any(|t| t == retired));
            assert_eq!(successor_theme_for(retired), Some(heir));
            assert_eq!(resolve_theme_name(retired), heir);
            assert_eq!(colors(retired), colors(heir));
        }
    }

    #[test]
    fn each_theme_defines_every_token_and_renders_all_of_them() {
        isolate();
        for theme in LIGHT_THEMES.iter().chain(DARK_THEMES.iter()) {
            let (name, raw) = raw_colors(theme);
            assert_eq!(&name, theme);
            let missing: Vec<String> = all_tokens()
                .into_iter()
                .filter(|t| !raw.contains_key(t))
                .collect();
            assert_eq!(missing, Vec::<String>::new(), "{theme}");
            renders_every_token(theme);
        }
    }

    #[test]
    fn leaves_a_name_nobody_retired_alone() {
        isolate();
        assert_eq!(successor_theme_for("dark"), None);
        assert_eq!(successor_theme_for("a-theme-that-never-existed"), None);
        assert_eq!(
            resolve_theme_name("a-theme-that-never-existed"),
            "a-theme-that-never-existed"
        );
        for shipped in shipped_themes() {
            assert_eq!(resolve_theme_name(shipped), shipped);
        }
    }
}

mod accessible_themes {
    use super::*;

    #[test]
    fn defines_every_color_token_explicitly() {
        for theme in ACCESSIBLE_THEMES {
            let (name, raw) = raw_colors(theme);
            assert_eq!(name, theme);
            let missing: Vec<String> = all_tokens()
                .into_iter()
                .filter(|t| !raw.contains_key(t))
                .collect();
            assert_eq!(missing, Vec::<String>::new());
            let defaults: Vec<String> = all_tokens().into_iter().filter(|t| raw[t] == "").collect();
            assert_eq!(defaults, Vec::<String>::new());
        }
    }

    #[test]
    fn renders_every_token_without_falling_back() {
        isolate();
        for theme in ACCESSIBLE_THEMES {
            renders_every_token(theme);
        }
    }

    #[test]
    fn keeps_every_foreground_at_aaa_contrast_on_every_surface() {
        for theme in ACCESSIBLE_THEMES {
            let failures = legibility_failures(theme, &surfaces(theme), |_| Some(MIN_CONTRAST));
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn keeps_tokens_that_mean_different_things_apart() {
        for theme in ACCESSIBLE_THEMES {
            // colorsafe-dark's agent palette is full at AAA (a known gap).
            let known_full = theme == "colorsafe-dark";
            let found = collisions(theme, |label| known_full && label == "agent identity");
            assert_eq!(found, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn keeps_its_surfaces_on_one_side_of_the_light_dark_split() {
        for theme in ACCESSIBLE_THEMES {
            assert_eq!(
                wrong_side(theme, theme.ends_with("-light")),
                Vec::<String>::new(),
                "{theme}"
            );
        }
    }

    #[test]
    fn colorsafe_light_keeps_its_semantic_pairs_apart_under_color_blind_vision() {
        let c = colors("colorsafe-light");
        let pairs = [
            ("success", "error"),
            ("error", "warning"),
            ("success", "warning"),
            ("toolDiffAdded", "toolDiffRemoved"),
            ("toolDiffAdded", "toolDiffContext"),
            ("toolDiffRemoved", "toolDiffContext"),
        ];
        let mut failures = Vec::new();
        for (a, b) in pairs {
            for kind in [Cvd::Protan, Cvd::Deutan, Cvd::Tritan] {
                let delta = difference(&simulate_cvd(&c[a], kind), &simulate_cvd(&c[b], kind));
                if delta < MIN_DIFFERENCE {
                    failures.push(format!("{a} ~ {b} under {kind:?}: ΔE {}", fixed(delta, 1)));
                }
            }
        }
        assert_eq!(failures, Vec::<String>::new());
    }

    #[test]
    fn marks_selection_and_tool_state_visibly_against_the_page() {
        for theme in ACCESSIBLE_THEMES {
            let c = colors(theme);
            let page_bg = get_theme_export_colors(Some(theme))
                .page_bg
                .expect("pageBg");
            assert!(contrast(&c["selectedBg"], &page_bg) > 1.2, "{theme}");
        }
    }
}

mod session_colour_slots {
    use super::*;

    /// `SESSION_COLOR_NAME_LIST` and where each name points on the wheel.
    const SLOT_HUE_ANCHORS: [(&str, f64); 6] = [
        ("cyan", 180.0),
        ("purple", 285.0),
        ("yellow", 50.0),
        ("magenta", 330.0),
        ("green", 120.0),
        ("blue", 220.0),
    ];

    fn available() -> Vec<String> {
        isolate();
        get_available_themes()
    }

    #[test]
    fn names_the_same_six_hues_the_identity_palette_is_addressed_by() {
        let names: Vec<&str> = SLOT_HUE_ANCHORS.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            hoocode_code_session::identity::session_color_name_list(),
            names
        );
        assert_eq!(names.len(), AGENT_TOKENS.len());
    }

    #[test]
    fn each_theme_puts_each_hue_in_the_slot_its_name_points_at() {
        for theme in available() {
            let c = colors(&theme);
            let hues: Vec<f64> = AGENT_TOKENS.iter().map(|t| hue(&c[*t])).collect();
            let anchors: Vec<f64> = SLOT_HUE_ANCHORS.iter().map(|(_, h)| *h).collect();
            let cost = |order: &[usize]| {
                order
                    .iter()
                    .enumerate()
                    .map(|(slot, &source)| hue_distance(hues[source], anchors[slot]).powi(2))
                    .sum::<f64>()
            };
            let shipped = cost(&[0, 1, 2, 3, 4, 5]);
            let better: Vec<String> = permutations(&[0, 1, 2, 3, 4, 5])
                .into_iter()
                .filter(|order| cost(order) < shipped)
                .map(|order| {
                    order
                        .iter()
                        .enumerate()
                        .map(|(slot, source)| {
                            format!("{}=agent{}", SLOT_HUE_ANCHORS[slot].0, source + 1)
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            assert_eq!(better, Vec::<String>::new(), "{theme}");
        }
    }

    /// Parse the fill and ink of a chip (`48;…` / `38;…`), 256-color indices
    /// converted through the cube.
    fn chip_parts(styled: &str) -> Option<(String, String)> {
        let param = |base: &str| {
            let start = styled.find(&format!("\x1b[{base};"))? + 2;
            let end = start + styled[start..].find('m')?;
            Some(styled[start..end].to_string())
        };
        const CUBE: [i64; 6] = [0, 95, 135, 175, 215, 255];
        let hex_of = |params: String| {
            let parts: Vec<i64> = params.split(';').map(|p| p.parse().unwrap()).collect();
            let ch = if parts[1] == 2 {
                vec![parts[2], parts[3], parts[4]]
            } else {
                let index = parts[2];
                if index >= 232 {
                    let g = 8 + (index - 232) * 10;
                    vec![g, g, g]
                } else {
                    vec![
                        CUBE[((index - 16) / 36) as usize],
                        CUBE[(((index - 16) % 36) / 6) as usize],
                        CUBE[((index - 16) % 6) as usize],
                    ]
                }
            };
            format!(
                "#{}",
                ch.iter().map(|c| format!("{c:02x}")).collect::<String>()
            )
        };
        Some((hex_of(param("48")?), hex_of(param("38")?)))
    }

    fn chips_of(theme: &str, mode: ColorMode) -> Vec<(String, String)> {
        let t = load_theme_from_path(&theme_path(theme), Some(mode)).unwrap();
        AGENT_TOKENS
            .iter()
            .map(|token| {
                chip_parts(&t.fill(token, " "))
                    .unwrap_or_else(|| panic!("{theme} {token}: not filled"))
            })
            .collect()
    }

    #[test]
    fn each_theme_fills_a_chip_light_enough_to_name_its_hue() {
        for theme in available() {
            let is_light = luminance(&colors(&theme)["userMessageBg"]) > 0.5;
            let mut failures = Vec::new();
            for (slot, (fill, ink)) in chips_of(&theme, ColorMode::Truecolor)
                .into_iter()
                .enumerate()
            {
                let slot = slot + 1;
                let magenta_below_floor = is_light
                    && hue(&fill) >= MAGENTA_HUE_MIN
                    && hue(&fill) <= MAGENTA_HUE_MAX
                    && saturation(&fill) > MIN_MAGENTA_SATURATION
                    && ink == "#ffffff"
                    && contrast(&fill, &ink) >= 5.5;
                if is_light && lightness(&fill) < 0.5 && !magenta_below_floor {
                    failures.push(format!(
                        "slot {slot} fill {fill}: lightness {}",
                        fixed(lightness(&fill), 2)
                    ));
                }
                if contrast(&fill, &ink) < 4.5 {
                    failures.push(format!(
                        "slot {slot} ink {ink} on {fill}: {}:1",
                        fixed(contrast(&fill, &ink), 2)
                    ));
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn each_theme_writes_a_chip_name_that_reads_on_every_terminal() {
        for theme in available() {
            let mut failures = Vec::new();
            for (mode, floor) in [(ColorMode::Truecolor, 5.5), (ColorMode::Color256, 4.5)] {
                for (index, (fill, ink)) in chips_of(&theme, mode).into_iter().enumerate() {
                    let ratio = contrast(&fill, &ink);
                    if ratio < floor {
                        failures.push(format!(
                            "{} slot {}: ink {ink} on {fill} is {}:1",
                            mode.as_str(),
                            index + 1,
                            fixed(ratio, 2)
                        ));
                    }
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn each_theme_fills_its_magenta_chip_with_white_ink() {
        for theme in available() {
            let mut failures = Vec::new();
            for mode in [ColorMode::Truecolor, ColorMode::Color256] {
                let (fill, ink) = chips_of(&theme, mode)[3].clone();
                if ink != "#ffffff" {
                    failures.push(format!(
                        "{}: slot 4 ink is {ink} on {fill}, want white",
                        mode.as_str()
                    ));
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn each_theme_keeps_its_six_chips_telling_themselves_apart() {
        for theme in available() {
            let mut failures = Vec::new();
            for mode in [ColorMode::Truecolor, ColorMode::Color256] {
                let fills: Vec<String> =
                    chips_of(&theme, mode).into_iter().map(|(f, _)| f).collect();
                for i in 0..fills.len() {
                    for j in i + 1..fills.len() {
                        if fills[i] == fills[j] {
                            failures.push(format!(
                                "{}: slot {} and slot {} are both {}",
                                mode.as_str(),
                                i + 1,
                                j + 1,
                                fills[i]
                            ));
                        }
                    }
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }

    #[test]
    fn each_theme_fills_each_chip_with_the_hue_its_own_token_carries() {
        for theme in available() {
            let c = colors(&theme);
            let mut failures = Vec::new();
            for (index, (fill, _)) in chips_of(&theme, ColorMode::Truecolor)
                .into_iter()
                .enumerate()
            {
                let token = &c[AGENT_TOKENS[index]];
                if saturation(token) < 0.05 {
                    continue;
                }
                let drift = hue_distance(hue(&fill), hue(token));
                if drift > 4.0 {
                    failures.push(format!(
                        "slot {}: token {token} filled with {fill}, hue moved {}°",
                        index + 1,
                        fixed(drift, 0)
                    ));
                }
                let lifted = fill.to_lowercase() != token.to_lowercase();
                if lifted
                    && saturation(&fill) > 0.9
                    && lightness(&fill) > 0.45
                    && lightness(&fill) < 0.62
                {
                    failures.push(format!("slot {}: fill {fill} is fluorescent", index + 1));
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "{theme}");
        }
    }
}
