# hoocode-code-tool-search

The `CodeSearch` tool and the runtime half of the search pipeline.

The lexical leg runs ripgrep's invocation (`--hidden --no-require-git
--ignore-case --sort path --glob '!**/.git/**'`) with ripgrep's own libraries
(`ignore`, `grep-searcher`, `grep-regex`), so no `rg` binary is needed. Hits go through the
grep→chunk adapter, reciprocal-rank fusion, the deterministic reranker, span merging and the
token-budgeted context assembler. Semantic and hybrid modes take an `EmbsearchService`; the
embsearch daemon client itself is not implemented yet, so without one every mode resolves to lexical.
