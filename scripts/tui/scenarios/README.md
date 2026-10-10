# Screen golden scenarios

Each `*.json` file is one scenario, run against the real `hoocode` binary in a fixed tmux
terminal with the mock LLM (`scripts/tui/goldens.py`). The screens are committed under
`tests/golden/tui/<scenario>/`. The hoocode-ts comparison this format was built for is retired
(TUI plan T0.6), so the `tool_needle` and ts-run notes below are history.

```jsonc
{
  "description": "what is exercised",
  "phase": "ledger task id that owns this scenario",
  "terminal": {"cols": 100, "rows": 30},          // fixed size; optional term/colorterm
  "args": ["--offline", "--provider", "mock", "--model", "mock-model"],  // default
  "files": {"notes.txt": "..."},                  // seeded into the workspace (cwd)
  "binary_files": {"i.png": "iVBOR..."},          // base64, seeded like files
  "symlinks": {"pkg/dist": "{HOOCODE_PKG}/dist"}, // links in the workspace; {HOOCODE_PKG} = the pinned package
  "git": false,                                   // git init the workspace
  "settings": {},                                 // written to ~/.hoocode and ~/.hoocode settings.json
  "env": {},                                      // extra env (API keys are never inherited); {WORK}/{HOME}/{TMP} expand to the run's dirs
  "llm": [ {"text": "...", "thinking": "...", "tool_calls": [{"id": "...", "name": "Read", "arguments": {}}]},
           {"error": "boom", "status": 500},       // one entry per model request, see mockllm.py
           // tool_calls use hoocode's tool names (Read, Shell, ...); the ts run gets the hoocode-ts names
           // from harness.py TOOL_NAMES (llm_for_app)
           {"text": "...", "delay_s": 5} ],         // delay_s: wait before answering
  "compare": "style",                             // "style" (default: text + colors/attrs) or "text"
  "mask_snapshots": ["pane"],                     // snapshots not compared (a known, intended difference); their contains asserts still run
  "compare_requests": false,                      // also require identical model requests
  "request_fields": ["messages", "tools"],        // subset compared when compare_requests
  "stdout_jsonl": {"mask_keys": ["timestamp"],    // stdout to a file, compared as JSON lines (key order kept);
                   "mask_fields": {"session": ["id"]}},  // scalars masked by key, whole fields per event type
  "normalize": [ {"pattern": "regex", "replace": "x", "style": "optional forced style"} ],
  "steps": [
    {"wait_for": "regex", "timeout": 15},
    {"wait_gone": "regex"},
    {"wait_stable": 1.0},                         // screen unchanged for N seconds
    {"wait_exit": true},                          // app process exited (print mode)
    {"wait_stdout": "regex"},                     // captured stdout matches (stdout_jsonl scenarios)
    {"type": "literal text"},
    {"keys": ["Enter", "C-c", "Escape", "Up", "Tab"]},   // tmux key names
    {"sleep": 0.5},
    {"write_settings": {}},                       // replace both apps' global settings.json mid-run
    {"write_files": {"a.md": "..."}},             // write workspace files mid-run
    {"snapshot": "name", "contains": ["..."], "contains_line": [["...", "..."]], "not_contains": ["..."], "history": false}
  ]
}
```

`contains_line` takes a list of needle lists: one screen line must hold every needle of an
entry (a tool's row and its switch value, say). Tool names in `contains` and `contains_line`
are hoocode's; the ts run looks for hoocode-ts's name (`tool_needle` in `harness.py`).

Rules:

1. Accept a new scenario's screens with `python3 scripts/tui/goldens.py update <name>` after
   checking them by eye. `goldens.py check <name>` must then pass. The set is kept small (about
   10 key interactive screens) and runs once before a release: `goldens.py check all`.
2. Prefer `wait_for` / `wait_stable` over `sleep`.
3. Global normalization lives in `../normalize.json`. Add scenario-local rules only for
   scenario-specific randomness, and document why.
