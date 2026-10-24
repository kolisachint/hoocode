# hoocode-ai-registry

API provider registry: streams are dispatched on
`model.api`, not on the provider name, so every provider whose models use an API
(e.g. the ~25 OpenAI-compatible ones on `openai-completions`) works without code
changes. Extensions can register or replace an API (`register_api_provider`).

Built-in APIs registered today: `anthropic-messages`, `openai-completions`,
`openai-responses`, `openai-codex-responses`, `google-generative-ai`, `google-vertex`,
`google-gemini-cli`.
