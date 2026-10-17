//! The marked lexer port and the Markdown component against the real
//! pinned `marked` + markdown.ts on a corpus of inputs
//! (`fixtures/markdown-gold.json`, from `archive/migration/tools/goldens/markdown.mjs`).

use hoocode_tui_components::markdown::lexer::lex;

use crate::common::markdown_json::{links_to_json, tokens_to_json};
use crate::common::{sgr, theme};
use hoocode_tui_components::{DefaultTextStyle, Markdown};
use hoocode_tui_images::{set_capabilities, TerminalCapabilities};
use hoocode_tui_render::Component;

fn gold() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("../fixtures/markdown-gold.json")).unwrap()
}

#[test]
fn token_streams_match_marked() {
    let mut failures = Vec::new();
    for case in gold() {
        let src = case["src"].as_str().unwrap();
        let lexed = lex(&src.replace('\t', "   "));
        let got = tokens_to_json(&lexed.tokens);
        if got != case["tokens"].as_str().unwrap() {
            failures.push(format!(
                "{src:?}\n  want {}\n  got  {got}",
                case["tokens"].as_str().unwrap()
            ));
        }
        let links = links_to_json(&lexed.links);
        if links != case["links"].as_str().unwrap() {
            failures.push(format!(
                "{src:?} links\n  want {}\n  got  {links}",
                case["links"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn renders_match_markdown_ts() {
    let _caps = crate::capabilities_lock();
    set_capabilities(TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks: false,
    });
    // Mission Control's code-block language label intentionally diverges from
    // the pinned markdown.ts renders; accept the new rows with UPDATE_GOLDENS=1.
    let accept = std::env::var("UPDATE_GOLDENS").as_deref() == Ok("1");
    let mut gold: Vec<serde_json::Value> = gold();
    let mut failures = Vec::new();
    for case in &mut gold {
        let src = case["src"].as_str().unwrap().to_string();
        let renders = &mut case["renders"];
        for (key, (px, py, width, styled)) in [
            ("80", (0, 0, 80, false)),
            ("24", (0, 0, 24, false)),
            ("padded", (2, 1, 40, false)),
            ("styled", (1, 0, 40, true)),
        ] {
            let style = styled.then(|| DefaultTextStyle {
                color: Some(sgr("\x1b[90m", "\x1b[39m")),
                italic: true,
                ..Default::default()
            });
            let got = Markdown::new(&src, px, py, theme(), style).render(width);
            if accept {
                renders[key] = serde_json::Value::Array(
                    got.iter()
                        .map(|l| serde_json::Value::String(l.clone()))
                        .collect(),
                );
                continue;
            }
            let want: Vec<String> = renders[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l.as_str().unwrap().to_string())
                .collect();
            if got != want {
                failures.push(format!("{src:?} [{key}]\n  want {want:?}\n  got  {got:?}"));
            }
        }
    }
    if accept {
        // Sidecar for the merge step (`scripts/`): the fixture stays
        // `json.dumps(indent=1, ensure_ascii=False)` formatted, which a
        // serde rewrite would churn.
        let sidecar: Vec<serde_json::Value> = gold
            .iter()
            .map(|case| serde_json::json!({"src": case["src"], "renders": case["renders"]}))
            .collect();
        std::fs::write(
            "/tmp/markdown-gold-new.json",
            serde_json::to_string_pretty(&sidecar).unwrap(),
        )
        .unwrap();
        return;
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn token_streams_match_marked_on_fuzzed_documents() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../fixtures/markdown-fuzz-gold.json")).unwrap();
    let mut failures = Vec::new();
    for case in &cases {
        let src = case["src"].as_str().unwrap();
        let lexed = lex(&src.replace('\t', "   "));
        let got = tokens_to_json(&lexed.tokens);
        let links = links_to_json(&lexed.links);
        if got != case["tokens"].as_str().unwrap() || links != case["links"].as_str().unwrap() {
            failures.push(format!(
                "{src:?}\n  want {} {}\n  got  {got} {links}",
                case["tokens"].as_str().unwrap(),
                case["links"].as_str().unwrap()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} mismatches:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
