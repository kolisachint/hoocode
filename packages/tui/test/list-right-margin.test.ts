/**
 * What the list widgets spend on their right margin: nothing.
 *
 * Both lists used to hold two columns back from the right edge — `- 2 // for
 * safety` in `SelectList`, an unexplained `- 2` and a `- 4` in `SettingsList`.
 * Nothing needed saving: every row is truncated to the width it was handed and
 * a selected row is padded out to it, so the margin was two columns of every
 * picker row, on every row, thrown away.
 *
 * These assert the recovery the only way that stays true: a description long
 * enough to fill the row reaches the last cell, and no row ever exceeds it.
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { SelectList } from "../src/components/select-list.js";
import { SettingsList } from "../src/components/settings-list.js";
import { visibleWidth } from "../src/utils.js";

const selectTheme = {
	selectedPrefix: (text: string) => text,
	selectedText: (text: string) => text,
	description: (text: string) => text,
	scrollInfo: (text: string) => text,
	noMatch: (text: string) => text,
};

const settingsTheme = {
	label: (text: string) => text,
	value: (text: string) => text,
	description: (text: string) => text,
	cursor: "› ",
	hint: (text: string) => text,
};

/** Widths wide enough that the description column is actually drawn. */
const WIDTHS = [120, 100, 80, 60];
/** Including the widths where a row falls back to value-only or to no room at all. */
const ALL_WIDTHS = [...WIDTHS, 41, 40, 24, 12, 6, 2];
const LONG = "wide ".repeat(60);

describe("list right margin", () => {
	it("runs a SelectList description to the last cell and never past it", () => {
		for (const width of WIDTHS) {
			const list = new SelectList([{ value: "alpha", label: "alpha", description: LONG }], 5, selectTheme);
			const row = list.render(width)[0];
			assert.equal(visibleWidth(row), width, `@${width}: row is ${visibleWidth(row)} cells`);
		}
	});

	it("never overflows, at any width", () => {
		for (const width of ALL_WIDTHS) {
			const list = new SelectList([{ value: "alpha", label: "alpha", description: LONG }], 5, selectTheme);
			for (const row of list.render(width)) {
				assert.ok(visibleWidth(row) <= width, `@${width}: row is ${visibleWidth(row)} cells`);
			}
		}
	});

	it("runs a SelectList value to the last cell when there is no description", () => {
		for (const width of ALL_WIDTHS) {
			const list = new SelectList([{ value: "v".repeat(400), label: "v".repeat(400) }], 5, selectTheme);
			const row = list.render(width)[0];
			assert.equal(visibleWidth(row), width, `@${width}: row is ${visibleWidth(row)} cells`);
		}
	});

	it("runs a SettingsList value to the last cell and never past it", () => {
		for (const width of WIDTHS) {
			const list = new SettingsList(
				[{ id: "a", label: "a setting", currentValue: LONG, values: [LONG] }],
				5,
				settingsTheme,
				() => {},
				() => {},
			);
			const row = list.render(width)[0];
			assert.equal(visibleWidth(row), width, `@${width}: row is ${visibleWidth(row)} cells`);
		}
	});

	it("wraps a SettingsList description against its own indent, not twice over", () => {
		const width = 60;
		const list = new SettingsList(
			[{ id: "a", label: "a setting", currentValue: "on", values: ["on"], description: LONG }],
			5,
			settingsTheme,
			() => {},
			() => {},
		);
		const wrapped = list.render(width).filter((line) => line.startsWith("  wide"));
		assert.ok(wrapped.length > 1, "the description wraps");
		// Two columns of indent, so the longest wrapped row reaches width - 2 at
		// worst and never exceeds the width.
		for (const line of wrapped) assert.ok(visibleWidth(line) <= width, `${visibleWidth(line)} > ${width}`);
		assert.ok(
			wrapped.some((line) => visibleWidth(line) > width - 8),
			`no wrapped row came near the margin: ${wrapped.map(visibleWidth).join(",")}`,
		);
	});
});
