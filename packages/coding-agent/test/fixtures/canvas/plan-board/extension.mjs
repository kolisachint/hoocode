/**
 * Fixture canvas extension. Deliberately shaped like the real
 * `pr-artifact-explorer`: it imports only `@github/copilot-sdk/extension` and
 * `node:` builtins, serves its UI from an ephemeral loopback port behind a
 * per-instance capability token, and returns that URL from `open`.
 *
 * It is a real extension, not a mock: the runner tests fork it exactly as they
 * would fork anything from the catalog.
 */

import { createCanvas, CanvasError, joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

const instances = new Map();

const session = await joinSession({
	canvases: [
		createCanvas({
			id: "plan-board",
			displayName: "Plan Board",
			description: "A fixture plan board.",
			inputSchema: { type: "object", properties: { title: { type: "string" } }, additionalProperties: false },
			actions: [
				{
					name: "add_step",
					description: "Append a step to the plan.",
					inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
					handler: (ctx) => {
						const entry = instances.get(ctx.instanceId);
						if (!entry) throw new CanvasError("no_instance", `Instance "${ctx.instanceId}" is not open.`);
						entry.steps.push(ctx.input.text);
						return { steps: entry.steps.length };
					},
				},
				{
					name: "needs_auth",
					description: "Always fails, to exercise CanvasError propagation.",
					handler: () => {
						throw new CanvasError("github_auth_required", "No usable account was found.");
					},
				},
				{
					name: "crash",
					description: "Exits the process, to exercise host recovery from a dead extension.",
					handler: () => {
						process.exit(9);
					},
				},
				{
					name: "leak_stdout",
					description: "Writes a stray line to stdout, to exercise stray reporting.",
					handler: () => {
						console.log("stray diagnostic from the extension");
						return { wrote: true };
					},
				},
			],
			open: async (ctx) => {
				const token = randomBytes(32).toString("base64url");
				const steps = [];
				const server = createServer((req, res) => {
					const url = new URL(req.url ?? "/", "http://127.0.0.1");
					if (url.searchParams.get("token") !== token) {
						res.writeHead(403, { "Content-Type": "text/plain" });
						res.end("forbidden");
						return;
					}
					res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
					res.end(`<!doctype html><title>Plan Board</title><p>${steps.length} steps`);
				});
				await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
				const { port } = server.address();
				instances.set(ctx.instanceId, { server, token, steps });
				await session.log(`opened ${ctx.instanceId}`);
				return {
					url: `http://127.0.0.1:${port}/?token=${token}`,
					title: "Plan Board",
					status: "fixture",
				};
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
