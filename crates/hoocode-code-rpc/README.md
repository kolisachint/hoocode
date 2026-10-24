# hoocode-code-rpc

RPC mode for the hoocode coding agent (`hoocode --mode rpc`): JSON commands on
stdin, responses and session events on stdout, one JSON object per line. The
protocol is documented in `docs/rpc.md`.

- `run_rpc_mode` over an `RpcHost`: `RuntimeHost` (an `AgentSessionRuntime`, so
  `new_session`, `switch_session`, `fork` and `clone` work) or `SingleSessionHost`.
- `client::RpcClient`: drives a `--mode rpc` process.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.
