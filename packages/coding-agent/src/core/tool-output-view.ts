/**
 * How much of a tool call the transcript shows. Three stops on one dial, least
 * to most, cycled with `app.view.cycleForward` / `app.view.cycleBackward`:
 *
 * - `radar` — one aligned signal line per call: tool, subject, and how much came
 *   back. A screen of them reads as a map of the run rather than a log of it.
 * - `glance` — the tool's own call line with a ▸ caret; the body is folded away
 *   until the expand key asks for it. The default.
 * - `full` — call line plus the result body, truncated then expandable.
 *
 * The expand key (`app.tools.expand`) is orthogonal: it opens what is in front
 * of you without moving the dial.
 *
 * This lives in core rather than next to the renderer because the settings
 * manager persists it, and core must not reach into the interactive UI.
 */
export type ToolOutputView = "radar" | "glance" | "full";

/** The dial's order, least to most. Cycling wraps at both ends. */
export const TOOL_OUTPUT_VIEWS: readonly ToolOutputView[] = ["radar", "glance", "full"];

/** One-line description per view, shared by the settings pane and `/hotkeys`. */
export const TOOL_OUTPUT_VIEW_DESCRIPTIONS: Record<ToolOutputView, string> = {
	radar: "one signal line per call: tool, subject, and how much came back",
	glance: "the call line with a ▸ caret; body folded away until you ask",
	full: "call line plus the result body",
};

/**
 * Values written by versions that had `collapsed` / `peek` / `standard`.
 *
 * `peek` was already what most people wanted and `standard` was the default
 * nobody kept, so the mapping is not a rename: `peek` becomes the new default
 * `glance`, and `collapsed` — which hid a call's result with no way to see it —
 * becomes `radar`, which hides the same body but says what came back.
 */
export const LEGACY_TOOL_OUTPUT_VIEWS: Record<string, ToolOutputView> = {
	collapsed: "radar",
	peek: "glance",
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
