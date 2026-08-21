/**
 * The rest of a canvas's life: naming it, renaming it, removing it, and seeing
 * what it can do.
 *
 * Design: `docs/canvas-extensions-design.md` §13.6. Creating and iterating were
 * covered first because they were missing outright; these are the operations a
 * person reaches for *after* the canvas exists, and each was reachable only by
 * hand — which for a canvas means getting four separate places to agree about
 * its name, one of which silently drops the open canvas when it is wrong.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE, type CanvasSearchRoot } from "../src/core/canvas/discovery.js";
import {
	canvasHomeRoot,
	isCanvasRefusal,
	removeCanvasExtension,
	renameCanvasExtension,
} from "../src/core/canvas/lifecycle.js";
import { CANVAS_ENTRY_TEMPLATE, canvasNameFromDescription } from "../src/core/canvas/scaffold.js";
import { CanvasSession } from "../src/core/canvas/session.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

/**
 * Naming is measured against real phrasings rather than asserted case by case.
 *
 * The rule it encodes is that a name should describe the *thing*, not the
 * request for it — and the way to know whether it does is to run a spread of the
 * ways people actually type this. Before the request words and participles were
 * dropped, seven of the first twelve named the request: `create-lightweight-games`,
 * `help-compare-two`, `want-review-pull`.
 */
describe("naming a canvas from a description", () => {
	const cases: [string, string][] = [
		["create lightweight games that can be played with keyboard arrow keys", "lightweight-games-keyboard"],
		["a kanban board for the release checklist", "kanban-board-release"],
		["build me a dashboard showing flaky tests over time", "dashboard-flaky-tests"],
		["make a triage board for incoming issues", "triage-board-issues"],
		["show the dependency graph of this repo", "dependency-graph-repo"],
		["I want to review pull requests visually", "review-pull-requests"],
		["a spreadsheet for tracking API latency budgets", "spreadsheet-api-latency"],
		["incident timeline", "incident-timeline"],
		["add a canvas that visualises our deploy pipeline", "visualises-deploy-pipeline"],
		["something to plan the migration", "plan-migration"],
		["generate a colour palette picker", "colour-palette-picker"],
		["help me compare two benchmark runs side by side", "compare-benchmark-runs"],
		["please build a retro board for the team", "retro-board-team"],
		["release checklist", "release-checklist"],
	];

	for (const [description, expected] of cases) {
		it(`"${description}" → ${expected}`, () => {
			expect(canvasNameFromDescription(description)).toBe(expected);
		});
	}

	/**
	 * The guard on preferring nouns. Dropping every `-ing`/`-ed` word would leave
	 * this with nothing, and a name we invented would hide that we did not
	 * understand the request — so where the participle is the only subject, it
	 * stays.
	 */
	it("keeps a participle when it is the subject rather than the verb", () => {
		expect(canvasNameFromDescription("a canvas for onboarding")).toBe("onboarding");
		expect(canvasNameFromDescription("a canvas for tracking and reporting")).toBe("tracking-reporting");
	});

	it("gives up rather than naming a canvas after the request alone", () => {
		expect(canvasNameFromDescription("please make me one")).toBeUndefined();
		expect(canvasNameFromDescription("!!!")).toBeUndefined();
	});
});

describe("renaming and removing a canvas", () => {
	let cwd: string;
	let roots: CanvasSearchRoot[];
	let agentsRoot: string;

	const extensionAt = (id: string) => ({
		id,
		dir: path.join(agentsRoot, id),
		entry: path.join(agentsRoot, id, CANVAS_ENTRY_FILE),
		scope: "agents" as const,
	});

	const scaffold = (id: string) => {
		fs.mkdirSync(path.join(agentsRoot, id), { recursive: true });
		fs.writeFileSync(path.join(agentsRoot, id, CANVAS_ENTRY_FILE), CANVAS_ENTRY_TEMPLATE(id), "utf8");
		return extensionAt(id);
	};

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-life-"));
		agentsRoot = path.join(cwd, ".agents", "extensions");
		fs.mkdirSync(agentsRoot, { recursive: true });
		roots = [{ dir: agentsRoot, scope: "agents" }];
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	/**
	 * The whole reason rename exists. The scaffold writes the name into the
	 * directory, the canvas `id`, its `displayName` and the header comment, and
	 * the `id` is the one that is not cosmetic: an open instance is bound to it,
	 * so a hand-rename that misses it drops the canvas on the next reload.
	 */
	it("moves the directory and renames the canvas inside the code", () => {
		const extension = scaffold("create-lightweight-games");
		const result = renameCanvasExtension(extension, "arcade", roots);
		if (isCanvasRefusal(result)) throw new Error(`refused: ${result.detail}`);

		expect(fs.existsSync(path.join(agentsRoot, "create-lightweight-games"))).toBe(false);
		const source = fs.readFileSync(path.join(agentsRoot, "arcade", CANVAS_ENTRY_FILE), "utf8");
		expect(source).toContain('const ID = "arcade"');
		expect(source).toContain('const NAME = "arcade"');
		expect(source).toContain("/canvas open arcade");
		// Nothing left over: the scaffold names itself in exactly the places the
		// rewrite covers, which is why the template names itself once rather than
		// six times.
		expect(source).not.toContain("create-lightweight-games");
		expect(result.leftovers).toEqual([]);
		expect(result.rewrites.length).toBeGreaterThanOrEqual(3);
	});

	/**
	 * A rename that silently rewrote prose would be worse than one that admits
	 * what it left behind — a description mentioning the old name is the author's
	 * sentence, not the canvas's identity.
	 */
	it("leaves prose alone and reports the lines it left", () => {
		const extension = scaffold("board");
		const entry = path.join(agentsRoot, "board", CANVAS_ENTRY_FILE);
		fs.writeFileSync(entry, `${fs.readFileSync(entry, "utf8")}\n// the board is the point of the board\n`, "utf8");

		const result = renameCanvasExtension(extension, "plan", roots);
		if (isCanvasRefusal(result)) throw new Error(`refused: ${result.detail}`);

		const source = fs.readFileSync(path.join(agentsRoot, "plan", CANVAS_ENTRY_FILE), "utf8");
		expect(source).toContain('const ID = "plan"');
		expect(source).toContain("// the board is the point of the board");
		expect(result.leftovers.length).toBeGreaterThan(0);
	});

	it("refuses a name that is not a directory slug, or one already taken", () => {
		const extension = scaffold("board");
		scaffold("taken");

		const bad = renameCanvasExtension(extension, "Not A Slug", roots);
		expect(isCanvasRefusal(bad) && bad.reason).toBe("invalid-name");
		const clash = renameCanvasExtension(extension, "taken", roots);
		expect(isCanvasRefusal(clash) && clash.reason).toBe("exists");
		// Neither attempt moved anything.
		expect(fs.existsSync(path.join(agentsRoot, "board", CANVAS_ENTRY_FILE))).toBe(true);
	});

	it("refuses to rename or delete a canvas that arrived inside a plugin", () => {
		// A plugin's canvases are named by its manifest, not by position, so the
		// directory sits somewhere no search root points at.
		const packagedDir = path.join(cwd, "plugins", "some-plugin", "com.github.copilot", "extensions", "shipped");
		fs.mkdirSync(packagedDir, { recursive: true });
		fs.writeFileSync(path.join(packagedDir, CANVAS_ENTRY_FILE), CANVAS_ENTRY_TEMPLATE("shipped"), "utf8");
		const packaged = {
			id: "shipped",
			dir: packagedDir,
			entry: path.join(packagedDir, CANVAS_ENTRY_FILE),
			scope: "user" as const,
		};

		expect(canvasHomeRoot(packaged, roots)).toBeUndefined();
		const renamed = renameCanvasExtension(packaged, "mine", roots);
		expect(isCanvasRefusal(renamed) && renamed.reason).toBe("packaged");
		expect(isCanvasRefusal(renamed) && renamed.detail).toContain("/plugin");

		const removed = removeCanvasExtension(packaged, roots);
		expect(isCanvasRefusal(removed) && removed.reason).toBe("packaged");
		// And it is still there, which is the point of refusing.
		expect(fs.existsSync(path.join(packagedDir, CANVAS_ENTRY_FILE))).toBe(true);
	});

	it("removes the directory it was told to remove, and nothing else", () => {
		const extension = scaffold("doomed");
		scaffold("kept");

		const result = removeCanvasExtension(extension, roots);
		expect(isCanvasRefusal(result)).toBe(false);
		expect(fs.existsSync(path.join(agentsRoot, "doomed"))).toBe(false);
		expect(fs.existsSync(path.join(agentsRoot, "kept", CANVAS_ENTRY_FILE))).toBe(true);
	});
});

describe("renaming and removing through a live session", () => {
	let cwd: string;
	let home: string;
	let agentDir: string;
	let session: CanvasSession;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-life-live-"));
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-life-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });
		trustWorkspace(cwd, agentDir);

		const dir = path.join(cwd, ".agents", "extensions", "board");
		fs.mkdirSync(dir, { recursive: true });
		fs.writeFileSync(path.join(dir, CANVAS_ENTRY_FILE), CANVAS_ENTRY_TEMPLATE("board"), "utf8");

		session = new CanvasSession({
			cwd,
			homeDir: home,
			agentDir,
			resolveRuntime: async () => ({ available: true, runtime: canvasTestRuntime() }),
		});
	});

	afterEach(async () => {
		await session.dispose();
		untrustWorkspace(cwd, agentDir);
		fs.rmSync(cwd, { recursive: true, force: true });
		fs.rmSync(home, { recursive: true, force: true });
	});

	/**
	 * The failure this rules out: a child left running against a directory that
	 * has moved keeps serving code from a path that no longer exists, and the
	 * registry still believes the instance is live.
	 */
	it("closes what the extension had open, and the renamed canvas opens under its new name", async () => {
		const opened = await session.open({ extensionId: "board" });
		expect(await (await fetch(opened.url as string)).status).toBe(200);

		const result = await session.rename("board", "plan");
		if (isCanvasRefusal(result)) throw new Error(`refused: ${result.detail}`);

		expect(session.instances()).toEqual([]);
		await expect(fetch(opened.url as string)).rejects.toThrow();

		const overview = await session.list();
		expect(overview.listings.map((listing) => listing.extensionId)).toEqual(["plan"]);

		// And the canvas really is called `plan` now, not just its directory.
		const reopened = await session.open({ extensionId: "plan" });
		expect(reopened.canvasId).toBe("plan");
	}, 40_000);

	it("removes a running canvas, releasing its port", async () => {
		const opened = await session.open({ extensionId: "board" });
		expect(await (await fetch(opened.url as string)).status).toBe(200);

		const result = await session.remove("board");
		expect(isCanvasRefusal(result)).toBe(false);

		await expect(fetch(opened.url as string)).rejects.toThrow();
		expect((await session.list()).listings).toEqual([]);
		expect(fs.existsSync(path.join(cwd, ".agents", "extensions", "board"))).toBe(false);
	}, 40_000);

	it("names the actions of an open instance, which nothing else showed the person", async () => {
		const opened = await session.open({ extensionId: "board" });
		const overview = await session.list();
		expect(overview.actionsByInstance.get(opened.instanceId)).toEqual(["add_note"]);
	}, 40_000);

	it("reports nothing for an extension that has never been forked", async () => {
		const overview = await session.list();
		expect(overview.listings).toHaveLength(1);
		// Not zero actions — none knowable, because a canvas has no passive half.
		expect(overview.actionsByInstance.size).toBe(0);
	});

	it("says what it cannot find, rather than failing silently", async () => {
		const renamed = await session.rename("nope", "other");
		expect(isCanvasRefusal(renamed) && renamed.detail).toContain('No canvas extension "nope"');
		const removed = await session.remove("nope");
		expect(isCanvasRefusal(removed) && removed.detail).toContain('No canvas extension "nope"');
	});
});
