import { describe, expect, test } from "vitest";
import { LexicalIndex, tokenize } from "../src/core/capabilities/lexical.js";
import type { CapabilityDoc } from "../src/core/capabilities/registry.js";
import { sectionLabel, splitIntoSections } from "../src/core/self-docs.js";

function doc(id: string, name: string, description: string): CapabilityDoc {
	return { id, kind: "doc", name, description, deferred: true };
}

describe("splitIntoSections", () => {
	test("splits at headings and records the heading trail and line", () => {
		const sections = splitIntoSections(
			"# Extensions\n\nIntro prose.\n\n## Custom Tools\n\nRegister tools here.\n",
			"extensions.md",
			"/pkg/docs/extensions.md",
		);

		expect(sections.map((s) => s.headings)).toEqual([["Extensions"], ["Extensions", "Custom Tools"]]);
		expect(sections[1]?.line).toBe(5);
		expect(sections[1]?.excerpt).toBe("Register tools here.");
		expect(sectionLabel(sections[1]!)).toBe("extensions.md § Extensions › Custom Tools");
	});

	test("pops the trail back up when a heading gets shallower", () => {
		const sections = splitIntoSections("# A\n\n## B\n\n### C\n\n## D\n", "x.md", "/x.md");

		expect(sections.map((s) => s.headings)).toEqual([["A"], ["A", "B"], ["A", "B", "C"], ["A", "D"]]);
	});

	test("does not split on a # comment inside a fenced code block", () => {
		const sections = splitIntoSections("# Title\n\n```bash\n# not a heading\nls\n```\n\n## Real\n", "x.md", "/x.md");

		expect(sections.map((s) => s.headings)).toEqual([["Title"], ["Title", "Real"]]);
	});

	test("keeps code content in the excerpt, since identifiers are what people search for", () => {
		const sections = splitIntoSections("# API\n\n```ts\npi.registerTool(definition);\n```\n", "x.md", "/x.md");

		expect(sections[0]?.excerpt).toContain("pi.registerTool");
	});

	test("disambiguates repeated headings so ids stay unique", () => {
		const sections = splitIntoSections("# T\n\n## Example\n\na\n\n## Example\n\nb\n", "x.md", "/x.md");

		const ids = sections.map((s) => s.id);
		expect(new Set(ids).size).toBe(ids.length);
	});

	test("returns nothing for a file with no headings", () => {
		expect(splitIntoSections("just prose\n", "x.md", "/x.md")).toEqual([]);
	});
});

describe("tokenize plural folding", () => {
	test("emits the singular alongside the original so both queries match", () => {
		expect(tokenize("themes")).toContain("themes");
		expect(tokenize("themes")).toContain("theme");
		expect(tokenize("capabilities")).toContain("capability");
		expect(tokenize("branches")).toContain("branch");
	});

	test("leaves words that only look plural alone", () => {
		expect(tokenize("status")).not.toContain("statu");
		expect(tokenize("css")).not.toContain("cs");
		expect(tokenize("bus")).not.toContain("bu");
	});

	test("still matches an exact tool name exactly", () => {
		expect(tokenize("mcp_github_create_pull_request")).toContain("mcp_github_create_pull_request");
	});

	test("a singular query finds a plural heading", () => {
		const index = new LexicalIndex([
			doc("doc:themes.md#custom", "themes.md § Themes › Creating a Custom Theme", "Define colors."),
			doc("doc:other.md#x", "other.md § Something Else", "Unrelated prose."),
		]);

		expect(index.search("theme", 1)[0]?.id).toBe("doc:themes.md#custom");
	});
});

describe("query stopwords", () => {
	test("filler words do not outweigh the one meaningful term", () => {
		const index = new LexicalIndex([
			doc("doc:themes", "themes.md § Creating a Custom Theme", "Define the theme colors."),
			// Matches "add", "how", and "to" but has nothing to do with themes.
			doc("doc:noise", "quickstart.md § Project instructions", "Add a file to tell it how to work in a project."),
		]);

		expect(index.search("how do I add a custom theme", 1)[0]?.id).toBe("doc:themes");
	});

	test("a query made only of stopwords still searches instead of matching nothing", () => {
		const index = new LexicalIndex([doc("doc:a", "a.md § What is it", "prose")]);

		expect(index.search("what is it", 5).length).toBeGreaterThan(0);
	});
});
