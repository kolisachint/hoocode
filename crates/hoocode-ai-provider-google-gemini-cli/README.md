# hoocode-ai-provider-google-gemini-cli

The `google-gemini-cli` API: Google's Cloud Code Assist endpoint
(`v1internal:streamGenerateContent`), used by the `google-gemini-cli` (Gemini CLI) and
`google-antigravity` providers.

The API key is the JSON `{"token": …, "projectId": …}` that the Google OAuth providers
(`hoocode-ai-oauth-google`) return from `get_api_key`. Message and tool conversion is shared
with `hoocode-ai-provider-google`.
