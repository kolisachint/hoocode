# Sign in with a subscription

hoocode can use a subscription instead of an API key. Sign in inside hoocode with
**`/login`**. There is no `hoocode login` command.

Start `hoocode`, type `/login`, and choose **Use a subscription**. Then pick a provider.
Sign out with **`/logout`**, which removes the saved login for one provider.

## Providers

| Shown in `/login` as | Id | Models it unlocks |
|---|---|---|
| Anthropic (Claude Pro/Max) | `anthropic` | `claude-*` models |
| ChatGPT Plus/Pro (Codex Subscription) | `openai-codex` | `gpt-5.*` and `gpt-6*` models your plan allows (the Go plan: `gpt-6-luna`, `gpt-5.6-luna`, `gpt-5.6-terra`) |
| Google Cloud Code Assist (Gemini CLI) | `google-gemini-cli` | `gemini-*` models |
| Google Antigravity (Gemini, Claude, GPT-OSS) | `google-antigravity` | Gemini, Claude and GPT-OSS models |
| GitHub Copilot | `github-copilot` | Models from the Copilot catalog, such as Claude and Gemini |

GitHub Copilot does not use a callback port. It shows a code to enter at GitHub instead.

## Google logins need a client

hoocode ships no Google OAuth client. Until you set one, `/login` for a Google provider
stops before the browser opens, with a message that names the variables to set.

| Provider | Set these two variables |
|---|---|
| Gemini CLI | `HOOCODE_GEMINI_CLI_CLIENT_ID` and `HOOCODE_GEMINI_CLI_CLIENT_SECRET` |
| Antigravity | `HOOCODE_ANTIGRAVITY_CLIENT_ID` and `HOOCODE_ANTIGRAVITY_CLIENT_SECRET` |

Use the installed-app client ID and secret of the client you sign in as. Then run `/login`
again. Keep both variables set every time you start hoocode: refreshing the token needs them
too, so without them the login stops working about an hour after you sign in.

**Paid and Workspace accounts** also need the Google Cloud project. Set `GOOGLE_CLOUD_PROJECT`,
or `GOOGLE_CLOUD_PROJECT_ID`. If both are set, `GOOGLE_CLOUD_PROJECT` is used. Free-tier
accounts do not need it.

## Environment key: `ANTHROPIC_OAUTH_TOKEN`

`ANTHROPIC_OAUTH_TOKEN` takes a subscription token from the environment, for example one
that starts with `sk-ant-oat`. It is used only when no Anthropic login or API key is saved.

| Order | Source |
|---|---|
| 1 | A login or API key saved by `/login` |
| 2 | `ANTHROPIC_OAUTH_TOKEN` |
| 3 | `ANTHROPIC_API_KEY` |

hoocode does not save a login from this variable. Nothing is written to `auth.json`.

## Callback ports

After you approve in the browser, the provider sends the browser back to a port on
`localhost`. hoocode listens on that port.

| Provider | Port | Path |
|---|---|---|
| Anthropic | 53692 | `/callback` |
| ChatGPT | 1455 | `/auth/callback` |
| Gemini CLI | 8085 | `/oauth2callback` |
| Antigravity | 51121 | `/oauth-callback` |

### When the port is taken

- **ChatGPT** keeps going. It shows this notice:
  `Port 1455 is in use, so the browser cannot return here. Paste the code or the full redirect URL instead.`
  Copy the code, or the full address from the browser's address bar, and paste it at the prompt.
- **Anthropic and Google** stop with the system's bind error, for example
  `Address already in use (os error 98)` on Linux or `(os error 48)` on macOS. Free the port,
  then run `/login` again. To find the program that holds it, run `lsof -i :53692` (use the
  port from the table).

### Signing in on another machine

When hoocode runs on a server or over SSH, the browser cannot reach that machine's port. The
browser then shows an error page. Copy the full address from the address bar and paste it at
the prompt. The Anthropic prompt says so. Pasted input is checked against the sign-in state,
as the callback is.

## Where the login is saved

Logins are saved in `~/.hoocode/auth.json`. With `HOOCODE_CODING_AGENT_DIR` set, the file is in
that folder. The file is owner-only (mode 0600). The folder is created owner-only (0700) when
hoocode makes it. The file holds refresh tokens, so treat it like a password.

Tokens refresh by themselves when they expire, so you do not sign in again each time.

## When a login stops working

A provider that rejects the token shows its error in the session:

| Provider | Error shape |
|---|---|
| ChatGPT | `ChatGPT API error (401): ...` |
| Gemini CLI and Antigravity | `Cloud Code Assist API error (401): ...` |
| Anthropic | `401 {...}` |

Fix it with `/login` for that provider.
