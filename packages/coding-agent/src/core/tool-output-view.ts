/**
 * How much of a tool call the transcript shows. Three stops on one dial, least
 * to most, cycled with `app.view.cycleForward` / `app.view.cycleBackward`:
 *
 * - `radar` — one line per *chain*: a run of consecutive tool calls collapsed to
 *   the shape it had while working, or what it amounted to once it is over.
 * - `peek` — the tool's own call line and the first few lines of what came back.
 *   The default.
 * - `full` — the same call line with nothing trimmed away.
 *
 * There is no separate expand state. `full` *is* the expanded view, and
 * `app.tools.expand` jumps straight to it from any stop and back again — which
 * is why the dial is the only thing that has to be remembered, and why `full`
 * can finally mean full.
 *
 * This lives in core rather than next to the renderer because the settings
 * manager persists it, and core must not reach into the interactive UI.
 */
export type ToolOutputView = "radar" | "peek" | "full";

/** The dial's order, least to most. Cycling wraps at both ends. */
export const TOOL_OUTPUT_VIEWS: readonly ToolOutputView[] = ["radar", "peek", "full"];

/**
 * How many lines of a result the `peek` stop shows.
 *
 * Short on purpose. `peek` is the working view — it replaced one that folded
 * the body away entirely — so it has to stay scannable while a run of a dozen
 * calls lands. Its job is "did this find anything, and roughly what", not
 * "read the output"; the whole result is one `app.tools.expand` away.
 */
export const PEEK_LINES = 5;

/** Where a fresh install starts: enough of the result to judge it, no wall of text. */
export const DEFAULT_TOOL_OUTPUT_VIEW: ToolOutputView = "peek";

/** The stop `app.tools.expand` jumps to, and returns from. */
export const MAX_TOOL_OUTPUT_VIEW: ToolOutputView = "full";

/** One-line description per view, shared by the settings pane and `/hotkeys`. */
export const TOOL_OUTPUT_VIEW_DESCRIPTIONS: Record<ToolOutputView, string> = {
	radar: "one line per run of tool calls; failures still show why they failed",
	peek: "the call line and the first few lines of the result",
	full: "the call line and the whole result",
};

/**
 * Values written by versions that had `collapsed` / `peek` / `standard` / `glance`.
 *
 * `peek` is a live name again and needs no entry: a config that still says it
 * lands on the stop it always meant, the light one. `glance` — the call line
 * with its body folded away — becomes that same stop, because a handful of
 * result lines answers "did this find anything" better than a folded body did.
 * `collapsed`, which hid a call's result with no way to see it, becomes `radar`,
 * which hides the same body but says what came back.
 */
export const LEGACY_TOOL_OUTPUT_VIEWS: Record<string, ToolOutputView> = {
	collapsed: "radar",
	glance: "peek",
	standard: "full",
};

export function isToolOutputView(value: unknown): value is ToolOutputView {
	return typeof value === "string" && (TOOL_OUTPUT_VIEWS as readonly string[]).includes(value);
}

/** Next stop on the dial, wrapping at both ends. */
export function cycleToolOutputView(current: ToolOutputView, direction: "forward" | "backward"): ToolOutputView {
	const index = TOOL_OUTPUT_VIEWS.indexOf(current);
	const step = direction === "forward" ? 1 : TOOL_OUTPUT_VIEWS.length - 1;
	return TOOL_OUTPUT_VIEWS[(index + step) % TOOL_OUTPUT_VIEWS.length];
}
