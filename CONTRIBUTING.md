# Contributing to HooCode

**Contributions are welcome.** Bug reports, feature ideas, docs fixes, a new tip,
a whole provider — all of it. This page is short on purpose: it exists to help
you land a change, not to fence you out.

If you are here to file a bug or an idea, you can stop reading and
[open an issue](https://github.com/kolisachint/hoocode/issues/new/choose). The
templates ask for exactly what a maintainer needs and nothing else.

> If HooCode is useful to you, [star the
> repo](https://github.com/kolisachint/hoocode) ★ — it is the single cheapest
> thing you can do to help other people find it, and it takes one click.

## Good first contributions

Genuinely small, genuinely useful, and each one ships to every user:

| What | Where | Why it is easy |
|------|-------|----------------|
| **Add a tip** | [`src/modes/interactive/tips.ts`](packages/coding-agent/src/modes/interactive/tips.ts) | One row in an array. If you learned a trick the hard way, nobody else should have to. |
| **Fix or sharpen a doc** | [`packages/coding-agent/docs/`](packages/coding-agent/docs) | These are the published docs. If something confused you, it will confuse the next person. |
| **Add a theme** | [`src/modes/interactive/theme/`](packages/coding-agent/src/modes/interactive/theme) | A JSON file against a published schema. |
| **Write an example extension** | [`examples/extensions/`](packages/coding-agent/examples/extensions) | Self-contained, no core changes, and the examples are how most people learn the API. |
| **Improve an error message** | anywhere | The best bug reports come from people who hit a bad error and fixed it. |

Issues tagged
[`good first issue`](https://github.com/kolisachint/hoocode/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
and [`help wanted`](https://github.com/kolisachint/hoocode/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
are picked for the same reason. Say "I'll take this" on one and it is yours — no
need to ask permission first.

## Reporting a bug

Use the [bug template](https://github.com/kolisachint/hoocode/issues/new/choose).
The three things that decide whether a bug gets fixed quickly:

1. **What you did**, precisely enough to repeat it.
2. **What happened**, versus what you expected.
3. **`hoocode --version`**, your OS, and how you installed it.

A report without a repro is still worth filing. "It sometimes garbles the
prompt on tmux under iTerm" is a real signal even without steps — say so
plainly rather than not filing.

## Suggesting a feature

Say what you are trying to do before what you want built. A request framed as a
problem usually gets a better answer than one framed as a solution, and
sometimes the answer is a flag that already exists.

HooCode's core stays small on purpose. Most features belong in an extension,
a skill, or a plugin rather than in the core — the
[extension API](packages/coding-agent/docs/extensions.md) is deliberately broad
so that this is a real option and not a brush-off. If you are not sure which
side of the line your idea falls on, open the issue and ask.

## Opening a pull request

No approval needed first. Fork, branch, push, open it.

For anything large — a new provider, a change to the agent loop, a new
top-level command — open an issue first so you do not spend a weekend on an
approach that was never going to land. For everything else, just send it.

### Before you push

bun is the toolchain:

```bash
bun install
bun run check     # lint, format, typecheck
./test.sh         # tests (LLM-dependent tests skip themselves without API keys)
```

Both must pass. CI runs the same two commands, so a green run locally is a green
run there.

### What makes a PR easy to merge

- **One change per PR.** Two unrelated fixes in one diff take more than twice as
  long to review.
- **Say why in the description.** The diff shows what changed; only you can
  explain what problem it solves.
- **Match the surrounding code.** This codebase comments the *why* rather than
  the *what*, sometimes at length. Follow the file you are editing.
- **Add a test if you fixed a bug.** The test is what stops it coming back.
- **Do not edit `CHANGELOG.md`.** Entries are written at release time.
- **Do not bump versions.** The merge pipeline does it — see below. A hand-edited
  `version` in any `packages/*/package.json` stacks on top of the pipeline's bump
  and burns a patch number for nothing.

### Using an agent to write the change

Fine — HooCode is a coding agent, it would be strange to object. Run it from the
repository root so it picks up [`AGENTS.md`](AGENTS.md), which carries the rules
this project actually holds code to.

One expectation, and it is the only hard one here: **you should be able to
explain your own PR.** If a reviewer asks why a change is the way it is and the
answer is "the model did that", the PR will sit until someone can answer.
That is not a rule about AI, it is a rule about review — it applies identically
to code copied from Stack Overflow.

### Adding a provider

See [`AGENTS.md`](AGENTS.md) for the required tests. Providers without them
cannot be verified and will not be merged, not because of the rule but because
nobody can tell whether they work.

## Releases and versioning

When a PR merges to `main`, the `Release on PR Merge` workflow
([`.github/workflows/merge-release.yml`](.github/workflows/merge-release.yml))
bumps the workspace version (patch by default, or per the PR's `npm:minor` /
`npm:major` label), tags it, publishes to npm, and attaches the standalone
binaries and installers to the GitHub release. Workspace packages are versioned
in lockstep.

## Getting help

- **Questions, ideas, "is this a bug?"** —
  [GitHub Issues](https://github.com/kolisachint/hoocode/issues/new/choose).
  Questions are welcome as issues; there is no separate forum to get lost in.
- **Collaboration, or anything that does not fit an issue** — tag
  [@kolisachint on X](https://x.com/kolisachint).

## Code of conduct

Be decent. Assume the person on the other side is doing their best with less
context than you have. Maintainers are volunteers; contributors are volunteers;
nobody here is anybody's support contract.

Harassment, bad-faith arguing, and mass-automated low-effort issues are the only
things that get anyone blocked.

---

Thanks for being here. ★ [Star the repo](https://github.com/kolisachint/hoocode)
if HooCode has been useful — it genuinely helps.
