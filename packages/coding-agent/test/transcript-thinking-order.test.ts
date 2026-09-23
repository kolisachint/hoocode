import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage, Usage } from "@kolisachint/hoocode-ai";
import { Container, type MarkdownTheme, type TUI } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeAll, describe, expect, test, vi } from "vitest";
import type { SessionContext } from "../src/core/session-manager.js";
import type { ToolOutputView } from "../src/core/tool-output-view.js";
import type { ToolChainComponent } from "../src/modes/interactive/components/tool-chain.js";
import type { ToolExecutionComponent } from "../src/modes/interactive/components/tool-execution.js";
import { InteractiveMode } from "../src/modes/interactive/interactive-mode.js";
import { getMarkdownTheme, initTheme } from "../src/modes/interactive/theme/theme.js";

/**
 * A thinking trace renders below the tool calls it led to.
 *
 * A chain collects consecutive calls, and it used to stay open until the agent
 * spoke. A message that only thinks and calls tools never speaks, so its
 * component — appended when the message opened, i.e. *below* the previous
 * message's still-open chain — was left stranded while its own calls joined
 * that chain above it. In peek and full, where the trace is drawn, the
 * transcript read: calls, then the thinking that led to them. Radar omits
 * traces, so it has nothing to misorder and keeps its run folded into one line.
 */

const EMPTY_USAGE: Usage = {
	input: 0,
	output: 0,
	cacheRead: 0,
	cacheWrite: 0,
	totalTokens: 0,
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

type FakeMode = {
	pendingTools: Map<string, ToolExecutionComponent>;
	chatContainer: Container;
	footer: { invalidate(): void };
	ui: TUI;
	settingsManager: { getShowImages(): boolean; getImageWidthCells(): number };
	sessionManager: { getCwd(): string };
	session: { retryAttempt: number };
	toolOutputExpanded: boolean;
	toolOutputView: ToolOutputView;
	hideThinkingBlock: boolean;
	hiddenThinkingLabel: string;
	openChain: ToolChainComponent | undefined;
	latestToolBlock: ToolExecutionComponent | undefined;
	latestChain: ToolChainComponent | undefined;
	isInitialized: boolean;
	updateEditorBorderColor(): void;
	getRegisteredToolDefinition(toolName: string): undefined;
	getMarkdownThemeWithSettings(): MarkdownTheme;
	trimTranscriptMemory(): void;
	renderSessionContext(sessionContext: SessionContext): void;
};

/** Borrow a private method off the prototype so the test drives real code. */
function borrow<K extends string>(name: K): FakeMode[keyof FakeMode] {
	return (InteractiveMode.prototype as unknown as Record<string, FakeMode[keyof FakeMode]>)[name];
}

function createFakeMode(view: ToolOutputView): FakeMode {
	const fake = {
		pendingTools: new Map<string, ToolExecutionComponent>(),
		chatContainer: new Container(),
		footer: { invalidate: vi.fn() },
		ui: { requestRender: vi.fn() } as unknown as TUI,
		settingsManager: { getShowImages: () => false, getImageWidthCells: () => 60 },
		sessionManager: { getCwd: () => process.cwd() },
		session: { retryAttempt: 0 },
		toolOutputExpanded: false,
		toolOutputView: view,
		hideThinkingBlock: false,
		hiddenThinkingLabel: "Thinking...",
		openChain: undefined,
		latestToolBlock: undefined,
		latestChain: undefined,
		isInitialized: true,
		updateEditorBorderColor: vi.fn(),
		getRegisteredToolDefinition: (_toolName: string) => undefined,
		getMarkdownThemeWithSettings: () => getMarkdownTheme(),
	} as unknown as FakeMode;
	for (const name of [
		"renderSessionContext",
		"addMessageToChat",
		"attachToolBlock",
		"closeOpenChain",
		"opensNewChain",
		"thinkingDisplayForView",
		"transcriptToolBlocks",
		"trimTranscriptMemory",
	]) {
		(fake as unknown as Record<string, unknown>)[name] = borrow(name);
	}
	return fake;
}

/** An assistant message that thinks, then calls one tool. Never speaks. */
function thinkThenCall(thinking: string, toolCallId: string, query: string): AssistantMessage {
	return {
		role: "assistant",
		content: [
			{ type: "thinking", thinking, thinkingSignature: "" },
			{ type: "toolCall", id: toolCallId, name: "SearchCodebase", arguments: { query } },
		],
		api: "test-api",
		provider: "test-provider",
		model: "test-model",
		usage: EMPTY_USAGE,
		stopReason: "toolUse",
		timestamp: Date.now(),
	};
}

function toolResult(toolCallId: string, text: string): AgentMessage {
	return {
		role: "toolResult",
		toolCallId,
		toolName: "SearchCodebase",
		content: [{ type: "text", text }],
		isError: false,
		timestamp: Date.now(),
	};
}

/** Two think-then-call turns back to back, the second thinking after a result. */
function twoTurns(): SessionContext {
	return {
		messages: [
			thinkThenCall("TRACE_ONE, before the first call", "call-1", "QUERY_ONE"),
			toolResult("call-1", "RESULT_ONE"),
			thinkThenCall("TRACE_TWO, before the second call", "call-2", "QUERY_TWO"),
			toolResult("call-2", "RESULT_TWO"),
		],
		thinkingLevel: "off",
		model: null,
	};
}

function renderLines(fake: FakeMode): string[] {
	return stripAnsi(fake.chatContainer.render(100).join("\n")).split("\n");
}

const lineOf = (lines: string[], needle: string) => lines.findIndex((line) => line.includes(needle));

describe("thinking traces and tool chains keep their order", () => {
	beforeAll(() => {
		initTheme("dark");
	});

	for (const view of ["peek", "full"] as ToolOutputView[]) {
		test(`${view} draws each trace above the call it led to`, () => {
			const fake = createFakeMode(view);
			fake.renderSessionContext(twoTurns());
			const lines = renderLines(fake);

			expect(lineOf(lines, "TRACE_ONE"), "first trace is on screen").toBeGreaterThanOrEqual(0);
			expect(lineOf(lines, "TRACE_TWO"), "second trace is on screen").toBeGreaterThanOrEqual(0);
			// The whole point: the second trace is above its own call, not below it.
			expect(lineOf(lines, "TRACE_TWO")).toBeLessThan(lineOf(lines, "QUERY_TWO"));
			expect(lineOf(lines, "TRACE_ONE")).toBeLessThan(lineOf(lines, "QUERY_ONE"));
			expect(lineOf(lines, "QUERY_ONE")).toBeLessThan(lineOf(lines, "TRACE_TWO"));
		});

		test(`${view} splits the run so a later call cannot land above an earlier trace`, () => {
			const fake = createFakeMode(view);
			fake.renderSessionContext(twoTurns());
			const chains = fake.chatContainer.children.filter(
				(child): child is ToolChainComponent => "toolBlocks" in child,
			);
			expect(chains).toHaveLength(2);
		});
	}

	test("a folded trace still ends the run — the label is on screen too", () => {
		// `hideThinkingBlock` trades the trace for a one-line label. A label is
		// something you can see, so it has the same ordering claim the trace has.
		const fake = createFakeMode("peek");
		fake.hideThinkingBlock = true;
		fake.renderSessionContext(twoTurns());
		const lines = renderLines(fake);
		expect(lines.filter((line) => line.includes("Thinking..."))).toHaveLength(2);
		const secondLabel = lines.map((line) => line.includes("Thinking...")).lastIndexOf(true);
		expect(lineOf(lines, "QUERY_ONE")).toBeLessThan(secondLabel);
		expect(secondLabel).toBeLessThan(lineOf(lines, "QUERY_TWO"));
		const chains = fake.chatContainer.children.filter((child): child is ToolChainComponent => "toolBlocks" in child);
		expect(chains).toHaveLength(2);
	});

	test("radar keeps the run whole, because it draws no trace to misorder", () => {
		const fake = createFakeMode("radar");
		fake.renderSessionContext(twoTurns());
		const lines = renderLines(fake);
		expect(lineOf(lines, "TRACE_ONE")).toBe(-1);
		expect(lineOf(lines, "TRACE_TWO")).toBe(-1);
		const chains = fake.chatContainer.children.filter((child): child is ToolChainComponent => "toolBlocks" in child);
		expect(chains).toHaveLength(1);
		expect(chains[0].toolBlocks).toHaveLength(2);
	});

	test("speaking is still a boundary on its own", () => {
		const fake = createFakeMode("radar");
		fake.renderSessionContext({
			messages: [
				thinkThenCall("TRACE", "call-1", "QUERY_ONE"),
				toolResult("call-1", "RESULT"),
				{
					role: "assistant",
					content: [
						{ type: "text", text: "SPOKEN" },
						{ type: "toolCall", id: "call-2", name: "SearchCodebase", arguments: { query: "QUERY_TWO" } },
					],
					api: "test-api",
					provider: "test-provider",
					model: "test-model",
					usage: EMPTY_USAGE,
					stopReason: "toolUse",
					timestamp: Date.now(),
				},
				toolResult("call-2", "RESULT_TWO"),
			],
			thinkingLevel: "off",
			model: null,
		});
		const chains = fake.chatContainer.children.filter((child): child is ToolChainComponent => "toolBlocks" in child);
		expect(chains).toHaveLength(2);
	});
});
