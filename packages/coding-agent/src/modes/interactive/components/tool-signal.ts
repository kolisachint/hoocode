import { type Component, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { getTextOutput } from "../../../core/tools/render-utils.js";
import { getCwdRelativePath } from "../../../utils/paths.js";
import { theme } from "../theme/theme.js";

/**
 * The "radar" view's one-line signal for a tool call.
 *
 * Radar answers a different question than the other two views. `glance` and
 * `full` show what a tool *said*; radar shows the *shape of the run* — the same
 * three columns for every tool, so a screen of them reads as a map:
 *
 *   ● bash    npm run check                                     412 lines
 *   ● read    packages/tui/src/keys.ts                          1451 lines
 *   ● grep    toolOutputView                                     27 lines
 *   ● edit    src/core/keybindings.ts                                  ok
 *   ● find    src/**.test.ts                                        error
 *
 * Column 1 is the status dot the other views already use. Column 2 is the tool,
 * padded to a fixed width so the verbs line up and a run of reads is a visible
 * block. Column 3 is the call's subject. Flush right is the signal: how much
 * came back, which is what tells you whether a search actually found anything
 * without opening it.
 */

/** Width of the tool-name column. Fits every built-in name (`websearch`, `TodoWrite`). */
const VERB_WIDTH = 9;

/** Gap between the verb column and the subject. */
const VERB_GAP = 2;

/** Minimum space kept for the flush-right signal before the subject is trimmed. */
const MIN_SIGNAL_GAP = 2;

/**
 * Argument names that carry a call's subject, most specific first.
 *
 * Extensions and MCP servers bring their own argument names, so the list is a
 * preference order rather than a contract: anything unmatched falls back to the
 * first string-valued argument.
 */
const SUBJECT_KEYS = [
	"command",
	"file_path",
	// Before `path`: for grep/find the pattern is what identifies the call and
	// the path is only the scope it was run over.
	"pattern",
	"query",
	"url",
	"path",
	"prompt",
	"description",
	"task_id",
	"subagent_type",
	"name",
	"id",
];

function firstStringArg(args: Record<string, unknown>): string | undefined {
	for (const value of Object.values(args)) {
		if (typeof value === "string" && value.trim()) return value;
	}
	return undefined;
}

/** The single most identifying argument of a call, as a one-line string. */
export function toolSubject(args: unknown, cwd: string): string {
	if (typeof args !== "object" || args === null || Array.isArray(args)) return "";

	const record = args as Record<string, unknown>;
	let subject: string | undefined;
	for (const key of SUBJECT_KEYS) {
		const value = record[key];
		if (typeof value === "string" && value.trim()) {
			subject = value;
			break;
		}
	}
	subject ??= firstStringArg(record);
	if (!subject) return "";

	// Paths are the most common subject and the least readable in absolute form.
	const relative = getCwdRelativePath(subject, cwd);
	const display = relative && relative.length < subject.length ? relative : subject;

	// Multi-line commands (heredocs, chained scripts) collapse to their first line.
	return display.replace(/\s+/g, " ").trim();
}

export interface ToolSignalInput {
	toolName: string;
	args: unknown;
	cwd: string;
	result?: { content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>; isError: boolean };
	isPartial: boolean;
	showImages: boolean;
	/** The newest call in the transcript. Themes that set `activeToolBg` mark it. */
	isLatest?: boolean;
}

/**
 * The flush-right signal: how much output came back, or why none did.
 *
 * Line count is the useful magnitude for nearly every tool here — it is what
 * separates a search that found one hit from one that found four hundred. A
 * result with no text (a successful `edit`, a `TodoWrite`) has no magnitude
 * worth printing, so it reports the outcome instead.
 */
export function toolSignal(input: ToolSignalInput): { text: string; color: "success" | "error" | "warning" } {
	if (!input.result) {
		return { text: input.isPartial ? "running" : "", color: "warning" };
	}
	if (input.result.isError) {
		return { text: "error", color: "error" };
	}

	const output = getTextOutput(input.result, input.showImages);
	if (!output.trim()) {
		return { text: "ok", color: "success" };
	}

	const lines = output.split("\n").length;
	return { text: `${lines} ${lines === 1 ? "line" : "lines"}`, color: "success" };
}

/**
 * Render one radar row, already padded and coloured, without the status dot —
 * the caller prepends that so the dot stays inline with the first line the same
 * way it does in the other two views.
 */
export function renderToolSignalLine(input: ToolSignalInput, width: number): string {
	const name = input.toolName.slice(0, VERB_WIDTH);
	const verb = name.padEnd(VERB_WIDTH + VERB_GAP, " ");
	const signal = toolSignal(input);
	const subject = toolSubject(input.args, input.cwd);

	// `width` already excludes the caller's status-dot prefix.
	const available = Math.max(0, width);
	const signalWidth = signal.text ? visibleWidth(signal.text) + MIN_SIGNAL_GAP : 0;
	const subjectWidth = Math.max(0, available - visibleWidth(verb) - signalWidth);
	const shownSubject = subjectWidth > 0 ? truncateToWidth(subject, subjectWidth) : "";

	const left = verb + shownSubject;
	const pad = Math.max(signal.text ? MIN_SIGNAL_GAP : 0, available - visibleWidth(left) - visibleWidth(signal.text));

	// The marker stroke covers the verb and the subject — the part you read —
	// and stops before the flush-right signal, which keeps its status colour on
	// the page. A highlighter runs over the words, not the whole line.
	const stroke = input.isLatest === true && theme.hasBg("activeToolBg");
	const content = theme.fg("toolTitle", theme.bold(verb)) + theme.fg("toolOutput", shownSubject);

	return (
		(stroke ? theme.bg("activeToolBg", content) : content) + " ".repeat(pad) + theme.fg(signal.color, signal.text)
	);
}

/**
 * Component wrapper so a radar row slots into the same container the call
 * renderers use, and re-renders on resize like everything else.
 */
export class ToolSignalComponent implements Component {
	private input: ToolSignalInput;
	private memo?: { width: number; out: string[] };

	constructor(input: ToolSignalInput) {
		this.input = input;
	}

	setInput(input: ToolSignalInput): void {
		this.input = input;
		this.memo = undefined;
	}

	invalidate(): void {
		this.memo = undefined;
	}

	render(width: number): string[] {
		if (this.memo && this.memo.width === width) return this.memo.out;
		const out = [renderToolSignalLine(this.input, width)];
		this.memo = { width, out };
		return out;
	}
}
