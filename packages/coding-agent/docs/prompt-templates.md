> hoocode can create prompt templates. Ask it to build one for your workflow.

# Prompt Templates

Prompt templates are Markdown snippets that expand into full prompts. Type `/name` in the editor to invoke a template, where `name` is the filename without `.md`.

## Locations

HooCode loads prompt templates from:

- Global: `~/.hoocode/agent/prompts/*.md`
- Project: `.hoocode/prompts/*.md`
- Packages: `prompts/` directories or `pi.prompts` entries in `package.json`
- Settings: `prompts` array with files or directories
- CLI: `--prompt-template <path>` (repeatable)

Disable discovery with `--no-prompt-templates`.

### Slash command directories

Templates are also loaded from dedicated command directories, invoked the same way (`/name`):

- Global: `~/.hoocode/commands/*.md`
- Project: `.hoocode/commands/*.md`
- Claude Code (native import): `~/.claude/commands/*.md` and `.claude/commands/*.md`, at lower precedence than `.hoocode/commands/`
- Settings: `slashCommands` array with files or directories
- CLI: `--slash-command <path>` (repeatable)

Disable discovery with `--no-slash-commands` (`-nsc`). Explicit `--slash-command` paths still load when discovery is disabled.

Claude Code commands are loaded with hoocode's prompt-template engine, so the supported subset is `description`/`argument-hint` frontmatter and `$1`/`$@`/`$ARGUMENTS` substitution. Claude-specific features (`allowed-tools`/`model` frontmatter, `!` bash execution, `@` file references, subdirectory `namespace:command` naming) are not interpreted, and discovery is non-recursive.

## Format

```markdown
---
description: Review staged git changes
---
Review the staged changes (`git diff --cached`). Focus on:
- Bugs and logic errors
- Security issues
- Error handling gaps
```

- The filename becomes the command name. `review.md` becomes `/review`.
- `description` is optional. If missing, the first non-empty line is used.
- `argument-hint` is optional. When set, the hint is displayed before the description in the autocomplete dropdown.

### Argument Hints

Use `argument-hint` in frontmatter to show expected arguments in autocomplete. Use `<angle brackets>` for required arguments and `[square brackets]` for optional ones:

```markdown
---
description: Review PRs from URLs with structured issue and code analysis
argument-hint: "<PR-URL>"
---
```

This renders in the autocomplete dropdown as:

```
→ pr   <PR-URL>       — Review PRs from URLs with structured issue and code analysis
  is   <issue>        — Analyze GitHub issues (bugs or feature requests)
  wr   [instructions] — Finish the current task end-to-end
  cl   — Audit changelog entries before release
```

## Usage

Type `/` followed by the template name in the editor. Autocomplete shows available templates with descriptions.

```
/review                           # Expands review.md
/component Button                 # Expands with argument
/component Button "click handler" # Multiple arguments
```

## Arguments

Templates support positional arguments and simple slicing:

- `$1`, `$2`, ... positional args
- `$@` or `$ARGUMENTS` for all args joined
- `${@:N}` for args from the Nth position (1-indexed)
- `${@:N:L}` for `L` args starting at N

Example:

```markdown
---
description: Create a component
---
Create a React component named $1 with features: $@
```

Usage: `/component Button "onClick handler" "disabled support"`

## Injection Type

Commands may set a `type` in frontmatter to control how the expanded text is injected. This applies to templates loaded from any location.

- `user` (default): the expanded text is sent as a normal user message.
- `system`: the expanded text is appended to the system prompt for the turn. If arguments are passed, the raw argument string is sent as the user message.
- `context`: the expanded text is added as a hidden context message (not displayed). If arguments are passed, the raw argument string is sent as the user message.

```markdown
---
description: Enforce strict review standards for the rest of the session
type: system
---
You are in strict review mode. Reject any change that lacks tests.
```

An unrecognized `type` value falls back to `user`.

## Loading Rules

- Template discovery in `prompts/` and `commands/` is non-recursive.
- If you want templates in subdirectories, add them explicitly via `prompts`/`slashCommands` settings or a package manifest.
