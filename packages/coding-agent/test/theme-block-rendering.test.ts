/**
 * Every theme loads, and every message block renders under it.
 *
 * The cut-out tokens are optional, which is a promise about the themes that do
 * not set them: they have to render exactly as they did before the tokens
 * existed. Pairing a block's fill with its paper edge made that promise
 * load-bearing in six components at once, so it is checked here against every
 * theme the package ships — discovered from disk, so a theme added later is
 * covered the day it lands — and against a custom theme that sets only what the
 * schema demands.
 */

import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Box, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { afterAll, describe, expect, it } from "vitest";
import { BranchSummaryMessageComponent } from "../src/modes/interactive/components/branch-summary-message.js";
import { CompactionSummaryMessageComponent } from "../src/modes/interactive/components/compaction-summary-message.js";
import { CustomMessageComponent } from "../src/modes/interactive/components/custom-message.js";
import { SkillInvocationMessageComponent } from "../src/modes/interactive/components/skill-invocation-message.js";
import { UserMessageComponent } from "../src/modes/interactive/components/user-message.js";
import {
	applyBlockFill,
	getAvailableThemes,
	getPaperShadowFn,
	initTheme,
	loadThemeFromPath,
	PAPER_INSET,
	setThemeInstance,
	theme,
} from "../src/modes/interactive/theme/theme.js";

const THEME_DIR = join(dirname(new URL(import.meta.url).pathname), "..", "src", "modes", "interactive", "theme");

/** The themes the package ships, straight off disk. */
const BUILT_INS = readdirSync(THEME_DIR)
	.filter((file) => file.endsWith(".json") && file !== "theme-schema.json")
	.map((file) => file.slice(0, -5))
	.sort();

const SHADOW_INK = /[▏▔█]/;
const WIDTHS = [80, 46, 24, 12];

/** Every component that paints a message block, built the way the mode builds it. */
function blocks(): Array<[string, { render: (width: number) => string[] }]> {
	const warning = new Box(1, 1);
	applyBlockFill(warning, "warningBg");
	warning.addChild(new Text("Wait for the current response to finish.", 0, 0));
	const error = new Box(1, 1);
	applyBlockFill(error, "toolErrorBg");
	error.addChild(new Text("Something went wrong.", 0, 0));
	return [
		["user message", new UserMessageComponent("hello **there**, a longer line to wrap")],
		[
			"extension message",
			new CustomMessageComponent({
				role: "custom",
				customType: "skill",
				content: "loaded `bq-sql-authoring`",
				timestamp: 0,
			} as never),
		],
		["skill invocation", new SkillInvocationMessageComponent({ name: "bq-sql-authoring", content: "x" } as never)],
		[
			"compaction summary",
			new CompactionSummaryMessageComponent({ tokensBefore: 100, tokensAfter: 10, summary: "s" } as never),
		],
		["branch summary", new BranchSummaryMessageComponent({ summary: "s" } as never)],
		["warning frame", warning],
		["error frame", error],
	];
}

/** What is wrong with this block's rows, if anything. */
function faults(label: string, lines: string[], width: number, sheet: boolean): string[] {
	const found: string[] = [];
	const rows = lines.filter((line) => visibleWidth(line) > 0);
	if (rows.length === 0) return [`${label}: rendered nothing`];
	for (const [i, line] of rows.entries()) {
		if (visibleWidth(line) > width) found.push(`${label} row ${i}: ${visibleWidth(line)} cells overflows ${width}`);
	}
	if (!sheet) {
		// A theme without `paperShadow` gets the band it always had: full width,
		// no gutter, and not one glyph of shadow ink anywhere.
		for (const [i, line] of rows.entries()) {
			if (visibleWidth(line) !== width)
				found.push(`${label} row ${i}: ${visibleWidth(line)} cells, expected ${width}`);
			if (SHADOW_INK.test(stripAnsi(line))) found.push(`${label} row ${i}: shadow ink on a theme with no paper`);
		}
		return found;
	}
	const band = width - PAPER_INSET;
	const run = stripAnsi(rows[rows.length - 1]);
	if (!run.startsWith(" ")) found.push(`${label}: bottom run does not start on page`);
	if (!run.endsWith("▏")) found.push(`${label}: bottom run ends on ${JSON.stringify(run.at(-1))}, expected ▏`);
	if (visibleWidth(run) !== band + 1)
		found.push(`${label}: bottom run is ${visibleWidth(run)} cells, expected ${band + 1}`);
	for (const [i, line] of rows.slice(1, -1).entries()) {
		if (!stripAnsi(line).includes("▏")) found.push(`${label} row ${i + 1}: no shadow column`);
	}
	return found;
}

describe("every theme the package ships", () => {
	it("has at least the themes this suite expects", () => {
		// A guard on the guard: if the discovery ever came back empty, every
		// theme case below would vacuously pass.
		expect(BUILT_INS.length).toBeGreaterThanOrEqual(8);
		expect(BUILT_INS).toContain("dark");
		expect(BUILT_INS).toContain("vox-cutout-dark");
		expect(getAvailableThemes()).toEqual(expect.arrayContaining(BUILT_INS));
	});

	it.each(BUILT_INS)("%s loads and renders every message block", (name) => {
		expect(() => initTheme(name, false)).not.toThrow();
		expect(theme.name).toBe(name);

		// Which shape this theme asks for, read the way the components read it.
		const sheet = getPaperShadowFn() !== undefined;
		const found: string[] = [];
		for (const width of WIDTHS) {
			for (const [label, block] of blocks()) {
				let lines: string[] = [];
				expect(() => {
					lines = block.render(width);
				}, `${name}: ${label} threw at width ${width}`).not.toThrow();
				found.push(...faults(`${label} @${width}`, lines, width, sheet));
			}
		}
		expect(found).toEqual([]);
	});

	it("follows a theme switch under blocks already on screen, both ways", () => {
		// Blocks outlive themes: what is on screen when the user switches keeps
		// the boxes it was built with. The treatment is resolved per frame for
		// exactly this reason, and routing six components through one call makes
		// that one promise rather than six — so it is checked on all of them.
		initTheme("dark", false);
		const built = blocks();
		for (const [label, block] of built) {
			expect(faults(`${label} built plain`, block.render(46), 46, false)).toEqual([]);
		}
		initTheme("vox-cutout-dark", false);
		for (const [label, block] of built) {
			expect(faults(`${label} switched to paper`, block.render(46), 46, true)).toEqual([]);
		}
		initTheme("light", false);
		for (const [label, block] of built) {
			expect(faults(`${label} switched back`, block.render(46), 46, false)).toEqual([]);
		}
	});

	it("gives exactly the two cut-out themes the paper treatment", () => {
		const withPaper = BUILT_INS.filter((name) => {
			initTheme(name, false);
			return getPaperShadowFn() !== undefined;
		});
		expect(withPaper).toEqual(["vox-cutout-dark", "vox-cutout-light"]);
	});
});

describe("a custom theme that sets only what the schema demands", () => {
	const dir = mkdtempSync(join(tmpdir(), "theme-minimal-"));
	afterAll(() => rmSync(dir, { recursive: true, force: true }));

	it("loads, and every block renders as a plain band", () => {
		// Built from the schema's own required list, so this is the least a theme
		// can legally be: no `warningBg` (it falls back to `customMessageBg`), no
		// `paperShadow`, none of the cut-out pairs.
		const schema = JSON.parse(readFileSync(join(THEME_DIR, "theme-schema.json"), "utf-8"));
		const required: string[] = schema.properties.colors.required;
		const colors = Object.fromEntries(required.map((token) => [token, "#808080"]));
		expect(colors.warningBg).toBeUndefined();
		const path = join(dir, "minimal.json");
		writeFileSync(path, JSON.stringify({ name: "minimal", colors }));

		expect(() => setThemeInstance(loadThemeFromPath(path))).not.toThrow();
		expect(getPaperShadowFn()).toBeUndefined();
		const found: string[] = [];
		for (const width of WIDTHS) {
			for (const [label, block] of blocks()) {
				found.push(...faults(`${label} @${width}`, block.render(width), width, false));
			}
		}
		expect(found).toEqual([]);
	});
});
