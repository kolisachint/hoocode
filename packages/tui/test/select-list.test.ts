import assert from "node:assert";
import { describe, it } from "node:test";
import { SelectList } from "../src/components/select-list.js";
import { visibleWidth } from "../src/utils.js";

const testTheme = {
	selectedPrefix: (text: string) => text,
	selectedText: (text: string) => text,
	description: (text: string) => text,
	scrollInfo: (text: string) => text,
	noMatch: (text: string) => text,
};

const visibleIndexOf = (line: string, text: string): number => {
	const index = line.indexOf(text);
	assert.notEqual(index, -1);
	return visibleWidth(line.slice(0, index));
};

describe("SelectList", () => {
	it("normalizes multiline descriptions to single line", () => {
		const items = [
			{
				value: "test",
				label: "test",
				description: "Line one\nLine two\nLine three",
			},
		];

		const list = new SelectList(items, 5, testTheme);
		const rendered = list.render(100);

		assert.ok(rendered.length > 0);
		assert.ok(!rendered[0].includes("\n"));
		assert.ok(rendered[0].includes("Line one Line two Line three"));
	});

	it("keeps descriptions aligned when the primary text is truncated", () => {
		const items = [
			{ value: "short", label: "short", description: "short description" },
			{
				value: "very-long-command-name-that-needs-truncation",
				label: "very-long-command-name-that-needs-truncation",
				description: "long description",
			},
		];

		const list = new SelectList(items, 5, testTheme);
		const rendered = list.render(80);

		assert.equal(visibleIndexOf(rendered[0], "short description"), visibleIndexOf(rendered[1], "long description"));
	});

	it("uses the configured minimum primary column width", () => {
		const items = [
			{ value: "a", label: "a", description: "first" },
			{ value: "bb", label: "bb", description: "second" },
		];

		const list = new SelectList(items, 5, testTheme, {
			minPrimaryColumnWidth: 12,
			maxPrimaryColumnWidth: 20,
		});
		const rendered = list.render(80);

		assert.equal(rendered[0].indexOf("first"), 14);
		assert.equal(rendered[1].indexOf("second"), 14);
	});

	it("uses the configured maximum primary column width", () => {
		const items = [
			{
				value: "very-long-command-name-that-needs-truncation",
				label: "very-long-command-name-that-needs-truncation",
				description: "first",
			},
			{ value: "short", label: "short", description: "second" },
		];

		const list = new SelectList(items, 5, testTheme, {
			minPrimaryColumnWidth: 12,
			maxPrimaryColumnWidth: 20,
		});
		const rendered = list.render(80);

		assert.equal(visibleIndexOf(rendered[0], "first"), 22);
		assert.equal(visibleIndexOf(rendered[1], "second"), 22);
	});

	it("leaves rows unpadded when the theme defines no selected background", () => {
		const items = [
			{ value: "first", label: "first" },
			{ value: "second", label: "second" },
		];

		const rendered = new SelectList(items, 5, testTheme).render(40);

		assert.equal(visibleWidth(rendered[0]), visibleWidth("→ first"));
	});

	it("fills the selected row to the full width when the theme paints a band", () => {
		const items = [
			{ value: "first", label: "first" },
			{ value: "second", label: "second" },
		];
		const bandTheme = { ...testTheme, selectedRow: (text: string) => `[BG${text}/BG]` };

		const rendered = new SelectList(items, 5, bandTheme).render(40);

		// The band wraps the row, and the row inside it reaches the right edge.
		assert.ok(rendered[0].startsWith("[BG"));
		assert.ok(rendered[0].endsWith("/BG]"));
		assert.equal(visibleWidth(rendered[0].slice(3, -4)), 40);
		// Unselected rows are untouched.
		assert.ok(!rendered[1].includes("[BG"));
		assert.equal(visibleWidth(rendered[1]), visibleWidth("  second"));
	});

	it("paints the band across the description column too", () => {
		const items = [{ value: "first", label: "first", description: "does a thing" }];
		const bandTheme = { ...testTheme, selectedRow: (text: string) => `[BG${text}/BG]` };

		const rendered = new SelectList(items, 5, bandTheme).render(60);

		assert.ok(rendered[0].includes("does a thing"));
		assert.equal(visibleWidth(rendered[0].slice(3, -4)), 60);
	});

	it("allows overriding primary truncation while preserving description alignment", () => {
		const items = [
			{
				value: "very-long-command-name-that-needs-truncation",
				label: "very-long-command-name-that-needs-truncation",
				description: "first",
			},
			{ value: "short", label: "short", description: "second" },
		];

		const list = new SelectList(items, 5, testTheme, {
			minPrimaryColumnWidth: 12,
			maxPrimaryColumnWidth: 12,
			truncatePrimary: ({ text, maxWidth }) => {
				if (text.length <= maxWidth) {
					return text;
				}

				return `${text.slice(0, Math.max(0, maxWidth - 1))}…`;
			},
		});
		const rendered = list.render(80);

		assert.ok(rendered[0].includes("…"));
		assert.equal(visibleIndexOf(rendered[0], "first"), visibleIndexOf(rendered[1], "second"));
	});
});
