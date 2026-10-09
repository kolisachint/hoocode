# Decisions, 2026-10-09 (subagents)

Made with the user in one review of how the parent agent drives subagents
([subagent-orchestration.md](subagent-orchestration.md)). Where a card disagrees
with this page, this page wins. The 2026-10-07 and 2026-10-08 pages still stand.

## Subagent orchestration

| # | Decision |
|---|---|
| D1 | Every subagent runs in the background by default, including `general-purpose` and custom agents. Foreground only when the parent passes `background: false`. |
| D2 | The parent does not idle on a barrier while it has independent work. It keeps dispatching and thinking. `AgentOutput(wait: true)` only when blocked on that one result. |
| D3 | Worktree isolation is done by instructions the parent passes to the subagent. No Rust worktree feature, no new tool parameter. |
| D4 | Only agents that can write get a worktree (`general-purpose`; custom agents with Edit, Write or Shell). `explore` and `plan` work in the parent tree. |
| D5 | Worktrees live in the repo at `.hoocode/worktrees/<label>`, already ignored by `.hoocode/` in `.gitignore`. Branch `agent/<label>`. |
| D6 | The subagent commits on its own branch and never pushes. The parent reports the branches. The user merges. |
| D7 | After a successful merge the parent removes the worktree and branch. On failure or conflict it keeps them and reports. |
| D8 | This round is design only. No code changes. |
| D9 | The card is [subagent-orchestration.md](subagent-orchestration.md). |

Not a decision: the main session's model is the user's choice. The user runs
subagents on Haiku by asking for it; nothing in settings changes.

## Answers to the card's open questions

| # | Answer |
|---|---|
| Q1 | Frontmatter `background: false` is honoured: that agent defaults to foreground. The parent's argument still wins. |
| Q2 | Writers build with `CARGO_TARGET_DIR` set to the main checkout's `target/`. |
| Q3 | Branches are `agent/<label>`. |
| Q4 | At session end the parent removes worktrees and branches that `git branch --merged <base>` lists. Unmerged ones stay and are reported. This refines D7. |
| Q8 | Every agent, read-only too, keeps AGENTS.md and CLAUDE.md. |
