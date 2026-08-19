/**
 * The session facade: what a user surface asks, with no TUI involved.
 *
 * Design: `docs/canvas-extensions-design.md` §11. The value of this layer is that
 * every decision a `/canvas` command makes is testable here — availability, trust
 * refusal, ref parsing, which canvas to open, what listing shows — leaving the
 * interactive-mode shell thin enough to be obvious by inspection.
 */

import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import type { CanvasAvailability } from "../src/core/canvas/launch.js";
import { CanvasSession, parseCanvasRef } from "../src/core/canvas/session.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board");

describe("parseCanvasRef", () => {
	it("reads a bare extension id", () => {
		expect(parseCanvasRef(" plan-board ")).toEqual({ extensionId: "plan-board" });
	});

	it("reads extension:canvas", () => {
		expect(parseCanvasRef("pr-artifact-explorer:pr-artifact-explorer")).toEqual({
			extensionId: "pr-artifact-explorer",
			canvasId: "pr-artifact-explorer",
		});
	});

	it("rejects empty input and half-written refs", () => {
		expect(parseCanvasRef("")).toBeUndefined();
		expect(parseCanvasRef("  ")).toBeUndefined();
		expect(parseCanvasRef("board:")).toBeUndefined();
		expect(parseCanvasRef(":board")).toBeUndefined();
	});
});

describe("CanvasSession", () => {
	let cwd: string;
	let home: string;
	let agentDir: string;
	let session: CanvasSession | undefined;

	const available = async (): Promise<CanvasAvailability> => ({ available: true, runtime: canvasTestRuntime() });
	const unavailable = async (): Promise<CanvasAvailability> => ({ available: false, reason: "no Node here" });

	beforeEach(() => {
		cwd = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-session-repo-"));
		home = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-session-home-"));
		agentDir = path.join(home, ".hoocode");
		mkdirSync(agentDir, { recursive: true });
	});

	afterEach(async () => {
		await session?.dispose();
		session = undefined;
		untrustWorkspace(cwd, agentDir);
		rmSync(cwd, { recursive: true, force: true });
		rmSync(home, { recursive: true, force: true });
	});

	/** Install the real fixture extension under a scope inside `cwd`, or in user scope. */
	function install(id: string, options: { scope: "project" | "user"; real?: boolean } = { scope: "user" }): void {
		const base =
			options.scope === "project"
				? path.join(cwd, ".github", "extensions")
				: path.join(home, ".copilot", "extensions");
		const dir = path.join(base, id);
		mkdirSync(dir, { recursive: true });
		if (options.real === false) writeFileSync(path.join(dir, CANVAS_ENTRY_FILE), "export {};\n");
		else copyFileSync(path.join(FIXTURE_DIR, CANVAS_ENTRY_FILE), path.join(dir, CANVAS_ENTRY_FILE));
	}

	function build(resolveRuntime = available): CanvasSession {
		session = new CanvasSession({ cwd, homeDir: home, agentDir, resolveRuntime });
		return session;
	}

	describe("list", () => {
		it("reports nothing installed without forking anything", async () => {
			const overview = await build().list();
			expect(overview.availability.available).toBe(true);
			expect(overview.listings).toEqual([]);
			expect(overview.withheldCount).toBe(0);
		});

		it("lists a user-scope extension with no canvasId, since naming it needs a fork", async () => {
			install("plan-board");
			const overview = await build().list();
			expect(overview.listings).toEqual([
				{
					extensionId: "plan-board",
					canvasId: undefined,
					displayName: undefined,
					scope: "user",
					withheld: undefined,
					open: [],
				},
			]);
		});

		it("surfaces a repository-supplied extension as withheld rather than hiding it", async () => {
			install("from-clone", { scope: "project" });
			const overview = await build().list();
			expect(overview.withheldCount).toBe(1);
			expect(overview.listings[0]).toMatchObject({ extensionId: "from-clone", withheld: "untrusted-workspace" });
		});

		it("reports it runnable once the workspace is trusted", async () => {
			install("from-clone", { scope: "project" });
			trustWorkspace(cwd, agentDir);
			const overview = await build().list();
			expect(overview.withheldCount).toBe(0);
			expect(overview.listings[0]).toMatchObject({ extensionId: "from-clone", withheld: undefined });
		});

		it("reports unavailability without pretending nothing is installed", async () => {
			install("plan-board");
			const overview = await build(unavailable).list();
			expect(overview.availability).toEqual({ available: false, reason: "no Node here" });
			expect(overview.listings).toHaveLength(1);
		});

		it("names the canvas and shows the instance once one is open", async () => {
			install("plan-board");
			const canvas = build();
			const instance = await canvas.open({ extensionId: "plan-board" });
			const overview = await canvas.list();
			expect(overview.listings[0]).toMatchObject({
				extensionId: "plan-board",
				canvasId: "plan-board",
				displayName: "Plan Board",
			});
			expect(overview.listings[0]?.open.map((open) => open.instanceId)).toEqual([instance.instanceId]);
		});
	});

	describe("open", () => {
		it("resolves the sole canvas without being told its id", async () => {
			install("plan-board");
			const instance = await build().open({ extensionId: "plan-board" });
			expect(instance.canvasId).toBe("plan-board");
			expect((await fetch(instance.url ?? "")).status).toBe(200);
		});

		it("accepts an explicit extension:canvas ref", async () => {
			install("plan-board");
			const ref = parseCanvasRef("plan-board:plan-board");
			expect(ref).toBeDefined();
			const instance = await build().open(ref as never);
			expect(instance.canvasId).toBe("plan-board");
		});

		it("refuses when canvases cannot run here, quoting the reason", async () => {
			install("plan-board");
			await expect(build(unavailable).open({ extensionId: "plan-board" })).rejects.toThrow(/no Node here/);
		});

		it("points at /plugin trust when the extension came with the repository", async () => {
			install("from-clone", { scope: "project" });
			await expect(build().open({ extensionId: "from-clone" })).rejects.toThrow(/\/plugin trust/);
		});

		it("lists what exists when the name is wrong", async () => {
			install("plan-board");
			await expect(build().open({ extensionId: "typo" })).rejects.toThrow(/found: plan-board/);
		});

		it("opens a repository-supplied extension once trusted", async () => {
			install("from-clone", { scope: "project" });
			trustWorkspace(cwd, agentDir);
			const instance = await build().open({ extensionId: "from-clone" });
			expect((await fetch(instance.url ?? "")).status).toBe(200);
		});

		it("passes a cancel signal through to the registry's abandon path", async () => {
			install("plan-board");
			const canvas = build();
			const controller = new AbortController();
			const opening = canvas.open({ extensionId: "plan-board" }, { signal: controller.signal });
			controller.abort();
			await expect(opening).rejects.toMatchObject({ code: "aborted" });
			expect(canvas.instances()).toEqual([]);
		});
	});

	describe("close", () => {
		it("closes an open instance and reports which one", async () => {
			install("plan-board");
			const canvas = build();
			const instance = await canvas.open({ extensionId: "plan-board" });
			expect((await canvas.close(instance.instanceId))?.instanceId).toBe(instance.instanceId);
			expect(canvas.instances()).toEqual([]);
		});

		it("is a no-op for an id that is not open", async () => {
			install("plan-board");
			expect(await build().close("nope")).toBeUndefined();
		});
	});

	describe("lifecycle", () => {
		it("has no registry until something is opened, so listing stays free", async () => {
			install("plan-board");
			const canvas = build();
			await canvas.list();
			expect(canvas.registryOrUndefined()).toBeUndefined();
			await canvas.open({ extensionId: "plan-board" });
			expect(canvas.registryOrUndefined()).toBeDefined();
		});

		it("resolves availability once even across many calls", async () => {
			let calls = 0;
			const canvas = build(async () => {
				calls += 1;
				return { available: true, runtime: canvasTestRuntime() };
			});
			await Promise.all([canvas.availability(), canvas.availability(), canvas.list()]);
			expect(calls).toBe(1);
		});

		it("disposes twice without complaint", async () => {
			install("plan-board");
			const canvas = build();
			await canvas.open({ extensionId: "plan-board" });
			await canvas.dispose();
			await expect(canvas.dispose()).resolves.toBeUndefined();
			expect(canvas.instances()).toEqual([]);
		});
	});
});
