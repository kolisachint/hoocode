# hoocode-code-tool-bash

Port of hoocode `core/tools/bash.ts`, `core/bash-executor.ts`,
`core/tools/output-accumulator.ts` and `utils/shell.ts` (v0.5.89): the `bash` tool the model
calls, and the executor behind user `!` commands.

Commands run through the resolved shell (`shellPath` setting, else `/bin/bash`, bash on
`PATH`, `sh`) in their own process group, so a timeout or abort kills the whole tree.
A command with an explicit positive `timeout` runs with that limit, and `timeout: 0`
means no limit, in every context.

The default for an omitted (or negative or non-finite) `timeout` depends on the context:

- **Subagent** (`HOOCODE_SUBAGENT_DEPTH` >= 1 in the process environment): 600 s,
  `DEFAULT_BASH_TIMEOUT_SECS`, overridable with `HOOCODE_BASH_DEFAULT_TIMEOUT_SECS`.
- **Main session**: no limit.

The schema and description name the default only in a subagent. Stdin is `/dev/null`. Output
streams into a bounded accumulator that keeps the tail, spills the full output to a temp file
once it passes the caps, and throttles progress updates to one per 100 ms.
