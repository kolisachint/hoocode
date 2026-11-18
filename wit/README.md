# hoocode:extension WIT package

This package defines `hoocode:extension@0.1.0`, the contract between hoocode
and an internal plugin (a WASM component). It is the source of truth for the
plugin ABI. Plugin repos take it by git tag `wit-v0.1.0`.

## What is in the package

- `types`: `tool-info`, `tool-error`.
- `tool`: the plugin exports `list-tools` and `call`.
- `host`: the host provides `config-get` and `log`.
- `world extension`: imports `host`, exports `tool`.

The package has no WASI dependencies. No WASI WIT is vendored here (hoocode is
MIT-only). The WASI interfaces come from the host, as described below.

## WASI the host provides

The host links these WASI 0.2 interfaces into every component, in addition to
the `extension` world. A plugin's own world must include the ones it uses.

- `wasi:cli/imports@0.2.12`: no environment variables, no arguments.
  stdout and stderr go to the host log. Sockets fail.
- `wasi:filesystem@0.2.12`: only the plugin-data folder is preopened
  (`~/.agents/plugin-data/<name>`). The workspace is never preopened.
- `wasi:clocks@0.2.12` and `wasi:random@0.2.12`.
- `wasi:http/outgoing-handler@0.2.12`: every request goes through the
  permission gate, as WebFetch. An SSRF block refuses private, loopback and
  link-local addresses on every hop.

## Example plugin world

```wit
world plugin {
    include wasi:cli/imports@0.2.12;
    import wasi:http/outgoing-handler@0.2.12;
    include hoocode:extension/extension@0.1.0;
}
```

## Versioning

- WIT semver. A breaking change to this package means a new version.
- The host supports the current and the previous minor version of the world.
- 0.2 adds workspace access (gated like Write and Edit).
- WASI 0.3 means a new world version.
