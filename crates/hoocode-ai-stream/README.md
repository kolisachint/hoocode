# hoocode-ai-stream

Streaming response utilities for hoocode AI

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

`EventStream` is a queue that a producer pushes into and a consumer reads as a
`futures::Stream`, plus a `final_result` future that resolves with the terminal event's
result. `AssistantMessageEventStream` is the instance every provider returns.
