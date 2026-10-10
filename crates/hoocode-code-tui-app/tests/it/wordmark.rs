//! `buildCompactWordmark` against output captured from the pinned build.

use hoocode_code_tui_app::wordmark::*;

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
    });
    assert_eq!(hoocode, "<a>▟▀▀▀▀▀▙</a>  <a>hoo</a><m>│</m>code\n<a>▌▟▙ ▟▙▐</a>  <d>tag</d> <m>·</m> <d>v1.0.0</d>\n<a>▜▄▄▄▄▄▛</a>  <d>/w</d>");
}
