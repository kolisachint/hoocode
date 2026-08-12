/**
 * Pins the shared listing layout.
 *
 * Nothing asserted on these listings before, which is how three surfaces ended
 * up rendering the same plugin three different ways. The properties that matter
 * are alignment, the hanging indent on wrapped detail, and the plain-text
 * guarantee for anything the model or an RPC client reads.
 */

import { describe, expect, it } from "vitest";
import {
	availablePluginGroups,
	installedPluginRows,
	pluginCapabilities,
} from "../src/core/extensions/plugins/listing.js";
import { plural, renderCompactRows, renderList } from "../src/core/format-list.js";

const availableFixture = [
	{
		name: "code-review",
		description: "Structured multi-pass code review over the current diff or a named PR.",
		supportPlatform: ["claude", "github"],
		sourceKind: "git",
		marketplaceName: "official",
		source: "https://example.test/code-review",
	},
	{
		name: "pdf",
		description: "Read and merge PDFs.",
		supportPlatform: ["claude"],
		sourceKind: "git",
		marketplaceName: "official",
		source: "https://example.test/pdf",
	},
	{
		name: "terraform-guard",
		supportPlatform: ["agents"],
		sourceKind: "git-subdir",
		marketplaceName: "acme",
		source: "https://example.test/acme/tree/main/terraform-guard",
	},
] as never as Parameters<typeof availablePluginGroups>[0];

describe("plural", () => {
	it("does not print the plugin(s) hedge", () => {
		expect(plural(1, "plugin")).toBe("1 plugin");
		expect(plural(0, "plugin")).toBe("0 plugins");
		expect(plural(2, "marketplace")).toBe("2 marketplaces");
	});
});

describe("renderList", () => {
	it("aligns the name column across every group, not per group", () => {
		const text = renderList(availablePluginGroups(availableFixture), { indent: 2 });
		// Rows are the lines carrying a fact separator; the longest name lives in
		// the first group and the shortest in the last, so per-group alignment
		// would show up as two different columns here.
		const rowLines = text.split("\n").filter((l) => l.includes(" · "));
		expect(rowLines).toHaveLength(3);

		const nameColumns = rowLines.map((l) => l.search(/\S/));
		expect(new Set(nameColumns).size).toBe(1);

		// Facts begin at a fixed column: indent + padded name + gap, whatever the
		// name's own length.
		const factColumns = rowLines.map((l) => l.length - l.replace(/^\s+\S+\s+/, "").length);
		expect(new Set(factColumns).size).toBe(1);
	});

	it("indents wrapped detail under its row instead of restarting at column 0", () => {
		const text = renderList(availablePluginGroups(availableFixture), { indent: 2, columns: 60 });
		const detailLines = text.split("\n").filter((l) => l.includes("code review") || l.includes("named PR"));
		expect(detailLines.length).toBeGreaterThan(1);
		for (const line of detailLines) {
			expect(line.startsWith("        ")).toBe(true);
		}
	});

	it("emits no escape codes when unstyled, so tool results and RPC stay plain", () => {
		const text = renderList(availablePluginGroups(availableFixture), { indent: 2, columns: 80 });
		expect(text).not.toContain("\x1b[");
	});

	it("keeps padding correct when styling wraps the name in escape codes", () => {
		const styled = renderList(
			[
				{
					rows: [
						{ name: "a", facts: ["x"] },
						{ name: "bbbb", facts: ["y"] },
					],
				},
			],
			{
				style: { name: (t) => `\x1b[31m${t}\x1b[39m` },
			},
		);
		const [first, second] = styled.split("\n");
		// Strip the codes: the fact column must still line up.
		const plain = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
		expect(plain(first).indexOf("x")).toBe(plain(second).indexOf("y"));
	});

	it("marks installed plugins and keeps unmarked rows aligned with them", () => {
		const text = renderList(availablePluginGroups(availableFixture, { installed: new Set(["code-review"]) }), {
			indent: 2,
		});
		const marked = text.split("\n").find((l) => l.includes("code-review"));
		const unmarked = text.split("\n").find((l) => l.includes("pdf"));
		expect(marked).toContain("✓");
		expect(marked?.indexOf("code-review")).toBe(unmarked?.indexOf("pdf"));
	});

	it("groups available plugins by the marketplace offering them", () => {
		const groups = availablePluginGroups(availableFixture);
		expect(groups.map((g) => g.title)).toEqual(["official", "acme"]);
		expect(groups[0]?.rows).toHaveLength(2);
	});

	it("falls back to the source when an entry has no description", () => {
		const text = renderList(availablePluginGroups(availableFixture), { indent: 2 });
		expect(text).toContain("https://example.test/acme/tree/main/terraform-guard");
	});
});

describe("renderCompactRows", () => {
	it("truncates to the terminal instead of wrapping to a second line", () => {
		const text = renderCompactRows(
			[
				{ name: "code-reviewer", detail: "a".repeat(300) },
				{ name: "Explore", detail: "short" },
			],
			{ columns: 60 },
		);
		const lines = text.split("\n");
		expect(lines).toHaveLength(2);
		for (const line of lines) {
			// Byte length, not visible width: truncation must not smuggle escape
			// codes in to make the width work out.
			expect(line).not.toContain("\x1b[");
			expect(line.length).toBeLessThanOrEqual(60);
		}
		expect(lines[0]).toContain("…");
	});

	it("aligns detail into one column", () => {
		const text = renderCompactRows([
			{ name: "code-reviewer", detail: "alpha" },
			{ name: "Explore", detail: "beta" },
		]);
		const [first, second] = text.split("\n");
		expect(first.indexOf("alpha")).toBe(second.indexOf("beta"));
	});

	it("emits nothing for an empty list rather than a stray blank line", () => {
		expect(renderCompactRows([])).toBe("");
	});
});

describe("installed plugin rows", () => {
	const base = { id: "p", root: "/p", manifestPath: "/p/m.json", format: "claude", supportPlatform: ["claude"] };

	it("reports themes and providers, which the old inline list omitted", () => {
		expect(pluginCapabilities({ ...base, themesDir: "/p/themes" } as never)).toEqual(["themes"]);
		expect(pluginCapabilities({ ...base, providers: [{}] } as never)).toEqual(["providers"]);
	});

	it("says so explicitly when a plugin ships no capabilities", () => {
		const rows = installedPluginRows([base] as never);
		expect(rows[0]?.facts).toContain("no capabilities");
	});

	it("emits the manifest format the tool description promises", () => {
		const rows = installedPluginRows([{ ...base, skillsDir: "/p/skills" }] as never);
		expect(rows[0]?.facts).toContain("(claude)");
	});

	it("carries the description through as row detail", () => {
		const rows = installedPluginRows([{ ...base, description: "Does a thing." }] as never);
		expect(rows[0]?.detail).toBe("Does a thing.");
	});

	it("appends the version to the name so the column stays a single identifier", () => {
		const rows = installedPluginRows([{ ...base, version: "1.2.3" }] as never);
		expect(rows[0]?.name).toBe("p@1.2.3");
	});
});
