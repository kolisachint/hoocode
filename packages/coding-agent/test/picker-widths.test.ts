import { setKeybindings, visibleWidth } from "@kolisachint/hoocode-tui";
import { beforeAll, beforeEach, describe, expect, test } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import type { SessionEntry, SessionInfo, SessionMessageEntry, SessionTreeNode } from "../src/core/session-manager.js";
import { AskOptionsComponent } from "../src/modes/interactive/components/ask-options.js";
import { ExtensionSelectorComponent } from "../src/modes/interactive/components/extension-selector.js";
import { SessionSelectorComponent } from "../src/modes/interactive/components/session-selector.js";
import { TreeSelectorComponent } from "../src/modes/interactive/components/tree-selector.js";
import { UserMessageSelectorComponent } from "../src/modes/interactive/components/user-message-selector.js";
import { initTheme, SELECT_CURSOR, SELECT_GUTTER, theme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => {
	initTheme("dark");
});

beforeEach(() => {
	setKeybindings(new KeybindingsManager());
});

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

/** Long enough that a picker which forgot to truncate will overrun the width. */
const LONG = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore";

function makeSession(id: string, firstMessage: string): SessionInfo {
	return {
		path: `/tmp/${id}.jsonl`,
		id,
		cwd: "/some/quite/deeply/nested/working/directory/for/the/project",
		created: new Date(0),
		modified: new Date(0),
		messageCount: 1,
		firstMessage,
		allMessagesText: firstMessage,
	};
}

function userMessage(id: string, parentId: string | null, content: string): SessionMessageEntry {
	return {
		type: "message",
		id,
		parentId,
		timestamp: new Date().toISOString(),
		message: { role: "user", content, timestamp: Date.now() },
	};
}

function assistantMessage(id: string, parentId: string | null, text: string): SessionMessageEntry {
	return {
		type: "message",
		id,
		parentId,
		timestamp: new Date().toISOString(),
		message: {
			role: "assistant",
			content: [{ type: "text", text }],
			api: "anthropic-messages",
			provider: "anthropic",
			model: "claude-sonnet-4.5",
			usage: {
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 0,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			stopReason: "stop",
			timestamp: Date.now(),
		},
	};
}

function buildTree(entries: SessionEntry[]): SessionTreeNode[] {
	const nodes: SessionTreeNode[] = entries.map((entry) => ({ entry, children: [] }));
	const byId = new Map(nodes.map((node) => [node.entry.id, node]));
	const roots: SessionTreeNode[] = [];
	for (const node of nodes) {
		const parent = node.entry.parentId === null ? undefined : byId.get(node.entry.parentId);
		if (parent) parent.children.push(node);
		else roots.push(node);
	}
	return roots;
}

/** Widths worth checking: cramped, ordinary, and wide. */
const WIDTHS = [40, 80, 160];

const overruns = (lines: string[], width: number): string[] =>
	lines.filter((line) => visibleWidth(line) > width).map((line) => `${visibleWidth(line)} > ${width}: ${line}`);

const bandedRows = (lines: string[]): string[] => lines.filter((line) => line.includes(theme.getBgAnsi("selectedBg")));

describe("picker widths", () => {
	test("the cursor and the gutter that replaces it are the same width", () => {
		expect(visibleWidth(SELECT_GUTTER)).toBe(visibleWidth(SELECT_CURSOR));
	});

	test.each(WIDTHS)("session picker fits within %i columns and fills its selected row", async (width) => {
		const selector = new SessionSelectorComponent(
			async () => [makeSession("a", LONG), makeSession("b", "short")],
			async () => [],
			() => {},
			() => {},
			() => {},
			() => {},
			{ showRenameHint: true, keybindings: new KeybindingsManager() },
		);
		await flush();

		const lines = selector.render(width);

		expect(overruns(lines, width)).toEqual([]);
		for (const row of bandedRows(lines)) {
			expect(visibleWidth(row)).toBe(width);
		}
	});

	test.each(WIDTHS)("session tree fits within %i columns and fills its selected row", (width) => {
		const entries: SessionEntry[] = [userMessage("user-1", null, LONG), assistantMessage("asst-1", "user-1", LONG)];
		const selector = new TreeSelectorComponent(
			buildTree(entries),
			"asst-1",
			24,
			() => {},
			() => {},
		);

		const lines = selector.render(width);

		expect(overruns(lines, width)).toEqual([]);
		for (const row of bandedRows(lines)) {
			expect(visibleWidth(row)).toBe(width);
		}
	});

	test.each(WIDTHS)("fork-from-message picker fits within %i columns", (width) => {
		const selector = new UserMessageSelectorComponent(
			[
				{ id: "1", text: LONG },
				{ id: "2", text: "short" },
			],
			() => {},
			() => {},
		);

		const lines = selector.render(width);

		expect(overruns(lines, width)).toEqual([]);
		for (const row of bandedRows(lines)) {
			expect(visibleWidth(row)).toBe(width);
		}
	});

	test.each(WIDTHS)("extension picker fits within %i columns", (width) => {
		const selector = new ExtensionSelectorComponent(
			"Pick one",
			[LONG, "short"],
			() => {},
			() => {},
		);

		const lines = selector.render(width);

		expect(overruns(lines, width)).toEqual([]);
		for (const row of bandedRows(lines)) {
			expect(visibleWidth(row)).toBe(width);
		}
	});

	test.each(WIDTHS)("ask-for-input pane fits within %i columns", (width) => {
		const component = new AskOptionsComponent(
			[
				{
					question: LONG,
					options: [{ label: LONG, description: LONG }, { label: "short" }],
					allowCustom: true,
				},
			],
			() => {},
			() => {},
		);

		expect(overruns(component.render(width), width)).toEqual([]);
	});

	test("ask-for-input keeps its numbering aligned between active and inactive rows", () => {
		const component = new AskOptionsComponent(
			[{ question: "q", options: [{ label: "first" }, { label: "second" }] }],
			() => {},
			() => {},
		);

		const lines = component.render(80);
		const first = lines.find((l) => l.includes("first"))!;
		const second = lines.find((l) => l.includes("second"))!;

		// The row cursor is wider than a space now, so the number column only
		// stays put if the inactive row is indented by the cursor's width.
		expect(visibleWidth(first.slice(0, first.indexOf("first")))).toBe(
			visibleWidth(second.slice(0, second.indexOf("second"))),
		);
	});
});
