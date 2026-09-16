/**
 * Every surface that asks the user for something wears the prompt's frame.
 *
 * These panes all take the prompt editor's place in `editorContainer`, and they
 * used to draw three different things there: two bare rules (most pickers), no
 * chrome at all (`/model`), or two rules plus a title row and a blank line
 * (`ask_options`, the extension input, the login dialog). The one surface the
 * eye keeps returning to changed shape whenever the agent needed an answer.
 *
 * So the geometry is asserted for all of them at once, against the editor
 * itself as the reference: same border style, same corners, same gutter, same
 * width to the last cell, at every width from a wide terminal down to a
 * pathological one. A new input surface that forgets the frame fails here.
 */

import { setKeybindings, visibleWidth } from "@kolisachint/hoocode-tui";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { AuthStorage } from "../src/core/auth-storage.js";
import type { AskQuestion } from "../src/core/extensions/types.js";
import { KeybindingsManager } from "../src/core/keybindings.js";
import type { SessionTreeNode } from "../src/core/session-manager.js";
import { AskOptionsComponent } from "../src/modes/interactive/components/ask-options.js";
import { ExtensionEditorComponent } from "../src/modes/interactive/components/extension-editor.js";
import { ExtensionInputComponent } from "../src/modes/interactive/components/extension-input.js";
import { ExtensionSelectorComponent } from "../src/modes/interactive/components/extension-selector.js";
import {
	getInputFrameBorder,
	InputFrame,
	setInputFrameBorder,
} from "../src/modes/interactive/components/input-frame.js";
import { LoginDialogComponent } from "../src/modes/interactive/components/login-dialog.js";
import { OAuthSelectorComponent } from "../src/modes/interactive/components/oauth-selector.js";
import { SessionColorSelectorComponent } from "../src/modes/interactive/components/session-color-selector.js";
import { SessionSelectorComponent } from "../src/modes/interactive/components/session-selector.js";
import { ShowImagesSelectorComponent } from "../src/modes/interactive/components/show-images-selector.js";
import { ThemeSelectorComponent } from "../src/modes/interactive/components/theme-selector.js";
import { ThinkingSelectorComponent } from "../src/modes/interactive/components/thinking-selector.js";
import { TreeSelectorComponent } from "../src/modes/interactive/components/tree-selector.js";
import { UserMessageSelectorComponent } from "../src/modes/interactive/components/user-message-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

/** Enough of a TUI for a component that only reads the terminal's size. */
const fauxTui = { terminal: { columns: 100, rows: 40 }, requestRender: () => {} } as never;

const ANSI = /\x1b\[[0-9;?]*[a-zA-Z]|\x1b[_\]][^\x07\x1b]*(\x07|\x1b\\)/g;
const strip = (line: string): string => line.replace(ANSI, "");

const questions: AskQuestion[] = [
	{
		question: "where should retry logic live?",
		short: "retry",
		detail: "the http client is shared by every provider adapter",
		allowCustom: true,
		options: [
			{ label: "hoocode-ai", description: "unified provider client" },
			{ label: "agent-core", description: "runtime wrap each tool call" },
		],
	},
];

const tree: SessionTreeNode[] = [
	{
		entry: {
			type: "message",
			id: "user-1",
			parentId: null,
			timestamp: new Date(0).toISOString(),
			message: { role: "user", content: "hello" },
		},
		children: [],
	} as never,
];

/** Every input surface, by the name a reader would call it. */
function surfaces(): Array<[string, { render(width: number): string[] }]> {
	return [
		[
			"ask_options",
			new AskOptionsComponent(
				questions,
				() => {},
				() => {},
			),
		],
		[
			"extension input",
			new ExtensionInputComponent(
				"name this session",
				undefined,
				() => {},
				() => {},
			),
		],
		[
			"extension editor",
			new ExtensionEditorComponent(
				fauxTui,
				KeybindingsManager.create(),
				"commit message",
				"wip",
				() => {},
				() => {},
			),
		],
		[
			"extension selector",
			new ExtensionSelectorComponent(
				"pick one",
				["alpha", "beta"],
				() => {},
				() => {},
			),
		],
		["login dialog", new LoginDialogComponent(fauxTui, "anthropic", () => {})],
		[
			"oauth selector",
			new OAuthSelectorComponent(
				"login",
				AuthStorage.inMemory(),
				[{ id: "anthropic", name: "Anthropic", authType: "oauth" }],
				() => {},
				() => {},
			),
		],
		[
			"session colour selector",
			new SessionColorSelectorComponent(
				"brisk-prairie",
				1,
				() => {},
				() => {},
			),
		],
		[
			"show images selector",
			new ShowImagesSelectorComponent(
				true,
				() => {},
				() => {},
			),
		],
		[
			"theme selector",
			new ThemeSelectorComponent(
				"dark",
				() => {},
				() => {},
				() => {},
			),
		],
		[
			"thinking selector",
			new ThinkingSelectorComponent(
				"off",
				["off", "low", "high"],
				() => {},
				() => {},
			),
		],
		[
			"tree selector",
			new TreeSelectorComponent(
				tree,
				"user-1",
				24,
				() => {},
				() => {},
			),
		],
		[
			"fork-from-message selector",
			new UserMessageSelectorComponent(
				[{ id: "user-1", text: "hello there", timestamp: new Date(0).toISOString() } as never],
				() => {},
				() => {},
			),
		],
	];
}

/** Wide, awkward, and narrower than a box can be drawn in. */
const WIDTHS = [160, 120, 100, 80, 60, 40, 24, 12, 8, 5, 4, 3, 2];

describe("every user-input surface wears the prompt's frame", () => {
	beforeAll(() => initTheme("dark"));
	beforeEach(() => {
		setKeybindings(new KeybindingsManager());
		setInputFrameBorder("box");
	});

	it("is a box, with corners, at every width a box fits", () => {
		for (const [name, surface] of surfaces()) {
			for (const width of [160, 120, 100, 80, 60, 40, 24, 12, 8, 4]) {
				const lines = surface.render(width).map(strip);
				expect(lines.length, `${name} @${width} rendered nothing`).toBeGreaterThan(2);
				expect(lines[0][0], `${name} @${width} top-left`).toBe("┌");
				expect(lines[0].at(-1), `${name} @${width} top-right`).toBe("┐");
				expect(lines.at(-1)![0], `${name} @${width} bottom-left`).toBe("└");
				expect(lines.at(-1)!.at(-1), `${name} @${width} bottom-right`).toBe("┘");
			}
		}
	});

	it("fills the terminal to the last cell and never past it, at every width", () => {
		for (const [name, surface] of surfaces()) {
			for (const width of WIDTHS) {
				for (const [row, line] of surface.render(width).map(strip).entries()) {
					expect(visibleWidth(line), `${name} @${width}: row ${row}`).toBe(width);
				}
			}
		}
	});

	it("insets its content by one column, the way the prompt does", () => {
		for (const [name, surface] of surfaces()) {
			for (const line of surface.render(100).map(strip).slice(1, -1)) {
				expect(line[0], `${name}: left border`).toBe("│");
				expect(line[1], `${name}: gutter`).toBe(" ");
				expect(line.at(-1), `${name}: right border`).toBe("│");
				expect(line.at(-2), `${name}: right gutter`).toBe(" ");
			}
		}
	});

	it("follows the editorBorder setting, so the prompt and its stand-ins agree", () => {
		setInputFrameBorder("rule");
		expect(getInputFrameBorder()).toBe("rule");
		for (const [name, surface] of surfaces()) {
			const lines = surface.render(100).map(strip);
			// No corners and no sides — but the top border still carries the
			// surface's name, exactly as the prompt's rule carries the session chip.
			expect(lines[0], `${name}: top rule`).toMatch(/^─+/);
			expect(lines[0], `${name}: top rule has no corner`).not.toContain("┌");
			expect(lines.at(-1), `${name}: bottom rule`).toBe("─".repeat(100));
			for (const [row, line] of lines.entries()) {
				expect(line, `${name} rule mode: row ${row} has no side border`).not.toContain("│");
				expect(visibleWidth(line), `${name} rule mode: row ${row}`).toBe(100);
			}
		}
	});

	it("names itself in the top border, not on a row of its own", () => {
		const named: Array<[{ render(width: number): string[] }, string]> = [
			[
				new ThinkingSelectorComponent(
					"off",
					["off"],
					() => {},
					() => {},
				),
				"thinking",
			],
			[
				new ThemeSelectorComponent(
					"dark",
					() => {},
					() => {},
					() => {},
				),
				"theme",
			],
			[
				new AskOptionsComponent(
					questions,
					() => {},
					() => {},
				),
				"input needed",
			],
			[
				new ExtensionInputComponent(
					"rename",
					undefined,
					() => {},
					() => {},
				),
				"rename",
			],
			[
				new TreeSelectorComponent(
					tree,
					"user-1",
					24,
					() => {},
					() => {},
				),
				"session tree",
			],
		];
		for (const [surface, title] of named) {
			const lines = surface.render(100).map(strip);
			expect(lines[0], `top border carries "${title}"`).toContain(title);
			// And only there: a title row inside the frame would be saying it twice.
			expect(lines.slice(1, -1).filter((line) => line.includes(title))).toEqual([]);
		}
	});

	it("re-reads the border style on a pane that is already open", () => {
		// A /settings edit has to reach the surface the user is looking at.
		const pane = new ThinkingSelectorComponent(
			"off",
			["off"],
			() => {},
			() => {},
		);
		expect(strip(pane.render(100)[0])[0]).toBe("┌");
		setInputFrameBorder("rule");
		expect(strip(pane.render(100)[0])[0]).toBe("─");
	});
});

describe("an input surface embedded in another framed surface", () => {
	beforeAll(() => initTheme("dark"));
	beforeEach(() => setKeybindings(new KeybindingsManager()));

	it("draws no frame of its own, and names itself on its first row instead", () => {
		const pane = new AskOptionsComponent(
			questions,
			() => {},
			() => {},
			{ framed: false },
		);
		const lines = pane.render(80).map(strip);
		expect(lines[0][0]).not.toBe("┌");
		expect(lines.some((line) => line.includes("│"))).toBe(false);
		expect(lines[0]).toContain("INPUT NEEDED");
	});
});

describe("InputFrame", () => {
	beforeAll(() => initTheme("dark"));
	beforeEach(() => setInputFrameBorder("box"));

	it("keeps the hints as the last row whatever order the pane was built in", () => {
		const frame = new InputFrame({ title: "demo" });
		frame.setHint("enter submit");
		frame.addChild({ render: () => ["a body row"], invalidate: () => {} });
		const lines = frame.render(60).map(strip);
		expect(lines[1]).toContain("a body row");
		expect(lines[2]).toContain("enter submit");
	});

	it("replaces the hints rather than stacking them", () => {
		const frame = new InputFrame({ title: "demo" });
		frame.setHint("first");
		frame.setHint("second");
		const lines = frame.render(60).map(strip);
		expect(lines.filter((line) => line.includes("first"))).toEqual([]);
		expect(lines.filter((line) => line.includes("second")).length).toBe(1);
	});
});

describe("the session picker", () => {
	beforeAll(() => initTheme("dark"));
	beforeEach(() => {
		setKeybindings(new KeybindingsManager());
		setInputFrameBorder("box");
	});

	it("wears the frame once its sessions have loaded", async () => {
		const picker = new SessionSelectorComponent(
			async () => [],
			async () => [],
			() => {},
			() => {},
			() => {},
			() => {},
			{ keybindings: KeybindingsManager.create() },
		);
		await new Promise((resolve) => setTimeout(resolve, 0));
		const lines = picker.render(100).map(strip);
		expect(lines[0][0]).toBe("┌");
		expect(lines[0]).toContain("sessions");
		for (const [row, line] of lines.entries()) {
			expect(visibleWidth(line), `row ${row}`).toBe(100);
		}
	});
});
