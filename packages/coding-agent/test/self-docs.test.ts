import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { formatSelfDocsForPrompt, listSelfDocs, resetSelfDocs } from "../src/core/self-docs.js";
import { buildSystemPrompt } from "../src/core/system-prompt.js";

/**
 * `getPackageDir()` honours HOOCODE_PACKAGE_DIR, so a fixture tree can stand in
 * for the real install without touching the repo's own docs.
 */
let fixture: string;
let previousPackageDir: string | undefined;

function writeFixture(files: Record<string, string>): void {
	for (const [rel, content] of Object.entries(files)) {
		const path = join(fixture, rel);
		mkdirSync(join(path, ".."), { recursive: true });
		writeFileSync(path, content, "utf-8");
	}
	resetSelfDocs();
}

beforeEach(() => {
	fixture = mkdtempSync(join(tmpdir(), "hoocode-self-docs-"));
	previousPackageDir = process.env.HOOCODE_PACKAGE_DIR;
	process.env.HOOCODE_PACKAGE_DIR = fixture;
	resetSelfDocs();
});

afterEach(() => {
	if (previousPackageDir === undefined) delete process.env.HOOCODE_PACKAGE_DIR;
	else process.env.HOOCODE_PACKAGE_DIR = previousPackageDir;
	resetSelfDocs();
	rmSync(fixture, { recursive: true, force: true });
});

describe("listSelfDocs", () => {
	test("returns [] when the docs directory is absent", () => {
		expect(listSelfDocs()).toEqual([]);
	});

	test("takes title and description from the curated index", () => {
		writeFixture({
			"docs/index.md": "# Docs\n\n- [Skills](skills.md) - Agent Skills for reusable capabilities.\n",
			"docs/skills.md": "# Something Else\n\nA paragraph that should lose to the index.\n",
		});

		const skills = listSelfDocs().find((d) => d.id === "skills.md");
		expect(skills).toBeDefined();
		expect(skills?.title).toBe("Skills");
		expect(skills?.description).toBe("Agent Skills for reusable capabilities.");
	});

	test("falls back to the doc's own heading and first paragraph when unindexed", () => {
		writeFixture({
			"docs/index.md": "# Docs\n\nNo links here.\n",
			"docs/routing.md": "# Subagent Delegation\n\nHooCode delegates focused work to subagents.\n",
		});

		const routing = listSelfDocs().find((d) => d.id === "routing.md");
		expect(routing?.title).toBe("Subagent Delegation");
		expect(routing?.description).toBe("HooCode delegates focused work to subagents.");
	});

	test("skips headings, lists, quotes, and fenced code when deriving a description", () => {
		writeFixture({
			"docs/x.md": "# Title\n\n> A note\n\n- bullet\n\n```bash\nnot prose\n```\n\nThe real first sentence.\n",
		});

		expect(listSelfDocs()[0]?.description).toBe("The real first sentence.");
	});

	test("puts the overview first and the rest alphabetically", () => {
		writeFixture({
			"docs/zebra.md": "# Zebra\n",
			"docs/alpha.md": "# Alpha\n",
			"docs/index.md": "# Overview\n",
		});

		expect(listSelfDocs().map((d) => d.id)).toEqual(["index.md", "alpha.md", "zebra.md"]);
	});

	test("includes README and CHANGELOG from the package root", () => {
		writeFixture({
			"docs/index.md": "# Docs\n",
			"README.md": "# hoocode\n",
			"CHANGELOG.md": "# Changelog\n",
		});

		expect(listSelfDocs().map((d) => d.id)).toContain("README.md");
		expect(listSelfDocs().map((d) => d.id)).toContain("CHANGELOG.md");
	});

	test("ignores non-markdown files", () => {
		writeFixture({ "docs/index.md": "# Docs\n", "docs/docs.json": "{}\n" });

		expect(listSelfDocs().map((d) => d.id)).toEqual(["index.md"]);
	});

	test("truncates a run-on opening line", () => {
		writeFixture({ "docs/x.md": `# X\n\n${"word ".repeat(200)}\n` });

		const description = listSelfDocs()[0]?.description ?? "";
		expect(description.length).toBeLessThanOrEqual(110);
		expect(description.endsWith("…")).toBe(true);
	});
});

describe("formatSelfDocsForPrompt", () => {
	test("is empty when there are no docs", () => {
		expect(formatSelfDocsForPrompt()).toBe("");
	});

	test("prints each directory once instead of repeating absolute paths", () => {
		writeFixture({
			"docs/index.md": "# Docs\n\n- [Skills](skills.md) - Reusable capabilities.\n",
			"docs/skills.md": "# Skills\n",
			"README.md": "# hoocode\n",
		});

		const section = formatSelfDocsForPrompt();
		expect(section).toContain(`In ${join(fixture, "docs")}/`);
		expect(section).toContain(`In ${fixture}/`);
		// Filenames are listed bare under their directory heading.
		expect(section).toContain("- skills.md (Skills) — Reusable capabilities.");
		expect(section).not.toContain(join(fixture, "docs", "skills.md"));
	});
});

describe("buildSystemPrompt self-docs section", () => {
	const docs = {
		"docs/index.md": "# Docs\n\n- [Skills](skills.md) - Reusable capabilities.\n",
		"docs/skills.md": "# Skills\n",
	};

	test("is included when the read tool is available", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({ selectedTools: ["read"], cwd: process.cwd() });
		expect(prompt).toContain("# About hoocode itself");
		expect(prompt).toContain("skills.md");
	});

	test("is omitted without the read tool, since nothing could open the files", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({ selectedTools: ["bash"], cwd: process.cwd() });
		expect(prompt).not.toContain("# About hoocode itself");
	});

	test("can be turned off explicitly", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({ selectedTools: ["read"], includeSelfDocs: false, cwd: process.cwd() });
		expect(prompt).not.toContain("# About hoocode itself");
	});

	test("stays out of a custom prompt unless asked", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({ customPrompt: "Custom.", selectedTools: ["read"], cwd: process.cwd() });
		expect(prompt).not.toContain("# About hoocode itself");
	});

	test("can be opted into alongside a custom prompt", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({
			customPrompt: "Custom.",
			selectedTools: ["read"],
			includeSelfDocs: true,
			cwd: process.cwd(),
		});
		expect(prompt).toContain("# About hoocode itself");
	});

	test("stays ahead of the trailing date and cwd lines", () => {
		writeFixture(docs);

		const prompt = buildSystemPrompt({ selectedTools: ["read"], cwd: process.cwd() });
		expect(prompt.indexOf("# About hoocode itself")).toBeLessThan(prompt.indexOf("Current date:"));
	});
});
