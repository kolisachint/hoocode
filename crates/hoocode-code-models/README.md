# hoocode-code-models

Model registry for the `hoocode` CLI: built-in models
from `hoocode-ai-models`, plus custom providers, provider overrides and per-model
overrides from `models.json`, and request auth (`apiKey` / `headers` / `authHeader`)
resolved from a `!command`, an environment variable, or a literal.

The models.json schema is kept stable across releases.
