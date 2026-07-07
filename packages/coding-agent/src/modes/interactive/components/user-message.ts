import { Box, Container, Markdown, type MarkdownTheme } from "@kolisachint/hoocode-tui";
import { getMarkdownTheme, theme } from "../theme/theme.js";

const OSC133_ZONE_START = "\x1b]133;A\x07";
const OSC133_ZONE_END = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL = "\x1b]133;C\x07";

/**
 * Component that renders a user message
 */
export class UserMessageComponent extends Container {
	private contentBox: Box;

	constructor(text: string, markdownTheme: MarkdownTheme = getMarkdownTheme()) {
		super();
		this.contentBox = new Box(1, 1, (content: string) => theme.bg("userMessageBg", content));
		this.contentBox.addChild(
			new Markdown(text, 0, 0, markdownTheme, {
				color: (content: string) => theme.fg("userMessageText", content),
			}),
		);
		this.addChild(this.contentBox);
	}

	/** OSC-zone wrap memo: Container.render returns the same array across
	 * frames when nothing changed, so it must not be mutated — the wrapped
	 * copy is cached keyed on the source array's identity. */
	private zoneMemo?: { src: string[]; out: string[] };

	override render(width: number): string[] {
		const lines = super.render(width);
		if (lines.length === 0) {
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
}
