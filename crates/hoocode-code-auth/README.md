# hoocode-code-auth

Credential storage for the `hoocode` CLI.

- `auth.json` in the agent dir: a JSON object from provider id
  to `{"type": "api_key", "key": ...}` or `{"type": "oauth", "refresh", "access", "expires", ...}`,
  written pretty-printed (2 spaces) with mode `0600`.
- Writes and OAuth refreshes hold a `proper-lockfile`-style lock (an
  `auth.json.lock` directory, stale after 10 s / 30 s), so several processes can share one file.
- API key priority: runtime override (`--api-key`), stored API key (resolved like models.json
  values: `!command`, env var name, literal), stored OAuth token (refreshed under the lock),
  environment variable, then the fallback resolver.
- `AuthStorage` implements `hoocode_code_models::AuthLookup`, and
  `AuthStorage::model_modifier` gives the registry the OAuth providers' `modifyModels` pass.

The file format is kept stable across releases.
