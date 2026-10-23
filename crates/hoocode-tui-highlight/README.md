# hoocode-tui-highlight

Syntax highlighting: a port of the highlight.js 10.7.3 engine
(mode compiler, multi-regex scanner, keyword and sub-language handling, auto-detection)
running highlight.js's own grammars. `data/hljs-grammars.json` holds them, dumped from
highlight.js's `node_modules` by `archive/migration/tools/goldens/hljs-grammars.mjs`. `src/grammar_index.rs`
and `data/grammars/` are generated from that dump (`grammar_index::regenerate`, see
`tests/it/grammar_index.rs`): the name index answers `supports_language`, and each language
loads only its own chunk.

`highlight` colors the token tree the way `cli-highlight` does (a caller theme, then its
`DEFAULT_THEME`); `hoocode-code-tui-theme` installs it as the default `CodeHighlighter`.
`tests/highlight_gold.rs` checks the output byte-for-byte against `cli-highlight`
(`archive/migration/tools/goldens/highlight.mjs`).
