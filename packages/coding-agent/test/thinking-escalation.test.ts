import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ThinkingLevel } from "@kolisachint/hoocode-agent-core";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import type { ExtensionAPI, ExtensionContext } from "../src/core/extensions/types.js";
import { setupThinkingEscalation } from "../src/extensions/core/thinking-escalation.js";

/**
 * Drives setupThinkingEscalation with a mock ExtensionAPI so we can fire
 * synthetic events deterministically (no model, no latency) and assert the
 * thinking-level state machine.
 */
function makeHarness(initialLevel: ThinkingLevel, cwd: string) {
	const handlers = new Map<string, (event: unknown, ctx: ExtensionContext) => unknown>();
	let level = initialLevel;
	const setCalls: ThinkingLevel[] = [];

	const pi = {
		on: (event: string, handler: (event: unknown, ctx: ExtensionContext) => unknown) => {
			handlers.set(event, handler);
		},
		getThinkingLevel: () => level,
		setThinkingLevel: (l: ThinkingLevel) => {
			level = l;
			setCalls.push(l);
		},
	} as unknown as ExtensionAPI;

	setupThinkingEscalation(pi);

	const ctx = { cwd } as ExtensionContext;
	const fire = (event: string, payload: Record<string, unknown> = {}) => handlers.get(event)?.(payload, ctx);

	return {
		fire,
		setCalls,
		get level() {
			return level;
		},
		toolError: (toolName = "bash") => fire("tool_execution_end", { toolName, result: "boom", isError: true }),
		toolOk: (toolName = "bash") => fire("tool_execution_end", { toolName, result: "ok", isError: false }),
		turnEnd: () => fire("turn_end", {}),
		agentStart: () => fire("agent_start", {}),
	};
}

function writeConfig(cwd: string, escalation: Record<string, unknown> | undefined): void {
	mkdirSync(join(cwd, ".hoocode"), { recursive: true });
	const cfg = escalation === undefined ? {} : { thinking_escalation: escalation };
	writeFileSync(join(cwd, ".hoocode", "hoo-config.json"), JSON.stringify(cfg));
}

describe("setupThinkingEscalation", () => {
	let cwd: string;
	beforeEach(() => {
		cwd = mkdtempSync(join(tmpdir(), "esc-test-"));
	});
	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	test("escalates on tool error, then restores after one turn (cooldown 1)", () => {
		writeConfig(cwd, { enabled: true, on_error: "high", cooldown_turns: 1 });
		const h = makeHarness("off", cwd);

		h.agentStart();
		h.toolError(); // error in turn N
		expect(h.level).toBe("high");

		h.turnEnd(); // turn N's own turn_end — cooldown not consumed yet
		expect(h.level).toBe("high");

		h.turnEnd(); // turn N+1 ran escalated; now restore
		expect(h.level).toBe("off");
		expect(h.setCalls).toEqual(["high", "off"]);
	});

	test("does not escalate on tool success", () => {
		writeConfig(cwd, { enabled: true, on_error: "high" });
		const h = makeHarness("off", cwd);

		h.toolOk();
		h.turnEnd();
		expect(h.level).toBe("off");
		expect(h.setCalls).toEqual([]);
	});

	test("does nothing when explicitly disabled", () => {
		writeConfig(cwd, { enabled: false, on_error: "high" });
		const h = makeHarness("off", cwd);

		h.toolError();
		h.turnEnd();
		h.turnEnd();
		expect(h.level).toBe("off");
		expect(h.setCalls).toEqual([]);
	});

	test("enabled by default when `enabled` is omitted", () => {
		// Project sets the object but omits `enabled`; project wins wholesale, so
		// this is deterministic regardless of any machine-global config.
		writeConfig(cwd, { on_error: "high" });
		const h = makeHarness("off", cwd);

		h.toolError(); // escalates because `enabled` defaults to true
		expect(h.level).toBe("high");
		h.turnEnd();
		h.turnEnd();
		expect(h.level).toBe("off");
	});

	test("cooldown_turns=2 stays escalated for two turns", () => {
		writeConfig(cwd, { enabled: true, on_error: "high", cooldown_turns: 2 });
		const h = makeHarness("low", cwd);

		h.toolError();
		h.turnEnd(); // error turn
		expect(h.level).toBe("high");
		h.turnEnd(); // escalated turn 1 (remaining 2 -> 1)
		expect(h.level).toBe("high");
		h.turnEnd(); // escalated turn 2 (remaining 1 -> 0) -> restore
		expect(h.level).toBe("low");
		expect(h.setCalls).toEqual(["high", "low"]);
	});

	test("tools filter restricts which errors escalate", () => {
		writeConfig(cwd, { enabled: true, on_error: "high", tools: ["bash"] });
		const h = makeHarness("off", cwd);

		h.toolError("read"); // not in filter — ignored
		expect(h.level).toBe("off");
		expect(h.setCalls).toEqual([]);

		h.toolError("bash"); // in filter — escalates
		expect(h.level).toBe("high");
	});

	test("consecutive errors extend the window without losing the original baseline", () => {
		writeConfig(cwd, { enabled: true, on_error: "high", cooldown_turns: 1 });
		const h = makeHarness("off", cwd);

		h.toolError(); // turn N error -> baseline "off" captured
		h.turnEnd(); // turn N end
		h.toolError(); // turn N+1 also errors -> baseline stays "off"
		h.turnEnd(); // turn N+1 end
		h.turnEnd(); // turn N+2 ran escalated -> restore to original baseline
		expect(h.level).toBe("off");
	});

	test("agent_start restores a lingering escalation", () => {
		writeConfig(cwd, { enabled: true, on_error: "high", cooldown_turns: 5 });
		const h = makeHarness("off", cwd);

		h.toolError();
		expect(h.level).toBe("high");
		h.agentStart(); // new user prompt — clean slate
		expect(h.level).toBe("off");
	});
});
