# Pending CI changes (apply manually)

The session's GitHub token can't modify `.github/workflows/`, so these changes are
staged here. To apply them:

```bash
git apply migration/ci/delete-crates-doc-step.patch      # drop the agent-mcp doc-test step (crate deleted, 0b)
git apply migration/ci/ci.yml.patch                     # ledger + dependency-firewall checks in CI
git apply migration/ci/rename-to-hoocode.patch            # step 0c: workflow binary and crate names -> hoocode (after the rename)
cp migration/ci/tui-parity.yml .github/workflows/       # manual/nightly Level-2 parity job
git apply migration/ci/fuzz.patch                       # nightly cargo-fuzz job for fuzz/ (plan 1.6); adds .github/workflows/fuzz.yml
# then paste migration/ci/msrv-job.yml under `jobs:` in .github/workflows/ci.yml
```

Tracked by ledger tasks 7.1 (CI gates) and 13.3 (parity workflow).

Level 1 parity (13.2, `crates/hoocode-code-main/tests/replay.rs`) needs no workflow
change: it runs in the existing `cargo test --workspace` step. `scripts/parity_test.sh` is a
local smoke test (binary starts, the replay test, and one Level-2 scenario when the pinned
hoocode is built).
