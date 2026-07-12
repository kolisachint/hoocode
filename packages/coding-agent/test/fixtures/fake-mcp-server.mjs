/**
 * Minimal MCP server over stdio for tests. Speaks just enough JSON-RPC 2.0 for
 * the mcp-loader handshake (initialize → tools/list) plus tools/call for one
 * `echo` tool. Exits when stdin ends so test workers never leave orphans.
 *
 * Usage: node fake-mcp-server.mjs [toolName]   (default tool name: "echo")
 */

import { createInterface } from "node:readline";

const toolName = process.argv[2] ?? "echo";

const rl = createInterface({ input: process.stdin });

function reply(id, result) {
	process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

rl.on("line", (line) => {
	if (!line.trim()) return;
	let msg;
	try {
		msg = JSON.parse(line);
	} catch {
		return;
	}
	if (msg.id === undefined) return; // notification (e.g. notifications/initialized)
	switch (msg.method) {
		case "initialize":
			reply(msg.id, {
				protocolVersion: "2024-11-05",
				capabilities: { tools: {} },
				serverInfo: { name: "fake-mcp-server", version: "1.0.0" },
			});
			break;
		case "tools/list":
			reply(msg.id, {
				tools: [
					{
						name: toolName,
						description: "Echo a message back",
						inputSchema: {
							type: "object",
							properties: { message: { type: "string", description: "Message to echo" } },
							required: ["message"],
						},
					},
				],
			});
			break;
		case "tools/call":
			reply(msg.id, {
				content: [{ type: "text", text: `echo: ${msg.params?.arguments?.message ?? ""}` }],
			});
			break;
		default:
			reply(msg.id, {});
	}
});

rl.on("close", () => process.exit(0));
