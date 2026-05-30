---
name: hoocode-design
description: Use this skill to generate well-branded interfaces and assets for HooCode (a deterministic terminal coding agent / pi-mono fork), either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping the terminal (TUI) experience.
user-invocable: true
---

Read the `README.md` file within this skill, and explore the other available files.

HooCode is a **terminal** product — its design language is a monospaced text grid
painted with ANSI colors and Unicode glyphs. There are no overlays, modals,
shadows, gradients, rounded cards, or emoji. Default to the transcript aesthetic;
resist web-app instincts.

Key files:
- `colors_and_type.css` — all color tokens (dark + light themes), the type system,
  and `@font-face` for the self-hosted brand fonts (JetBrains Mono, MesloLGS NF).
- `README.md` — product context, content/voice rules, visual foundations, the full
  glyph/iconography vocabulary, and the file index.
- `preview/` — small reference cards for colors, type, spacing, components, brand.
- `ui_kits/hoocode/` — an interactive HTML/JSX recreation of the terminal
  (transcript, tool blocks, diff, task panel, footer, editor, inline selectors).
- `assets/wordmark.txt` — the ASCII wordmark.

If creating visual artifacts (slides, mocks, throwaway prototypes, etc), copy assets
out and create static HTML files for the user to view. If working on production
code, copy assets and read the rules here to become an expert in designing with
this brand.

If the user invokes this skill without any other guidance, ask them what they want
to build or design, ask some questions, and act as an expert designer who outputs
HTML artifacts _or_ production code, depending on the need.
