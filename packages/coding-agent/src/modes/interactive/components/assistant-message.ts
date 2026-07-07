import type { AssistantMessage } from "@kolisachint/hoocode-ai";
import { Container, type DefaultTextStyle, Markdown, type MarkdownTheme, Spacer, Text } from "@kolisachint/hoocode-tui";
import { getMarkdownTheme, theme } from "../theme/theme.js";

const OSC133_ZONE_START = "\x1b]133;A\x07";
const OSC133_ZONE_END = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL = "\x1b]133;C\x07";

// Streaming messages below this size render as a single Markdown; above it the
// text is segmented at stable block boundaries so each throttle tick re-parses
// only the growing tail instead of the whole accumulated message.
const SEGMENT_MIN_CHARS = 2048;

const LIST_ITEM_RE = /^ {0,3}(?:[-*+] |\d{1,9}[.)] )/;
const FENCE_RE = /^ {0,3}(?:```|~~~)/;
const LINK_DEF_RE = /^ {0,3}\[[^\]]+\]: /m;
const SETEXT_UNDERLINE_RE = /^ {0,3}(?:=+|-+)\s*$/;

/** True when a blank-line gap between `prev` and `next` (both non-blank) is a
 * safe place to cut the markdown into independently-parseable chunks. */
function isSafeBoundary(prev: string, next: string): boolean {
	// Indented continuation (loose list item body, indented code) binds to the
	// block above the gap.
	if (/^\s/.test(next)) return false;
	// A blank line between two list items is a loose list, not two lists.
	if (LIST_ITEM_RE.test(prev) && LIST_ITEM_RE.test(next)) return false;
	// Tables and raw HTML blocks can span blank lines in surprising ways.
	if (next.startsWith("|") || prev.trimStart().startsWith("|")) return false;
	if (next.startsWith("<") || prev.trimStart().startsWith("<")) return false;
	// A bare ===/--- line after the gap could lex as setext underline or hr
	// differently without its preceding text; keep it attached.
	if (SETEXT_UNDERLINE_RE.test(next)) return false;
	return true;
}

/**
 * Split markdown into chunks at blank-line boundaries where each chunk lexes
 * independently to the same blocks the whole text would. Prefix-stable:
 * appending text never changes earlier boundaries (decisions depend only on
 * preceding fence parity and the lines adjacent to each gap), so during
 * streaming every chunk except the last is byte-identical across updates and
 * its Markdown render cache keeps hitting. Exported for tests.
 */
export function segmentStreamingMarkdown(text: string): string[] {
	// Reference-style link/footnote definitions resolve across the whole
	// document; segmenting would break lookups from other chunks.
	if (LINK_DEF_RE.test(text)) return [text];
	const lines = text.split("\n");
	const chunks: string[] = [];
	let chunkStart = 0;
	let fenceOpen = false;
	let prevNonblank = "";
	let i = 0;
	while (i < lines.length) {
		const line = lines[i];
		if (FENCE_RE.test(line)) fenceOpen = !fenceOpen;
		if (line.trim() === "" && !fenceOpen) {
			let j = i + 1;
			while (j < lines.length && lines[j].trim() === "") j++;
			// Only cut when the gap has content on both sides; a trailing blank
			// run stays attached so the decision never has to be revisited.
			if (j < lines.length && chunkStart < i && isSafeBoundary(prevNonblank, lines[j])) {
				chunks.push(lines.slice(chunkStart, i).join("\n"));
				chunkStart = j;
			}
			i = j;
			continue;
		}
		if (line.trim() !== "") prevNonblank = line;
		i++;
	}
	const last = lines.slice(chunkStart).join("\n");
	if (last.trim() !== "") {
		chunks.push(last);
	}
	return chunks.length > 0 ? chunks : [text];
}

/**
 * Component that renders a complete assistant message
 */
export class AssistantMessageComponent extends Container {
	private contentContainer: Container;
	private hideThinkingBlock: boolean;
	private markdownTheme: MarkdownTheme;
	private hiddenThinkingLabel: string;
	private lastMessage?: AssistantMessage;
	private hasToolCalls = false;
	// Markdown children reused across streaming updates, keyed by content index +
	// kind. Markdown caches its rendered lines by (text, width); recreating the
	// instances on every streamed delta discarded those caches and re-parsed the
	// entire message (thinking trace included) per frame. Reuse keeps finished
	// blocks cached so only the block whose text actually changed re-parses.
	private markdownCache = new Map<string, { md: Markdown; text: string }>();

	constructor(
		message?: AssistantMessage,
		hideThinkingBlock = false,
		markdownTheme: MarkdownTheme = getMarkdownTheme(),
		hiddenThinkingLabel = "Thinking...",
	) {
		super();

		this.hideThinkingBlock = hideThinkingBlock;
		this.markdownTheme = markdownTheme;
		this.hiddenThinkingLabel = hiddenThinkingLabel;

		// Container for text/thinking content
		this.contentContainer = new Container();
		this.addChild(this.contentContainer);

		if (message) {
			this.updateContent(message);
		}
	}

	override invalidate(): void {
		// Cached Markdown blocks may be detached right now (e.g. hidden thinking);
		// drop their render caches too so a theme/width change can't resurface
		// stale styling when they re-attach.
		for (const { md } of this.markdownCache.values()) {
			md.invalidate();
		}
		super.invalidate();
		if (this.lastMessage) {
			this.updateContent(this.lastMessage);
		}
	}

	private reuseMarkdown(key: string, text: string, style?: DefaultTextStyle): Markdown {
		const entry = this.markdownCache.get(key);
		if (entry) {
			if (entry.text !== text) {
				entry.md.setText(text);
				entry.text = text;
			}
			return entry.md;
		}
		const md = new Markdown(text, 1, 0, this.markdownTheme, style);
		this.markdownCache.set(key, { md, text });
		return md;
	}

	/** Set when the current children include streaming segments; the next
	 * non-streaming (final/rebuild) render purges their cache entries. */
	private hasSegmentedBlocks = false;

	/**
	 * Add one markdown content block. Large blocks still being streamed are
	 * segmented at stable boundaries so only the tail chunk re-parses per
	 * update; every earlier chunk is byte-stable and stays cached. The final
	 * (non-streaming) render collapses back to one canonical Markdown, so any
	 * segmentation artifact is transient by construction.
	 */
	private addMarkdownBlock(keyBase: string, text: string, streaming: boolean, style?: DefaultTextStyle): void {
		if (streaming && text.length >= SEGMENT_MIN_CHARS) {
			const chunks = segmentStreamingMarkdown(text);
			if (chunks.length > 1) {
				for (let k = 0; k < chunks.length; k++) {
					if (k > 0) this.contentContainer.addChild(new Spacer(1));
					this.contentContainer.addChild(this.reuseMarkdown(`${keyBase}:seg:${k}`, chunks[k], style));
				}
				this.hasSegmentedBlocks = true;
				return;
			}
		}
		this.contentContainer.addChild(this.reuseMarkdown(keyBase, text, style));
	}

	setHideThinkingBlock(hide: boolean): void {
		this.hideThinkingBlock = hide;
		if (this.lastMessage) {
			this.updateContent(this.lastMessage);
		}
	}

	setHiddenThinkingLabel(label: string): void {
		this.hiddenThinkingLabel = label;
		if (this.lastMessage) {
			this.updateContent(this.lastMessage);
		}
	}

	/** OSC-zone wrap memo: Container.render returns the same array across
	 * frames when nothing changed, so it must not be mutated — the wrapped
	 * copy is cached keyed on the source array's identity. */
	private zoneMemo?: { src: string[]; out: string[] };

	override render(width: number): string[] {
		const lines = super.render(width);
		if (this.hasToolCalls || lines.length === 0) {
			return lines;
		}

		if (this.zoneMemo?.src === lines) {
			return this.zoneMemo.out;
		}
		const out = lines.slice();
		out[0] = OSC133_ZONE_START + out[0];
		out[out.length - 1] = OSC133_ZONE_END + OSC133_ZONE_FINAL + out[out.length - 1];
		this.zoneMemo = { src: lines, out };
		return out;
	}

	updateContent(message: AssistantMessage, streaming = false): void {
		this.lastMessage = message;

		// A final/rebuild render replaces streaming segments with the canonical
		// single-Markdown form; drop the segment cache entries they used.
		if (!streaming && this.hasSegmentedBlocks) {
			for (const key of this.markdownCache.keys()) {
				if (key.includes(":seg:")) this.markdownCache.delete(key);
			}
			this.hasSegmentedBlocks = false;
		}

		// Clear content container
		this.contentContainer.clear();

		const hasVisibleContent = message.content.some(
			(c) => (c.type === "text" && c.text.trim()) || (c.type === "thinking" && c.thinking.trim()),
		);

		if (hasVisibleContent) {
			this.contentContainer.addChild(new Spacer(1));
		}

		// Render content in order
		for (let i = 0; i < message.content.length; i++) {
			const content = message.content[i];
			if (content.type === "text" && content.text.trim()) {
				// Assistant text messages with no background - trim the text
				// Set paddingY=0 to avoid extra spacing before tool executions
				this.addMarkdownBlock(`${i}:text`, content.text.trim(), streaming);
			} else if (content.type === "thinking" && content.thinking.trim()) {
				// Add spacing only when another visible assistant content block follows.
				// This avoids a superfluous blank line before separately-rendered tool execution blocks.
				const hasVisibleContentAfter = message.content
					.slice(i + 1)
					.some((c) => (c.type === "text" && c.text.trim()) || (c.type === "thinking" && c.thinking.trim()));

				if (this.hideThinkingBlock) {
					// Show static thinking label when hidden
					this.contentContainer.addChild(
						new Text(theme.italic(theme.fg("thinkingText", this.hiddenThinkingLabel)), 1, 0),
					);
					if (hasVisibleContentAfter) {
						this.contentContainer.addChild(new Spacer(1));
					}
				} else {
					// Thinking traces in thinkingText color, italic, with ✻ prefix
					this.addMarkdownBlock(`${i}:thinking`, `✻ ${content.thinking.trim()}`, streaming, {
						color: (text: string) => theme.fg("thinkingText", text),
						italic: true,
					});
					if (hasVisibleContentAfter) {
						this.contentContainer.addChild(new Spacer(1));
					}
				}
			}
		}

		// Check if aborted - show after partial content
		// But only if there are no tool calls (tool execution components will show the error)
		const hasToolCalls = message.content.some((c) => c.type === "toolCall");
		this.hasToolCalls = hasToolCalls;
		if (!hasToolCalls) {
			if (message.stopReason === "aborted") {
				const abortMessage =
					message.errorMessage && message.errorMessage !== "Request was aborted"
						? message.errorMessage
						: "Operation aborted";
				if (hasVisibleContent) {
					this.contentContainer.addChild(new Spacer(1));
				} else {
					this.contentContainer.addChild(new Spacer(1));
				}
				this.contentContainer.addChild(new Text(theme.fg("error", abortMessage), 1, 0));
			} else if (message.stopReason === "error") {
				const errorMsg = message.errorMessage || "Unknown error";
				this.contentContainer.addChild(new Spacer(1));
				this.contentContainer.addChild(new Text(theme.fg("error", `Error: ${errorMsg}`), 1, 0));
			}
		}
	}
}
