# hoocode-ai-models-catalog

The model catalog as data. `data/models.json` lists hoocode's LLM models, grouped
by provider, in hoocode's order. `data/image-models.json` lists the image models.
Lookup logic lives in `hoocode-ai-models`.

## Where the data comes from

`models.json` is generated from [models.dev](https://models.dev/api.json) by the
`models-sync` tool (crate `hoocode-models-sync`). It never removes an entry. It
refreshes what models.dev reports (prices, and new models), and it keeps the
fields models.dev cannot tell us: `api`, `baseUrl`, `headers`, `compat`,
`thinkingLevelMap` and `name`.

`image-models.json` is not generated. models.dev lists no image models, so it is
maintained by hand.

## Regenerate

Run from the workspace root:

    cargo run -p hoocode-models-sync --bin models-sync

Useful flags:

- `--dry-run` prints the report and writes nothing.
- `--input FILE` reads a saved `api.json` instead of fetching it. Handy for
  offline work and for reviewing a change before it lands.

The tool prints a report: what was added, which prices changed, which prices
differ upstream but were not applied, what was skipped and why. Read it before
you commit.

## Weekly bot PR

`.github/workflows/models-sync.yml` runs the tool every Monday at 06:00 UTC, and
on demand. If the output changed, it opens a PR on branch `bot/models-sync` with
the report as the PR body. Review the report, check the zero prices and the
price changes, then merge.

The PR needs the `MODELS_SYNC_TOKEN` secret (a fine-grained PAT). A PR opened
with the default `GITHUB_TOKEN` does not trigger CI.

## Overrides

`data/overrides.json` holds the hand-maintained rules. The tool reads it on every
run. It has four keys:

- `add`: full model entries hoocode ships that models.dev does not list. An entry
  is added only when it is missing, so the file can rebuild the catalog from
  nothing.
- `allow`: a map from provider to the upstream model ids that may be added as new
  entries. A provider listed here adds only the ids on its list. Existing entries
  are not affected.
- `exclude`: `{ "provider": ..., "id": ... }` pairs that are never emitted, even
  when models.dev lists them. Use this to drop a model for good.
- `keep_prices`: providers whose prices are never refreshed from models.dev. Their
  own API is the source of truth (for example `openrouter`). Differences are still
  reported as "prices differ upstream, not applied".

## What the sync never does

- It does not delete an entry. A model models.dev no longer lists is kept and
  counted in the report as "kept … not listed upstream any more". Remove it with
  `exclude` if it should go.
- It does not overwrite a real price with a zero or missing one. models.dev often
  has `0` where it has no price.
- It does not change curated fields (`reasoning`, `input`, context and output
  limits). A disagreement is reported as drift and not applied.
- It does not set `thinkingLevelMap` on new entries. Review new reasoning models
  for it by hand.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.
