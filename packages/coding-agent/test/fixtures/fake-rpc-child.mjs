// Minimal fake hoocode RPC child for warm-pool tests. Speaks just enough of the
// RPC protocol (rpc-types) to exercise the WarmSubagentWorker/WarmSubagentPool
// driver without a real model: it answers prompt/new_session/get_last_assistant_text/
// get_session_stats and emits agent_start → turn_end → agent_end on each prompt.
//
// State it tracks lets tests assert reuse and reset:
//   - pid in the answer text proves whether a worker was reused (same process)
//   - generation increments on new_session (reset between tasks)
//   - promptCount increments per prompt
//
// Behavior knobs via argv/env:
//   --fail-prompt        emit turn_end with stopReason "error" (task failure, not infra)
//   FAKE_EXIT_ON_PROMPT  exit(1) on first prompt (simulates a worker crash → infra failure)

import { createInterface } from "node:readline";

const argv = process.argv.slice(2);
const failPrompt = argv.includes("--fail-prompt");
const exitOnPrompt = process.env.FAKE_EXIT_ON_PROMPT === "1";

let generation = 0;
let promptCount = 0;
let lastPrompt = "";

const write = (obj) => process.stdout.write(`${JSON.stringify(obj)}\n`);
const respond = (id, command, data) => write({ id, type: "response", command, success: true, data });

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	const trimmed = line.trim();
	if (!trimmed) return;
	let cmd;
	try {
		cmd = JSON.parse(trimmed);
	} catch {
		return;
	}
	switch (cmd.type) {
		case "prompt": {
			if (exitOnPrompt) process.exit(1);
			promptCount += 1;
			lastPrompt = cmd.message ?? "";
			// Authoritative prompt response (preflight success), then the event stream.
			respond(cmd.id, "prompt");
			write({ type: "agent_start" });
			write({ type: "tool_execution_start", toolName: "grep" });
			write({ type: "tool_execution_end", toolName: "grep" });
			write({
				type: "turn_end",
				message: { usage: { input: 11, output: 7 }, stopReason: failPrompt ? "error" : "stop", errorMessage: failPrompt ? "boom" : undefined },
			});
			write({ type: "agent_end", messages: [] });
			return;
		}
		case "new_session": {
			generation += 1;
			respond(cmd.id, "new_session", { cancelled: false });
			return;
		}
		case "get_last_assistant_text": {
			respond(cmd.id, "get_last_assistant_text", {
				text: `ANSWER pid=${process.pid} gen=${generation} n=${promptCount} for=${lastPrompt}`,
			});
			return;
		}
		case "get_session_stats": {
			respond(cmd.id, "get_session_stats", {
				tokens: { input: 11, output: 7, cacheRead: 3, cacheWrite: 0 },
				cost: 0.002,
			});
			return;
		}
		default:
			// Unknown commands get a generic success so the driver never stalls.
			if (cmd.id) respond(cmd.id, cmd.type ?? "unknown");
	}
});

// Stay alive until stdin closes (worker dispose / SIGTERM).
process.stdin.resume();
