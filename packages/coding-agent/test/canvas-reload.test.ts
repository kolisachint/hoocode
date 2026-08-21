/**
 * Reloading a canvas — the iterate loop.
 *
 * Design: `docs/canvas-extensions-design.md` §13. The gap this covers was silent
 * and total: a canvas has no passive half, so editing `extension.mjs` while the
 * canvas is open changed *nothing at all* — not the open page, and not even a
 * freshly opened second instance, because the registry hands back the child it
 * already forked. "Ask for a UI change and watch it appear", the thing
 * `/create-canvas` is for, was not reachable at any speed.
 *
 * So these tests drive the loop the way the model does, through the real tools
 * over the real runner: write the file, call `reload_canvas`, and check that what
 * the browser is served and what `invoke_canvas_action` can reach both changed.
 * Asserting on the registry alone would prove the table was updated, not that the
 * person's surface was.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CANVAS_ENTRY_FILE, type DiscoveredCanvasExtension } from "../src/core/canvas/discovery.js";
import { CanvasRegistry } from "../src/core/canvas/registry.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import type { ToolDefinition } from "../src/core/extensions/types.js";
import {
	createCanvasToolDefinitions,
	INVOKE_CANVAS_ACTION_TOOL_NAME,
	LIST_CANVAS_CAPABILITIES_TOOL_NAME,
	RELOAD_CANVAS_TOOL_NAME,
} from "../src/core/tools/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

/**
 * A minimal canvas whose page and action inventory are parameterised, so a test
 * can "edit" it the way a model would and see what the change reaches.
 */
function extensionSource(options: { body: string; extraAction?: boolean; canvasId?: string }): string {
	const canvasId = options.canvasId ?? "board";
	return `\
import { createCanvas, joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

const instances = new Map();

const session = await joinSession({
	canvases: [
		createCanvas({
			id: ${JSON.stringify(canvasId)},
			displayName: ${JSON.stringify(canvasId)},
			actions: [
				{
					name: "read_state",
					description: "Read the canvas state.",
					inputSchema: { type: "object", properties: {} },
					handler: (ctx) => ({ opened: instances.get(ctx.instanceId)?.input ?? null }),
				},
${
	options.extraAction
		? `				{
					name: "set_title",
					description: "Set the board title.",
					inputSchema: { type: "object", properties: { title: { type: "string" } }, required: ["title"] },
					handler: (ctx) => {
						instances.get(ctx.instanceId).title = ctx.input.title;
						return { title: ctx.input.title };
					},
				},
`
		: ""
}			],
			open: async (ctx) => {
				const token = randomBytes(16).toString("base64url");
				const entry = { input: ctx.input ?? null, title: null };
				const server = createServer((req, res) => {
					const url = new URL(req.url ?? "/", "http://127.0.0.1");
					if (url.searchParams.get("token") !== token) {
						res.writeHead(403).end("forbidden");
						return;
					}
					res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
					res.end(${JSON.stringify(options.body)} + " title=" + entry.title);
				});
				await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
				entry.server = server;
				instances.set(ctx.instanceId, entry);
				return { url: \`http://127.0.0.1:\${server.address().port}/?token=\${token}\`, title: ${JSON.stringify(canvasId)} };
			},
			onClose: async (ctx) => {
				const entry = instances.get(ctx.instanceId);
				if (!entry) return;
				instances.delete(ctx.instanceId);
				await new Promise((resolve) => entry.server.close(resolve));
			},
		}),
	],
});

await session.log("ready");
`;
}

/**
 * Call one of the canvas tools the way the agent loop does. The two arguments a
 * canvas tool never reads — the streaming callback and the extension context —
 * are passed as undefined rather than being stubbed, which is also an assertion
 * that they stay unread.
 */
function invoke(
	tools: Map<string, ToolDefinition>,
	name: string,
	params: unknown,
): Promise<{ content?: unknown[] } | undefined> {
	const tool = tools.get(name);
	if (!tool) throw new Error(`no tool "${name}"`);
	return tool.execute("call-1", params as never, new AbortController().signal, undefined, undefined as never);
}

describe("reloading a canvas", () => {
	let cwd: string;
	let agentDir: string;
	let registry: CanvasRegistry;
	let extension: DiscoveredCanvasExtension;
	let entryFile: string;

	const write = (source: string) => fs.writeFileSync(entryFile, source, "utf8");

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-reload-"));
		agentDir = path.join(cwd, ".agent-home");
		fs.mkdirSync(agentDir, { recursive: true });
		// The extension lives in the working tree, which is what the trust gate
		// asks about; `/new-canvas` grants this for the same reason.
		trustWorkspace(cwd, agentDir);

		const dir = path.join(cwd, ".agents", "extensions", "board");
		fs.mkdirSync(dir, { recursive: true });
		entryFile = path.join(dir, CANVAS_ENTRY_FILE);
		write(extensionSource({ body: "<p>version one" }));
		extension = { id: "board", dir, entry: entryFile, scope: "project" };

		registry = new CanvasRegistry({
			runtime: canvasTestRuntime(),
			cwd,
			agentDir,
			requestTimeoutMs: { "canvas.open": 20_000, "canvas.action.invoke": 20_000 },
		});
	});

	afterEach(async () => {
		await registry.shutdown();
		untrustWorkspace(cwd, agentDir);
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	const page = async (url: string | undefined) => (await fetch(url as string)).text();

	it("serves the edited code, which nothing else does", async () => {
		const before = await registry.open(extension, "board");
		expect(await page(before.url)).toContain("version one");

		write(extensionSource({ body: "<p>version two" }));

		// The premise: the write alone is invisible. This is the bug, asserted so a
		// future change that makes reload unnecessary fails here rather than
		// leaving a reload nobody needs.
		expect(await page(before.url)).toContain("version one");

		const result = await registry.reload("board");

		expect(result.reopened).toHaveLength(1);
		expect(await page(result.reopened[0]?.url)).toContain("version two");
	});

	it("keeps instance ids across a reload, and hands back new urls because the old ports are gone", async () => {
		const before = await registry.open(extension, "board");
		write(extensionSource({ body: "<p>version two" }));

		const result = await registry.reload("board");
		const after = result.reopened[0];

		expect(after?.instanceId).toBe(before.instanceId);
		expect(after?.url).not.toBe(before.url);
		// Saying "open the new url" is only honest if the old one is really dead.
		await expect(fetch(before.url as string)).rejects.toThrow();
	});

	it("replays the input the instance was opened with, so a reload restores rather than resets", async () => {
		const before = await registry.open(extension, "board", { seed: "kept" });
		expect(await registry.invokeAction(before, "read_state")).toEqual({ opened: { seed: "kept" } });

		write(extensionSource({ body: "<p>version two" }));
		const result = await registry.reload("board");

		expect(await registry.invokeAction(result.reopened[0] as never, "read_state")).toEqual({
			opened: { seed: "kept" },
		});
	});

	/**
	 * The case that decides whether iterating is safe. A broken edit is the normal
	 * failure while building, so it must not cost the person the canvas they are
	 * looking at — which is why the registry forks the new child before touching
	 * the old one.
	 */
	it("leaves the running canvas untouched when the edit does not run", async () => {
		const before = await registry.open(extension, "board");

		write("this is not valid javascript at all (((");

		await expect(registry.reload("board")).rejects.toThrow();

		// Still the same instance, still the same url, still serving.
		expect(registry.listInstances().map((instance) => instance.instanceId)).toEqual([before.instanceId]);
		expect(await page(before.url)).toContain("version one");
	});

	it("reports an instance whose canvas the edit removed, and keeps going", async () => {
		const before = await registry.open(extension, "board");

		write(extensionSource({ body: "<p>renamed", canvasId: "board-v2" }));
		const result = await registry.reload("board");

		expect(result.reopened).toEqual([]);
		expect(result.canvases).toEqual(["board-v2"]);
		expect(result.dropped).toHaveLength(1);
		expect(result.dropped[0]?.instanceId).toBe(before.instanceId);
		expect(result.dropped[0]?.reason).toContain("no longer declares");
		expect(registry.listInstances()).toEqual([]);
	});

	it("refuses to reload something that was never opened", async () => {
		await expect(registry.reload("board")).rejects.toThrow(/not running/);
	});

	// ── The model's path ──────────────────────────────────────────────────────

	/**
	 * End to end through the three tools, which is the loop a person actually
	 * sees: the canvas is open, they ask for a change, the model edits the file,
	 * reloads, and drives the capability it just added.
	 */
	it("lets the agent add a capability to an open canvas and immediately use it", async () => {
		const opened = await registry.open(extension, "board");
		const tools = new Map(createCanvasToolDefinitions(registry).map((tool) => [tool.name, tool as ToolDefinition]));
		const call = async (name: string, params: unknown): Promise<string> => {
			const result = await invoke(tools, name, params);
			return (result?.content?.[0] as { text: string } | undefined)?.text ?? "";
		};

		// Before the edit the canvas has one action, and the new one does not exist.
		expect(await call(LIST_CANVAS_CAPABILITIES_TOOL_NAME, {})).not.toContain("set_title");
		await expect(
			call(INVOKE_CANVAS_ACTION_TOOL_NAME, {
				instanceId: opened.instanceId,
				action: "set_title",
				input: { title: "Release 1" },
			}),
		).rejects.toThrow();

		// The model edits the extension: a new action, and a page that shows it.
		write(extensionSource({ body: "<p>board", extraAction: true }));

		const reloaded = await call(RELOAD_CANVAS_TOOL_NAME, { extensionId: "board" });
		expect(reloaded).toContain("Reloaded board");
		// The model has to be told the url changed, or it tells the person to look
		// at a tab pointing at a closed port.
		expect(reloaded).toMatch(/http:\/\/127\.0\.0\.1:\d+\/\?token=/);

		// The action it just wrote is now discoverable and invocable.
		expect(await call(LIST_CANVAS_CAPABILITIES_TOOL_NAME, {})).toContain("set_title");
		expect(
			await call(INVOKE_CANVAS_ACTION_TOOL_NAME, {
				instanceId: opened.instanceId,
				action: "set_title",
				input: { title: "Release 1" },
			}),
		).toContain("Release 1");

		// And the person, on the url the reload handed back, sees it.
		const url = reloaded.match(/http:\/\/127\.0\.0\.1:\d+\/\?token=\S+/)?.[0] as string;
		expect(await page(url)).toContain("title=Release 1");
	});

	it("tells the agent its edit is broken rather than pretending the reload worked", async () => {
		await registry.open(extension, "board");
		const tools = new Map(createCanvasToolDefinitions(registry).map((tool) => [tool.name, tool as ToolDefinition]));

		write("still not valid javascript (((");

		await expect(invoke(tools, RELOAD_CANVAS_TOOL_NAME, { extensionId: "board" })).rejects.toThrow();
	});

	it("refuses an extension the agent has nothing open for, naming what it does have", async () => {
		await registry.open(extension, "board");
		const tools = new Map(createCanvasToolDefinitions(registry).map((tool) => [tool.name, tool as ToolDefinition]));

		await expect(invoke(tools, RELOAD_CANVAS_TOOL_NAME, { extensionId: "not-open" })).rejects.toThrow(/board/);
	});
});
