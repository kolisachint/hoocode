<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/hoocode.svg">
    <img alt="HooCode" src="assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Deterministic terminal coding agent.</p>

## Why HooCode?

Most AI coding tools are black boxes — you type a request and hope the output is correct. HooCode takes a different approach:

| | HooCode | Other AI editors |
|---|---|---|
| **Approval gates** | Every file edit and shell command requires your sign-off | Changes land silently |
| **Mode-driven focus** | Separate Ask / Plan / Build / Debug modes keep the agent on-task | Single chat interface does everything |
| **Provider flexibility** | 25+ LLM providers — Claude, GPT, Gemini, Groq, Ollama… — swappable in one config line | Locked to one vendor |
| **Extensible** | MCP servers, TypeScript extensions, and custom profiles per project | Closed plugin systems |
| **Runs anywhere** | Compiles to a self-contained binary; no Node.js required at runtime | Requires IDE or cloud subscription |

**The core workflow:**

1. **Ask** — read-only Q&A about your codebase. The agent never writes.
2. **Plan** — the agent explores your repo and drafts `.hoocode/plan.md`. You review and approve before anything changes.
3. **Build** — implements the approved plan, prompting you before each edit or shell command.
4. **Debug** — root-cause analysis that explains without touching files.

```bash
hoocode /mode plan   # draft a plan first
hoocode /approve     # execute it when you're ready
```

---

## Demo

See all features in action → **[`assets/demo.html`](assets/demo.html)**

---

## Credits

HooCode is a fork of the upstream [`pi-mono`](https://github.com/earendil-works/pi-mono) project (originally [`badlogic/pi-mono`](https://github.com/badlogic/pi-mono)) by **Mario Zechner** ([@badlogicgames](https://github.com/badlogic)). The upstream project is MIT-licensed and all original copyright is preserved in [LICENSE](LICENSE). Huge thanks to Mario and the upstream contributors — without their work, this fork would not exist.

## Packages

| Package | Description |
|---------|-------------|
| **[@kolisachint/hoocode-agent](packages/coding-agent)** | Interactive coding agent CLI (`hoocode` / `hoo`) |
| **[@kolisachint/hoocode-agent-core](packages/agent)** | Agent runtime with tool calling and state management |
| **[@kolisachint/hoocode-ai](packages/ai)** | Unified multi-provider LLM API (OpenAI, Anthropic, Google, …) |
| **[@kolisachint/hoocode-tui](packages/tui)** | Terminal UI library with differential rendering |

## Install

```bash
npm install -g @kolisachint/hoocode-agent
hoocode --help
```

## Development

```bash
npm install          # Install all dependencies
npm run build        # Build all packages
npm run check        # Lint, format, and type check
./test.sh            # Run tests (skips LLM-dependent tests without API keys)
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines and [AGENTS.md](AGENTS.md) for project-specific rules (for both humans and agents).

## License

MIT — see [LICENSE](LICENSE).
