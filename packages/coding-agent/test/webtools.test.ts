import { chmodSync, mkdirSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { createWebFetchTool } from "../src/core/tools/webfetch.js";
import { createWebSearchTool } from "../src/core/tools/websearch.js";

// A fake `webtools` binary placed on PATH. It echoes canned --json output for
// the fetch/search subcommands so the tools' spawn + parse + filter paths are
// exercised without real network access or the real Rust binary.
const FAKE_BIN = `#!/bin/sh
sub="$1"
if [ "$sub" = "fetch" ]; then
  cat <<'JSON'
{
  "title": "Example Domain",
  "final_url": "https://example.com/",
  "content": "Example Domain\\nSee more [1]\\n\\nReferences:\\n[1] https://iana.org/domains/example",
  "content_type": "text",
  "media": "html",
  "token_estimate": 42,
  "references": [{ "index": 1, "url": "https://iana.org/domains/example", "text": "See more" }],
  "metadata": { "lang": "en" },
  "source": "https://example.com/"
}
JSON
elif [ "$sub" = "search" ]; then
  cat <<'JSON'
{
  "query": "rust async",
  "results": [
    { "title": "Smol runtime", "snippet": "A small async runtime.", "url": "https://github.com/smol-rs/smol", "ref_index": 1 },
    { "title": "Blocked Site", "snippet": "Should be filtered.", "url": "https://blocked.example/page", "ref_index": 2 }
  ],
  "references": [
    { "index": 1, "url": "https://github.com/smol-rs/smol" },
    { "index": 2, "url": "https://blocked.example/page" }
  ],
  "token_estimate": 50,
  "result_count": 2
}
JSON
else
  echo "unknown subcommand" 1>&2
  exit 1
fi
`;

function getText(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

describe("web tools", () => {
	let binDir: string;
	let cwd: string;
	let originalPath: string | undefined;

	beforeAll(() => {
		binDir = join(tmpdir(), `webtools-fake-bin-${Date.now()}`);
		mkdirSync(binDir, { recursive: true });
		const binPath = join(binDir, "webtools");
		writeFileSync(binPath, FAKE_BIN);
		chmodSync(binPath, 0o755);
		originalPath = process.env.PATH;
		process.env.PATH = `${binDir}:${process.env.PATH ?? ""}`;
	});

	afterAll(() => {
		process.env.PATH = originalPath;
		rmSync(binDir, { recursive: true, force: true });
	});

	beforeEach(() => {
		cwd = join(tmpdir(), `webtools-cwd-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	describe("webfetch", () => {
		it("fetches a URL and returns title + final url + content", async () => {
			const tool = createWebFetchTool(cwd);
			const result = await tool.execute("call-1", { url: "https://example.com" });
			const text = getText(result);
			expect(text).toContain("Example Domain");
			expect(text).toContain("https://example.com/");
			expect(text).toContain("[1] https://iana.org/domains/example");
			expect((result.details as { tokenEstimate?: number }).tokenEstimate).toBe(42);
		});

		it("blocks a host listed in .webtoolsignore", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "example.com\n");
			const tool = createWebFetchTool(cwd);
			await expect(tool.execute("call-2", { url: "https://example.com" })).rejects.toThrow(/\.webtoolsignore/);
		});

		it("allows a host re-included with a ! rule", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "*\n!example.com\n");
			const tool = createWebFetchTool(cwd);
			const result = await tool.execute("call-3", { url: "https://example.com" });
			expect(getText(result)).toContain("Example Domain");
		});
	});

	describe("websearch", () => {
		it("returns ranked results in reference style", async () => {
			const tool = createWebSearchTool(cwd);
			const result = await tool.execute("call-4", { query: "rust async" });
			const text = getText(result);
			expect(text).toContain("Smol runtime [1]");
			expect(text).toContain("References:");
			expect(text).toContain("[1] https://github.com/smol-rs/smol");
		});

		it("filters result links blocked by .webtoolsignore and notes the hidden count", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "blocked.example\n");
			const tool = createWebSearchTool(cwd);
			const result = await tool.execute("call-5", { query: "rust async" });
			const text = getText(result);
			expect(text).toContain("Smol runtime [1]");
			expect(text).not.toContain("blocked.example");
			expect(text).toContain("1 result hidden");
			expect((result.details as { hiddenCount?: number }).hiddenCount).toBe(1);
		});
	});
});
