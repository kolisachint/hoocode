# Scoped models design

Status: Locked 2026-10-09 (reviewed). Not implemented.
Not on the core plan's build order. Ask the user before scheduling it.

## Goal

One list of models in the TUI that also says how each model is used. Each scoped model gets
an effort level and, optionally, a category (`fast`, `standard`, `capable`, `cheap`). The main
model can then send subagent work by category ("cheap subagent") or by model ("use haiku").
Today `enabledModels` and `modelCategories` are two separate settings, and `modelCategories`
is read only by subagents.

## Current state (verified)

| Piece | Where |
|---|---|
| `enabledModels` (`provider/id`, glob, optional `:level`) | `crates/hoocode-code-settings/src/manager.rs` (~1281); resolved by `resolve_model_scope` in `crates/hoocode-code-models/src/resolver.rs` (~291) |
| `/scoped-models` picker | `crates/hoocode-code-tui-selectors/src/scoped_models_selector.rs`; save in `crates/hoocode-code-tui-app/src/mode/models.rs` (~235) writes ids only and drops the effort suffix |
| Cycling (alt+m, shift+alt+m) | `keybindings.rs` (~85); `cycle_scoped_model` in `crates/hoocode-code-agent-session/src/session.rs` (~1737) |
| `modelCategories` (`fast`, `standard`, `capable`), one model each | `crates/hoocode-code-subagents/src/model_categories.rs`; used only by subagents, in `pool.rs` `resolve_task_model` (~1760-1795) |
| Agent tool `complexity` param | `crates/hoocode-code-subagents/src/tools.rs` (~576-599, 797-808) |
| Category resolution ignores the scope | `ToolContext.available_models` (`session.rs` ~1074) is all logged-in models |
| Agent frontmatter `model:` | `crates/hoocode-code-resources/src/agent_frontmatter.rs` (~131). Today a pinned agent model beats the tool's ask |

The main model never sees concrete model names today.

## Decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Flow | In the TUI the user picks scoped models. For each one: set an effort (thinking level), then optionally a category: `fast`, `standard`, `capable` or `cheap` (`cheap` is new). Then save. alt+m cycles the scoped models as before. |
| 2 | Editor | Upgrade `/scoped-models` in place. Each row gains an effort column and a category column. `/settings` gets a "Models" row that opens it. |
| 3 | Cardinality | Each scoped model has 0 or 1 category. Several models may share a category. |
| 4 | Storage | New key `scopedModels` in `~/.hoocode/settings.json`: an ordered list of `{ "model": "provider/id", "effort"?: "<level>", "category"?: "fast\|standard\|capable\|cheap", "alias"?: "short-name" }`. A project override replaces the global list (decision 15). **Migration:** on first load, if `scopedModels` is missing, build it once from `enabledModels` (a `:level` suffix becomes `effort`) plus `modelCategories` (model to category). After that only `scopedModels` is read or written. Old keys are ignored. hoocode-ts will not see the new scope; the user accepted this. |
| 5 | Cycling | Switching to a scoped model also applies its saved effort. A model with no saved effort keeps the current level. |
| 6 | Subagent ask | The Agent tool's `model` takes either a category name or a scoped model reference. It is the only model param; `complexity` is removed now. An optional `effort` param overrides the scoped effort; the default is the scoped entry's effort. The system prompt lists the scoped models (alias or id, effort, category) so the model can choose. |
| 7 | Name matching | Alias first (exact). Otherwise exact id. Otherwise a unique substring of the id. If more than one matches, it is an error that lists the candidates. |
| 8 | Hard limit | Subagents may use only scoped models. If nothing is scoped, use all logged-in models as today, with the existing derived-category logic. |
| 9 | Tie-break | If several models share a category, take the first in list order. The picker's reorder sets the priority. |
| 10 | Empty category | Fall back to the nearest tier within the scope. Order: `cheap` to `fast` to `standard` to `capable`. When a higher tier is missing, step down. The tool result states which model was actually used. |
| 11 | Untagged models | Usable by name or alias only. Never picked for a category. |
| 12 | Precedence | An explicit ask from the main model (model or category) beats the agent's frontmatter `model:` pin. The pin is the default. This flips today's order. |
| 13 | `--models` flag | Kept, session only. It overrides `scopedModels` for that run, as `id:effort` with no categories. A category ask then uses the nearest-tier fallback over that list. With no categories, "nearest tier" uses the existing derived-category logic, restricted to that list. |
| 14 | Lowest tier empty | Step up to the nearest tier above. This applies when the lowest available tier is empty (e.g. no `cheap` or `fast` model). The tool result names the model used. |
| 15 | Project `scopedModels` | A project `scopedModels` replaces the global list. It does not merge. |
| 16 | `complexity` | Removed now, not aliased. `model` is the only param. |
| 17 | Unsupported effort | Clamp to the closest level the model supports. The picker offers only the levels that model supports. |
| 18 | Empty list | `scopedModels: []` counts as present, so no migration runs. |
| 19 | Aliases | Unique across the list. Only `[a-z0-9-]` characters. |
| 20 | Migration write | Migration writes `scopedModels` to disk on first load. |

## What we build

Affected files and crates (from Current state):

- `crates/hoocode-code-settings`: `src/types.rs` gets the `scopedModels` type; `src/manager.rs`
  reads and writes it, runs the migration from `enabledModels` and `modelCategories` on first
  load, and applies the project replace (decision 15).
- `crates/hoocode-code-models`: `src/resolver.rs` `resolve_model_scope` reads `scopedModels`;
  name matching (decision 7); nearest-tier fallback (decisions 10, 14); clamp effort (decision 17).
- `crates/hoocode-code-subagents`: `src/model_categories.rs` resolves categories over the scope;
  `src/tools.rs` removes `complexity`, makes `model` the only model param, and adds `effort`;
  `src/pool.rs` `resolve_task_model` applies decisions 8 to 12; `templates/prompts/task-main.md`
  gets the scoped-model list the main model chooses from.
- `crates/hoocode-code-resources`: `src/agent_frontmatter.rs` `model:` becomes the default, not
  the pin (decision 12).
- `crates/hoocode-code-agent-session`: `src/session.rs` `cycle_scoped_model` applies the saved
  effort (decision 5); the system prompt lists the scoped models.
- `crates/hoocode-code-tui-selectors`: `src/scoped_models_selector.rs` gains the effort and
  category columns (decisions 1, 2, 3, 17); `src/settings_selector.rs` gets the "Models" row.
- `crates/hoocode-code-tui-app`: `src/mode/models.rs` saves the full entries (the save currently
  drops the effort suffix); `src/mode/settings.rs` opens the picker from the "Models" row;
  `keybindings.rs` unchanged (alt+m and shift+alt+m keep their bindings).
- `crates/hoocode-code-cli`: `src/args.rs` `--models` parser stays, session-only override
  (decision 13); `src/runtime.rs` takes the override in place of `scopedModels` for that run.
- `crates/hoocode-app-server`: `src/server.rs` line ~56 doc comment names `enabledModels`; update
  the wording with the rest of the change.
- `docs/maps/ui.md` must be updated in the same commit as the screen and command changes.
  `docs/maps/packages.md` too, if a crate's contents change in a way the map lists.

## Test plan

- **Unit:** migration (both source keys, `:level` to `effort`, missing and present cases);
  migration runs on first load and is written to disk, and a second load does not re-migrate;
  `scopedModels: []` counts as present, so no migration runs; old keys are ignored after
  migration; project `scopedModels` replaces the global list and does not merge (decision 15);
  resolution by alias, id and substring; the ambiguous-match error; alias validation (unique
  across the list, `[a-z0-9-]` only; decision 19); tie-break by list order; nearest-tier fallback
  in both directions; step-up fallback when the lowest tier (`cheap`) is empty, with the tool
  result naming the model used; untagged models never picked for a category; effort clamped to
  the closest level the model supports (decision 17); hard limit: subagents never get an
  unscoped model, and an empty scope falls back to all logged-in models (decision 8);
  precedence of an explicit ask over the frontmatter pin, and the pin is used when the main model
  asks for nothing (decision 12); the Agent tool schema has `model` and `effort` only, with no
  `complexity` (decisions 6, 16); an `effort` param overrides the scoped effort; `--models`
  overrides `scopedModels` for that run only and is not written to disk (decision 13); cycling
  (alt+m) applies the saved effort, and a model with no saved effort keeps the current level
  (decision 5); the picker saves `effort` and `category` for each entry, not ids only.
- **TUI component golden:** the picker row with the effort and category columns (in
  `hoocode-code-tui-selectors`'s golden tests, per `tests/golden/<crate>/`).
- **Screen golden:** `/scoped-models` end to end (`scripts/tui/goldens.py`), accepted with
  `goldens.py update <scenario>` once reviewed.
- Done bar: L1 plus `python3 scripts/tui/goldens.py check all`, and a review bundle for the
  visual change.

## Not doing

- Changing hoocode-ts. It keeps reading its own settings and does not see `scopedModels`.
- Category picks for the main model. Categories choose subagent models only.
- A separate settings key for the picker. `scopedModels` is the only key read or written after
  migration.

## Closed questions

All questions were closed on 2026-10-09 (decisions 14 to 20).
