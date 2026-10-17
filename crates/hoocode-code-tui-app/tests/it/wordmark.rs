//! `buildCompactWordmark` against output captured from the pinned build.

use hoocode_code_tui_app::wordmark::*;

use crate::support::lock;

fn w(tag: &'static str) -> impl Fn(&str) -> String {
    move |s: &str| format!("<{tag}>{s}</{tag}>")
}

#[test]
fn builds_the_banner_like_hoocode() {
    let (a, g, d, m, c) = (w("a"), w("g"), w("d"), w("m"), w("c"));
    let hoo = build_compact_wordmark(&CompactWordmarkOptions {
        app_name: "hoocode",
        version: "0.5.89",
        cwd: "~/p",
        tagline: None,
        accent: &a,
        glyph: Some(&g),
        dim: &d,
        muted: &m,
        cursor: Some(&c),
        brand: None,
        gradient: false,
    });
    assert_eq!(hoo, "<g>▟▀▀▀▀▀▙</g>  <a>hoo</a><m>│</m>code<c>_</c>\n<g>▌▟▙ ▟▙▐</g>  <d>coding agent</d> <m>·</m> <d>v0.5.89</d>\n<g>▜▄▄▄▄▄▛</g>  <d>~/p</d>");
    let hoocode = build_compact_wordmark(&CompactWordmarkOptions {
        app_name: "hoocode",
        version: "1.0.0",
        cwd: "/w",
        tagline: Some("tag"),
        accent: &a,
        glyph: None,
        dim: &d,
        muted: &m,
        cursor: None,
        brand: None,
        gradient: false,
    });
    assert_eq!(hoocode, "<a>▟▀▀▀▀▀▙</a>  <a>hoo</a><m>│</m>code\n<a>▌▟▙ ▟▙▐</a>  <d>tag</d> <m>·</m> <d>v1.0.0</d>\n<a>▜▄▄▄▄▄▛</a>  <d>/w</d>");
}

#[test]
fn plain_brand_renders_whole_with_no_split_and_no_cursor() {
    let (a, g, d, m, c, b) = (w("a"), w("g"), w("d"), w("m"), w("c"), w("b"));
    let plain = build_compact_wordmark(&CompactWordmarkOptions {
        app_name: "hoocode",
        version: "0.5.89",
        cwd: "~/p",
        tagline: None,
        accent: &a,
        glyph: Some(&g),
        dim: &d,
        muted: &m,
        cursor: Some(&c),
        brand: Some(&b),
        gradient: false,
    });
    assert_eq!(plain, "<g>▟▀▀▀▀▀▙</g>  <b>hoocode</b>\n<g>▌▟▙ ▟▙▐</g>  <d>coding agent</d> <m>·</m> <d>v0.5.89</d>\n<g>▜▄▄▄▄▄▛</g>  <d>~/p</d>");
}

/// The gradient owl under the dark theme: accent `#5cc8bb` to violet
/// `#9b8cff`, one truecolor run per column.
#[test]
fn gradient_paints_the_owl_accent_to_violet_per_column() {
    let _g = lock(Some("dark"));
    let (a, g, d, m) = (w("a"), w("g"), w("d"), w("m"));
    let banner = build_compact_wordmark(&CompactWordmarkOptions {
        app_name: "hoocode",
        version: "0.5.89",
        cwd: "~/p",
        tagline: None,
        accent: &a,
        glyph: Some(&g),
        dim: &d,
        muted: &m,
        cursor: None,
        brand: None,
        gradient: true,
    });
    let col = |r: u8, gr: u8, b: u8, ch: char| format!("\x1b[38;2;{r};{gr};{b}m{ch}\x1b[39m");
    let row0 = ['▟', '▀', '▀', '▀', '▀', '▀', '▙']
        .into_iter()
        .enumerate()
        .map(|(i, ch)| {
            let rgb = [
                (92, 200, 187),
                (102, 190, 198),
                (113, 180, 209),
                (123, 170, 221),
                (134, 160, 232),
                (144, 150, 243),
                (155, 140, 255),
            ][i];
            col(rgb.0, rgb.1, rgb.2, ch)
        })
        .collect::<String>();
    assert!(banner.starts_with(&format!("{row0}  ")), "{banner}");
}
