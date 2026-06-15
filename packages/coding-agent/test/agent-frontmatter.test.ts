import { describe, expect, test } from "vitest";
import {
	CLAUDE_TOOL_ALIASES,
	HOOCODE_TOOL_NAMES,
	MODEL_INHERIT,
	normalizeModel,
	normalizeTools,
	parseAgentDefinition,
} from "../src/core/agent-frontmatter.js";

describe("normalizeTools (D7 Claude Code shim)", () => {
	test("maps Claude tool names (case-insensitive) to hoocode tools", () => {
		const { tools, diagnostics } = normalizeTools("Read, Grep, Glob, Bash");
		expect(tools).toEqual(["read", "grep", "find", "bash"]);
		expect(diagnostics).toHaveLength(0);
	});

	test("accepts a YAML list but emits a format warning", () => {
		const { tools, diagnostics } = normalizeTools(["Read", "LS"]);
		expect(tools).toEqual(["read", "ls"]);
		expect(diagnostics).toHaveLength(1);
		expect(diagnostics[0]!.type).toBe("warning");
		expect(diagnostics[0]!.message).toMatch(/comma-separated string/);
	});

	test("drops unknown tools with a diagnostic", () => {
		const { tools, diagnostics } = normalizeTools("Read, WebFetch, Task, MultiEdit");
		expect(tools).toEqual(["read"]);
		expect(diagnostics).toHaveLength(3);
		expect(diagnostics.every((d) => d.type === "warning")).toBe(true);
		expect(diagnostics.some((d) => d.message.includes("WebFetch"))).toBe(true);
	});

	test("dedupes resolved tools (Glob and find both map to find)", () => {
		const { tools } = normalizeTools("Glob, find");
		expect(tools).toEqual(["find"]);
	});

	test("alias map only targets known hoocode tools", () => {
		for (const target of Object.values(CLAUDE_TOOL_ALIASES)) {
			expect(HOOCODE_TOOL_NAMES).toContain(target);
		}
	});
});

describe("normalizeModel", () => {
	test("preserves the inherit sentinel", () => {
		expect(normalizeModel("inherit")).toBe(MODEL_INHERIT);
	});

	test("passes through aliases and trims", () => {
		expect(normalizeModel("  sonnet ")).toBe("sonnet");
	});

	test("returns undefined for empty or missing", () => {
		expect(normalizeModel("")).toBeUndefined();
		expect(normalizeModel(undefined)).toBeUndefined();
	});
});

describe("parseAgentDefinition", () => {
	const validClaudeAgent = `---
name: explorer
description: Use this agent to explore the codebase read-only.
tools: Read, Grep, Glob, Bash
model: sonnet
---
You are a read-only explorer.`;

	test("parses a Claude Code style agent natively", () => {
		const { agent, diagnostics } = parseAgentDefinition(validClaudeAgent, { source: "claude-project" });
		expect(diagnostics).toHaveLength(0);
		expect(agent).not.toBeNull();
		expect(agent?.name).toBe("explorer");
		expect(agent?.tools).toEqual(["read", "grep", "find", "bash"]);
		expect(agent?.model).toBe("sonnet");
		expect(agent?.prompt).toBe("You are a read-only explorer.");
		expect(agent?.source).toBe("claude-project");
	});

	test("omitted tools means inherit all (undefined)", () => {
		const raw = `---
name: agent-a
description: An agent that inherits all parent tools.
---
body`;
		const { agent } = parseAgentDefinition(raw, { source: "project" });
		expect(agent?.tools).toBeUndefined();
	});

	test("falls back to fallbackName when name is omitted", () => {
		const raw = `---
description: Description long enough to be valid.
---
body`;
		const { agent } = parseAgentDefinition(raw, { source: "builtin", fallbackName: "explore" });
		expect(agent?.name).toBe("explore");
	});

	test("returns null when description is missing", () => {
		const raw = `---
name: no-desc
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent).toBeNull();
		expect(diagnostics.some((d) => d.message.includes("description is required"))).toBe(true);
	});

	test("returns null for an invalid name", () => {
		const raw = `---
name: Bad_Name
description: Description long enough to be valid.
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent).toBeNull();
		expect(diagnostics.some((d) => d.message.includes("invalid characters"))).toBe(true);
	});

	test("captures the hoocode maxTurns extension", () => {
		const raw = `---
name: capped
description: An agent with a turn cap.
maxTurns: 12
---
body`;
		const { agent } = parseAgentDefinition(raw, { source: "project" });
		expect(agent?.maxTurns).toBe(12);
	});

	test("captures the background flag and leaves it undefined when absent", () => {
		const bg = `---
name: watcher
description: A non-blocking background agent.
background: true
---
body`;
		expect(parseAgentDefinition(bg, { source: "project" }).agent?.background).toBe(true);

		const plain = `---
name: plain
description: A normal foreground agent.
---
body`;
		expect(parseAgentDefinition(plain, { source: "project" }).agent?.background).toBeUndefined();
	});

	test("warns when background is not a boolean", () => {
		const raw = `---
name: bad-bg
description: Agent with invalid background.
background: yes
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent?.background).toBeUndefined();
		expect(diagnostics.some((d) => d.message.includes("background must be a boolean"))).toBe(true);
	});

	test("captures the delegate flag and leaves it undefined when absent", () => {
		const orchestrator = `---
name: orchestrator
description: An agent that delegates to other subagents.
delegate: true
---
body`;
		expect(parseAgentDefinition(orchestrator, { source: "project" }).agent?.delegate).toBe(true);

		const plain = `---
name: plain
description: A normal agent.
---
body`;
		expect(parseAgentDefinition(plain, { source: "project" }).agent?.delegate).toBeUndefined();
	});

	test("warns when delegate is not a boolean", () => {
		const raw = `---
name: bad-delegate
description: Agent with invalid delegate.
delegate: sure
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent?.delegate).toBeUndefined();
		expect(diagnostics.some((d) => d.message.includes("delegate must be a boolean"))).toBe(true);
	});

	test("warns on unknown model alias", () => {
		const raw = `---
name: weird-model
description: Agent using a non-standard model name.
model: gpt-4o
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent).not.toBeNull();
		expect(agent?.model).toBe("gpt-4o");
		expect(diagnostics.some((d) => d.message.includes("not a recognized Claude alias"))).toBe(true);
	});

	test("allows full Claude model IDs without warning", () => {
		const raw = `---
name: full-id
description: Agent using a full model ID.
model: claude-sonnet-4-6
---
body`;
		const { diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(diagnostics.every((d) => !d.message.includes("not a recognized Claude alias"))).toBe(true);
	});

	test("warns when tools is a YAML list (prefer comma string)", () => {
		const raw = `---
name: list-tools
description: Agent declaring tools as a YAML list.
tools:
  - read
  - bash
---
body`;
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "project" });
		expect(agent?.tools).toEqual(["read", "bash"]);
		expect(diagnostics.some((d) => d.message.includes("comma-separated string"))).toBe(true);
	});
});
