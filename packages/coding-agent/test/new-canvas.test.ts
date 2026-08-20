/**
 * `/new-canvas` — the authoring half of canvas extensions
 * (`docs/canvas-extensions-design.md` §9, Phase 3).
 *
 * The interesting assertion is not that a file appeared. A canvas has no passive
 * half — its id, its actions and its UI all come from running its code — so a
 * scaffold that does not *run* is worthless, and a snapshot of the template text
 * would only confirm that we wrote what we wrote. So the template is forked here
 * for real, through the same runner production uses, and driven over the
 * protocol: it must announce itself, serve a page, answer its action, and close.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE } from "../src/core/canvas/discovery.js";
import { type CanvasExtensionProcess, spawnCanvasExtension } from "../src/core/canvas/runner.js";
import { setPlatforms } from "../src/core/extensions/plugins/formats/platform-targets.js";
import { setupScaffold } from "../src/extensions/core/scaffold.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

describe("/new-canvas", () => {
	let cwd: string;
	let commands: Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>;
	let notifications: string[];
	let ctx: unknown;
	let running: CanvasExtensionProcess | undefined;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-new-canvas-"));
		commands = new Map();
		notifications = [];
		const pi = {
			registerCommand: (name: string, def: { handler: (args: string, ctx: unknown) => Promise<void> }) =>
				commands.set(name, def),
		} as never;
		setupScaffold(pi);
		ctx = { cwd, ui: { notify: (message: string) => notifications.push(message) } } as never;
	});

	afterEach(async () => {
		await running?.terminate();
		running = undefined;
		setPlatforms(undefined);
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	const run = (args: string) => commands.get("new-canvas")?.handler(args, ctx);

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
		expect(notifications.join("\n")).toContain("no canvas home");
	});

	it("rejects a name that is not a valid directory slug", async () => {
		await run("Not A Slug");
		expect(fs.existsSync(path.join(cwd, ".agents"))).toBe(false);
		expect(notifications.join("\n")).toContain("lowercase");
	});

	it("never clobbers an existing extension", async () => {
		const file = path.join(cwd, ".agents", "extensions", "kept", CANVAS_ENTRY_FILE);
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, "// mine", "utf8");

		await run("kept");
		expect(fs.readFileSync(file, "utf8")).toBe("// mine");
		expect(notifications.join("\n")).toContain("already exist");
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
});
