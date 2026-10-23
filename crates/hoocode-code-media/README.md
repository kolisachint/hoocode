# hoocode-code-media

Image handling for the hoocode coding agent. This is the adapter crate for the `image`
dependency (see `scripts/ci/dep-firewall.json`); other crates use its own types.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

- MIME sniffing: `detect_supported_image_mime_type` (magic bytes, limited to
  jpeg/png/gif/webp).
- `resize_image`: fit within 2000x2000 and 4.5MB of base64, PNG vs JPEG,
  lower JPEG quality, then shrink by 0.75 steps; `format_dimension_note` describes the result.
- EXIF orientation is applied before measuring (via `image`).
- Clipboard support: `clipboard`, `clipboard_image`, `rich_clipboard` and
  `markdown_to_html`.

Encoded bytes depend on the `image` crate's encoders; dimensions, formats and notes are stable.
