/**
 * Coverage for the visual tie-in between the chat, task panel, and TaskOutput:
 *
 * - stable per-agent-type identity colors (theme palette + hash), applied to
 *   the Task call line, panel rows/roster, and TaskOutput
 * - header elapsed as a wall-clock span (not a sum that double-counts
 *   concurrent runs) + per-run timers on running rows
 * - one duration format shared by the panel and TaskOutput
 * - TodoWrite ↔ dispatch linkage: runs nest under the in_progress plan item
 *   in the flat lens, and the link is only recorded when unambiguous
 *
 * Everything runs against local fakes — no real APIs.
 */

import { EventEmitter } from "node:events";
import { mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { subagentInbox } from "../../src/core/subagent-inbox.js";
import type { SubagentPool, TaskResult } from "../../src/core/subagent-pool.js";
import { setSubagentPoolForTesting } from "../../src/core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../../src/core/subagent-result.js";
import { taskStore } from "../../src/core/task-store.js";
import { createTaskOutputToolDefinition, createTaskToolDefinition } from "../../src/core/tools/subagent.js";
import { TaskPanelComponent } from "../../src/modes/interactive/components/task-panel.js";
import { AGENT_COLOR_TOKENS, agentColorFor, initTheme, theme } from "../../src/modes/interactive/theme/theme.js";

const cleanups: Array<() => void> = [];

function makeTempDir(): string {
	const dir = join(tmpdir(), `pi-tie-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	mkdirSync(dir, { recursive: true });
	cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
	return dir;
}

const stripAnsi = (s: string): string => s.replace(/\x1b\[[0-9;]*m/g, "");

function fakeSuccess(taskId: string): TaskResult {
	const data: SubagentResultFile = { summary: "done", files_changed: [], confidence: 0.9, status: "complete" };
	return {
		handled_inline: false,
		task_id: taskId,
		agent_type: "explore",
		result: {
			task_id: taskId,
			ok: true,
			stdout: "",
			stderr: "",
			exit_code: 0,
			status: "complete",
			result_data: data as unknown as Record<string, unknown>,
		},
	};
}

/** A fake pool whose dispatches resolve immediately with a canned success. */
function makeInstantPool(): SubagentPool {
	const pool = new EventEmitter() as EventEmitter & {
		dispatch: (prompt: string, options: { taskId?: string }) => Promise<TaskResult>;
	};
	pool.dispatch = async (_prompt, options) => fakeSuccess(options.taskId ?? "t");
	return pool as unknown as SubagentPool;
}

beforeAll(() => {
	initTheme("dark");
});

afterEach(() => {
	vi.useRealTimers();
	setSubagentPoolForTesting(undefined);
	subagentInbox.clear();
	taskStore.clear();
	while (cleanups.length > 0) {
		cleanups.pop()?.();
	}
});

// ---------------------------------------------------------------------------
// Identity colors
// ---------------------------------------------------------------------------

describe("agent identity colors", () => {
	it("hashes an agent type to a stable palette token", () => {
		const color = agentColorFor("explore");
		expect(AGENT_COLOR_TOKENS).toContain(color);
		// Deterministic: same type, same color — across calls and sessions.
		expect(agentColorFor("explore")).toBe(color);
		// Types spread over the palette rather than collapsing onto one token.
		const distinct = new Set(["explore", "plan", "general-purpose", "review", "edit", "doc"].map(agentColorFor));
		expect(distinct.size).toBeGreaterThan(1);
	});

	it("resolves agent palette tokens against the built-in theme without throwing", () => {
		for (const token of AGENT_COLOR_TOKENS) {
			expect(() => theme.fg(token, "x")).not.toThrow();
		}
		// The built-in dark theme defines real palette values (not the accent fallback).
		expect(theme.getFgAnsi("agent1")).not.toBe(theme.getFgAnsi("accent"));
	});

	it("colors the Task call line by agent type", () => {
		const tool = createTaskToolDefinition();
		const component = tool.renderCall?.(
			{ subagent_type: "explore", description: "d", prompt: "p" },
			theme as never,
			{} as never,
		);
		const line = (component as { render(width: number): string[] }).render(120).join("");
		expect(stripAnsi(line)).toContain("Agent [explore]");
		expect(line).toContain(theme.getFgAnsi(agentColorFor("explore")));
	});

	it("colors panel rows and roster names with the same per-type hue", () => {
		const panel = new TaskPanelComponent();
		panel.setView("subagents");
		const run = taskStore.create("scan the repo", { source: "subagent", subagentMode: "explore", agent: "run-1" });
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running" });
		taskStore.update(run.id, { status: "in_progress" });

		const ansi = theme.getFgAnsi(agentColorFor("explore"));
		const row = panel.render(120).find((l) => l.includes("scan the repo")) as string;
		expect(row).toContain(ansi); // tag + glyph carry the identity color
		panel.dispose();
	});

	it("colors TaskOutput's call target and roster lines by agent type", () => {
		const tool = createTaskOutputToolDefinition();
		const ansi = theme.getFgAnsi(agentColorFor("explore"));

		const call = tool.renderCall?.({ task_id: "explore#1" }, theme as never, {} as never);
		expect((call as { render(w: number): string[] }).render(120).join("")).toContain(ansi);

		const roster = [
			"2 background subagents (1 running):",
			"- explore#1  running  34s  · grep",
			"- plan#1  done (uncollected)  1m10s — drafted the plan",
		].join("\n");
		const result = tool.renderResult?.(
			{ content: [{ type: "text", text: roster }], details: { status: "list", ok: true } as never },
			{ expanded: false, isPartial: false },
			theme as never,
			{} as never,
		);
		const rendered = (result as { render(w: number): string[] }).render(200).join("\n");
		expect(rendered).toContain(ansi);
		expect(rendered).toContain(theme.getFgAnsi(agentColorFor("plan")));
		expect(stripAnsi(rendered)).toContain("- explore#1  running  34s  · grep");
	});
});

// ---------------------------------------------------------------------------
// Elapsed time
// ---------------------------------------------------------------------------

describe("elapsed time displays", () => {
	it("header shows the wall-clock span, not the sum of concurrent runs", () => {
		vi.useFakeTimers();
		const panel = new TaskPanelComponent();
		panel.setView("subagents");
		for (const name of ["run a", "run b"] as const) {
			const t = taskStore.create(name, { source: "subagent", subagentMode: "explore" });
			taskStore.update(t.id, {
				status: "in_progress",
				usage: { input: 1000, output: 100, cacheRead: 0, cacheWrite: 0, cost: 0 },
			});
		}

		vi.advanceTimersByTime(30_000);
		const header = stripAnsi(panel.render(120)[0] as string);
		// Two agents running 30s of wall time is 30s, not 1m00s ("double time").
		expect(header).toContain("30s");
		expect(header).not.toContain("1m00s");
		panel.dispose();
	});

	it("each running row carries its own live timer next to the activity", () => {
		vi.useFakeTimers();
		const panel = new TaskPanelComponent();
		panel.setView("subagents");
		const run = taskStore.create("scan the repo", { source: "subagent", subagentMode: "explore", agent: "run-1" });
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running", activity: "grep" });
		taskStore.update(run.id, { status: "in_progress" });

		vi.advanceTimersByTime(34_000);
		const row = stripAnsi(panel.render(120).find((l) => l.includes("scan the repo")) as string);
		expect(row).toContain("⋯ grep · 34s");
		panel.dispose();
	});

	it("TaskOutput reports elapsed in the panel's format (1m34s, not 94s)", async () => {
		vi.useFakeTimers();
		setSubagentPoolForTesting(makeInstantPool());
		subagentInbox.start("t1", "explore#1", "explore");
		vi.advanceTimersByTime(94_000);

		const tool = createTaskOutputToolDefinition();
		const result = await tool.execute("c1", { list: true }, undefined, undefined, { cwd: makeTempDir() } as never);
		const text = (result.content[0] as { text: string }).text;
		expect(text).toContain("1m34s");
		expect(text).not.toContain("94s");
	});
});

// ---------------------------------------------------------------------------
// TodoWrite ↔ dispatch linkage
// ---------------------------------------------------------------------------

describe("todo ↔ subagent linkage", () => {
	it("links a dispatch to the single in_progress plan item", async () => {
		setSubagentPoolForTesting(makeInstantPool());
		const ctx = { cwd: makeTempDir(), hasUI: true } as never;
		const todo = taskStore.create("Wire the parser");
		taskStore.update(todo.id, { status: "in_progress" });

		const tool = createTaskToolDefinition();
		await tool.execute(
			"c1",
			{ description: "scan repo", prompt: "scan the repo", subagent_type: "explore" },
			undefined,
			undefined,
			ctx,
		);

		const run = taskStore.list().find((t) => t.source === "subagent");
		expect(run?.linkedTaskId).toBe(todo.id);
	});

	it("records no link when zero or several plan items are in_progress", async () => {
		setSubagentPoolForTesting(makeInstantPool());
		const ctx = { cwd: makeTempDir(), hasUI: true } as never;
		const a = taskStore.create("Step A");
		const b = taskStore.create("Step B");
		taskStore.update(a.id, { status: "in_progress" });
		taskStore.update(b.id, { status: "in_progress" });

		const tool = createTaskToolDefinition();
		await tool.execute(
			"c1",
			{ description: "scan repo", prompt: "scan the repo", subagent_type: "explore" },
			undefined,
			undefined,
			ctx,
		);

		const run = taskStore.list().find((t) => t.source === "subagent");
		expect(run?.linkedTaskId).toBeUndefined();
	});

	it("nests a linked run under its plan item in the flat lens (and counts it)", () => {
		const panel = new TaskPanelComponent();
		const todo = taskStore.create("Wire the parser");
		taskStore.update(todo.id, { status: "in_progress" });
		const other = taskStore.create("Write docs"); // stays pending
		taskStore.update(other.id, { status: "pending" });
		const run = taskStore.create("scan the repo", {
			source: "subagent",
			subagentMode: "explore",
			agent: "run-1",
			linkedTaskId: todo.id,
		});
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running", activity: "grep" });
		taskStore.update(run.id, { status: "in_progress" });
		// An unlinked run must stay out of the flat lens entirely.
		const unlinked = taskStore.create("free-floating run", { source: "subagent", subagentMode: "plan" });
		taskStore.update(unlinked.id, { status: "in_progress" });

		panel.setView("flat");
		const lines = panel.render(120).map(stripAnsi);
		const todoIdx = lines.findIndex((l) => l.includes("Wire the parser"));
		const runIdx = lines.findIndex((l) => l.includes("scan the repo"));
		expect(todoIdx).toBeGreaterThan(0);
		expect(runIdx).toBe(todoIdx + 1); // directly under its plan item
		expect(lines[runIdx]).toContain("└─"); // drawn as a nested child
		expect(lines[runIdx]).toContain("[explore]");
		expect(lines[runIdx]).toContain("⋯ grep"); // live activity flows through
		expect(lines.some((l) => l.includes("free-floating run"))).toBe(false);
		// Header counts what the lens shows: todo + linked run + pending todo = 0/3.
		expect(lines[0]).toContain("0/3");

		// The same run still appears in the subagents lens (both views stay whole).
		panel.setView("subagents");
		const sa = panel.render(120).map(stripAnsi);
		expect(sa.some((l) => l.includes("scan the repo"))).toBe(true);
		expect(sa.some((l) => l.includes("free-floating run"))).toBe(true);
		panel.dispose();
	});

	it("drops a dangling link (todo replaced) from the flat lens without losing the run", () => {
		const panel = new TaskPanelComponent();
		const todo = taskStore.create("Old plan item");
		taskStore.update(todo.id, { status: "in_progress" });
		const survivor = taskStore.create("Still on the plan"); // keeps the flat lens non-empty
		taskStore.update(survivor.id, { status: "in_progress" });
		const run = taskStore.create("scan the repo", {
			source: "subagent",
			subagentMode: "explore",
			linkedTaskId: todo.id,
		});
		taskStore.update(run.id, { status: "in_progress" });
		taskStore.remove(todo.id); // TodoWrite replaced the list

		panel.setView("flat");
		const flat = panel.render(120).map(stripAnsi);
		expect(flat.some((l) => l.includes("Still on the plan"))).toBe(true);
		expect(flat.some((l) => l.includes("scan the repo"))).toBe(false);

		panel.setView("subagents");
		const sa = panel.render(120).map(stripAnsi);
		expect(sa.some((l) => l.includes("scan the repo"))).toBe(true);
		panel.dispose();
	});
});
