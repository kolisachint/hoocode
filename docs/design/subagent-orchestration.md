# Subagent orchestration: background by default, worktrees for writers

Status: **agreed 2026-10-09. Design only; nothing is built this round (D8).**
Decisions D1–D9 are the user's, taken 2026-10-09. Builds on
[subagents.md](subagents.md) (as-built) and [subagent-evals.md](subagent-evals.md).

## Goal

- The parent keeps working while children run. It does not sit in a barrier it
  does not need.
- Children that write never collide with each other or with the parent's files.
- Every child reports in a shape the parent can act on without the transcript.

Problems this card fixes (verified against the code on 2026-10-09):

| ID | Problem | Where |
|---|---|---|
| P1 | general-purpose has no `background: true`, so it blocks the parent | `general-purpose.md:1-17` |
| P2 | `background: false` parses to None, so an agent cannot opt out | `agent_frontmatter.rs:272-286` |
| P3 | Results arrive only at turn boundaries. The prompt does not say so, so models use `AgentOutput(wait:true)` as a barrier | `agent-loop/src/lib.rs:181-187`, `:302`; `tools.rs:1134` |
| P4 | No eval checks that the parent works while a background child runs | `scripts/eval/subagent_evals.py:276-299` |
| P5 | Children get the delegation block, but at depth cap they have no Agent tool | `runtime.rs:620`, `:626` |
| P6 | A custom body replaces the base prompt, so children lose base guidelines | `system_prompt.rs:351-369` |
| P7 | AGENTS.md is appended to every child, read-only ones too. Kept on purpose (Q8) | `system_prompt.rs:356` |
| P8 | The child prompt is a bare `Task:` line. No workspace, branch or done criteria. `context` is always empty | `pool.rs:1225-1234`, `tools.rs:834` |
| P9 | Only general-purpose and plan say the final message must stand alone | `general-purpose.md:35`, `plan.md:36` |
| P10 | code-review and security-review have Shell. "Read-only" is only the prompt | `code-review.md:16`, `security-review.md:17` |
| P11 | A partial result deletes its dispatch dir, so `resume_task_id` likely fails (not run) | `pool.rs:1594-1601`, `:842-856`, `tools.rs:488-491` |
| P12 | "Isolated" means a separate process and context only. All writers share one tree, up to 5 at once, with no locks | `tools.rs:6`, `:599`; `pool.rs:381` |
| P13 | No git worktree create or remove exists anywhere. Only reading worktrees | `hoocode-code-paths/src/git_branch.rs` |
| P14 | subagent-evals.md §5 says stall-reap is KNOWN. It was fixed 2026-10-05 | `subagent-evals.md:89` |

## Decisions

| # | Decision | Notes |
|---|---|---|
| D1 | All subagents run in background by default (including general-purpose and custom agents). Foreground only when the parent passes `background:false`. Frontmatter `background: false` is honoured as the agent's default (Q1). |
| D2 | The parent never idles on a barrier when it has independent work. It keeps dispatching and working. `AgentOutput(wait:true)` only when blocked on that one result. | Results still arrive at turn boundaries. See Details. |
| D3 | Worktree isolation is done by instructions, not code. The parent passes an instruction set to the subagent. No new Rust worktree feature and no new tool parameter. | A harness check is a later card. See Q6. |
| D4 | Only agents that can write get a worktree: general-purpose, and custom agents with Edit, Write or Shell. Read-only agents (explore, plan) run in the parent tree. | |
| D5 | Worktree at `.hoocode/worktrees/<label>` inside the repo. Branch `agent/<label>`. | `.hoocode/` is gitignored (`.gitignore:10`). Branch name agreed (Q3). |
| D6 | The subagent commits on its own branch. It never pushes. The parent reports branches. The user merges. | |
| D7 | After a successful merge, the parent removes the worktree and branch. On failure or conflict it keeps both and reports. | Timing agreed in Q4: at session end, merged branches only. |
| D8 | Design only this round. No code changes. | User decision |
| D9 | This card lives at `docs/design/subagent-orchestration.md`. | User decision |

## What we build

Nothing here is built this round (D8). Tags show what each item needs later:
**[text]** prompt or template wording only. **[rust]** Rust change. **[eval]** test
or harness change.

### 1. Parent guidance [text] — fixes P3, P12

Files: `templates/prompts/task-main.md` and the two `task-background-*.md` files.

With D1 every agent is background, so the "agents or none" split in
`tools.rs:83-91` goes. Keep one template. Replace `{{BACKGROUND_GUIDANCE}}` with:

```
- Dispatch every independent subtask in one message. Then keep working on
  anything that does not need their results.
- Results arrive as messages at the next turn boundary. Do not call
  AgentOutput with wait:true just to pass time.
- Call AgentOutput with wait:true only when your next step needs one named
  result. If it times out, say so, do other work, and check again.
- End your message only when you have nothing else to do. The loop then waits
  for background results and wakes you with them.
- For a writing subagent, send the brief (§2) and follow the worktree protocol (§3).
```

Fix "isolated" wording. It means separate process and context, never files.

| File | Line | Change |
|---|---|---|
| `task-main.md` | 1 | "run in their own isolated context" → "run as separate processes with their own context" |
| `task-main.md` | 3 | "fix one isolated bug" → "fix one bug" |
| `tools.rs` | 6, 599 | Same wording change in the doc comment and the tool description |
| `explore.md` | 22 | Same |
| `general-purpose.md` | 19 | Same |
| `plan.md` | 23 | Same |
| `code-review.md` | 21-22 | Same |
| `security-review.md` | 22 | Same |

### 2. The subagent brief [text] — fixes P8, P9

The parent writes this into every `prompt`:

```
Goal: <the outcome, in one sentence>
Context: <files to read (absolute paths), findings so far, why it matters>
Workspace: <absolute path> · branch <name or none> · base <commit or none>
Constraints: <read-only, or may edit under <path>> · may commit: yes/no · never push
Done when: <a check you can run, e.g. "cargo test -p X passes">
Report: status (complete | partial | failed) · summary · files changed ·
  commits · verified (what you ran, and the result) · not verified · open questions
```

Rules:

- **Absolute paths only.** The Agent schema has no cwd field (`tools.rs:570-589`).
  The child runs in the parent's cwd (`tools.rs:622-624`). Read, Edit and Write take paths.
- **Use `git -C <path>`.** Whether a Shell `cd` carries between calls is not verified.
- **Same Report rule in every body** (§4d), so the final message stands alone.
- Later [rust]: the harness fills Workspace itself (Q6).

### 3. Worktree protocol for writing agents [text] — fixes P12, P13

Who does what:

| Step | Parent | Writing child | Read-only child | User |
|---|---|---|---|---|
| Create worktree and branch | yes | — | — | — |
| Edit and run checks | — | yes, in the worktree only | — (no worktree) | — |
| Commit | — | yes, on its own branch | — | — |
| Push | never | never | never | pushes if they want |
| Review the branch | yes, or dispatch code-review | — | code-review | — |
| Merge | — | — | — | yes |
| Remove worktree and branch | yes, at session end, merged branches only | — | — | — |

**Parent, before dispatch**

1. Check the task needs a writer (general-purpose, or a custom agent with Edit,
   Write or Shell). Read-only agents skip this section.
2. Note the base commit: `git rev-parse HEAD`. Worktrees start from HEAD, so
   uncommitted parent changes are not in them (Q7).
3. Pick a unique `<label>`. Check `git branch --list agent/<label>` is empty.
4. Create it: `git worktree add -b agent/<label> .hoocode/worktrees/<label> <base>`.
5. One writer per worktree. Never give two writers files that overlap, and check
   the parent's own files too.
6. Put the Workspace path, branch and base in the brief.

**Subagent, inside the worktree**

1. Work only under the worktree path. Use absolute paths for Read, Edit and Write.
   Use `git -C <path>` for git.
2. Run the project's checks there (CLAUDE.md commands), for the crates touched.
   Rust builds use the main checkout's target dir:
   `CARGO_TARGET_DIR=<repo>/target` (Q2). The parent puts it in the brief.
3. Commit on the branch with a clear message. Never push. Never switch branches.
   Never write to the parent tree.
4. Report: branch, commit list, `git diff --stat <base>..HEAD`, and checks run
   with their results.

**Parent, after the child settles**

1. Read the work: `git -C <path> log --oneline <base>..agent/<label>` and
   `git diff --stat <base>..agent/<label>`.
2. Verify: run the checks on the branch, or dispatch code-review (background,
   read-only) with the brief pointing at the branch diff.
3. Report to the user: branch, summary, verification, risks, and the merge command
   (`git merge agent/<label>`). The user merges.
4. At session end, for each `agent/*` branch that `git branch --merged <base>`
   lists: `git worktree remove .hoocode/worktrees/<label>`, then
   `git branch -d agent/<label>` (Q4). `-d` refuses unmerged branches, which is
   the safety net. Report what was removed.
5. Unmerged, failed, conflicting or red: keep the worktree and branch. Report both paths.

### 4. Child agent fixes — mixed tags

| # | Change | Where | Fixes | Tag |
|---|---|---|---|---|
| 4a | Add `background: true` to the frontmatter. It stays true if the default changes | `general-purpose.md:15-17` | P1 | text |
| 4b | Keep `background: false` as a value. Stop dropping it to None | `agent_frontmatter.rs:272-286`, `b.then_some(true)` | P2 | rust, small |
| 4c | Background by default for every agent. Tool param still overrides | `tools.rs:73-80`, `:810-813` | D1 | rust, small |
| 4d | Final-answer rule in every body (text below) | `explore.md`, `code-review.md`, `security-review.md`; `general-purpose.md:35` and `plan.md:36` already say a version | P9 | text |
| 4e | Gate the delegation block on the same depth check as the tool. Children at the cap get no block | `runtime.rs:620`, `:626`; compare `:347` | P5 | rust, small |
| 4f | Keep base guidelines for custom bodies. Append a short base block, do not replace | `pool.rs:1152-1154` passes the body as `--system-prompt`; `system_prompt.rs:351-369` returns before the base guidelines | P6 | rust, small |
| 4g | Dropped (Q8): every agent keeps AGENTS.md and CLAUDE.md | — | P7 | — |
| 4h | Review agents: the brief lists the only git commands allowed (`git diff`, `git log`, `git show`) and says "no other Shell" | `code-review.md:16`, `security-review.md:17` | P10 | text now; rust later (a Shell filter) |
| 4i | One prompt wrapper for all three paths. Give `context` a real value or remove it | `pool.rs:1111`, `:1225-1234`; `tools.rs:834` | P8 | rust, small |

Final-answer rule for 4d (same text in every body):

```
Your final message is the only thing the caller receives. Start with the
status (complete, partial or failed). Then the result, with the Report fields
from the brief. Do not say "see above".
```

### 5. Evals — fixes P4, and checks §3 [eval]

Two new scenarios in `scripts/eval/subagent_evals.py`:

| Scenario | Route | Checks |
|---|---|---|
| `parent_works_while_background_runs` | parent: background `Agent`, then `Read`, then final text. Child: a summary | A parent tool call sits between the dispatch and the child's result. Needs a new `Expect` field, `parent_tool_calls_before_result` |
| `worktree_writer_commits_on_branch` | child (mock) runs `git -C <wt> commit` on `agent/<label>` | Branch has one commit. Parent `git status` is unchanged. Nothing pushed |

The mock LLM tests the mechanics: the parent's tool order, and the child's git
commands. It cannot show that a real model follows the brief. That needs a manual
trial on a real session.

### 6. Small fixes [rust or text]

| # | Item | Fix | Fixes | Tag |
|---|---|---|---|---|
| 6a | Resume | Keep the dispatch dir when a result is partial. Add a test that `resume_task_id` finds the session. Confirm the bug with a run first | P11 | rust, small |
| 6b | Eval doc | Re-run `parent_stall_reaped`. Then change `subagent-evals.md:89` and §5 from KNOWN to the measured result. Do not mark a pass without a run | P14 | text, after a run |
| 6c | Tool wording | "Isolated" fix in §1. Remove the template split in §1 | P12 | text |

## Not doing

- A Rust worktree manager, or a `worktree` or `cwd` parameter on Agent (this round).
- Auto-merge, or any merge the user did not run.
- Pushing, by a subagent or the parent.
- Worktrees for read-only agents.
- Mid-turn injection of results. They stay at turn boundaries.
- Locks or a scheduler for file overlap. The parent checks overlap by hand (§3).

## Open questions

Answered 2026-10-09 (recorded in [decisions-2026-10-09-subagents.md](decisions-2026-10-09-subagents.md)):

| Q | Answer |
|---|---|
| Q1 frontmatter `background: false` | Honoured: the agent defaults to foreground. The parent's argument still wins. |
| Q2 Rust builds per worktree | Shared `CARGO_TARGET_DIR` with the main checkout. Writers take turns on cargo's lock. |
| Q3 branch names | `agent/<label>`. |
| Q4 cleanup timing | Parent checks `git branch --merged <base>` at session end and removes merged worktrees and branches itself. Unmerged ones stay and are reported. |
| Q8 AGENTS.md for read-only agents | Every agent keeps it. 4g dropped. |

Still open (recommendation first):

**Q5. Is a worktree inside the repo safe for search and build?**
- `.hoocode/` is gitignored (`.gitignore:10`), so git status and rg skip it. CodeSearch is not verified.
- Check before first real use: `cargo metadata` in the parent ignores the worktrees.
- Risk: a tool that ignores `.gitignore` sees duplicate code.

**Q6. Should the harness enforce the worktree later?**
- Recommend: after a few weeks of the text version, a code card where the harness fills Workspace and checks the child's cwd.

**Q7. What about uncommitted parent changes?**
- Worktrees start from HEAD, so children do not see them.
- Recommend: the parent warns in its report if the tree is dirty. It commits only if the user asks.

**New risk from Q2:** branches with different dependencies share one target dir,
so builds can thrash the cache. Watch build times; switch to per-worktree target
dirs if it hurts.

## Details

### Standard practice

- Fan out independent tasks in one message.
- Background by default. The parent keeps working.
- Self-contained briefs: goal, context, done criteria.
- Structured results: status, files, verified, not verified.
- One writer per worktree. No overlapping files.
- The parent integrates. Children never merge or push.
- Notify, don't poll.
- Tools match capabilities. Read-only agents get read-only tools.

### Facts the design relies on

| Fact | Where |
|---|---|
| Results reach the parent at turn boundaries | `agent-loop/src/lib.rs:280`, `:302` |
| With background work in flight and nothing to do, the loop waits and runs again. Ending a message is a wait with no tool call | `agent-loop/src/lib.rs:181-187` |
| Agent schema has no cwd or workspace field | `tools.rs:570-589` |
| Child cwd is the parent's cwd | `tools.rs:622-624` |
| Concurrency cap is 5; nested cap is 2 | `pool.rs:381`; `depth.rs:36` |
| A clean success deletes the dispatch dir | `pool.rs:1594-1601` |
| The wait default is 120 s | `tools.rs:58` |
