# Modes

A mode swaps the system prompt hoocode runs under, so the same session can be
steered between answering questions, designing a change, and making it.

A mode prompt is instructions; what actually blocks a tool call is the mode's
entry in `hoo-config.json`. The two are meant to agree, and the shipped defaults
below make them agree for the read-only modes — but if you write a custom mode
prompt that forbids something, add the matching policy or nothing enforces it.

The active mode is `build` unless configured otherwise.

## Switching

```
/mode ask       # read-only Q&A
/mode plan      # explore and design, no source edits
/mode build     # careful implementation (default)
/mode debug     # root-cause analysis, no file modifications
/plan           # shorthand for /mode plan
```

The active mode persists in `hoo-config.json` under `active_mode`. Switching
back to `build` clears the key rather than writing the default.

## The four built-in modes

| Mode | For | Tells the model to |
|------|-----|--------------------|
| `ask` | Questions about a codebase | Read, SearchCodebase, trace, and explain, citing paths and line numbers; decline edits and suggest `/mode build` |
| `plan` | Designing a change before making it | Explore, ask blocking questions with `ask_options`, then write a plan with Goal / Files to modify / New files / Tests / Verification |
| `build` | Implementing | Read before editing, one verified change at a time, name what an irreversible operation destroys before running it, never commit or push unasked, run tests after each unit of work |
| `debug` | Finding a root cause | Gather evidence, reproduce, trace the call path, state the root cause in one sentence, describe the fix without applying it |

## What each mode actually permits

The mode prompts above say what the model should do; these are the shipped
defaults in `hoo-config.json` that decide what it *can* do. Edit them per mode,
globally in `~/.hoocode/hoo-config.json` or per project in
`./.hoocode/hoo-config.json`.

| Mode | `auto_allow` (runs without a prompt) | Hard policy |
|------|--------------------------------------|-------------|
| `ask` | `read` | `denied_tools: [edit, write]` |
| `plan` | `read`, `write` | `allowed_write_paths: [.hoocode/plans/*, .hoocode/plan.md]` |
| `build` | `read` | none — every `bash`/`edit`/`write` is gated |
| `debug` | `read`, `bash` | `denied_tools: [edit, write]` |

- `denied_tools` is hard enforcement: it applies with or without a UI, so it
  holds in `--print` runs too.
- `allowed_write_paths` covers both `edit` and `write`, and is checked *before*
  `auto_allow` — which is what confines plan mode to its plan file even though
  `write` is auto-allowed there. It is currently only applied in interactive
  sessions.
- `bash` is not restricted by tool policy in any shipped mode. In `ask` and
  `debug` the prompt is what keeps it read-only; if you need that enforced, set
  `allowed_bash_commands` (a hard-enforced regex allowlist) for the mode.

## The planning workflow

Plan mode writes to `.hoocode/plans/<session-id>.md`. Each session gets its own
plan file, so two sessions in one project do not overwrite each other.
(`.hoocode/plan.md`, the older single-file location, is still read as a
fallback.)

Once a plan exists:

```
/grill [me|plan]   # stress-test it before committing to it
/approve           # accept it and switch to build mode to execute
/goal [--max-turns N] [objective]
```

`/approve` switches to `build`. `/goal` runs autonomously toward an objective —
if you do not type one, it takes the plan's **Goal** section, and the plan's
**Verification** section becomes the completion condition either way. Cap the
run with `--max-turns`.

`/grill` is worth the round trip when a plan carries real risk; it argues
against the plan rather than refining it.

## Custom modes

A mode is a directory containing `system.md`. hoocode resolves a mode name
against these locations, first match winning:

1. `./.hoocode/modes/<name>/system.md` (project)
2. `~/.hoocode/modes/<name>/system.md` (user)
3. Directories passed with `--mode-path`
4. The built-in prompt, for the four names above

The built-in prompt is the same text `/init` scaffolds into a project: both come
from `templates/modes/<name>/system.md`. There is no second copy, so a project
that ran `/init` and one that did not behave identically until you edit the file.

So `./.hoocode/modes/build/system.md` overrides the shipped build prompt for one
project, and `./.hoocode/modes/review/system.md` adds a `review` mode that
`/mode review` will find.

In a plan-mode template, `{{PLAN_PATH}}` is substituted with the session's plan
file path, relative to the working directory.

```
.hoocode/
└── modes/
    └── review/
        └── system.md
```

Start one with `hoocode --mode review`, or switch mid-session with
`/mode review`.

## Related

- [Settings](settings.md) — tool policy, which is what actually restricts writes
- [Prompt templates](prompt-templates.md) — reusable prompts, as opposed to a persistent stance
- [Subagent delegation](routing.md) — handing focused work to a separate agent
