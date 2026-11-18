---
name: plugin-authoring
description: How to author a portable, reusable hoocode plugin. Covers when a capability is worth extracting, how to name and describe it so it triggers again, and what makes it portable across repos and machines. Use before calling ProposePlugin, or when deciding whether a recipe you just completed is worth keeping.
allowed-tools: Read, Write, Edit, CodeSearch
---

# Authoring a plugin

This is the craft half of the plugin tools. `ProposePlugin` writes the package;
this describes how to write a *good* one. Read it before authoring, not after.

## When a capability is worth extracting

Extract when both are true:

- You completed a multi-step recipe you would plausibly repeat, or you repeated
  the same pattern twice in one session.
- `SearchPlugins` found nothing that already covers it.

Do not extract a one-off. The test is whether the *shape* recurs, not whether
the task did.

## Name and describe by the capability, not the occasion

The description is the only thing loaded on every turn, and it is the entire
basis on which the plugin is chosen later. Write it for the next situation, not
for the one that prompted it.

- Bad: `fix-flaky-auth-test`. It names the incident and will never trigger again.
- Good: `flaky-test-triage`. It names the capability, with a description saying
  when to reach for it.

Say *when to use it* in the description, in the words someone would use when
they need it. A description that only says what the plugin is will not trigger.

## Portability

The plugin has to work in a repo you have never seen, on a machine you do not
control:

- No absolute paths, no machine-specific paths. Prefer relative paths and
  runtime discovery.
- No embedded secrets, tokens, or environment-specific values.
- No assumptions about the current repo unless that *is* the capability's point.
- State prerequisites in the body rather than assuming them.

The package format is Agent Plugins 1.0. You never choose another layout.

## What goes in the package

- `plugin.json` at the root. Fields: `$schema`, `name`, `version`, `description`,
  `author`, `homepage`, `repository`, `license`, `keywords`, `extensions`. An
  unknown field is a warning. Any other violation rejects the plugin.
- `name`: 1 to 64 characters of `a-z`, `0-9`, `.` and `-`. Start and end with a
  letter or digit. No `--` or `..`.
- `skills/<name>/SKILL.md`: one folder per skill, directly under `skills/`.
- `mcp.json` (optional): MCP servers. Use `${PLUGIN_ROOT}` and `${PLUGIN_DATA}`
  for paths. Only for a tool that needs its own process, daemon, or runtime.
- `dev.hoocode/extension.wasm` (optional): a WASI 0.2 component implementing
  `hoocode:extension@0.1.0`. Its tools appear as `ext_<plugin>_<tool>`. Network
  calls go through gated `wasi:http`. Settings come from
  `pluginConfig.<plugin>.<key>`. `ProposePlugin` does not write it.

Hooks, commands, prompts, agents, themes, and providers are not plugin
components. They are skipped with a warning, so do not write them. Plugins ship
no settings.

## Tools and trust

With `enablePluginTools` on, the model has these tools:

- `SearchPlugins` and `ListPlugins` only read. Search first, so you do not write
  a plugin that already exists.
- `InstallPlugin` installs without asking, but only from a trusted marketplace.
- `UninstallPlugin` asks.
- `ProposePlugin` writes a skills and MCP package and asks. A proposal is
  untrusted. Its MCP servers wait for a grant in `/mcp`. A project proposal
  waits for `/plugin trust`.

Trust is one decision per plugin, and it covers its MCP servers and its
component. The built-in trusted marketplaces are `claude-plugins-official` and
`hoo-plugins`. A plugin may suggest permissions in its text, but it never grants
them. The permission gate still applies to every gated action.

## Changing a plugin

Updates are a human step: `/plugin update [name]`. No tool edits an installed
plugin. A plugin written by `ProposePlugin` has no marketplace, so it cannot be
updated that way. Edit its files, or remove it and propose it again. A changed
tool is live from the next turn; its skills need `/reload`.

## Two things that are never yours to do

- Never give a subagent a plugin-management tool (`InstallPlugin`,
  `ProposePlugin`, ...). It is rejected, because an agent that can install
  plugins for other agents is a self-propagation primitive.
- Never publish to a marketplace yourself. There is no publish tool. Packaging is
  yours; publishing is a human action, because pushing executable code into a
  marketplace other agents install from unattended is a supply-chain risk.
