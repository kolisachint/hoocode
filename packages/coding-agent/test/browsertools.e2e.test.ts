import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { createBrowserContinueTool } from "../src/core/tools/browser/browser-continue.js";
import { type BrowserRunDetails, createBrowserRunTool } from "../src/core/tools/browser/browser-run.js";
import { disposeAllSessions } from "../src/core/tools/browser/browsertools-shared.js";

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
	let tier2FlowPath: string;
	let pageUrl: string;

	// browsertools (chromiumoxide) launches Chromium against a fixed profile dir
	// guarded by a SingletonLock, so only one instance runs at a time: a stale lock
	// (crashed run) blocks new launches, and a launch started before the previous
	// browser fully exits collides. The browser removes its own lock on clean exit,
	// so we treat the lock's absence as "released" and serialize launches on it.
	const lockPath = join(tmpdir(), "chromiumoxide-runner", "SingletonLock");
	const waitForLockReleased = async (timeoutMs = 10_000) => {
		const deadline = Date.now() + timeoutMs;
		while (existsSync(lockPath) && Date.now() < deadline) {
			await new Promise((r) => setTimeout(r, 100));
		}
		// If a stale lock outlived the browser, best-effort clear it.
		rmSync(lockPath, { force: true });
	};

	beforeAll(() => {
		dir = mkdtempSync(join(tmpdir(), "bt-e2e-"));
		mkdirSync(dir, { recursive: true });
		const pagePath = join(dir, "page.html");
		writeFileSync(pagePath, `<!doctype html><html><body><h1>Hello E2E</h1><p class="p">ok</p></body></html>`);
		pageUrl = `file://${pagePath}`;

		// Tier 1: deterministic flow that completes with no parent decision.
		const flow = {
			id: "e2e_smoke",
			name: "e2e smoke: open a local page and assert h1",
			version: 1,
			start_url: pageUrl,
			vars: [],
			steps: [
				{ id: "s01", action: { action: "navigate", url: pageUrl }, on_fail: "halt" },
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

		// Tier 2: a `classify` step suspends with a NeedsParent(classify_state)
		// request that the parent answers with a ParentResponse::State.
		const tier2 = {
			id: "e2e_tier2",
			name: "e2e tier2: classify the current page state",
			version: 1,
			start_url: pageUrl,
			vars: [],
			steps: [
				{ id: "s01", action: { action: "navigate", url: pageUrl }, on_fail: "halt" },
				{ id: "s02", action: { action: "wait_settle" }, on_fail: "halt" },
				{ id: "s03", action: { action: "classify" }, on_fail: "halt" },
			],
			outputs: [{ key: "url", source: { from: "url" } }],
		};
		tier2FlowPath = join(dir, "tier2.flow.json");
		writeFileSync(tier2FlowPath, JSON.stringify(tier2, null, 2));
	});

	beforeEach(() => {
		// afterEach already waited for the prior browser to release the lock, so any
		// lock present now is an orphan from a crashed run — clear it immediately.
		rmSync(lockPath, { force: true });
	});

	afterEach(async () => {
		// Tear down any client this test left alive, then wait for its browser to
		// release the profile lock before the next test launches.
		disposeAllSessions();
		await waitForLockReleased();
	});

	afterAll(() => {
		disposeAllSessions();
		rmSync(dir, { recursive: true, force: true });
	});

	it("runs a real deterministic flow to completion and writes evidence", async () => {
		const tool = createBrowserRunTool(dir, {
			binaryPath: BIN,
			browserPath: CHROME,
			requestTimeoutMs: 60_000,
		});
		const result = await tool.execute("e2e-1", { flow_path: flowPath, store: join(dir, "store") });
		const details = result.details as BrowserRunDetails;
		console.log("[e2e] flow result:", JSON.stringify(details.result));
		expect(details.status).toBe("complete");
		expect(result.content.some((c) => c.type === "text" && /Flow complete/.test(c.text ?? ""))).toBe(true);
	}, 90_000);

	it("suspends on a real Tier-2 classify step and resumes to completion", async () => {
		const flowTool = createBrowserRunTool(dir, { binaryPath: BIN, browserPath: CHROME, requestTimeoutMs: 60_000 });
		const resumeTool = createBrowserContinueTool(dir, {
			binaryPath: BIN,
			browserPath: CHROME,
			requestTimeoutMs: 60_000,
		});

		// flow_start suspends with a NeedsParent(classify_state) request.
		const started = await flowTool.execute("e2e-t2", { flow_path: tier2FlowPath, store: join(dir, "store2") });
		const startDetails = started.details as BrowserRunDetails;
		expect(startDetails.status).toBe("needs_parent");
		expect(startDetails.requestKind).toBe("classify_state");
		expect(typeof startDetails.token).toBe("string");
		// The suspension screenshot was fetched from the real binary via get_resource
		// and surfaced as a real image block (not a string).
		const image = started.content.find((c) => c.type === "image") as { mimeType?: string } | undefined;
		expect(image?.mimeType).toBe("image/png");

		// Answer with a ParentResponse::State; the flow resumes and completes.
		const resumed = await resumeTool.execute("e2e-t2r", {
			token: startDetails.token!,
			response: { response: "state", state: "home_page" },
		});
		const resumeDetails = resumed.details as BrowserRunDetails;
		console.log("[e2e] tier2 resume result:", JSON.stringify(resumeDetails.result));
		expect(resumeDetails.status).toBe("complete");
	}, 90_000);
});
