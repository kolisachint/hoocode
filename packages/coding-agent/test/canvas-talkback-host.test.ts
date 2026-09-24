/**
 * A canvas talking back, end to end through the `/canvas` extension: a real
 * child opened with `/canvas open`, its `session.send` reaching the model as a
 * labelled user message, and the agent's lifecycle reaching its `session.on`.
 *
 * The policy itself is covered in `canvas-talkback.test.ts`; this checks the
 * wiring — that `setupCanvas` routes both directions and uses the inbox.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { trustWorkspace, untrustWorkspace } from "../src/core/extensions/plugins/trust.js";
import { INVOKE_CANVAS_ACTION_TOOL_NAME } from "../src/core/tools/canvas.js";
import { setupCanvas } from "../src/extensions/core/canvas.js";
import { canvasTestRuntime } from "./canvas-test-runtime.js";

const FIXTURE = path.join(import.meta.dirname, "fixtures", "canvas", "talkback", "extension.mjs");

type Handler = (event: unknown, ctx: unknown) => unknown;
type Tool = { execute: (id: string, params: unknown) => Promise<{ content: Array<{ text?: string }> }> };

describe("canvas talking back through /canvas", () => {
	let cwd: string;
	let home: string;
	let agentDir: string;
	let priorAgentDir: string | undefined;
	let handlers: Map<string, Handler[]>;
	let tools: Map<string, Tool>;
	let command: ((args: string, ctx: unknown) => Promise<void>) | undefined;
	let sent: Array<{ text: string; deliverAs?: string }>;
	let idle: boolean;
	let ctx: unknown;

	const fire = (event: string, payload: unknown = { type: event }) => {
		for (const handler of handlers.get(event) ?? []) handler(payload, ctx);
	};

	const invoke = async (instanceId: string, action: string, input?: unknown) => {
		const result = await tools.get(INVOKE_CANVAS_ACTION_TOOL_NAME)?.execute("call", { instanceId, action, input });
		return result?.content.map((part) => part.text ?? "").join("") ?? "";
	};

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-talkback-"));
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-talkback-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = agentDir;
		const dir = path.join(cwd, ".agents", "extensions", "talkback");
		fs.mkdirSync(dir, { recursive: true });
		fs.copyFileSync(FIXTURE, path.join(dir, "extension.mjs"));
		trustWorkspace(cwd, agentDir);

		handlers = new Map();
		tools = new Map();
		sent = [];
		idle = true;
		const hoo = {
			registerCommand: (name: string, def: { handler: (args: string, ctx: unknown) => Promise<void> }) => {
				if (name === "canvas") command = def.handler;
			},
			registerTool: (tool: Tool & { name: string }) => tools.set(tool.name, tool),
			on: (event: string, handler: Handler) => handlers.set(event, [...(handlers.get(event) ?? []), handler]),
			sendUserMessage: (text: string, options?: { deliverAs?: string }) => {
				sent.push({ text, deliverAs: options?.deliverAs });
			},
		} as never;
		setupCanvas(hoo, {
			homeDir: home,
			resolveRuntime: async () => ({ available: true, runtime: canvasTestRuntime() }),
		});
		ctx = { cwd, hasUI: false, isIdle: () => idle, ui: { notify: () => {} } };
	});

	afterEach(() => {
		fire("session_shutdown");
		untrustWorkspace(cwd, agentDir);
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(cwd, { recursive: true, force: true });
		fs.rmSync(home, { recursive: true, force: true });
	});

	it("routes session.send to the model and the agent's lifecycle to session.on", { timeout: 30_000 }, async () => {
		await command?.("open talkback", ctx);
		const listing = await tools.get("list_canvas_capabilities")?.execute("call", {});
		const instanceId = /"instanceId":\s*"([^"]+)"/.exec(listing?.content[0]?.text ?? "")?.[1] as string;
		expect(instanceId).toBeTruthy();

		// Busy: the message waits for the turn to end.
		idle = false;
		fire("agent_start");
		await invoke(instanceId, "ask", { prompt: "the person asked for help" });
		expect(sent).toEqual([]);
		idle = true;
		fire("agent_end", { type: "agent_end", messages: [] });
		expect(sent).toEqual([{ text: "[canvas talkback] the person asked for help", deliverAs: "followUp" }]);

		// The canvas subscribed to session.idle and heard the agent_end above.
		expect(await invoke(instanceId, "heard")).toContain("session.idle");
	});
});
