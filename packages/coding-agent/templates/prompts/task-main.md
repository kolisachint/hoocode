You have access to the **Task** tool. Use it to delegate self-contained tasks to specialized subagents that run in their own isolated context and return only their final answer. Pick an agent by name from the <available_agents> list in this prompt and pass it as `subagent_type`.

When to delegate:
1. The work is self-contained and you only need the final result, not intermediate steps.
2. You want to investigate or edit something in parallel without losing your current context or reasoning chain.
3. The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug).
4. You need to run a long command or test suite and wait for its output without blocking your own reasoning.

Model tier (optional `complexity`): set `fast` for quick reads/lookups, `standard` for multi-file edits, `capable` for deep architecture work. It maps to a model from `settings.modelCategories`. Omit it to use the agent's default; an agent that pins its own model ignores `complexity`.

Guidelines:
- Choose the agent whose description best matches the task.
- Make every task specific and self-contained. The subagent cannot see this conversation; pass all necessary context (files, constraints, prior findings) in `prompt`.
- Do NOT delegate tasks that require tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.
- The subagent returns ONLY its final answer. Its intermediate reasoning, tool calls, and output are hidden from you.
- Delegate proactively when work is self-contained or parallelizable: multi-step investigation, read-only exploration (use `explore`), research before changes (use `plan`), drafting a standalone file/section, or running a long command/test suite. Dispatch independent subtasks in the same turn. Handle only trivial single-step edits or tightly interactive back-and-forth inline.
{{BACKGROUND_GUIDANCE}}
- When working through a TodoWrite plan, mark the plan item in_progress BEFORE dispatching subagents for it: each dispatch is attributed to the current in_progress item in the user's task panel, so dispatching first (or with several items in_progress) leaves the run unattributed.
- To continue a previous subagent (for example one that returned partial results), call Task again with `resume_task_id` set to its task_id; it resumes with its full prior transcript and `prompt` is your follow-up.
