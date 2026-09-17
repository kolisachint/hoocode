/**
 * Copying a conversation into a document, with its structure.
 *
 * The bug behind all of this: what the terminal shows is markdown already
 * rendered — a table is box drawing, a code block is a bordered panel, every
 * line is wrapped to whatever width the window was. Dragging over that and
 * pasting into Word or Confluence pastes the drawing. So `/copy` goes back to
 * the source the model wrote and offers it twice, as markdown and as HTML, and
 * these are the two halves of that: what gets copied, and what it turns into.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { describe, expect, it } from "vitest";
import { sessionToMarkdown } from "../src/core/agent-session-stats.js";
import { markdownToHtml } from "../src/utils/markdown-to-html.js";
import { wrapCfHtml } from "../src/utils/rich-clipboard.js";

function user(text: string): AgentMessage {
	return { role: "user", content: text, timestamp: 0 } as AgentMessage;
}

function agent(text: string): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "text", text }],
		api: "anthropic-messages",
		provider: "anthropic",
		model: "test",
		usage: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		stopReason: "stop",
		timestamp: 0,
	} as AgentMessage;
}

describe("the transcript as markdown", () => {
	const session: AgentMessage[] = [
		user("first question"),
		agent("first answer"),
		user("second question"),
		agent("second answer"),
		user("third question"),
		agent("third answer"),
	];

	it("is the conversation, each turn under its own heading", () => {
		expect(sessionToMarkdown(session.slice(0, 2), { agentLabel: "hoo" })).toBe(
			"## You\n\nfirst question\n\n## hoo\n\nfirst answer",
		);
	});

	it("takes the last n exchanges, counting back from the newest", () => {
		const copied = sessionToMarkdown(session, { turns: 2 });
		expect(copied).toContain("second question");
		expect(copied).toContain("third answer");
		expect(copied).not.toContain("first question");
	});

	it("takes the whole session when nobody says otherwise", () => {
		expect(sessionToMarkdown(session)).toContain("first question");
	});

	it("leaves tool calls out — they are the working, not the answer", () => {
		const withTool: AgentMessage[] = [
			user("read the file"),
			{
				role: "toolResult",
				toolCallId: "1",
				toolName: "read",
				content: [{ type: "text", text: "FILE BODY" }],
				isError: false,
				timestamp: 0,
			} as AgentMessage,
			agent("done"),
		];
		expect(sessionToMarkdown(withTool)).not.toContain("FILE BODY");
	});

	it("says nothing at all about an empty session", () => {
		expect(sessionToMarkdown([])).toBe("");
	});
});

describe("markdown as the HTML a word processor takes", () => {
	it("turns headings into headings", () => {
		expect(markdownToHtml("## Findings")).toBe("<h2>Findings</h2>");
	});

	it("turns a GFM table into a real table", () => {
		const html = markdownToHtml(["| Name | Count |", "| --- | ----: |", "| alpha | 2 |"].join("\n"));
		expect(html).toContain("<table");
		expect(html).toContain("<th");
		expect(html).toContain(">Name<");
		expect(html).toContain(">alpha<");
		// Not a row of pipes and dashes, which is what a terminal selection gives.
		expect(html).not.toContain("---");
	});

	it("keeps a code block whole and literal", () => {
		const html = markdownToHtml(["```ts", "const a = **not bold**;", "```"].join("\n"));
		expect(html).toContain("<pre");
		expect(html).toContain("const a = **not bold**;");
		expect(html).not.toContain("<strong>");
	});

	it("does not read emphasis inside a code span", () => {
		expect(markdownToHtml("run `ls *.ts` first")).toContain("<code");
		expect(markdownToHtml("run `ls *.ts` first")).toContain("ls *.ts");
	});

	it("renders emphasis, links and bare URLs", () => {
		expect(markdownToHtml("**bold** and *italic*")).toContain("<strong>bold</strong>");
		expect(markdownToHtml("**bold** and *italic*")).toContain("<em>italic</em>");
		expect(markdownToHtml("[docs](https://example.com)")).toContain('<a href="https://example.com">docs</a>');
		expect(markdownToHtml("see https://example.com/x for more")).toContain('<a href="https://example.com/x">');
	});

	it("nests a list the way it was written", () => {
		const html = markdownToHtml(["- one", "  - inner", "- two"].join("\n"));
		expect(html).toBe("<ul>\n<li>one</li>\n<ul>\n<li>inner</li>\n</ul>\n<li>two</li>\n</ul>");
	});

	it("keeps an ordered list ordered", () => {
		expect(markdownToHtml("1. first\n2. second")).toContain("<ol>");
	});

	it("renders a quote and a rule", () => {
		expect(markdownToHtml("> quoted")).toContain("<blockquote");
		expect(markdownToHtml("---")).toBe("<hr>");
	});

	it("never lets source markdown become markup", () => {
		// The source is model output: it is text, all of it, however it is spelled.
		const html = markdownToHtml('<script>alert("x")</script> & <b>raw</b>');
		expect(html).not.toContain("<script>");
		expect(html).not.toContain("<b>raw</b>");
		expect(html).toContain("&lt;script&gt;");
		expect(html).toContain("&amp;");
	});

	it("carries its styling on the elements, because a fragment has no stylesheet", () => {
		expect(markdownToHtml("`x`")).toContain("style=");
	});
});

describe("the CF_HTML envelope Windows requires", () => {
	it("points at the fragment it actually contains", () => {
		const wrapped = wrapCfHtml("<p>hello</p>");
		const offset = (name: string): number => Number(wrapped.match(new RegExp(`${name}:(\\d+)`))?.[1]);
		const bytes = Buffer.from(wrapped, "utf8");
		expect(bytes.subarray(offset("StartHTML"), offset("EndHTML")).toString("utf8")).toMatch(/^<html>/);
		expect(bytes.subarray(offset("StartFragment"), offset("EndFragment")).toString("utf8")).toBe("<p>hello</p>");
		expect(offset("EndHTML")).toBe(bytes.length);
	});

	it("counts bytes, not characters", () => {
		const wrapped = wrapCfHtml("<p>héllo — ✓</p>");
		const offset = (name: string): number => Number(wrapped.match(new RegExp(`${name}:(\\d+)`))?.[1]);
		const bytes = Buffer.from(wrapped, "utf8");
		expect(bytes.subarray(offset("StartFragment"), offset("EndFragment")).toString("utf8")).toBe("<p>héllo — ✓</p>");
	});
});
