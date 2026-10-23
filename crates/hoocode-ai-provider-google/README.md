# hoocode-ai-provider-google

Google Gemini / Vertex AI provider for hoocode AI.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Streams completions from the `:streamGenerateContent?alt=sse` REST endpoint
shared by Google Generative AI (`stream`) and Vertex AI (`stream_vertex`),
translating the SSE event stream into the shared
`hoocode_ai_types::AssistantMessageEvent` sequence used by the agent loop.

Supports text, thinking (including Gemini 3 `thinkingLevel` and budget-based
models), tool calls, tool-result images (inlined for Gemini 3+, separate turn
for older models), and `GEMINI_API_KEY` credentials.

Vertex AI credentials are an already-minted OAuth2 access token
(`GOOGLE_VERTEX_ACCESS_TOKEN` or an explicit `api_key`), or Application Default
Credentials: the `GOOGLE_APPLICATION_CREDENTIALS` file, gcloud's well-known
credentials file, then the GCE/GKE metadata server. Service-account and
authorized-user files are supported; external-account (workload identity
federation) files are not.
