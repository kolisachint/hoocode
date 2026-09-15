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

```bash
npm install -g @kolisachint/hoocode-agent
hoocode --help
```

## Demo

https://github.com/user-attachments/assets/a6dcf3d5-ea4d-42ee-afdd-45d8df3938ea

## Docs

- **[Product](docs/product.md)** — features, modes, tools, and extensibility
- **[Install](docs/install.md)** — installation and building from source
- **[Contributing](CONTRIBUTING.md)** — contribution guidelines
- **[AGENTS.md](AGENTS.md)** — project-specific rules for humans and agents

## Packages

| Package | Description |
|---------|-------------|
| **[@kolisachint/hoocode-agent](packages/coding-agent)** | Interactive coding agent CLI (`hoocode` / `hoo`) |
| **[@kolisachint/hoocode-agent-core](packages/agent)** | Agent runtime with tool calling and state management |
| **[@kolisachint/hoocode-ai](packages/ai)** | Unified multi-provider LLM API (OpenAI, Anthropic, Google, …) |
| **[@kolisachint/hoocode-tui](packages/tui)** | Terminal UI library with differential rendering |

## Credits

HooCode is developed independently, but it began from the [`pi-mono`](https://github.com/earendil-works/pi-mono) project (originally [`badlogic/pi-mono`](https://github.com/badlogic/pi-mono)) by **Mario Zechner** ([@badlogicgames](https://github.com/badlogic)) and still contains work derived from it. The upstream project is MIT-licensed and all original copyright is preserved in [LICENSE](LICENSE). Huge thanks to Mario and the upstream contributors — without their work, HooCode would not exist.

## License

MIT — see [LICENSE](LICENSE).
