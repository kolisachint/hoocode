# hoocode-ai-provider-anthropic

Anthropic provider for hoocode AI.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Streams completions from the Anthropic Messages API (`POST /v1/messages`,
`stream: true`), translating Anthropic's SSE event stream into the shared
`hoocode_ai_types::AssistantMessageEvent` sequence used by the agent loop.

Supports text, extended thinking, tool use, image input, prompt caching
(`cache_control`), and both API-key (`ANTHROPIC_API_KEY`) and OAuth
(`ANTHROPIC_OAUTH_TOKEN`) credentials.

Ported from TypeScript `@kolisachint/hoocode-ai` → `providers/anthropic.ts`.
