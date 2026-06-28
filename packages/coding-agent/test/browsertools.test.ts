import { chmodSync, mkdirSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { type BrowserFlowDetails, createBrowserFlowTool } from "../src/core/tools/browser-flow.js";
import { createBrowserResumeTool } from "../src/core/tools/browser-resume.js";
import { disposeAllSessions, pausedSessionCount } from "../src/core/tools/browsertools-shared.js";

// A fake `browsertools` binary implementing the `serve` JSON-RPC protocol from
// the real engine (src/serve.rs): newline-delimited {id, method, params} on
// stdin, {id, result} / {id, error} on stdout. A single process == one flow
// session and holds the round counter in memory, exactly like the real serve
// process holds browser + paused-flow state. The `vars.scenario` chooses the
// suspend/resume shape so the tools' start -> NeedsParent -> resume loop, the
// session registry, and screenshot fetching are all exercised without Chromium.
const FAKE_SERVE = `#!/usr/bin/env node
let scenario = "complete";
let round = 0;
let buf = "";
const respond = (id, result) => process.stdout.write(JSON.stringify({ id, result }) + "\\n");
const respondErr = (id, message) => process.stdout.write(JSON.stringify({ id, error: { message } }) + "\\n");
process.stdin.setEncoding("utf8");
process.stdin.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\\n")) !== -1) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (line) handle(JSON.parse(line));
  }
});
function handle(req) {
  const { id, method, params } = req;
  if (method === "flow_start") {
    scenario = (params && params.vars && params.vars.scenario) || "complete";
    round = 0;
    return step(id);
  }
  if (method === "flow_resume") {
    round++;
    return step(id, params.response);
  }
  if (method === "get_resource") {
    return respond(id, { ref: params.ref, mime: "image/png", len: 8, png_base64: "iVBORw0KGgo=" });
  }
  if (method === "live_view_start") {
    // Record that the viewer was started and whether headful was requested via env,
    // so the test can assert on the protocol without a real browser.
    return respond(id, { url: "http://127.0.0.1:65535/", transport: "websocket", headful: process.env.BROWSERTOOLS_HEADFUL || "" });
  }
  if (method === "shutdown") { respond(id, { ok: true }); process.exit(0); return; }
  respondErr(id, "unknown method " + method);
}
function step(id, response) {
  if (scenario === "complete") return respond(id, { outcome: "complete", result: { evidence: { ok: true } } });
  if (scenario === "fail") return respond(id, { outcome: "failed", step_id: "s1", kind: "selector_not_found", detail: "boom" });
  if (scenario === "one_round") {
    if (round === 0) return respond(id, { outcome: "needs_parent", token: "tok-1", request: { request: "classify_state", screenshot_ref: "shot-1", observation: {} } });
    return respond(id, { outcome: "complete", result: { evidence: { ok: true }, answered: response } });
  }
  if (scenario === "two_rounds") {
    if (round === 0) return respond(id, { outcome: "needs_parent", token: "tok-1", request: { request: "classify_state", screenshot_ref: "shot-1", observation: {} } });
    if (round === 1) return respond(id, { outcome: "needs_parent", token: "tok-2", request: { request: "verify_visual", screenshot_ref: "shot-2", expected_state: "logged_in" } });
    return respond(id, { outcome: "complete", result: { evidence: { ok: true }, answered: response } });
  }
  respondErr(id, "unknown scenario " + scenario);
}
`;

function textOf(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

describe("browser tools", () => {
	let binDir: string;
	let binPath: string;
	const cwd = tmpdir();

	beforeAll(() => {
		binDir = join(tmpdir(), `browsertools-fake-${Date.now()}`);
		mkdirSync(binDir, { recursive: true });
		binPath = join(binDir, "browsertools");
		writeFileSync(binPath, FAKE_SERVE);
		chmodSync(binPath, 0o755);
	});

	afterAll(() => {
		rmSync(binDir, { recursive: true, force: true });
	});

	afterEach(() => {
		// Tear down any sessions a test left parked so the registry never leaks
		// across tests.
		disposeAllSessions();
		expect(pausedSessionCount()).toBe(0);
	});

	it("runs a flow to completion with no parent decision", async () => {
		const tool = createBrowserFlowTool(cwd, { binaryPath: binPath });
		const result = await tool.execute("c1", { flow_path: "x.flow.json", vars: { scenario: "complete" } });
		const details = result.details as BrowserFlowDetails;
		expect(details.status).toBe("complete");
		expect(textOf(result)).toContain("Flow complete");
		expect(pausedSessionCount()).toBe(0);
	});

	it("suspends with a NeedsParent request, returns the screenshot, and parks the session", async () => {
		const flow = createBrowserFlowTool(cwd, { binaryPath: binPath });
		const result = await flow.execute("c2", { flow_path: "x.flow.json", vars: { scenario: "one_round" } });
		const details = result.details as BrowserFlowDetails;

		expect(details.status).toBe("needs_parent");
		expect(details.token).toBe("tok-1");
		expect(details.requestKind).toBe("classify_state");
		expect(textOf(result)).toContain("NeedsParent");
		// The suspension screenshot is surfaced as an image block, not a string.
		const image = result.content.find((c) => c.type === "image") as { type: string; mimeType?: string } | undefined;
		expect(image).toBeDefined();
		expect(image?.mimeType).toBe("image/png");
		// The live serve session is parked under its resume token.
		expect(pausedSessionCount()).toBe(1);
	});

	it("resumes a parked flow with a ParentResponse and completes", async () => {
		const flow = createBrowserFlowTool(cwd, { binaryPath: binPath });
		const resume = createBrowserResumeTool(cwd, { binaryPath: binPath });

		const started = (await flow.execute("c3", { flow_path: "x.flow.json", vars: { scenario: "one_round" } }))
			.details as BrowserFlowDetails;
		expect(started.token).toBe("tok-1");

		const resumed = await resume.execute("c3r", {
			token: started.token!,
			response: { response: "state", state: "logged_in" },
		});
		const details = resumed.details as BrowserFlowDetails;
		expect(details.status).toBe("complete");
		// The parent response was forwarded over flow_resume and echoed back.
		expect(textOf(resumed)).toContain("logged_in");
		expect(pausedSessionCount()).toBe(0);
	});

	it("handles multiple NeedsParent rounds, re-keying the session on each new token", async () => {
		const flow = createBrowserFlowTool(cwd, { binaryPath: binPath });
		const resume = createBrowserResumeTool(cwd, { binaryPath: binPath });

		const r1 = (await flow.execute("c4", { flow_path: "x.flow.json", vars: { scenario: "two_rounds" } }))
			.details as BrowserFlowDetails;
		expect(r1.token).toBe("tok-1");
		expect(pausedSessionCount()).toBe(1);

		const r2 = (await resume.execute("c4r1", { token: r1.token!, response: { response: "state", state: "form" } }))
			.details as BrowserFlowDetails;
		expect(r2.status).toBe("needs_parent");
		expect(r2.token).toBe("tok-2");
		expect(r2.requestKind).toBe("verify_visual");
		// Still exactly one parked session — re-keyed from tok-1 to tok-2.
		expect(pausedSessionCount()).toBe(1);

		const r3 = await resume.execute("c4r2", { token: r2.token!, response: { response: "verified", passed: true } });
		expect((r3.details as BrowserFlowDetails).status).toBe("complete");
		expect(pausedSessionCount()).toBe(0);
	});

	it("throws a clear error when a flow fails", async () => {
		const tool = createBrowserFlowTool(cwd, { binaryPath: binPath });
		await expect(tool.execute("c5", { flow_path: "x.flow.json", vars: { scenario: "fail" } })).rejects.toThrow(
			/flow failed at step "s1".*boom.*selector_not_found/s,
		);
		expect(pausedSessionCount()).toBe(0);
	});

	it("rejects resume for an unknown/expired token", async () => {
		const resume = createBrowserResumeTool(cwd, { binaryPath: binPath });
		await expect(
			resume.execute("c6", { token: "nope", response: { response: "state", state: "x" } }),
		).rejects.toThrow(/No paused browser flow/);
	});

	it("requires either flow_path or flow", async () => {
		const tool = createBrowserFlowTool(cwd, { binaryPath: binPath });
		await expect(tool.execute("c7", {})).rejects.toThrow(/requires either `flow_path` or `flow`/);
	});

	it("starts the live viewer and surfaces its URL when live_view is set", async () => {
		// Suppress the actual browser-open so the test never spawns `open`/`xdg-open`.
		const prev = process.env.HOOCODE_BROWSERTOOLS_NO_OPEN;
		process.env.HOOCODE_BROWSERTOOLS_NO_OPEN = "1";
		try {
			const tool = createBrowserFlowTool(cwd, { binaryPath: binPath });
			const result = await tool.execute("clv", {
				flow_path: "x.flow.json",
				vars: { scenario: "complete" },
				live_view: true,
			});
			const text = textOf(result);
			expect(text).toContain("Live view available at: http://127.0.0.1:65535/");
			expect(text).toContain("Flow complete");
		} finally {
			if (prev === undefined) delete process.env.HOOCODE_BROWSERTOOLS_NO_OPEN;
			else process.env.HOOCODE_BROWSERTOOLS_NO_OPEN = prev;
		}
	});

	it("defaults the live viewer on via the liveView instance option", async () => {
		const prev = process.env.HOOCODE_BROWSERTOOLS_NO_OPEN;
		process.env.HOOCODE_BROWSERTOOLS_NO_OPEN = "1";
		try {
			const tool = createBrowserFlowTool(cwd, { binaryPath: binPath, liveView: true });
			const result = await tool.execute("clv2", { flow_path: "x.flow.json", vars: { scenario: "complete" } });
			expect(textOf(result)).toContain("Live view available at:");
		} finally {
			if (prev === undefined) delete process.env.HOOCODE_BROWSERTOOLS_NO_OPEN;
			else process.env.HOOCODE_BROWSERTOOLS_NO_OPEN = prev;
		}
	});

	it("does not start the live viewer by default", async () => {
		const tool = createBrowserFlowTool(cwd, { binaryPath: binPath });
		const result = await tool.execute("clv3", { flow_path: "x.flow.json", vars: { scenario: "complete" } });
		expect(textOf(result)).not.toContain("Live view");
	});

	it("enforces the NeedsParent round cap", async () => {
		const flow = createBrowserFlowTool(cwd, { binaryPath: binPath, maxParentRounds: 1 });
		const resume = createBrowserResumeTool(cwd, { binaryPath: binPath, maxParentRounds: 1 });

		const r1 = (await flow.execute("c8", { flow_path: "x.flow.json", vars: { scenario: "two_rounds" } }))
			.details as BrowserFlowDetails;
		expect(r1.token).toBe("tok-1");

		await expect(
			resume.execute("c8r", { token: r1.token!, response: { response: "state", state: "form" } }),
		).rejects.toThrow(/exceeded the maximum of 1 NeedsParent rounds/);
		expect(pausedSessionCount()).toBe(0);
	});

	it("reaps an idle parked session after the idle timeout", async () => {
		const flow = createBrowserFlowTool(cwd, { binaryPath: binPath, idleTimeoutMs: 50 });
		const started = (await flow.execute("c9", { flow_path: "x.flow.json", vars: { scenario: "one_round" } }))
			.details as BrowserFlowDetails;
		expect(started.token).toBe("tok-1");
		expect(pausedSessionCount()).toBe(1);

		await new Promise((r) => setTimeout(r, 200));
		expect(pausedSessionCount()).toBe(0);

		const resume = createBrowserResumeTool(cwd, { binaryPath: binPath });
		await expect(
			resume.execute("c9r", { token: started.token!, response: { response: "state", state: "x" } }),
		).rejects.toThrow(/No paused browser flow/);
	});
});
