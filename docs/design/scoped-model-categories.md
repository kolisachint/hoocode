# Scoped models and model categories

Status: **design locked 2026-10-09 (all questions closed), not implemented.** Not on the core plan's build order.
Ask the user before scheduling it.

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
| 4 | Storage | New key `scopedModels` in `~/.hoocode/settings.json`: an ordered list of `{ "model": "provider/id", "effort"?: "<level>", "category"?: "fast\|standard\|capable\|cheap", "alias"?: "short-name" }`. A project override follows the existing settings scoping rules. **Migration:** on first load, if `scopedModels` is missing, build it once from `enabledModels` (a `:level` suffix becomes `effort`) plus `modelCategories` (model to category). After that only `scopedModels` is read or written. Old keys are ignored. hoocode-ts will not see the new scope; the user accepted this. |
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

- `crates/hoocode-code-settings`: `scopedModels` type, read and write, migration from
  `enabledModels` and `modelCategories`, project override.
- `crates/hoocode-code-models`: `resolve_model_scope` reads `scopedModels`; name matching;
  nearest-tier fallback.
- `crates/hoocode-code-subagents`: `model_categories.rs` resolves over the scope; `tools.rs`
  removes `complexity`, makes `model` the only model param, and adds `effort`; `pool.rs` `resolve_task_model`
  applies decisions 8 to 12.
- `crates/hoocode-code-resources`: frontmatter `model:` becomes the default, not the pin.
- `crates/hoocode-code-agent-session`: `cycle_scoped_model` applies the saved effort; the
  system prompt lists the scoped models.
- `crates/hoocode-code-tui-selectors`: `scoped_models_selector.rs` gains the effort and
  category columns.
- `crates/hoocode-code-tui-app`: `mode/models.rs` saves the full entries; `/settings` "Models"
  row; `keybindings.rs` unchanged.
- The `--models` flag parser (crate not yet located; find it with `grep -rn '"--models"'`):
  session-only override.
- `docs/maps/ui.md` must be updated in the same commit as the screen and command changes.
  `docs/maps/packages.md` too, if a crate's contents change in a way the map lists.

## Test plan

- **Unit:** migration (both source keys, `:level` to `effort`, missing and present cases);
  resolution by alias, id and substring; the ambiguous-match error; tie-break by list order;
  nearest-tier fallback in both directions; untagged models never picked for a category;
  precedence of an explicit ask over the frontmatter pin.
- **TUI component golden:** the picker row with the effort and category columns (in
  `hoocode-code-tui-widgets` or the selector's crate, per `tests/golden/<crate>/`).
- **Screen golden:** `/scoped-models` end to end (`scripts/tui/goldens.py`), accepted with
  `goldens.py update <scenario>` once reviewed.
- Done bar: L1 plus `python3 scripts/tui/goldens.py check all`, and a review bundle for the
  visual change.

## Not doing

- Changing hoocode-ts. It keeps reading its own settings and does not see `scopedModels`.
- Category picks for the main model. Categories choose subagent models only.
- A separate settings key for the picker. `scopedModels` is the only key read or written after
  migration.

## Open questions

None. All questions closed 2026-10-09 (decisions 14 to 20).
