# Development Rules

## Repo Map

Before searching, check these maps:

- `docs/package-map.md` - packages, dependency graph, build order, and a "where is X" index
- `docs/npm-packages.md` - install/build/test mechanics, the src-vs-dist resolution split, and common traps
- `docs/ui-map.md` - tui library and interactive-mode components grouped by purpose
- `docs/bun-migration.md` - record of the completed npm -> bun migration and the post-migration rules
- `docs/agent-spec-tree-map.md` - on-disk agent-spec surfaces, their standard status, and what hoocode scans/supports

## Recent Changes

- **TUI vertical rhythm**: one blank line separates two blocks, and it is
  never paid for twice. A block's separator is the `Spacer(1)` before it (so
  the block itself takes `paddingY: 0`), a `DynamicBorder` needs no blank
  beside it, and nothing pads its own bottom edge. Rules and reference cases:
  `docs/ui-map.md` -> "Vertical rhythm".
- **`grep`/`find`/`ls` tools removed**: `search` is the only dedicated
  code-discovery tool; exact matching lines, counts, and raw directory listings
  are a shell job through `bash`. Claude Code's `Grep`/`Glob`/`Find` all
  normalize to `search`; `LS` has no counterpart and is dropped with a
  diagnostic. `TOOL_FACTORIES` is 7 built-ins.
- **Browser + document tools removed**: `browser_*`, `Doc*`, `filetools-shared.ts`, and their `--enable-browsertools`/`--enable-browser-live-preview`/`--enable-filetools` flags are gone; `TOOL_FACTORIES` shrank accordingly.
- **Providers removed**: `amazon-bedrock`, `mistral`, `cloudflare-workers-ai`, `cloudflare-ai-gateway`. `mistralai/*` ids via OpenRouter still work.
- **MCP Standard Config Support**: Now reads standard `mcp.json` format from:
  - `~/.agents/mcp.json` (user-level)
  - `.agents/mcp.json` (project-level)
  - `~/.config/claude/mcp.json` (Claude Desktop)
  - Existing per-server JSON files in `mcp-servers/` still work as fallback
  - First-wins deduplication across all sources

## Prompt token surface

Everything in the system prompt and in an active tool's schema is re-sent on
**every** request. Measure before and after any change to either:

```bash
hoocode --print-token-surface   # src/main.ts -> measurePromptSurface (core/light.ts)
```

It reports the assembled system prompt plus each active tool's serialized
`{name, description, parameters}`, estimated at chars/4. Treat it as a floor:
providers add their own envelope, and schema JSON tokenizes worse than prose.

Current baseline (default tools, no context files): **~3,380 tokens** — ~1,350
system prompt, ~2,030 tool schemas. (Dropping `grep`/`find`/`ls` took ~760 off
the old ~4,140: ~680 of schema and ~80 of prompt.) Adding this repo's
`AGENTS.md` as a context file costs another ~3,600, which makes it the single
largest line item.

**A tool's guidance belongs in exactly one place.** A tool contributes text
through four channels, and it is easy to pay for the same sentence twice:

| Channel | Ships in | Use it for |
|---|---|---|
| `description` | tool schema | what the tool does, its contract, its limits |
| `parameters` descriptions | tool schema | per-argument semantics |
| `promptSnippet` | system prompt, `Available tools:` | one line, for picking between tools |
| `promptGuidelines` | system prompt, `Guidelines:` | behavior the schema cannot express |

Rules that follow from that:

- **Never restate a parameter's semantics in `promptGuidelines`.** Both ship on
  every turn. `edit` carried five guidelines, four of which repeated its own
  `oldText`/`edits`/`replaceAll` descriptions verbatim.
- **Never restate a cross-tool routing rule in a tool.** `buildSystemPrompt`
  already emits the canonical search-vs-bash and file-exploration guidelines
  whenever the relevant tools are registered. `search`, `read`, and `bash` each
  carried their own copy; that rule was shipping three times.
- **Don't spell out what the schema already encodes.** An enum of `f`/`d`/`l`
  does not need prose naming all three.
- **Examples are the most expensive thing in a schema.** One is usually enough;
  the retired `find` tool's `pattern` had three and was the priciest built-in
  schema in the repo.
- Keep per-tool `promptGuidelines` under ~200 chars, and prefer adding to a
  tool's `description` over adding a guideline — descriptions at least stay next
  to the contract they describe.

When adding a tool, budget it: a built-in should land under ~250 tokens
serialized. Anything materially above that needs a reason, and the
when-to-use guidance probably belongs in a system-prompt block instead — that
is why `Task` keeps its mechanics in `description` and its 2.5KB of
when-to-delegate guidance in `buildTaskMainPrompt`.

## Where a capability belongs

Before writing a capability as code, decide which of the four homes it wants.
The mistake this rule exists to prevent is shipping prose as TypeScript: a
prompt compiled into a string constant is not type-checked, not editable as
prose, and often resident on every turn.

| Home | Always-on cost | Body loaded | Isolation |
|---|---|---|---|
| Tool | full schema, every turn | never (always resident) | no |
| Agent | summarized `description` in `<available_agents>` | on dispatch | **yes** — own context, enforced tool allowlist, own model tier |
| Skill | `<name>` + `<description>` + `<location>` | on `read` | no |
| Slash command | **nothing** | on invocation | no |

The rule that follows:

> **Agent when you need isolation. Skill when you need instruction. Command
> when a human triggers it. Tool only when the model must call it with
> structured arguments.**

Skills and agents cost about the same per turn, so "convert an agent to a skill"
never saves tokens — pick between them on isolation, not price. A subagent is a
whole extra model context that cannot see the parent conversation; a skill is a
`read` into the context you already have.

Two corollaries:

- **Prose lives in a file, not a string constant.** `templates/prompts/*.md`
  and `templates/modes/*/system.md` are embedded at build time by
  `scripts/embed-templates.mjs`. A prompt with no interpolation and no
  branching belongs there, and so does one with a single substitution slot — use
  a `{{TOKEN}}` placeholder (`{{PLAN_PATH}}`, `{{BACKGROUND_GUIDANCE}}`) and
  keep the variant text in its own file. Only prompts that need real logic stay
  in the module that builds them. Keeping a second hand-written copy in `.ts` is
  how the mode prompts silently diverged from the ones `/init` scaffolds.
- **Craft guidance is not a tool contract.** A tool's `promptGuidelines` are
  resident on every turn; a how-to-do-this-well guide is not a contract and
  does not belong there. Ship it as a built-in skill — add the directory under
  `templates/skills/` and an entry to `BUILTIN_SKILLS` in
  `core/builtin-skills.ts` — and point the tool at it in one line. Give it a
  `gate` unless it is useful in every session: a skill costs its description
  every turn, so one that only makes sense alongside a feature rides that
  feature's switch.

## Conversational Style

- Keep answers short and concise
- No emojis in commits, issues, PR comments, or code
- No fluff or cheerful filler text
- Technical prose only, be kind but direct (e.g., "Thanks @user" not "Thanks so much @user!")

## Code Quality

- Read files in full before making wide-ranging changes, before editing files you have not already fully inspected, and when the user asks you to investigate or audit something. Do not rely only on search snippets for broad changes.
- No `any` types unless absolutely necessary
- Check node_modules for external API type definitions instead of guessing
- **NEVER use inline imports** - no `await import("./foo.js")`, no `import("pkg").Type` in type positions, no dynamic imports for types. Always use standard top-level imports.
- NEVER remove or downgrade code to fix type errors from outdated dependencies; upgrade the dependency instead
- Always ask before removing functionality or code that appears to be intentional
- Do not preserve backward compatibility unless the user explicitly asks for it
- Never hardcode key checks with, eg. `matchesKey(keyData, "ctrl+x")`. All keybindings must be configurable. Add default to matching object (`DEFAULT_EDITOR_KEYBINDINGS` or `DEFAULT_APP_KEYBINDINGS`)
- NEVER modify `packages/ai/src/models.generated.ts` directly. Update `packages/ai/scripts/generate-models.ts` instead.

## Commands

- After code changes (not documentation changes): `bun run check` (get full output, no tail). Fix all errors, warnings, and infos before committing.
- Note: `bun run check` does not run tests.
- NEVER run: `bun run dev`, `bun run build`, `bun test`
- Package manager: bun is the toolchain (`packageManager: bun@1.3.13`). `bun.lock`
  is the single authoritative lockfile. `bunfig.toml` pins `linker = "hoisted"`,
  so plain `bun install` yields an npm-compatible flat tree; never use bun's
  isolated linker. After any `bun install`, run `bun run check`; recover a broken
  tree with `bun install`. npm is used only for `npm publish` during releases.
  See `docs/bun-migration.md` for the completed migration.
- Only run specific tests if user instructs: `npx tsx ../../node_modules/vitest/dist/cli.js --run test/specific.test.ts`
- Run tests from the package root, not the repo root.
- If you create or modify a test file, you MUST run it and iterate on the test or the implementation until it passes.
- For `packages/coding-agent/test/suite/`, use `test/suite/harness.ts` plus the faux provider. Do not use real provider APIs, real API keys, or paid tokens.
- Put issue-specific regressions under `packages/coding-agent/test/suite/regressions/` and name them `<issue-number>-<short-slug>.test.ts`.
- NEVER commit unless user asks

## Contribution Gate

- New issues from new contributors are auto-closed by `.github/workflows/issue-gate.yml`
- New PRs from new contributors without PR rights are auto-closed by `.github/workflows/pr-gate.yml`
- Maintainer approval comments are handled by `.github/workflows/approve-contributor.yml`
- Maintainers review auto-closed issues daily
- Issues that do not meet the quality bar in `CONTRIBUTING.md` are not reopened and do not receive a reply
- `lgtmi` approves future issues
- `lgtm` approves future issues and rights to submit PRs

When creating issues:

- Add `pkg:*` labels to indicate which package(s) the issue affects
  - Available labels: `pkg:agent`, `pkg:ai`, `pkg:coding-agent`, `pkg:tui`
- If an issue spans multiple packages, add all relevant labels

When posting issue/PR comments:

- Write the full comment to a temp file and use `gh issue comment --body-file` or `gh pr comment --body-file`
- Never pass multi-line markdown directly via `--body` in shell commands
- Preview the exact comment text before posting
- Post exactly one final comment unless the user explicitly asks for multiple comments
- If a comment is malformed, delete it immediately, then post one corrected comment
- Keep comments concise, technical, and in the user's tone

When closing issues via commit:

- Include `fixes #<number>` or `closes #<number>` in the commit message
- This automatically closes the issue when the commit is merged

## PR Workflow

- Analyze PRs without pulling locally first
- If the user approves: create a feature branch, pull PR, rebase on main, apply adjustments, commit, merge into main, push, close PR, and leave a comment in the user's tone
- You never open PRs yourself. We work in feature branches until everything is according to the user's requirements, then merge into main, and push.

### Slash commands

- `/pr [patch|minor|major]` - opens a PR on a feature branch. With a bump it labels the PR `npm:<bump>` so the merge-release workflow publishes on merge; without one it only opens a PR (no publish). Defined in `.agents/commands/pr.md`.
- `/postmerge [pr-number]` - after a PR merges, verifies CI, npm publish, version bump, tag and GitHub release, then returns to an up-to-date `main`. Defined in `.agents/commands/postmerge.md`.
- Slash-command definitions live in `.agents/commands/` (also read from `.hoocode/commands/`, `.claude/commands/`, and the user-level equivalents). Scaffold a new one with `/new-command <name>`.

## Testing hoocode Interactive Mode with tmux

To test hoocode's TUI in a controlled terminal environment:

```bash
# Create tmux session with specific dimensions
tmux new-session -d -s hoocode-test -x 80 -y 24

# Start hoocode from source
tmux send-keys -t hoocode-test "cd <repo-root> && ./hoocode-test.sh" Enter

# Wait for startup, then capture output
sleep 3 && tmux capture-pane -t hoocode-test -p

# Send input
tmux send-keys -t hoocode-test "your prompt here" Enter

# Send special keys
tmux send-keys -t hoocode-test Escape
tmux send-keys -t hoocode-test C-o  # ctrl+o

# Cleanup
tmux kill-session -t hoocode-test
```

## Changelog

Location: `packages/*/CHANGELOG.md` (each package has its own)

### Format

Use these sections under `## [Unreleased]`:

- `### Breaking Changes` - API changes requiring migration
- `### Added` - New features
- `### Changed` - Changes to existing functionality
- `### Fixed` - Bug fixes
- `### Removed` - Removed features

### Rules

- Before adding entries, read the full `[Unreleased]` section to see which subsections already exist
- New entries ALWAYS go under `## [Unreleased]` section
- Append to existing subsections (e.g., `### Fixed`), do not create duplicates
- NEVER modify already-released version sections (e.g., `## [0.12.2]`)
- Each version section is immutable once released

### Attribution

- **Internal changes (from issues)**: `Fixed foo bar ([#123](https://github.com/kolisachint/hoocode/issues/123))`
- **External contributions**: `Added feature X ([#456](https://github.com/kolisachint/hoocode/pull/456) by [@username](https://github.com/username))`

## Adding a New LLM Provider (packages/ai)

Adding a new provider requires changes across multiple files:

### 1. Core Types (`packages/ai/src/types.ts`)

- Add API identifier to `Api` type union (e.g., `"bedrock-converse-stream"`)
- Create options interface extending `StreamOptions`
- Add mapping to `ApiOptionsMap`
- Add provider name to `KnownProvider` type union

### 2. Provider Implementation (`packages/ai/src/providers/`)

Create provider file exporting:

- `stream<Provider>()` function returning `AssistantMessageEventStream`
- `streamSimple<Provider>()` for `SimpleStreamOptions` mapping
- Provider-specific options interface
- Message/tool conversion functions
- Response parsing emitting standardized events (`text`, `tool_call`, `thinking`, `usage`, `stop`)

### 3. Provider Exports and Lazy Registration

- Add a package subpath export in `packages/ai/package.json` pointing at `./dist/providers/<provider>.js`
- Add `export type` re-exports in `packages/ai/src/index.ts` for provider option types that should remain available from the root entry
- Register the provider in `packages/ai/src/providers/register-builtins.ts` via lazy loader wrappers, do not statically import provider implementation modules there
- Add credential detection in `packages/ai/src/env-api-keys.ts`

### 4. Model Generation (`packages/ai/scripts/generate-models.ts`)

- Add logic to fetch/parse models from provider source
- Map to standardized `Model` interface

### 5. Tests (`packages/ai/test/`)

- Always add the provider to `stream.test.ts` with at least one representative model, even if it reuses an existing API implementation such as `openai-completions`.
- Add the provider to the broader provider matrix where applicable: `tokens.test.ts`, `abort.test.ts`, `empty.test.ts`, `context-overflow.test.ts`, `unicode-surrogate.test.ts`, `tool-call-without-result.test.ts`, `image-tool-result.test.ts`, `total-tokens.test.ts`, `cross-provider-handoff.test.ts`.
- For `cross-provider-handoff.test.ts`, add at least one provider/model pair. If the provider exposes multiple model families (for example GPT and Claude), add at least one pair per family.
- For non-standard auth, create utility (e.g., `bedrock-utils.ts`) with credential detection.

### 6. Coding Agent (`packages/coding-agent/`)

- `src/core/model-resolver.ts`: Add default model ID to `defaultModelPerProvider`
- `src/core/provider-display-names.ts`: Add API-key login display name so `/login` and related UI show the provider for built-in API-key auth.
- `src/cli/args.ts`: Add env var documentation
- `README.md`: Add provider setup instructions
- `docs/providers.md`: Add setup instructions, env var, and `auth.json` key

### 7. Documentation

- `packages/ai/README.md`: Add to providers table, document options/auth, add env vars
- `packages/ai/CHANGELOG.md`: Add entry under `## [Unreleased]`

## Releasing

**Lockstep versioning**: All packages always share the same version number. Every release updates all packages together.

**Version semantics** (no major releases):

- `patch`: Bug fixes and new features
- `minor`: API breaking changes

### Steps

1. **Update CHANGELOGs**: Ensure all changes since last release are documented in the `[Unreleased]` section of each affected package's CHANGELOG.md

2. **Run release script**:
   ```bash
   npm run release:patch    # Fixes and additions
   npm run release:minor    # API breaking changes
   ```

The script handles: version bump, CHANGELOG finalization, commit, tag, publish, and adding new `[Unreleased]` sections.

## Git Rules for Parallel Agents

Other agents may have uncommitted work in this worktree.

- Stage only the paths you created/modified/deleted this session; run `git status` first and verify. Never `git add -A` or `git add .`.
- `packages/ai/src/models.generated.ts` may always be staged alongside your files.
- Include `fixes #<number>` / `closes #<number>` in the commit message when there is a related issue or PR.
- Never run: force push, `git reset --hard`, `git checkout .`, `git clean -fd`, `git stash`, `git commit --no-verify`.
- Update with `git pull --rebase`; resolve conflicts only in your own files, and abort and ask if a conflict is in a file you did not touch.
- If user instructions conflict with these rules, ask for confirmation before overriding them.
