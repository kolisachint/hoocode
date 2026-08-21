/**
 * `/new-canvas` — the authoring half of canvas extensions
 * (`docs/canvas-extensions-design.md` §9 Phase 3, §13).
 *
 * The interesting assertion is not that a file appeared. A canvas has no passive
 * half — its id, its actions and its UI all come from running its code — so a
 * scaffold that does not *run* is worthless, and a snapshot of the template text
 * would only confirm that we wrote what we wrote. So the template is forked here
 * for real, through the same runner production uses, and driven over the
 * protocol: it must announce itself, serve a page, answer its action, and close.
 *
 * The second half of the file is about the shape the command borrowed from
 * Copilot's `/create-canvas`: a sentence rather than a slug, opened before it is
 * built, and handed to the model as a brief. Those are behaviours a person would
 * notice missing, so each is asserted on the command's own output rather than on
 * the helpers underneath it.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import { type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { canvasNameFromDescription, parseCanvasRequest } from "../src/core/canvas/scaffold.js";
import { CanvasSession } from "../src/core/canvas/session.js";
import { setPlatforms } from "../src/core/extensions/plugins/formats/platform-targets.js";
import { isWorkspaceTrusted, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { setupCanvas } from "../src/extensions/core/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

describe("/new-canvas", () => {
	let cwd: string;
	let commands: Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>;
	let shutdown: (() => void) | undefined;
	let notifications: string[];
	let sentToModel: string[];
	let ctx: unknown;
	let running: CanvasExtensionProcess | undefined;
	let home: string;
	let agentDir: string;
	let priorAgentDir: string | undefined;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-new-canvas-"));
		// The command reaches for `getAgentDir()` to record workspace trust, so the
		// trust store has to be redirected somewhere disposable — otherwise these
		// tests would trust a temp directory on the developer's real machine.
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-new-canvas-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = agentDir;
		commands = new Map();
		notifications = [];
		sentToModel = [];
		const pi = {
			registerCommand: (name: string, def: { handler: (args: string, ctx: unknown) => Promise<void> }) =>
				commands.set(name, def),
			registerTool: () => {},
			on: (_event: string, handler: () => void) => {
				shutdown = handler;
			},
			sendUserMessage: (content: string) => {
				sentToModel.push(content);
			},
		} as never;
		setupCanvas(pi, {
			homeDir: home,
			resolveRuntime: async () => ({ available: true, runtime: canvasTestRuntime() }),
		});
		// `hasUI: false` is the --print/RPC path, which opens without a loader. The
		// loader needs a terminal, and cancelling through it is covered by
		// `canvas-cancel.test.ts`.
		ctx = { cwd, hasUI: false, ui: { notify: (message: string) => notifications.push(message) } } as never;
	});

	afterEach(async () => {
		// `/new-canvas` opens what it creates, so nearly every test here leaves a
		// child holding a port. The session's own shutdown hook is what production
		// uses; firing it is both the cleanup and a check that it is wired.
		shutdown?.();
		shutdown = undefined;
		await running?.terminate();
		running = undefined;
		setPlatforms(undefined);
		untrustWorkspace(cwd, agentDir);
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(cwd, { recursive: true, force: true });
		fs.rmSync(home, { recursive: true, force: true });
	});

	const run = (args: string) => commands.get("new-canvas")?.handler(args, ctx);
	const said = () => notifications.join("\n");

	it("scaffolds into .agents/extensions, the native canvas search root", async () => {
		await run("my-board");
		expect(fs.existsSync(path.join(cwd, ".agents", "extensions", "my-board", CANVAS_ENTRY_FILE))).toBe(true);
	});

	it("--platform github scaffolds into Copilot's project scope", async () => {
		setPlatforms(["github"]);
		await run("my-board");
		expect(fs.existsSync(path.join(cwd, ".github", "extensions", "my-board", CANVAS_ENTRY_FILE))).toBe(true);
		expect(fs.existsSync(path.join(cwd, ".agents"))).toBe(false);
	});

	it("declines rather than inventing a home for a platform that has no canvas surface", async () => {
		setPlatforms(["claude"]);
		await run("my-board");
		expect(fs.existsSync(path.join(cwd, ".claude"))).toBe(false);
		expect(said()).toContain("no canvas home");
	});

	it("never clobbers an existing extension", async () => {
		const file = path.join(cwd, ".agents", "extensions", "kept", CANVAS_ENTRY_FILE);
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, "// mine", "utf8");

		await run("kept");
		expect(fs.readFileSync(file, "utf8")).toBe("// mine");
		expect(said()).toContain("already exist");
	});

	/**
	 * The bug this locks down: `.agents/extensions/` is a project-scope root, so
	 * the trust gate withheld a canvas the moment it was scaffolded — the command
	 * printed "Open it now" and `/canvas open` then refused with "came with this
	 * repository", of a file created seconds earlier.
	 */
	it("leaves the scaffolded canvas openable, and says that it trusted the workspace to do it", async () => {
		expect(isWorkspaceTrusted(cwd, agentDir)).toBe(false);

		await run("my-board");

		expect(isWorkspaceTrusted(cwd, agentDir)).toBe(true);
		// Granting is wider than the canvas, so it must be visible and reversible.
		expect(said()).toContain("Trusted this workspace");
		expect(said()).toContain("/plugin untrust");

		const session = new CanvasSession({
			cwd,
			homeDir: home,
			agentDir,
			resolveRuntime: async () => ({ available: true, runtime: canvasTestRuntime() }),
		});
		try {
			const overview = await session.list();
			expect(overview.listings.map((l) => l.extensionId)).toEqual(["my-board"]);
			expect(overview.listings[0]?.withheld).toBeUndefined();
			expect(overview.withheldCount).toBe(0);
		} finally {
			await session.dispose();
		}
	});

	it("does not re-trust, or claim to, a workspace that was already trusted", async () => {
		await run("first");
		notifications.length = 0;
		await run("second");
		expect(said()).not.toContain("Trusted this workspace");
	});

	it("grants nothing when it created nothing", async () => {
		await run("   ");
		expect(isWorkspaceTrusted(cwd, agentDir)).toBe(false);
	});

	it("scaffolds a canvas that actually runs: it declares itself and answers its action", async () => {
		await run("my-board");
		const entry = path.join(cwd, ".agents", "extensions", "my-board", CANVAS_ENTRY_FILE);

		running = spawnCanvasExtension({
			extensionId: "my-board",
			entry,
			runtime: canvasTestRuntime(),
			requestTimeoutMs: { "canvas.open": 20_000, "canvas.action.invoke": 20_000 },
		});

		const ready = await running.ready;
		expect(ready.extensionId).toBe("my-board");
		expect(ready.canvases).toHaveLength(1);
		expect(ready.canvases[0]?.id).toBe("my-board");
		expect(ready.canvases[0]?.actions?.map((action) => action.name)).toEqual(["add_note"]);

		const params = { sessionId: "s1", extensionId: "my-board", canvasId: "my-board", instanceId: "i1" };
		const opened = (await running.open(params)) as { url?: string; title?: string };
		expect(opened.title).toBe("my-board");
		expect(opened.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		// The scaffolded server is token-gated, so the URL it handed back must work
		// and a request without the token must not.
		const page = await fetch(opened.url as string);
		expect(page.status).toBe(200);
		expect(await page.text()).toContain("0 note(s)");
		const url = new URL(opened.url as string);
		url.searchParams.delete("token");
		expect((await fetch(url)).status).toBe(403);

		const result = (await running.invokeAction({ ...params, actionName: "add_note", input: { text: "hi" } })) as {
			notes: number;
		};
		expect(result.notes).toBe(1);

		await running.close(params);
		// The close handler shuts the server down, so the port stops answering.
		await expect(fetch(opened.url as string)).rejects.toThrow();
	});

	// ── Copilot's `/create-canvas` shape ──────────────────────────────────────

	/**
	 * The gap that started this: `/new-canvas <a sentence>` was refused with "name
	 * must be lowercase a-z", so the only way in was to have already decided on a
	 * directory name. Copilot's `/create-canvas` takes the sentence.
	 */
	it("takes a description, derives a directory name from it, and says which one it picked", async () => {
		await run("a kanban board for the release checklist");

		expect(fs.existsSync(path.join(cwd, ".agents", "extensions", "kanban-board-release", CANVAS_ENTRY_FILE))).toBe(
			true,
		);
		expect(said()).toContain("kanban-board-release");
	});

	it("takes an explicit name alongside a description", async () => {
		await run("release-board: a kanban board for the release checklist");

		expect(fs.existsSync(path.join(cwd, ".agents", "extensions", "release-board", CANVAS_ENTRY_FILE))).toBe(true);
		expect(sentToModel.join("\n")).toContain("a kanban board for the release checklist");
	});

	it("asks for a name rather than inventing one when nothing usable survives", async () => {
		await run("!!! ???");
		expect(fs.existsSync(path.join(cwd, ".agents"))).toBe(false);
		expect(said()).toContain("could not derive a directory name");
	});

	/**
	 * Opening is what makes the rest of it work: Copilot puts the canvas in front
	 * of the person and *then* builds it, and a brief that can name a live url and
	 * instance id is a different instruction than one that cannot.
	 */
	it("opens the canvas it created, without a separate /canvas open", async () => {
		await run("my-board");

		expect(said()).toContain("Opened my-board");
		expect(said()).toMatch(/http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		const url = said().match(/http:\/\/127\.0\.0\.1:\d+\/\?token=\S+/)?.[0] as string;
		expect((await fetch(url)).status).toBe(200);
	});

	it("hands the model a build brief naming the file, the live instance, and the reload it must call", async () => {
		await run("a kanban board for the release checklist");

		expect(sentToModel).toHaveLength(1);
		const brief = sentToModel[0] as string;
		expect(brief).toContain("a kanban board for the release checklist");
		expect(brief).toContain(path.join(".agents", "extensions", "kanban-board-release", CANVAS_ENTRY_FILE));
		expect(brief).toContain("reload_canvas");
		// The three contract rules whose symptom does not name the cause.
		expect(brief).toContain("@github/copilot-sdk/extension");
		expect(brief).toContain("session.log");
		// Found by using the command: renaming the canvas id drops the open instance.
		expect(brief).toContain("`id`");
		expect(brief).toContain("token");
		// It must name the instance it opened, or the model has nothing to reload.
		const instanceId = said().match(/Opened kanban-board-release \(([^)]+)\)/)?.[1] as string;
		expect(brief).toContain(instanceId);
	});

	/**
	 * `/new-canvas my-board` still means "give me the template to edit". Starting
	 * a build nobody asked for would burn a turn and overwrite the file they were
	 * about to open.
	 */
	it("starts no build when only a name was given", async () => {
		await run("my-board");
		expect(sentToModel).toEqual([]);
		expect(said()).toContain("/canvas reload my-board");
	});
});

/**
 * The parse is where a person's sentence becomes a directory, so its edges are
 * worth pinning: a bare slug must never be re-read as a one-word description,
 * and a derived name must be a legal directory without further cleaning.
 */
describe("reading what /new-canvas was given", () => {
	it("treats a bare slug as a name, with nothing to build", () => {
		expect(parseCanvasRequest("my-board")).toEqual({ name: "my-board", description: undefined });
		// A single word is a slug, not a description of a canvas.
		expect(parseCanvasRequest("board")).toEqual({ name: "board", description: undefined });
	});

	it("splits an explicit name from a description on the colon", () => {
		expect(parseCanvasRequest("release-board: track the release")).toEqual({
			name: "release-board",
			description: "track the release",
		});
	});

	it("falls back to derivation when the part before the colon is not a name", () => {
		// "Note:" is prose, not a slug, so the whole line is the description.
		const parsed = parseCanvasRequest("Note: a board for triage");
		expect(parsed).toEqual({ name: "note-board-triage", description: "Note: a board for triage" });
	});

	it("drops grammar but never subject matter when deriving a name", () => {
		expect(canvasNameFromDescription("a board for tracking the release checklist")).toBe("board-tracking-release");
		expect(canvasNameFromDescription("A Kanban Board!")).toBe("kanban-board");
		// "canvas" is dropped as a category word, unless it is all there is.
		expect(canvasNameFromDescription("a canvas showing flaky tests")).toBe("showing-flaky-tests");
		expect(canvasNameFromDescription("a canvas")).toBe("canvas");
	});

	it("gives up rather than inventing a name", () => {
		expect(canvasNameFromDescription("!!!")).toBeUndefined();
		expect(parseCanvasRequest("  ")).toBe("name or description is required");
	});
});
