// Minimal stdio MCP server used by mcp-tools tests: answers initialize,
// tools/list (one "echo" tool), and tools/call (echoes its text argument).
import { createInterface } from "node:readline";

const rl = createInterface({ input: process.stdin });

function respond(id, result) {
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
	if (msg.id === undefined) return; // notification
	switch (msg.method) {
		case "initialize":
			respond(msg.id, {
				protocolVersion: "2024-11-05",
				capabilities: { tools: {} },
				serverInfo: { name: "stub", version: "1.0.0" },
			});
			break;
		case "tools/list":
			respond(msg.id, {
				tools: [
					{
						name: "echo",
						description: "Echo the given text back",
						inputSchema: {
							type: "object",
							properties: { text: { type: "string", description: "Text to echo" } },
							required: ["text"],
						},
					},
				],
			});
			break;
		case "tools/call":
			respond(msg.id, {
				content: [{ type: "text", text: `echo: ${msg.params?.arguments?.text ?? ""}` }],
			});
			break;
		default:
			respond(msg.id, {});
	}
});
