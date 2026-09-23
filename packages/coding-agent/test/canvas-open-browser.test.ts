/**
 * Opening a canvas from a terminal: the browser goes there by itself, and the
 * url stays pinned above the prompt for as long as the canvas is open.
 *
 * Driven end to end: a real canvas is scaffolded, forked through the production
 * runner and served over loopback, and the command runs through the same
 * loader path a terminal takes (`hasUI: true`). The browser is the only thing
 * stood in for, so the test can see what would have opened without a window
 * appearing on the machine running it.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { hyperlinkAt } from "@kolisachint/hoocode-tui";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { CANVAS_LINKS_WIDGET, CanvasLinksBand, setupCanvas } from "../src/extensions/core/canvas.js";
import { initTheme, theme } from "../src/modes/interactive/theme/theme.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

type Handler = (args: string, ctx: unknown) => Promise<void>;
type WidgetFactory = ((tui: unknown, theme: unknown) => CanvasLinksBand) | undefined;

const URL_PATTERN = /^http:\/\/127\.0\.0\.1:\d+\/\?token=/;
const stripAnsi = (text: string) => text.replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;]*m/g, "");

describe("opening a canvas from a terminal", () => {
	let cwd: string;
	let home: string;
	let agentDir: string;
	let priorAgentDir: string | undefined;
	let commands: Map<string, { handler: Handler }>;
	let shutdown: (() => void) | undefined;
	let notifications: string[];
	let browsed: string[];
	let widgets: Map<string, WidgetFactory>;
	let makeCtx: (hasUI: boolean) => unknown;

	beforeAll(() => initTheme("dark"));

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-browser-"));
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-canvas-browser-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = agentDir;
		commands = new Map();
		notifications = [];
		browsed = [];
		widgets = new Map();
		const hoo = {
			registerCommand: (name: string, def: { handler: Handler }) => commands.set(name, def),
			registerTool: () => {},
			on: (_event: string, handler: () => void) => {
				shutdown = handler;
			},
			sendUserMessage: () => {},
		} as never;
		setupCanvas(hoo, {
			homeDir: home,
			resolveRuntime: async () => ({ available: true, runtime: canvasTestRuntime() }),
			openUrl: (url) => browsed.push(url),
		});
		makeCtx = (hasUI) => ({
			cwd,
			hasUI,
			ui: {
				notify: (message: string) => notifications.push(message),
				setWidget: (key: string, content: WidgetFactory) => {
					if (content === undefined) widgets.delete(key);
					else widgets.set(key, content);
				},
				// The loader path, as a terminal runs it: the factory builds the
				// loader and the open settles it through `done`.
				custom: <T>(factory: (tui: unknown, t: unknown, kb: unknown, done: (value: T) => void) => unknown) =>
					new Promise<T>((resolve) => {
						let component: { dispose?(): void } | undefined;
						component = factory({ requestRender() {} }, theme, {}, (value) => {
							component?.dispose?.();
							resolve(value);
						}) as { dispose?(): void };
					}),
			},
		});
	});

	afterEach(() => {
		shutdown?.();
		shutdown = undefined;
		untrustWorkspace(cwd, agentDir);
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(cwd, { recursive: true, force: true });
		fs.rmSync(home, { recursive: true, force: true });
	});

	const run = (command: string, args: string, hasUI = true) =>
		commands.get(command)?.handler(args, makeCtx(hasUI)) as Promise<void>;
	const band = (width = 120): string[] => {
		const factory = widgets.get(CANVAS_LINKS_WIDGET);
		return factory ? factory({}, theme).render(width) : [];
	};

	it("sends the browser to a canvas /new-canvas opened, and pins its url", async () => {
		await run("new-canvas", "board");

		expect(browsed).toHaveLength(1);
		expect(browsed[0]).toMatch(URL_PATTERN);
		// The url the browser was sent to is live, not merely well-formed.
		expect((await fetch(browsed[0] as string)).status).toBe(200);

		const rows = band();
		expect(rows).toHaveLength(1);
		expect(stripAnsi(rows[0] as string)).toContain(browsed[0]);
		expect(stripAnsi(rows[0] as string)).toContain("click to reopen");
		// The pinned row is a real link, so a click anywhere on the url opens it.
		const column = stripAnsi(rows[0] as string).indexOf("http");
		expect(hyperlinkAt(rows[0] as string, column)).toBe(browsed[0]);
		expect(notifications.join("\n")).toContain("Opened in your browser");
	});

	it("does the same for /canvas open, and takes the pin down on close", async () => {
		await run("new-canvas", "board");
		const listed = notifications.join("\n");
		const instanceId = /Opened board \(([^)]+)\)/.exec(stripAnsi(listed))?.[1];
		expect(instanceId).toBeDefined();

		await run("canvas", "open board");
		expect(browsed).toHaveLength(2);
		expect(browsed[1]).toMatch(URL_PATTERN);
		expect(band()).toHaveLength(2);

		await run("canvas", `close ${instanceId}`);
		expect(band()).toHaveLength(1);
		expect(stripAnsi(band()[0] as string)).toContain(browsed[1]);

		const second = [...stripAnsi(notifications.join("\n")).matchAll(/Opened board \(([^)]+)\)/g)][1]?.[1];
		await run("canvas", `close ${second}`);
		expect(widgets.has(CANVAS_LINKS_WIDGET)).toBe(false);
	});

	it("follows a reload to the new url, which the old tab cannot reach", async () => {
		await run("new-canvas", "board");
		const before = browsed[0];

		await run("canvas", "reload board");

		expect(browsed).toHaveLength(2);
		expect(browsed[1]).toMatch(URL_PATTERN);
		expect(browsed[1]).not.toBe(before);
		expect(stripAnsi(band()[0] as string)).toContain(browsed[1]);
	});

	it("opens nothing and pins nothing outside a terminal", async () => {
		await run("new-canvas", "board", false);

		expect(browsed).toEqual([]);
		expect(widgets.size).toBe(0);
		const said = notifications.join("\n");
		// Plain text for --print and RPC: no escape codes in the url they are given.
		expect(said).toMatch(/Open in a browser: http:\/\/127\.0\.0\.1:\d+\/\?token=\S+/);
		expect(said).not.toContain("\x1b]8;");
	});

	it("clears the pin when the session shuts down", async () => {
		await run("new-canvas", "board");
		expect(widgets.has(CANVAS_LINKS_WIDGET)).toBe(true);
		shutdown?.();
		shutdown = undefined;
		expect(widgets.has(CANVAS_LINKS_WIDGET)).toBe(false);
	});
});

describe("CanvasLinksBand", () => {
	beforeAll(() => initTheme("dark"));

	const instance = (id: string, url: string | undefined) =>
		({ instanceId: id, canvasId: "board", extensionId: "board", title: "Board", url }) as never;

	it("keeps the whole url behind a row too narrow to show it", () => {
		const url = `http://127.0.0.1:4000/?token=${"a".repeat(80)}`;
		const rows = new CanvasLinksBand(() => [instance("i1", url)], theme).render(40);
		expect(rows).toHaveLength(1);
		expect(stripAnsi(rows[0] as string).length).toBeLessThanOrEqual(40);
		expect(stripAnsi(rows[0] as string)).not.toContain("click to reopen");
		expect(hyperlinkAt(rows[0] as string, 15)).toBe(url);
	});

	it("draws nothing for instances with no url, and summarises past four", () => {
		expect(new CanvasLinksBand(() => [instance("i1", undefined)], theme).render(80)).toEqual([]);
		const many = Array.from({ length: 6 }, (_, i) => instance(`i${i}`, `http://127.0.0.1:${4000 + i}/`));
		const rows = new CanvasLinksBand(() => many, theme).render(80);
		expect(rows).toHaveLength(5);
		expect(stripAnsi(rows[4] as string)).toContain("+2 more open");
	});
});
