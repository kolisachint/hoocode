import { visibleWidth } from "@kolisachint/hoocode-tui";
import { beforeAll, describe, expect, test } from "vitest";
import { SelectedRowList } from "../src/modes/interactive/components/selected-row-list.js";
import { initTheme, theme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => {
	initTheme("dark");
});

const rows = [{ text: "first" }, { text: "second", selected: true }, { text: "third" }];

describe("SelectedRowList", () => {
	test("fills the selected row to the full width and leaves the rest alone", () => {
		const width = 80;

		const rendered = new SelectedRowList(rows).render(width);

		const bgOpen = theme.getBgAnsi("selectedBg");
		expect(rendered.filter((line) => line.includes(bgOpen))).toHaveLength(1);
		expect(rendered[1]).toContain("second");
		expect(visibleWidth(rendered[1])).toBe(width);
		// Unselected rows keep their natural width — no band, no padding.
		expect(visibleWidth(rendered[0])).toBe(visibleWidth("first"));
	});

	test("puts the left margin inside the band so it starts at column 0", () => {
		const width = 80;

		const rendered = new SelectedRowList(rows, 2).render(width);

		expect(rendered[1].indexOf(theme.getBgAnsi("selectedBg"))).toBe(0);
		expect(visibleWidth(rendered[1])).toBe(width);
		expect(visibleWidth(rendered[0])).toBe(visibleWidth("  first"));
	});

	test("keeps a row that overruns the width down to the width", () => {
		const width = 20;

		const rendered = new SelectedRowList([{ text: "x".repeat(60), selected: true }]).render(width);

		expect(visibleWidth(rendered[0])).toBe(width);
	});
});
