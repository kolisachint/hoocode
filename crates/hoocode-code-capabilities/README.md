# hoocode-code-capabilities

The in-process index behind `DocSearch`: what this session can do, searchable by
describing it. Entries are `{kind, name, description, source}` with kind `skill`,
`subagent` or `plugin`.

- `index.rs`: code-aware tokenization (camelCase, snake_case, kebab-case, plural folding)
  and a BM25 index. Exact name matches rank first.
- `producers.rs`: builds entries from the loaded skills and subagent definitions.
  Plugins are a TODO hook; the MCP and plugin cards add them there.

Design: [semantic-search.md](../../docs/design/semantic-search.md) (Part A).
