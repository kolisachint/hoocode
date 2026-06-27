import { mkdirSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createDocGrepTool } from "../src/core/tools/docgrep.js";
import { createDocPeekTool } from "../src/core/tools/docpeek.js";
import { createDocReadTool } from "../src/core/tools/docread.js";
import { createDocScanTool } from "../src/core/tools/docscan.js";
import { estimateTextTokens } from "../src/core/tools/filetools-shared.js";

// ---------------------------------------------------------------------------
// Token-cost benchmark: full DocRead vs the scan → grep → peek loop.
//
// Measures the rendered tool output the agent actually pays for (estimated via
// the same chars/4 heuristic the agent uses), on a document large enough that a
// full DocRead truncates at its token budget. The loop should be both much
// cheaper AND able to reach a block past the truncation ceiling that DocRead
// cannot. Numbers are logged so the run doubles as a measurement.
// ---------------------------------------------------------------------------

function getText(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

// A deep-in-the-document marker placed past where a full DocRead truncates.
const NEEDLE = "ZZNEEDLEZZ";

describe("document tools token cost", () => {
	let cwd: string;
	let docPath: string;

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-tok-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
		// ~80 sections of sizeable text — comfortably past DocRead's render budget.
		// The needle lives in a late section so it falls beyond the truncation point.
		const filler = "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(12);
		let xml = "<doc>";
		for (let i = 0; i < 80; i++) {
			const tail = i === 70 ? ` ${NEEDLE}` : "";
			xml += `<section><title>Section ${i}</title><body>${filler}${tail}</body></section>`;
		}
		xml += "</doc>";
		docPath = join(cwd, "big.xml");
		writeFileSync(docPath, xml);
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	it("the scan→grep→peek loop costs far fewer tokens than a full DocRead", async () => {
		// Full read: the whole id-addressed tree, capped by the render budget.
		const fullText = getText(await createDocReadTool(cwd).execute("read", { path: docPath }));
		const fullTokens = estimateTextTokens(fullText);

		// Discovery loop.
		const scanText = getText(await createDocScanTool(cwd).execute("scan", { path: docPath, limit: 100 }));
		const scanTokens = estimateTextTokens(scanText);

		const grepText = getText(await createDocGrepTool(cwd).execute("grep", { path: docPath, pattern: NEEDLE }));
		const grepTokens = estimateTextTokens(grepText);

		// Peek ONE specific leaf block (a section body), as you would after locating
		// the part you want — not the root (peeking the root hydrates everything).
		const leafId = scanText.match(/#(node\[body:\d+\]) \[/)?.[1];
		expect(leafId).toBeDefined();
		const peekText = getText(await createDocPeekTool(cwd).execute("peek", { path: docPath, id: [leafId as string] }));
		const peekTokens = estimateTextTokens(peekText);

		const loopTokens = scanTokens + grepTokens + peekTokens;

		// eslint-disable-next-line no-console
		console.log(
			`\n[token cost] full DocRead=${fullTokens}  |  loop=${loopTokens} ` +
				`(scan=${scanTokens} + grep=${grepTokens} + peek=${peekTokens})  ` +
				`→ ${(fullTokens / loopTokens).toFixed(1)}x cheaper`,
		);

		// The full read is large enough to have truncated.
		expect(fullText).toContain("[Truncated:");
		// The loop is materially cheaper than a full read.
		expect(loopTokens).toBeLessThan(fullTokens / 2);

		// Truncation ceiling: the full read dropped the late section, but grep still
		// reaches the needle — the loop sees content a full DocRead cannot.
		expect(fullText).not.toContain(NEEDLE);
		expect(grepText).toContain(NEEDLE);
		expect((await createDocGrepTool(cwd).execute("g2", { path: docPath, pattern: NEEDLE })).details).toMatchObject({
			matches: 1,
		});
	});
});
