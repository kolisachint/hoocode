import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { type BrowserFlowDetails, createBrowserFlowTool } from "../src/core/tools/browser-flow.js";
import { disposeAllSessions } from "../src/core/tools/browsertools-shared.js";

// Opt-in end-to-end test against the REAL browsertools binary driving a real
// headless Chromium. Skipped unless both are provided:
//   BROWSERTOOLS_E2E_BIN  -> path to the browsertools binary
//   CHROME_PATH           -> path to a Chromium/Chrome executable
// Exercises the actual serve client + session registry + flow_start path end to
// end (no fakes), using a self-contained file:// page so no server is needed.
const BIN = process.env.BROWSERTOOLS_E2E_BIN;
const CHROME = process.env.CHROME_PATH;
const runE2E = Boolean(BIN && CHROME);

describe.skipIf(!runE2E)("browser tools (e2e, real binary)", () => {
	let dir: string;
	let flowPath: string;

	beforeAll(() => {
		dir = mkdtempSync(join(tmpdir(), "bt-e2e-"));
		mkdirSync(dir, { recursive: true });
		const pagePath = join(dir, "page.html");
		writeFileSync(pagePath, `<!doctype html><html><body><h1>Hello E2E</h1><p class="p">ok</p></body></html>`);
		const flow = {
			id: "e2e_smoke",
			name: "e2e smoke: open a local page and assert h1",
			version: 1,
			start_url: `file://${pagePath}`,
			vars: [],
			steps: [
				{ id: "s01", action: { action: "navigate", url: `file://${pagePath}` }, on_fail: "halt" },
				{ id: "s02", action: { action: "wait_settle" }, on_fail: "halt" },
				{
					id: "s03",
					action: { action: "checkpoint", asserts: [{ kind: "element_present", selector: "h1" }] },
					on_fail: "halt",
				},
			],
			outputs: [
				{ key: "title", source: { from: "text", selector: "h1" } },
				{ key: "url", source: { from: "url" } },
			],
		};
		flowPath = join(dir, "smoke.flow.json");
		writeFileSync(flowPath, JSON.stringify(flow, null, 2));
	});

	afterAll(() => {
		disposeAllSessions();
		rmSync(dir, { recursive: true, force: true });
	});

	it("runs a real deterministic flow to completion and writes evidence", async () => {
		const tool = createBrowserFlowTool(dir, {
			binaryPath: BIN,
			browserPath: CHROME,
			requestTimeoutMs: 60_000,
		});
		const result = await tool.execute("e2e-1", { flow_path: flowPath, store: join(dir, "store") });
		const details = result.details as BrowserFlowDetails;
		console.log("[e2e] flow result:", JSON.stringify(details.result));
		expect(details.status).toBe("complete");
		expect(result.content.some((c) => c.type === "text" && /Flow complete/.test(c.text ?? ""))).toBe(true);
	}, 90_000);
});
