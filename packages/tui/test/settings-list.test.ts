import assert from "node:assert";
import { describe, it } from "node:test";
import { SettingsList } from "../src/components/settings-list.js";
import { visibleWidth } from "../src/utils.js";

const testTheme = {
	label: (text: string) => text,
	value: (text: string) => text,
	description: (text: string) => text,
	cursor: "→ ",
	hint: (text: string) => text,
};

const items = [
	{ id: "theme", label: "Theme", currentValue: "dark" },
	{ id: "thinking", label: "Thinking", currentValue: "medium" },
];

const noop = () => {};

/** The rendered rows, without the trailing hint/description chrome. */
const rowsOf = (lines: string[]): string[] =>
	lines.filter((line) => line.includes("Theme") || line.includes("Thinking"));

describe("SettingsList", () => {
	it("leaves rows unpadded when the theme defines no selected background", () => {
		const list = new SettingsList(items, 5, testTheme, noop, noop);

		const rows = rowsOf(list.render(60));

		assert.ok(visibleWidth(rows[0]) < 60);
	});

	it("fills the selected row to the full width when the theme paints a band", () => {
		const bandTheme = { ...testTheme, selectedRow: (text: string) => `[BG${text}/BG]` };
		const list = new SettingsList(items, 5, bandTheme, noop, noop);

		const rows = rowsOf(list.render(60));

		assert.ok(rows[0].startsWith("[BG"));
		assert.ok(rows[0].endsWith("/BG]"));
		assert.equal(visibleWidth(rows[0].slice(3, -4)), 60);
		// Only the selected row is banded.
		assert.ok(!rows[1].includes("[BG"));
	});
});
