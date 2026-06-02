import { setKeybindings } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { AskQuestion } from "../src/core/extensions/types.js";
import { KeybindingsManager } from "../src/core/keybindings.js";
import { AskOptionsComponent } from "../src/modes/interactive/components/ask-options.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

const UP = "\x1b[A";
const DOWN = "\x1b[B";
const RIGHT = "\x1b[C";
const LEFT = "\x1b[D";
const ESC = "\x1b";

const questions: AskQuestion[] = [
	{
		question: "where should retry logic live?",
		short: "retry lives in",
		detail: "the http client is shared by every provider adapter",
		allowCustom: true,
		options: [
			{ label: "hoocode-ai", description: "unified provider client" },
			{ label: "agent-core", description: "runtime wrap each tool call" },
		],
	},
	{
		question: "backoff strategy?",
		short: "backoff",
		allowCustom: true,
		options: [
			{ label: "exponential + jitter", description: "100ms x2" },
			{ label: "fixed 1s", description: "simple" },
		],
	},
];

describe("AskOptionsComponent", () => {
	beforeAll(() => {
		initTheme("dark");
	});

	beforeEach(() => {
		setKeybindings(new KeybindingsManager());
	});

	it("renders the header, current question, options, and custom row", () => {
		const c = new AskOptionsComponent(
			questions,
			() => {},
			() => {},
		);
		const out = stripAnsi(c.render(80).join("\n"));

		expect(out).toContain("INPUT NEEDED");
		expect(out).toContain("1/2 where should retry logic live?");
		expect(out).toContain("the http client is shared by every provider adapter");
		expect(out).toContain("1 hoocode-ai");
		expect(out).toContain("2 agent-core");
		expect(out).toContain("custom answer");
		expect(out).toContain("(1/3)"); // two options + custom row, index 0
	});

	it("moves the cursor with up/down and wraps", () => {
		const c = new AskOptionsComponent(
			questions,
			() => {},
			() => {},
		);

		c.handleInput(DOWN);
		let out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("> 2 agent-core");
		expect(out).toContain("(2/3)");

		// Up from index 1 -> index 0
		c.handleInput(UP);
		out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("> 1 hoocode-ai");

		// Up from top wraps to the custom row (last)
		c.handleInput(UP);
		out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("(3/3)");
	});

	it("advances with right and submits all answers on the last step", () => {
		let submitted: string[] | undefined;
		const c = new AskOptionsComponent(
			questions,
			(a) => {
				submitted = a;
			},
			() => {},
		);

		// Step 1: pick option 1 (hoocode-ai) with right/confirm
		c.handleInput(RIGHT);
		const out = stripAnsi(c.render(80).join("\n"));
		// Breadcrumb of the answered first step
		expect(out).toContain("retry lives in");
		expect(out).toContain("hoocode-ai");
		expect(out).toContain("2/2 backoff strategy?");

		// Step 2: move to second option then submit
		c.handleInput(DOWN);
		c.handleInput(RIGHT);
		expect(submitted).toEqual(["hoocode-ai", "fixed 1s"]);
	});

	it("quick-picks an option with number keys", () => {
		let submitted: string[] | undefined;
		const c = new AskOptionsComponent(
			questions,
			(a) => {
				submitted = a;
			},
			() => {},
		);

		c.handleInput("2"); // agent-core, advances
		c.handleInput("1"); // exponential + jitter, last step -> submit
		expect(submitted).toEqual(["agent-core", "exponential + jitter"]);
	});

	it("steps back with left, keeping earlier answers editable", () => {
		let submitted: string[] | undefined;
		const c = new AskOptionsComponent(
			questions,
			(a) => {
				submitted = a;
			},
			() => {},
		);

		c.handleInput("1"); // step 1 -> hoocode-ai, advance to step 2
		let out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("2/2 backoff strategy?");

		c.handleInput(LEFT); // back to step 1
		out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("1/2 where should retry logic live?");

		// Re-pick a different first answer, then finish
		c.handleInput("2"); // agent-core
		c.handleInput("1"); // exponential + jitter -> submit
		expect(submitted).toEqual(["agent-core", "exponential + jitter"]);
	});

	it("accepts a typed custom answer", () => {
		let submitted: string[] | undefined;
		const c = new AskOptionsComponent(
			questions,
			(a) => {
				submitted = a;
			},
			() => {},
		);

		// Move to the custom row (index 2) and type
		c.handleInput(DOWN); // 1 -> 2
		c.handleInput(DOWN); // 2 -> custom
		for (const ch of "per-provider") c.handleInput(ch);
		let out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("per-provider");

		c.handleInput(RIGHT); // confirm custom, advance
		out = stripAnsi(c.render(80).join("\n"));
		expect(out).toContain("2/2 backoff strategy?");

		c.handleInput("2"); // fixed 1s -> submit
		expect(submitted).toEqual(["per-provider", "fixed 1s"]);
	});

	it("does not submit an empty custom answer", () => {
		let submitted: string[] | undefined;
		const c = new AskOptionsComponent(
			questions,
			(a) => {
				submitted = a;
			},
			() => {},
		);

		c.handleInput(DOWN);
		c.handleInput(DOWN); // custom row
		c.handleInput(RIGHT); // attempt confirm with empty custom
		const out = stripAnsi(c.render(80).join("\n"));
		// Still on step 1
		expect(out).toContain("1/2 where should retry logic live?");
		expect(submitted).toBeUndefined();
	});

	it("skips with escape", () => {
		let cancelled = false;
		const c = new AskOptionsComponent(
			questions,
			() => {},
			() => {
				cancelled = true;
			},
		);

		c.handleInput(ESC);
		expect(cancelled).toBe(true);
	});
});
