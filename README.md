<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/hoocode.svg">
    <img alt="HooCode" src="assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Deterministic terminal coding agent.</p>

---

HooCode is a deterministic terminal coding agent — four scoped modes
(Ask · Plan · Build · Debug), 25+ providers, hybrid search, and one-click
plugins in a single binary. Nothing applies without your approval.

**macOS / Linux**

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

**Windows**

```powershell
irm https://kolisachint.github.io/hoocode/install.ps1 | iex
```

**npm** (needs Node ≥ 20)

```bash
npm install -g @kolisachint/hoocode-agent
```

Then:

```bash
hoocode --help
```

The one-click installers need no root, install into `~/.hoocode`, and pre-seed
the optional Rust helpers (`fd`, `rg`, `embsearch`, `webtools`, `voicetools`) so
your first session is fast even offline. Full details, including the standalone
archives and container setups, are in **[Install](docs/install.md)**.

## Demo

https://github.com/user-attachments/assets/3fd55892-c4be-4d78-86ce-d14cbe7be644

## Docs

Full documentation: **[kolisachint.github.io/hoocode](https://kolisachint.github.io/hoocode/)**

- **[Product](docs/product.md)** — features, modes, tools, and extensibility
- **[Install](docs/install.md)** — installation and building from source
- **[Contributing](CONTRIBUTING.md)** — how to get a change in (newcomers welcome)
- **[AGENTS.md](AGENTS.md)** — project-specific rules for humans and agents

## Packages

| Package | Description |
|---------|-------------|
| **[@kolisachint/hoocode-agent](packages/coding-agent)** | Interactive coding agent CLI (`hoocode` / `hoo`) |
| **[@kolisachint/hoocode-agent-core](packages/agent)** | Agent runtime with tool calling and state management |
| **[@kolisachint/hoocode-ai](packages/ai)** | Unified multi-provider LLM API (OpenAI, Anthropic, Google, …) |
| **[@kolisachint/hoocode-tui](packages/tui)** | Terminal UI library with differential rendering |

## Contributing

Contributions are welcome — bug reports, ideas, docs fixes, a new tip, a whole
provider. No approval needed before opening a PR.

Genuinely easy places to start:

- **[Add a tip](packages/coding-agent/src/modes/interactive/tips.ts)** — one row
  in an array. If you learned a trick the hard way, nobody else should have to.
- **[Fix a doc](packages/coding-agent/docs)** — if something confused you, it
  will confuse the next person.
- **[Write an example extension](packages/coding-agent/examples/extensions)** —
  self-contained, no core changes.
- **[good first issue](https://github.com/kolisachint/hoocode/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)**
  — say "I'll take this" and it is yours.

See **[CONTRIBUTING.md](CONTRIBUTING.md)**. Questions are welcome as
[issues](https://github.com/kolisachint/hoocode/issues/new/choose); for
collaboration or anything that does not fit one, tag
[@kolisachint on X](https://x.com/kolisachint).

> ⭐ **If HooCode is useful to you, [star the
> repo](https://github.com/kolisachint/hoocode).** One click, and it is the
> single cheapest way to help other people find it.

## Credits

HooCode is developed independently, but it began from the [`pi-mono`](https://github.com/earendil-works/pi-mono) project (originally [`badlogic/pi-mono`](https://github.com/badlogic/pi-mono)) by **Mario Zechner** ([@badlogicgames](https://github.com/badlogic)) and still contains work derived from it. The upstream project is MIT-licensed and all original copyright is preserved in [LICENSE](LICENSE). Huge thanks to Mario and the upstream contributors — without their work, HooCode would not exist.

## Related

A small set of offline-first tools for agents:

- **[embeddingsearchtools](https://github.com/kolisachint/embeddingsearchtools)** —
  semantic + BM25 retrieval, HNSW from scratch (Rust)
- **[webtools](https://github.com/kolisachint/webtools)** — token-efficient web
  fetch and search (Rust)
- **[voicetools](https://github.com/kolisachint/voicetools)** — offline speech
  recognition, mic to stdout (Rust)

Built by [Sachin Koli](https://kolisachint.github.io).

## License

MIT — see [LICENSE](LICENSE).
