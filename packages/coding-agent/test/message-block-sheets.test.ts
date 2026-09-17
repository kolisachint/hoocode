/**
 * Every message block is the same kind of object: a sheet of coloured paper.
 *
 * The fill and the edge treatment that goes with it used to be two independent
 * decisions made at each call site, and they drifted exactly where you would
 * expect. `[skill]`, `[compaction]` and `[branch]` blocks were each written to
 * use "the same background colour as custom messages for visual consistency"
 * and each stopped at the colour, so under a cut-out theme they rendered as
 * flat full-width bands beside the sheets they were copying — same paint, no
 * gutter, no cut edge, no shadow, in the same transcript.
 *
 * `applyBlockFill` makes it one decision. These tests hold both halves of it:
 * that every block renders as a sheet, and that no new one can be added with
 * the fill alone.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Box, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeEach, describe, expect, it } from "vitest";
import { BranchSummaryMessageComponent } from "../src/modes/interactive/components/branch-summary-message.js";
import { CompactionSummaryMessageComponent } from "../src/modes/interactive/components/compaction-summary-message.js";
import { CustomMessageComponent } from "../src/modes/interactive/components/custom-message.js";
import { SkillInvocationMessageComponent } from "../src/modes/interactive/components/skill-invocation-message.js";
import { UserMessageComponent } from "../src/modes/interactive/components/user-message.js";
import { applyBlockFill, initTheme, PAPER_INSET } from "../src/modes/interactive/theme/theme.js";

const WIDTH = 46;

/** Every component that paints a message block, built the way the mode builds it. */
const BLOCKS: Array<[string, () => { render: (width: number) => string[] }]> = [
	["user message", () => new UserMessageComponent("hello there")],
	[
		"extension message",
		() =>
			new CustomMessageComponent({
				role: "custom",
				customType: "skill",
				content: "loaded",
				timestamp: 0,
			} as never),
	],
	["skill invocation", () => new SkillInvocationMessageComponent({ name: "bq-sql-authoring", content: "x" } as never)],
	[
		"compaction summary",
		() => new CompactionSummaryMessageComponent({ tokensBefore: 100, tokensAfter: 10, summary: "s" } as never),
	],
	["branch summary", () => new BranchSummaryMessageComponent({ summary: "s" } as never)],
	[
		"warning frame",
		() => {
			// The shape interactive-mode's showBlock() builds for errors and warnings.
			const box = new Box(1, 1);
			applyBlockFill(box, "warningBg");
			box.addChild(new Text("Wait for the current response to finish.", 0, 0));
			return box;
		},
	],
];

describe("every message block is a sheet under a cut-out theme", () => {
	beforeEach(() => initTheme("vox-cutout-dark", false));

	it.each(BLOCKS)("%s", (_label, build) => {
		const lines = build()
			.render(WIDTH)
			.filter((line) => visibleWidth(line) > 0);
		const band = WIDTH - PAPER_INSET;

		// It holds back from the right margin, so it has an edge to cut.
		for (const line of lines.slice(0, -1)) {
			expect(visibleWidth(line)).toBeLessThanOrEqual(band + 1);
		}
		// Rows below the first carry the shadow's column, and it ends on the run.
		expect(lines.slice(1, -1).every((line) => stripAnsi(line).includes("▏"))).toBe(true);
		const run = stripAnsi(lines[lines.length - 1]);
		expect(run.startsWith(" ")).toBe(true);
		expect(run.endsWith("▏")).toBe(true);
		expect(visibleWidth(run)).toBe(band + 1);
	});

	it("draws the plain full-width band on a theme with no paper", () => {
		// The other half of the promise: nothing here is cut-out-only.
		initTheme("dark", false);
		for (const [, build] of BLOCKS) {
			const lines = build()
				.render(WIDTH)
				.filter((line) => visibleWidth(line) > 0);
			expect(lines.every((line) => visibleWidth(line) === WIDTH)).toBe(true);
			expect(lines.some((line) => /[▏▔█]/.test(stripAnsi(line)))).toBe(false);
		}
	});
});

describe("the block fill and its edge cannot drift apart", () => {
	// A source scan, because the failure it guards against is a component that
	// was never wired up at all — which no amount of rendering the components we
	// know about would catch.
	const ROOT = join(import.meta.dirname, "..", "src", "modes", "interactive");
	const BLOCK_FILLS = ["userMessageBg", "customMessageBg", "warningBg", "toolErrorBg"];

	it("no component reaches for a block fill on its own", () => {
		const dir = join(ROOT, "components");
		const files = [
			...readdirSync(dir)
				.filter((name) => name.endsWith(".ts"))
				.map((name) => join(dir, name)),
			join(ROOT, "interactive-mode.ts"),
		];
		const offenders: string[] = [];
		for (const file of files) {
			const source = readFileSync(file, "utf-8");
			for (const fill of BLOCK_FILLS) {
				// `theme.bg("customMessageBg", …)` in a component means the fill was
				// applied without the edge that belongs with it. Route it through
				// applyBlockFill instead.
				if (source.includes(`theme.bg("${fill}"`)) offenders.push(`${file}: theme.bg("${fill}")`);
			}
		}
		expect(offenders).toEqual([]);
	});
});
