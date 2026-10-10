# hoocode 0.5.89 replay fixtures (frozen)

These recordings are frozen. They were captured from the TypeScript hoocode (`a6cd96e7`)
by the replay harness, which has been removed (2026-10-10). Nothing re-records them now.
The scenario list is `tests/fixtures/replay.json`. Each scenario ran headless (stdin a pipe
or /dev/null, stdout/stderr to files) against a scripted mock LLM.

Per scenario directory:

| File | Content |
|---|---|
| `meta.json` | pin commit, the run's temp `HOME`/`WORK`/`TMP` (for normalization), exit status, work-file map |
| `stdout`, `stderr` | raw process output |
| `requests.jsonl` | every model request (`{"body", "path"}`) the mock received |
| `sessions/N.jsonl` | session files left under `~/.hoocode/sessions` |
| `file-N` | `work_files` of the scenario |

The rendered output is the insta snapshot `tests/snapshots/replay__<scenario>.snap`.
`cargo test -p hoocode-code-main --test replay` runs `hoocode` the same way and asserts that
snapshot. The test does not read these raw files.

Kept for reference. Do not edit these files or the snapshots by hand, and do not
`cargo insta accept` new output into them.
