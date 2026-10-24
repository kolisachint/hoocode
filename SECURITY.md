# Security

hoocode runs shell commands, edits files and sends your code to model providers, on
your machine and with your credentials. A security bug here can reach people's source
code, keys and shells, so please report anything you find privately, not in a public
issue.

## How to report

Use GitHub's private vulnerability reporting: use
[Report a vulnerability](https://github.com/kolisachint/hoocode/security/advisories/new).
Only the maintainers see the report. If that link does not work for you, send a direct message to [@kolisachint on X](https://x.com/kolisachint) asking
for a private channel, without the details.

Please include:

- the hoocode version (`hoocode --version`), your OS, and how you installed it
- what is affected (a tool, the approval gate, a mode, the installer, a release
  artifact, ...)
- steps to reproduce it, and what an attacker could do with it

## Supported versions

Fixes go into the next release from `main`. Only the latest release is supported;
`hoocode update` prints the command that updates your install.

## What counts

- **The approval gate.** Shell, Edit, Write or another gated tool running without the
  approval it should need, in the main agent or in a subagent. A mode (ask, plan,
  build, debug) letting the agent do more than it should.
- **Prompt injection that escapes the gate.** Content from a file, a tool result, a web
  page or an MCP server that gets a gated action to run without asking. (A model
  *proposing* a bad command that you then approve is not a vulnerability.)
- **Credentials.** API keys or OAuth tokens (`~/.hoocode/auth.json`, environment
  variables) written to logs, sessions, exports or anywhere a provider other than the
  intended one can read them.
- **Untrusted project files.** A project's `.hoocode/` settings, `.agents/mcp.json`,
  skills or context files doing something they shouldn't before you trust the folder.
- **RPC and app-server modes** accepting commands from someone they shouldn't, or not
  failing closed when no client can answer an approval.
- **Distribution.** The install script, the npm or PyPI launcher, or a release archive
  installing something other than the checked release binary, or skipping the
  `SHA256SUMS` check.
- **TLS.** Certificate verification being turned off where it shouldn't be.

Things that are working as designed: a command you approved doing what it says, a
tool you put in `auto_allow` running without asking, and anything the model provider
does with the data you send it.

## What happens next

You'll get a reply within a few days. Fixes ship in the next release, with credit to
you in the changelog unless you'd rather stay anonymous. Please give a fix a
reasonable chance to ship before you publish details.
