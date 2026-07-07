import { Container, Markdown, Spacer } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeAll, describe, expect, test } from "vitest";
import { segmentStreamingMarkdown } from "../src/modes/interactive/components/assistant-message.js";
import { getMarkdownTheme, initTheme } from "../src/modes/interactive/theme/theme.js";

/** Render text as one Markdown vs as segments joined by Spacer(1), and return
 * both outputs normalized (ANSI stripped, right-trimmed, outer blanks dropped). */
function renderBothWays(text: string, width = 100): { single: string; segmented: string; chunks: string[] } {
	const theme = getMarkdownTheme();
	const normalize = (lines: string[]) =>
		lines
			.map((l) => stripAnsi(l).trimEnd())
			.join("\n")
			.replace(/^\n+|\n+$/g, "");
	const single = normalize(new Markdown(text, 1, 0, theme).render(width));
	const chunks = segmentStreamingMarkdown(text);
	const container = new Container();
	for (let k = 0; k < chunks.length; k++) {
		if (k > 0) container.addChild(new Spacer(1));
		container.addChild(new Markdown(chunks[k], 1, 0, theme));
	}
	return { single, segmented: normalize(container.render(width)), chunks };
}

describe("segmentStreamingMarkdown", () => {
	beforeAll(() => {
		initTheme("dark");
	});

	const CASES: Record<string, string> = {
		paragraphs: "First paragraph with some text.\n\nSecond paragraph here.\n\nThird one closes it out.",
		"heading + code + text": [
			"## The plan",
			"",
			"Some intro prose about the change.",
			"",
			"```ts",
			"const x = 1;",
			"",
			"const y = 2; // blank line above must stay inside the fence",
			"```",
			"",
			"Closing remarks after the code block.",
		].join("\n"),
		"loose list stays whole": [
			"Intro line.",
			"",
			"- first item",
			"",
			"- second item of the same loose list",
			"",
			"- third item",
			"",
			"Outro line.",
		].join("\n"),
		"ordered list then paragraph": ["1. one", "2. two", "3. three", "", "A paragraph after the list."].join("\n"),
		"blockquote and hr": ["> quoted wisdom", "> more of it", "", "---", "", "After the rule."].join("\n"),
		"table stays attached": ["Before table.", "", "| a | b |", "| - | - |", "| 1 | 2 |", "", "After table."].join(
			"\n",
		),
		"indented continuation binds up": [
			"- item with body",
			"",
			"  continued body of the item",
			"",
			"Plain after.",
		].join("\n"),
	};

	for (const [name, text] of Object.entries(CASES)) {
		test(`segmented render matches single render: ${name}`, () => {
			const { single, segmented } = renderBothWays(text);
			expect(segmented).toBe(single);
		});
	}

	test("never splits inside a code fence", () => {
		const text = ["```", "a", "", "b", "```"].join("\n");
		expect(segmentStreamingMarkdown(text)).toEqual([text]);
	});

	test("does not segment when link reference definitions are present", () => {
		const text = "See [the docs][ref].\n\nMore text.\n\n[ref]: https://example.com";
		expect(segmentStreamingMarkdown(text)).toEqual([text]);
	});

	test("prefix stability: appending text never changes earlier chunks", () => {
		const parts = [
			"First paragraph of the answer.",
			"Some more prose in a second block.",
			"```ts\nconst z = 42;\n```",
			"- alpha\n- beta",
			"A closing paragraph.",
		];
		let text = "";
		let previous: string[] = [];
		for (const part of parts) {
			text = text ? `${text}\n\n${part}` : part;
			const chunks = segmentStreamingMarkdown(text);
			// All chunks except the last from the previous step must be unchanged.
			for (let i = 0; i < previous.length - 1; i++) {
				expect(chunks[i]).toBe(previous[i]);
			}
			previous = chunks;
		}
		expect(previous.length).toBeGreaterThan(1);
	});
});
