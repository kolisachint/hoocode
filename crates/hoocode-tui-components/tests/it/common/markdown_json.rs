//! `JSON.stringify` of `marked`'s tokens, for the markdown golden corpus.
//! Moved out of the lexer (N12): only the golden test prints tokens.

#![allow(dead_code)]

use hoocode_tui_components::markdown::lexer::{Align, LinkDef, TableCell, Token};

// ---- JSON dump -------------------------------------------------------------
//
// `JSON.stringify` of `marked`'s tokens with keys sorted and `undefined`
// fields dropped — the form the golden corpus stores, so a token stream can
// be compared with the real `marked` byte for byte.

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_obj(fields: Vec<(&str, String)>) -> String {
    let mut fields = fields;
    fields.sort_by(|a, b| a.0.cmp(b.0));
    let body: Vec<String> = fields
        .into_iter()
        .map(|(k, v)| format!("{}:{v}", json_str(k)))
        .collect();
    format!("{{{}}}", body.join(","))
}

fn json_opt_str(s: &Option<String>) -> String {
    s.as_deref().map_or_else(|| "null".to_string(), json_str)
}

fn json_align(a: Option<Align>) -> String {
    match a {
        None => "null".into(),
        Some(Align::Left) => json_str("left"),
        Some(Align::Center) => json_str("center"),
        Some(Align::Right) => json_str("right"),
    }
}

fn json_cell(c: &TableCell) -> String {
    json_obj(vec![
        ("align", json_align(c.align)),
        ("header", c.header.to_string()),
        ("text", json_str(&c.text)),
        ("tokens", tokens_to_json(&c.tokens)),
    ])
}

/// A token as `JSON.stringify` (sorted keys) prints `marked`'s object.
pub fn token_to_json(t: &Token) -> String {
    let mut f: Vec<(&str, String)> = vec![
        ("type", json_str(t.kind.as_str())),
        ("raw", json_str(&t.raw)),
    ];
    if let Some(text) = &t.text {
        f.push(("text", json_str(text)));
    }
    if let Some(tokens) = &t.tokens {
        f.push(("tokens", tokens_to_json(tokens)));
    }
    if let Some(d) = t.depth {
        f.push(("depth", d.to_string()));
    }
    if let Some(lang) = &t.lang {
        f.push(("lang", json_str(lang)));
    }
    if t.indented {
        f.push(("codeBlockStyle", json_str("indented")));
    }
    if let Some(href) = &t.href {
        f.push(("href", json_str(href)));
    }
    if let Some(title) = &t.title {
        f.push(("title", json_opt_str(title)));
    }
    for (k, v) in [
        ("block", t.block),
        ("pre", t.pre),
        ("inLink", t.in_link),
        ("inRawBlock", t.in_raw_block),
        ("escaped", t.escaped),
    ] {
        if let Some(v) = v {
            f.push((k, v.to_string()));
        }
    }
    if let Some(list) = &t.list {
        f.push(("ordered", list.ordered.to_string()));
        f.push((
            "start",
            list.start.map_or_else(|| json_str(""), |n| n.to_string()),
        ));
        f.push(("loose", list.loose.to_string()));
        let items: Vec<String> = list
            .items
            .iter()
            .map(|i| {
                let mut g = vec![
                    ("type", json_str("list_item")),
                    ("raw", json_str(&i.raw)),
                    ("task", i.task.to_string()),
                    ("loose", i.loose.to_string()),
                    ("text", json_str(&i.text)),
                    ("tokens", tokens_to_json(&i.tokens)),
                ];
                if let Some(c) = i.checked {
                    g.push(("checked", c.to_string()));
                }
                json_obj(g)
            })
            .collect();
        f.push(("items", format!("[{}]", items.join(","))));
    }
    if let Some(table) = &t.table {
        let align: Vec<String> = table.align.iter().map(|a| json_align(*a)).collect();
        f.push(("align", format!("[{}]", align.join(","))));
        let header: Vec<String> = table.header.iter().map(json_cell).collect();
        f.push(("header", format!("[{}]", header.join(","))));
        let rows: Vec<String> = table
            .rows
            .iter()
            .map(|r| {
                format!(
                    "[{}]",
                    r.iter().map(json_cell).collect::<Vec<_>>().join(",")
                )
            })
            .collect();
        f.push(("rows", format!("[{}]", rows.join(","))));
    }
    json_obj(f)
}

/// A token list as `JSON.stringify` (sorted keys) prints it.
pub fn tokens_to_json(tokens: &[Token]) -> String {
    format!(
        "[{}]",
        tokens
            .iter()
            .map(token_to_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// `Object.entries(tokens.links)` with sorted keys, as JSON.
pub fn links_to_json(links: &[(String, LinkDef)]) -> String {
    let entries: Vec<String> = links
        .iter()
        .map(|(tag, def)| {
            let mut f = vec![("href", json_str(&def.href))];
            if let Some(title) = &def.title {
                f.push(("title", json_str(title)));
            }
            format!("[{},{}]", json_str(tag), json_obj(f))
        })
        .collect();
    format!("[{}]", entries.join(","))
}
