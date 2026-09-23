/**
 * Acceptance: drawio-canvas, end to end through hoocode.
 *
 * `kolisachint/drawio-canvas` is a canvas where a person edits in the full
 * draw.io editor while the agent edits the same document through actions. This
 * runs it the way a session does — discovery, `/canvas open` through the
 * session facade, the agent's own `list_canvas_capabilities` /
 * `invoke_canvas_action` / `reload_canvas` tools — with the person played by
 * Chromium on the URL the canvas returned.
 *
 * Skipped unless a checkout is named:
 *
 *   git clone https://github.com/kolisachint/drawio-canvas /tmp/drawio-canvas
 *   npm install --prefix /tmp/pw playwright       # optional: the browser half
 *   HOOCODE_DRAWIO_CANVAS_DIR=/tmp/drawio-canvas \
 *   HOOCODE_PLAYWRIGHT=/tmp/pw/node_modules/playwright \
 *     bunx vitest run test/canvas-acceptance-drawio.test.ts
 *
 * Without `HOOCODE_PLAYWRIGHT` the agent half still runs; the browser half
 * (person edits, draw.io screenshots) is skipped.
 */

import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { CanvasAvailability } from "../src/core/canvas/launch.js";
import { CanvasSession } from "../src/core/canvas/session.js";
import type { ExtensionContext } from "../src/core/extensions/types.js";
import {
	createCanvasToolDefinitions,
	INVOKE_CANVAS_ACTION_TOOL_NAME,
	LIST_CANVAS_CAPABILITIES_TOOL_NAME,
	RELOAD_CANVAS_TOOL_NAME,
} from "../src/core/tools/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const CHECKOUT = process.env.HOOCODE_DRAWIO_CANVAS_DIR;
const PLAYWRIGHT = process.env.HOOCODE_PLAYWRIGHT;
const present = CHECKOUT !== undefined && existsSync(path.join(CHECKOUT, "extension.mjs"));
if (CHECKOUT && !present)
	throw new Error(`HOOCODE_DRAWIO_CANVAS_DIR=${CHECKOUT} has no extension.mjs; refusing to skip.`);

const NO_CTX = {} as unknown as ExtensionContext;
type Tool = ReturnType<typeof createCanvasToolDefinitions>[number];

describe.skipIf(!present)("acceptance: drawio-canvas through hoocode", () => {
	let cwd: string;
	let home: string;
	let session: CanvasSession;
	let tools: Map<string, Tool>;
	let instanceId: string;
	let url: string;

	const text = (result: unknown) =>
		(result as { content: { text?: string }[] }).content.map((part) => part.text ?? "").join("");
	async function call(name: string, params: Record<string, unknown>) {
		const tool = tools.get(name);
		if (!tool) throw new Error(`no tool ${name}`);
		return text(await tool.execute("call", params as never, undefined, undefined, NO_CTX));
	}
	async function act(action: string, input: Record<string, unknown> = {}) {
		return JSON.parse(await call(INVOKE_CANVAS_ACTION_TOOL_NAME, { instanceId, action, input }));
	}

	beforeAll(async () => {
		cwd = mkdtempSync(path.join(tmpdir(), "hoocode-drawio-workspace-"));
		home = mkdtempSync(path.join(tmpdir(), "hoocode-drawio-home-"));
		// User scope, where `git clone … ~/.copilot/extensions/drawio-canvas` puts it.
		mkdirSync(path.join(home, ".copilot", "extensions"), { recursive: true });
		symlinkSync(path.resolve(CHECKOUT as string), path.join(home, ".copilot", "extensions", "drawio-canvas"));
		const available = async (): Promise<CanvasAvailability> => ({ available: true, runtime: canvasTestRuntime() });
		session = new CanvasSession({
			cwd,
			homeDir: home,
			agentDir: path.join(home, ".hoocode"),
			resolveRuntime: available,
		});
	});

	afterAll(async () => {
		await session?.dispose();
		rmSync(cwd, { recursive: true, force: true });
		rmSync(home, { recursive: true, force: true });
	});

	it("is discovered and opens, with the agent's tools", async () => {
		const overview = await session.list();
		expect(JSON.stringify(overview)).toContain("drawio-canvas");
		const instance = await session.open({ extensionId: "drawio-canvas" });
		instanceId = instance.instanceId;
		url = instance.url ?? "";
		expect(url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/[A-Za-z0-9_-]{20,}\/$/);
		const registry = session.registryOrUndefined();
		if (!registry) throw new Error("no registry after open");
		tools = new Map(createCanvasToolDefinitions(registry).map((tool) => [tool.name, tool]));
		const listed = await call(LIST_CANVAS_CAPABILITIES_TOOL_NAME, {});
		for (const action of [
			"get_diagram",
			"get_changes",
			"edit_diagram",
			"search_shapes",
			"insert_shapes",
			"manage_layers",
			"screenshot",
			"focus",
			"layout",
		]) {
			expect(listed).toContain(`"${action}"`);
		}
	}, 60_000);

	it("lets the agent draw with real library icons", async () => {
		const { shapes } = await act("search_shapes", { query: "aws lambda" });
		expect(shapes[0].id).toBe("aws4Compute/lambda");
		await act("edit_diagram", {
			operations: [
				{
					operation: "add",
					cell_id: "api",
					new_xml:
						'<mxCell value="API" style="rounded=1;" vertex="1" parent="1"><mxGeometry x="40" y="40" width="120" height="60" as="geometry"/></mxCell>',
				},
			],
		});
		const inserted = await act("insert_shapes", {
			shapes: [{ shape_id: shapes[0].id, cell_id: "fn", x: 260, y: 30, label: "Resize" }],
		});
		expect(inserted.inserted).toEqual(["fn"]);
		const diagram = await act("get_diagram", {});
		expect(diagram.page.shapes).toBe(2);
		expect(diagram.cells_xml).toContain("resIcon=mxgraph.aws4.lambda");
	}, 30_000);

	describe.skipIf(!PLAYWRIGHT)("with the person in draw.io", () => {
		let browser: any;
		let page: any;
		const problems: string[] = [];

		beforeAll(async () => {
			const entry = pathToFileURL(path.join(PLAYWRIGHT as string, "index.js")).href;
			const playwright = await import(entry);
			const chromium = playwright.chromium ?? playwright.default.chromium;
			browser = await chromium.launch({
				executablePath: process.env.HOOCODE_CHROMIUM || undefined,
				args: ["--no-sandbox"],
			});
			page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
			page.on("pageerror", (error: Error) => problems.push(error.message));
			page.on("request", (request: { url(): string }) => {
				if (!request.url().startsWith("http://127.0.0.1:")) problems.push(`left the machine: ${request.url()}`);
			});
			await page.goto(url);
			await page.waitForFunction(() => (globalThis as { drawioCanvas?: unknown }).drawioCanvas, null, {
				timeout: 120_000,
			});
		}, 150_000);

		afterAll(async () => {
			await browser?.close();
		});

		const frame = () => page.frame({ url: /drawio\/index\.html/ });

		it("shows the agent's work in draw.io and reports the person's back", async () => {
			const cells: string[] = await frame().evaluate("Object.keys(window.drawioCanvasUi.editor.graph.model.cells)");
			expect(cells).toEqual(expect.arrayContaining(["api", "fn"]));
			// The person's edits, through draw.io's own graph API.
			await frame().evaluate(`(() => {
				const graph = window.drawioCanvasUi.editor.graph;
				graph.model.setValue(graph.model.getCell("api"), "Public API");
				graph.insertVertex(graph.getDefaultParent(), "db", "Orders DB", 40, 200, 120, 60, "shape=cylinder3;");
			})()`);
			await page.waitForTimeout(800);
			const { changes } = await act("get_changes");
			expect(changes.join("\n")).toMatch(/human .*relabelled "API" → "Public API"/);
			expect(changes.join("\n")).toMatch(/human .*added cylinder3 "Orders DB" \[db\]/);
			await expect(
				act("edit_diagram", {
					operations: [
						{
							operation: "update",
							cell_id: "api",
							new_xml:
								'<mxCell value="API v2" style="rounded=1;" vertex="1" parent="1"><mxGeometry x="40" y="40" width="120" height="60" as="geometry"/></mxCell>',
						},
					],
				}),
			).rejects.toThrow(/stale_cells/);
		}, 60_000);

		it("takes a screenshot the agent can read", async () => {
			const shot = await act("screenshot");
			const png = readFileSync(path.join(cwd, shot.path));
			expect(png.subarray(1, 4).toString()).toBe("PNG");
			expect(shot.note).toMatch(/Rendered by the person's draw\.io/);
			expect(problems).toEqual([]);
		}, 60_000);

		it("survives reload_canvas with the document intact", async () => {
			const reloaded = await call(RELOAD_CANVAS_TOOL_NAME, { extensionId: "drawio-canvas" });
			expect(reloaded).toMatch(/reopened|1/);
			const instance = session.instances().find((open) => open.instanceId === instanceId);
			expect(instance?.url).not.toBe(url);
			const diagram = await act("get_diagram", {});
			expect(diagram.cells_xml).toContain("Orders DB");
			expect(diagram.cells_xml).toContain("Public API");
		}, 60_000);
	});
});
