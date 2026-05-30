# HooCode UI kit

A high-fidelity, interactive recreation of the **HooCode** terminal coding agent
— built from the real `@kolisachint/hoocode-tui` components, not screenshots.
Open **`index.html`** and you land in a live session.

## Try it
- **Type a prompt** and press Enter → a deterministic agent turn streams in:
  a *thinking* trace, a `read` tool block (syntax-highlighted, truncated preview),
  an `edit` block with an inline **diff**, a `bash` block that goes
  *pending (braille spinner) → success*, and a final markdown summary. The
  **task panel** above the prompt and the **footer** token/cost/context counters
  update live as the turn progresses.
- **`/model`** → inline model selector (↑↓ to move, Enter to pick, Esc to close).
- **`/config`** → resource toggle list (`[x]`/`[ ]`, Space to toggle).
- **`/help`** → command list. **`/clear`** → reset transcript.
- **`!`** at the start of the prompt → **bash mode** (frame turns green); submit
  to run a (faux) command.

Everything is faked client-side — no model is ever called.

## Files
- `index.html` — loads React + Babel, the design tokens, and the kit.
- `styles.css` — terminal-window chrome + block/diff/footer/editor styling.
- `components.jsx` — presentational pieces: `Banner`, `TaskPanel`, `Footer`,
  `ToolBlock`, `Diff`, `Selector`, `UserMsg`, `AssistantText`, `Thinking`,
  `CustomBlock`. All exported to `window`.
- `app.jsx` — the shell: state, the scripted turn engine, slash commands, the
  bash-mode editor, and the `Spinner`.

## Fidelity notes
- Colors, tints, glyphs, the footer token format (`↑12.4k ↓3.2k R45k W1.1k
  $0.084 (sub) 72.0%/200k (auto @ 90%)`), task icons (`●◐✓✗`), the `> `/`!`
  prompt, the braille spinner, and the inline-selector layout are lifted directly
  from the source components.
- This is a cosmetic recreation: tool execution, model calls, and persistence are
  stubbed. It exists to compose pixel-accurate HooCode screens, not to run code.
- Pulls all tokens + self-hosted brand fonts from `../../colors_and_type.css`.
