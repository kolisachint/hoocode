# hoocode-ai-models-catalog

The model catalog as data: hoocode's `models.generated.ts` and `image-models.generated.ts`
at the pinned commit, flattened to JSON arrays in hoocode's order. Lookup logic lives in
`hoocode-ai-models` (LLM models).

Regenerate on every hoocode pin bump (after `migration/tui-parity/setup_hoocode.sh`):

    python3 scripts/convert_models_to_json.py

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.
