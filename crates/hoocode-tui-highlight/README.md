# hoocode-tui-highlight

Syntax highlighting as the pinned hoocode does it: a port of highlight.js 10.7.3's engine
(mode compiler, multi-regex scanner, keyword and sub-language handling, auto-detection)
running highlight.js's own grammars. `data/hljs-grammars.json` holds them, dumped from the
pin's node_modules by `archive/migration/tools/goldens/hljs-grammars.mjs`.

`highlight` colors the token tree the way `cli-highlight` does (a caller theme, then its
`DEFAULT_THEME`); `hoocode-code-tui-theme` installs it as the default `CodeHighlighter`.
`tests/highlight_gold.rs` checks the output byte-for-byte against the pin's cli-highlight
(`archive/migration/tools/goldens/highlight.mjs`).
