# drawio-canvas

[drawio-canvas](https://github.com/kolisachint/drawio-canvas) is the full
draw.io editor as a canvas that a person and the agent edit at the same time.
The person works in draw.io in their browser, and the agent works through
actions on the same document. Neither waits for the other, and neither
silently overwrites the other. It is the reference for a collaborative
canvas; see [Canvas](canvas.md) for canvases in general.

draw.io 31.4.6 is bundled and served on loopback, so opening it uses no
network. Files are ordinary `.drawio`.

## Install and open

```
/plugin marketplace add https://github.com/kolisachint/drawio-canvas
/plugin install drawio-canvas --scope user
/canvas open drawio-canvas
```

`--scope project` installs it for one project instead. A local checkout works
too: `/plugin marketplace add /path/to/drawio-canvas`. To update, run
`/plugin marketplace refresh`, then `/plugin install drawio-canvas` again.

`/canvas open` prints a url and, in a terminal, opens it. The very first open
unpacks draw.io (about 5 s); after that the editor is ready in about 2 s.

## Working with the diagram

The loop is `get_diagram`, then `edit_diagram` by cell id. Every result also
carries `person`: what they changed since you were last told, and what they
are looking at. Build on their work, never over it.

| Action | Use it to |
|---|---|
| `get_diagram` | Read pages, layers and one page's cells as XML. Large pages return an outline; then pass `cell_ids`. Ids that are not there come back in `missing`. |
| `get_changes` | See what the person did since you were last told, one line per change. Cheap; call it at the start of a turn. |
| `edit_diagram` | Add, update or delete cells, each as a complete `<mxCell>`. Put many operations in one call. |
| `search_shapes` | Find library shapes (AWS, Azure, GCP, Kubernetes, Cisco, UML, BPMN, …) by name. |
| `insert_shapes` | Insert library shapes by id, with draw.io's exact icon style. |
| `replace_diagram` | Replace a page or the document. Refused over the person's unseen work unless `force`. |
| `manage_pages` / `manage_layers` | Pages and layers; rename and delete need a page named explicitly. |
| `screenshot` | A PNG rendered by the person's draw.io, written into the workspace; read the file to see it. |
| `focus` | Select and scroll to cells in the person's editor, with a one-line message. |
| `layout` | Run a draw.io layout, animated for the person. |
| `open_file` / `save_file` | `.drawio` / `.xml`; `.svg` and `.png` rendered by draw.io. |

### Rules the canvas enforces

- **Edits are gated per cell.** Updating or deleting a cell the person changed
  since you last read it is refused with `stale_cells`, and nothing in that
  batch applies. The refusal includes what the person did and the cell as it
  is now, and counts as reading it, so resend your edit built on their version.
  Editing an unread page is refused as `no_context` the same way. Only when
  what you missed is large do you need `get_diagram` first.
- **Batches apply partly.** An operation that fails (unknown id, bad XML, a
  parent that is not on the page) is listed in `errors`, and the rest apply.
  Do not resend the ones that worked.
- **Input is checked against the action's schema.** A wrong field is refused as
  `invalid_input` with what would have worked, including "did you mean" for a
  misspelled field. `null` on an optional field means "not given".
- **Labels are XML attributes.** Write `<`, `>`, `&` and `"` inside `value` as
  `&lt;`, `&gt;`, `&amp;` and `&quot;`. An HTML label also needs `html=1` in the
  style.
- **Layouts are draw.io's own.** Presets are `verticalFlow`, `horizontalFlow`,
  `verticalTree`, `horizontalTree`, `radialTree`, `organic`, `circle`,
  `parallels` and `libavoid`. You can also pass layout JSON such as
  `[{"layout":"mxHierarchicalLayout","config":{"orientation":"west"}}]`. An
  unknown layout is refused at once with the list.
- **Browser actions need the person's tab.** `screenshot`, `focus`, `layout` and
  `.png` export run in their draw.io. While the tab is still loading they wait
  up to 15 s. With no tab open, `focus`, `layout` and `.png` fail with
  `no_editor`; `screenshot` and `.svg` fall back to an approximate render and
  say so.

### How fast

Measured with the person's draw.io open:

| What | Median |
|---|---|
| Your edit appearing in the person's editor | ~30 ms |
| The person's edit reaching `get_changes` | ~12 ms |
| `get_diagram`, `get_changes`, `edit_diagram`, `insert_shapes`, pages, layers | ~1 ms (a 300-cell batch ~50 ms) |
| `search_shapes` | ~4 ms (the first call ~80 ms) |
| `screenshot`, `.png` export | ~30 ms |
| `focus` | ~20 ms (up to ~0.4 s right after another selection change) |
| `layout` (animated) | ~0.2 s |
| A refused call | under 1 ms |

A model turn takes far longer than any of these. What makes the canvas feel
fast is fewer turns: batch operations, and retry from the refusal instead of
re-reading. `list_canvas_capabilities` shows `observed_ms` for this session.

## Error codes

| Code | Meaning | Next step |
|---|---|---|
| `stale_cells` | The person changed a cell you are updating or deleting | Build on the cell as the refusal shows it, and resend |
| `no_context` | You have not read this page | Use the page in the refusal, or `get_diagram` if it was too large to include |
| `invalid_input` | The input does not match the schema | Fix the field the message names |
| `all_operations_failed` | Every operation in the batch failed | Read each operation's reason |
| `invalid_parent` | A cell's parent is not on the page | Use a layer or container id, or omit `parent` |
| `invalid_xml`, `unrecognized_xml` | The XML does not parse, or is not a diagram | Follow the hint in the message |
| `page_not_found`, `layer_not_found` | No such page or layer | The message lists what exists |
| `unknown_shape` | Not a library shape id | `search_shapes` first |
| `invalid_layout` | Not a draw.io layout | The message lists the valid ones |
| `no_editor` | No draw.io tab is open for this action | Ask the person to open the canvas url |
| `file_not_found`, `not_a_file`, `outside_workspace`, `unsupported_extension` | File actions | Workspace-relative `.drawio`, `.xml`, `.svg` or `.png` paths |
| `no_pages`, `last_page`, `last_layer` | The change would leave nothing | Keep at least one page and one layer |

## Testing it

With a checkout of drawio-canvas and Playwright installed outside it:

```bash
# every action, and real draw.io in Chromium
DRAWIO_CANVAS_PLAYWRIGHT=/tmp/pw/node_modules/playwright node --test "test/*.test.mjs"

# the whole collaboration through a real hoocode, with a scripted model
HOOCODE_BIN=<hoocode>/packages/coding-agent/bin/hoocode.js \
DRAWIO_CANVAS_PLAYWRIGHT=/tmp/pw/node_modules/playwright \
  node scripts/e2e-hoocode.mjs --out /tmp/drawio-e2e
```

hoocode's own acceptance test runs it through discovery, `/canvas open` and the
agent tools:

```bash
HOOCODE_DRAWIO_CANVAS_DIR=/path/to/drawio-canvas \
HOOCODE_PLAYWRIGHT=/tmp/pw/node_modules/playwright \
  bunx vitest run test/canvas-acceptance-drawio.test.ts
```

## Related

- [Canvas](canvas.md) — commands, discovery, trust, the agent tools, and troubleshooting
- [Plugins](plugins.md) — marketplaces and install scopes
