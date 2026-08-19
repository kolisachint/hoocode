/**
 * The trust gate: a canvas that arrived in a clone does not run unasked.
 *
 * Design: `docs/canvas-extensions-design.md` §5. These tests drive the real trust
 * store (`trustWorkspace` / `untrustWorkspace` against a temp agent dir) rather
 * than a stub, because the property under test is precisely that the record lives
 * outside the repository and cannot be forged by repository content.
 *
 * The registry case is the one that matters most: it proves the gate is enforced
 * at the choke point where a process would actually start, not only in the helper
 * a caller is expected to remember to call.
 */

import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE, type DiscoveredCanvasExtension } from "../src/core/canvas/discovery.js";
import { CanvasRegistry } from "../src/core/canvas/registry.js";
import {
	CanvasTrustError,
	gateCanvasExtensions,
	isProjectSuppliedCanvas,
	shouldWithholdCanvas,
} from "../src/core/canvas/trust.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures", "canvas", "plan-board");

describe("canvas trust gate", () => {
	let cwd: string;
	let agentDir: string;

	beforeEach(() => {
		cwd = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-trust-repo-"));
		// The trust store sits beside the agent dir, so give each test its own home.
		agentDir = path.join(mkdtempSync(path.join(tmpdir(), "hoocode-canvas-trust-home-")), ".hoocode");
		mkdirSync(agentDir, { recursive: true });
	});

	afterEach(() => {
		untrustWorkspace(cwd, agentDir);
		rmSync(cwd, { recursive: true, force: true });
		rmSync(path.dirname(agentDir), { recursive: true, force: true });
	});

	/** Place an extension under `<cwd>/<marker>/extensions/<id>` or an absolute dir. */
	function place(id: string, marker: string | undefined, options: { real?: boolean } = {}): DiscoveredCanvasExtension {
		const base =
			marker === undefined
				? mkdtempSync(path.join(tmpdir(), "hoocode-canvas-user-"))
				: path.join(cwd, marker, "extensions");
		const dir = path.join(base, id);
		mkdirSync(dir, { recursive: true });
		if (options.real) cpSync(FIXTURE_DIR, dir, { recursive: true });
		else writeFileSync(path.join(dir, CANVAS_ENTRY_FILE), "export {};\n");
		return { id, dir, entry: path.join(dir, CANVAS_ENTRY_FILE), scope: marker === ".github" ? "project" : "agents" };
	}

	describe("isProjectSuppliedCanvas", () => {
		it("counts .github/extensions — the home that travels in every clone", () => {
			expect(isProjectSuppliedCanvas(place("board", ".github"), cwd)).toBe(true);
		});

		it("counts .agents/extensions", () => {
			expect(isProjectSuppliedCanvas(place("board", ".agents"), cwd)).toBe(true);
		});

		it("does not count a user-scope extension outside any repository", () => {
			expect(isProjectSuppliedCanvas(place("board", undefined), cwd)).toBe(false);
		});

		it("is segment-aware, so a sibling directory is not mistaken for the scope", () => {
			const dir = path.join(cwd, ".githubextra", "extensions", "board");
			mkdirSync(dir, { recursive: true });
			const extension: DiscoveredCanvasExtension = {
				id: "board",
				dir,
				entry: path.join(dir, CANVAS_ENTRY_FILE),
				scope: "project",
			};
			expect(isProjectSuppliedCanvas(extension, cwd)).toBe(false);
		});
	});

	describe("shouldWithholdCanvas", () => {
		it("withholds a repository-supplied canvas in an untrusted workspace", () => {
			expect(shouldWithholdCanvas(place("board", ".github"), cwd, agentDir)).toBe(true);
		});

		it("releases it once the workspace is trusted", () => {
			const extension = place("board", ".github");
			trustWorkspace(cwd, agentDir);
			expect(shouldWithholdCanvas(extension, cwd, agentDir)).toBe(false);
		});

		it("withholds it again when trust is revoked", () => {
			const extension = place("board", ".github");
			trustWorkspace(cwd, agentDir);
			untrustWorkspace(cwd, agentDir);
			expect(shouldWithholdCanvas(extension, cwd, agentDir)).toBe(true);
		});

		it("never withholds a user-scope canvas, trusted or not", () => {
			expect(shouldWithholdCanvas(place("board", undefined), cwd, agentDir)).toBe(false);
		});

		it("cannot be forged from inside the repository", () => {
			const extension = place("board", ".github");
			// Anything a repository could commit — including a file claiming trust —
			// lives in the working tree, and the record does not.
			mkdirSync(path.join(cwd, ".agents"), { recursive: true });
			writeFileSync(
				path.join(cwd, ".agents", "trusted-workspaces.json"),
				JSON.stringify({ workspaces: [{ path: cwd, at: new Date().toISOString() }] }),
			);
			expect(shouldWithholdCanvas(extension, cwd, agentDir)).toBe(true);
		});
	});

	describe("gateCanvasExtensions", () => {
		it("lists a withheld canvas with its reason instead of dropping it", () => {
			const project = place("from-clone", ".github");
			const user = place("mine", undefined);
			const gated = gateCanvasExtensions([project, user], cwd, agentDir);
			expect(gated.runnable.map((extension) => extension.id)).toEqual(["mine"]);
			expect(gated.withheld).toEqual([{ extension: project, reason: "untrusted-workspace" }]);
		});

		it("moves everything to runnable once the workspace is trusted", () => {
			const extensions = [place("a", ".github"), place("b", ".agents")];
			trustWorkspace(cwd, agentDir);
			const gated = gateCanvasExtensions(extensions, cwd, agentDir);
			expect(gated.runnable).toHaveLength(2);
			expect(gated.withheld).toEqual([]);
		});
	});

	describe("registry enforcement", () => {
		let registry: CanvasRegistry | undefined;

		afterEach(async () => {
			await registry?.shutdown();
			registry = undefined;
		});

		function build(): CanvasRegistry {
			registry = new CanvasRegistry({ runtime: canvasTestRuntime(), cwd, agentDir });
			return registry;
		}

		it("refuses to fork a repository-supplied canvas in an untrusted workspace", async () => {
			const extension = place("plan-board", ".github", { real: true });
			await expect(build().open(extension, "plan-board")).rejects.toBeInstanceOf(CanvasTrustError);
		});

		it("refuses even to read its declarations, since that also requires forking", async () => {
			const extension = place("plan-board", ".github", { real: true });
			await expect(build().declarations(extension)).rejects.toBeInstanceOf(CanvasTrustError);
		});

		it("explains what running it would do", async () => {
			const extension = place("plan-board", ".github", { real: true });
			await expect(build().open(extension, "plan-board")).rejects.toThrow(
				/came with this repository.*not a trusted workspace/s,
			);
		});

		it("forks it once the workspace is trusted", async () => {
			const extension = place("plan-board", ".github", { real: true });
			trustWorkspace(cwd, agentDir);
			const instance = await build().open(extension, "plan-board");
			expect(instance.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/\?token=/);
			expect((await fetch(instance.url ?? "")).status).toBe(200);
		});

		it("leaves nothing running after a refusal", async () => {
			const extension = place("plan-board", ".github", { real: true });
			const reg = build();
			await expect(reg.open(extension, "plan-board")).rejects.toBeInstanceOf(CanvasTrustError);
			expect(reg.listInstances()).toEqual([]);
			expect(reg.activeActions()).toEqual([]);
		});
	});
});
