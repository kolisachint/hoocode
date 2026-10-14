//! The grammar index (`src/grammar_index.rs`) and the per-language chunks
//! (`data/grammars/`) are generated from `data/hljs-grammars.json`, the dump
//! of highlight.js's grammars. `index_matches_the_dump` fails when they
//! drift from it. Regenerate after changing the dump with:
//!
//!     cargo test -p hoocode-tui-highlight --test it grammar_index::regenerate -- --ignored
//!
//! A chunk holds the node records reachable from one language, keyed by the
//! dump's node ids (`{"root": id, "nodes": {"id": {"frozen", "props"}}}`).

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

const DUMP: &str = include_str!("../../data/hljs-grammars.json");

const HEADER: &str = "//! Generated from `data/hljs-grammars.json` by `tests/it/grammar_index.rs`\n\
//! (`grammar_index::regenerate`). Do not edit by hand; `index_matches_the_dump` fails on drift.\n\n";

struct Generated {
    index_rs: String,
    /// Files under the crate root, with their contents.
    files: Vec<(String, String)>,
}

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn collect_refs(value: &Value, out: &mut Vec<u64>) {
    match value {
        Value::Object(map) => match map.get("$ref").and_then(Value::as_u64) {
            Some(id) => out.push(id),
            None => map.values().for_each(|v| collect_refs(v, out)),
        },
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

/// The chunk for `root`: every node reachable from it.
fn chunk(raw_nodes: &[Value], root: u64) -> String {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if seen.insert(id) {
            collect_refs(&raw_nodes[id as usize], &mut stack);
        }
    }
    let mut nodes = Map::new();
    for id in &seen {
        nodes.insert(id.to_string(), raw_nodes[*id as usize].clone());
    }
    format!("{}\n", json!({ "root": root, "nodes": nodes }))
}

fn generate() -> Generated {
    let doc: Value = serde_json::from_str(DUMP).expect("dump parses");
    let raw_nodes = doc["nodes"].as_array().expect("nodes");
    let languages = doc["languages"].as_object().expect("languages");
    let aliases = doc["aliases"].as_object().expect("aliases");

    let mut index = String::from(HEADER);
    let mut files = Vec::new();
    index.push_str("#[rustfmt::skip]\npub const LANGUAGES: &[(&str, u64, &str)] = &[\n");
    for (name, value) in languages {
        let root = value["$ref"].as_u64().expect("language is a $ref");
        writeln!(
            index,
            "    ({name:?}, {root}, include_str!(\"../data/grammars/{name}.json\")),"
        )
        .unwrap();
        files.push((format!("data/grammars/{name}.json"), chunk(raw_nodes, root)));
    }
    index.push_str("];\n\n#[rustfmt::skip]\npub const ALIASES: &[(&str, &str)] = &[\n");
    for (alias, target) in aliases {
        let target = target.as_str().expect("alias target");
        writeln!(index, "    ({alias:?}, {target:?}),").unwrap();
    }
    index.push_str("];\n\n");
    index.push_str(
        "pub const SYSTEM_SYMBOLS: &str = include_str!(\"../data/grammars/system-symbols.json\");\n",
    );
    files.push((
        "data/grammars/system-symbols.json".to_string(),
        format!("{}\n", doc["systemSymbols"]),
    ));
    Generated {
        index_rs: index,
        files,
    }
}

/// Regenerates `src/grammar_index.rs` and `data/grammars/` from the dump.
#[test]
#[ignore = "writes the generated grammar index; run with --ignored"]
fn regenerate() {
    let generated = generate();
    fs::write(
        crate_dir().join("src/grammar_index.rs"),
        &generated.index_rs,
    )
    .unwrap();
    let dir = crate_dir().join("data/grammars");
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let keep = generated
            .files
            .iter()
            .any(|(p, _)| crate_dir().join(p) == path);
        if !keep {
            fs::remove_file(path).unwrap();
        }
    }
    for (path, contents) in &generated.files {
        fs::write(crate_dir().join(path), contents).unwrap();
    }
}

/// `supports_language` answers for every name and alias in the dump, and for
/// nothing else.
#[test]
fn supports_language_matches_the_dump() {
    let doc: Value = serde_json::from_str(DUMP).expect("dump parses");
    let names = doc["languages"].as_object().expect("languages").keys();
    let aliases = doc["aliases"].as_object().expect("aliases").keys();
    for name in names.chain(aliases) {
        assert!(
            hoocode_tui_highlight::supports_language(name),
            "{name} is in the dump but not supported"
        );
        assert!(hoocode_tui_highlight::supports_language(
            &name.to_uppercase()
        ));
    }
    assert!(!hoocode_tui_highlight::supports_language(
        "no-such-language"
    ));
    assert!(!hoocode_tui_highlight::supports_language(""));
}

/// Every name and alias `supports_language` answers for loads: `highlight`
/// finds its grammar and compiles it, so the index never promises a language
/// the chunks cannot provide.
#[test]
fn every_supported_name_loads() {
    let doc: Value = serde_json::from_str(DUMP).expect("dump parses");
    let names = doc["languages"].as_object().expect("languages").keys();
    let aliases = doc["aliases"].as_object().expect("aliases").keys();
    for name in names.chain(aliases) {
        assert!(
            hoocode_tui_highlight::highlight_tree("x", name, true).is_some(),
            "{name} is supported but highlight cannot load it"
        );
    }
}

/// The index and chunks are exactly what the dump generates.
#[test]
fn index_matches_the_dump() {
    let generated = generate();
    let mut drift = Vec::new();
    let committed =
        fs::read_to_string(crate_dir().join("src/grammar_index.rs")).unwrap_or_default();
    if committed != generated.index_rs {
        drift.push("src/grammar_index.rs".to_string());
    }
    for (path, contents) in &generated.files {
        let committed = fs::read_to_string(crate_dir().join(path)).unwrap_or_default();
        if &committed != contents {
            drift.push(path.clone());
        }
    }
    let expected: HashSet<String> = generated.files.iter().map(|(p, _)| p.clone()).collect();
    for entry in fs::read_dir(crate_dir().join("data/grammars")).unwrap() {
        let rel = format!(
            "data/grammars/{}",
            entry.unwrap().file_name().to_string_lossy()
        );
        if !expected.contains(&rel) {
            drift.push(format!("{rel} (stale)"));
        }
    }
    assert!(
        drift.is_empty(),
        "grammar index drifted from data/hljs-grammars.json: {drift:?}\n\
         regenerate with: cargo test -p hoocode-tui-highlight --test it grammar_index::regenerate -- --ignored"
    );
}
