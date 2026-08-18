/**
 * Regression tests for the duplicated `Agent [explore]` row.
 *
 * A dispatch used to `console.error` its `[DISPATCH]` line straight to the
 * terminal. The interactive TUI is a differential renderer that repositions by
 * relative row deltas from its own bookkeeping, so a write it did not make
 * scrolls the screen underneath it and every later partial repaint lands one row
 * off — painting a second `Agent [explore]` row for a single dispatch, plus
 * fragments of the log line inside the transcript.
 *
 * The end-to-end case below drives the real TUI against a real terminal emulator
 * with `process.stderr` wired into it, exactly as a tty would be, and dispatches
 * through a real SubagentPool. It fails (two rows) without the agent-log sink.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Component, TUI } from "@kolisachint/hoocode-tui";
import { afterEach, describe, expect, it } from "vitest";
import { VirtualTerminal } from "../../tui/test/virtual-terminal.js";
import { agentLog, isTerminalOwnedByTui, setTerminalOwnedByTui } from "../src/core/agent-log.js";
import { SubagentPool } from "../src/core/subagent-pool.js";

afterEach(() => {
	setTerminalOwnedByTui(false);
});

/** Capture everything written to fd 2 while `run` executes. */
async function captureStderr(run: () => void | Promise<void>, sink?: (chunk: string) => void): Promise<string> {
	const chunks: string[] = [];
	const realWrite = process.stderr.write.bind(process.stderr);
	process.stderr.write = ((chunk: unknown) => {
		const text = typeof chunk === "string" ? chunk : String(chunk);
		chunks.push(text);
		sink?.(text);
		return true;
	}) as typeof process.stderr.write;
	try {
		await run();
	} finally {
		process.stderr.write = realWrite;
	}
	return chunks.join("");
}

/** A pool that spawns a trivial child, so a dispatch is real but cheap. */
function makePool(): { pool: SubagentPool; cwd: string } {
	const cwd = mkdtempSync(join(tmpdir(), "dispatch-tui-"));
	return { pool: new SubagentPool({ executable: "/bin/true", cwd, maxConcurrency: 1 }), cwd };
}

class Lines implements Component {
	lines: string[] = [];
	render(_width: number): string[] {
		return this.lines;
	}
	invalidate(): void {}
}

describe("agentLog", () => {
	it("writes to stderr when no TUI owns the terminal", async () => {
		const output = await captureStderr(() => agentLog("[DISPATCH] agent=explore"));
		expect(output).toContain("[DISPATCH] agent=explore");
	});

	it("stays silent while a TUI owns the terminal", async () => {
		setTerminalOwnedByTui(true);
		const output = await captureStderr(() => agentLog("[DISPATCH] agent=explore"));
		expect(output).toBe("");
		expect(isTerminalOwnedByTui()).toBe(true);
	});
});

describe("SubagentPool dispatch logging", () => {
	it("logs the dispatch line in non-interactive modes (--print, rpc, CI)", async () => {
		const { pool, cwd } = makePool();
		const output = await captureStderr(async () => {
			pool.dispatchDetached("find the bug", { forceAgent: "explore" });
			await new Promise((r) => setTimeout(r, 200));
		});
		pool.dispose();
		rmSync(cwd, { recursive: true, force: true });

		expect(output).toContain("[DISPATCH] agent=explore");
		expect(output).toContain("depth=1");
	});

	it("writes nothing to the terminal while the interactive TUI owns it", async () => {
		setTerminalOwnedByTui(true);
		const { pool, cwd } = makePool();
		const output = await captureStderr(async () => {
			pool.dispatchDetached("find the bug", { forceAgent: "explore" });
			await new Promise((r) => setTimeout(r, 200));
		});
		pool.dispose();
		rmSync(cwd, { recursive: true, force: true });

		expect(output).toBe("");
	});
});

/**
 * Replays interactive mode's frame order around a dispatch, on a real emulated
 * terminal, with fd 2 wired into that same terminal the way a tty is.
 */
async function renderDispatchTurn(): Promise<string[]> {
	const terminal = new VirtualTerminal(100, 24);
	const tui = new TUI(terminal);

	const transcript = ["> find the render bug", "", "I'll delegate the search.", ""];
	const chat = new Lines();
	const panel = new Lines();
	const editor = new Lines();
	tui.addChild(chat);
	tui.addChild(panel);
	tui.addChild(editor);

	chat.lines = [...transcript];
	editor.lines = ["", "> "];
	tui.start();
	await terminal.waitForRender();

	// message_update: the tool call streams in and renderCall paints the row.
	chat.lines = [...transcript, "○ Agent [explore]", "  Search for the duplicate row", ""];
	panel.lines = ["▎ subagents 0/1", "▎ ◐ ◇ [explore] Search for the duplicate row      0s"];
	tui.requestRender();
	await terminal.waitForRender();

	const { pool, cwd } = makePool();
	await captureStderr(
		async () => {
			// tool_execution_start -> subagent tool -> pool.dispatch -> beginDispatch.
			pool.dispatchDetached("find the duplicate row bug", { forceAgent: "explore" });
			await new Promise((r) => setTimeout(r, 50));
		},
		// Anything reaching fd 2 lands on the emulated screen, as on a real tty.
		(chunk) => terminal.write(chunk),
	);

	// markExecutionStarted(): the tool row repaints with a filled status dot.
	chat.lines = [...transcript, "● Agent [explore]", "  Search for the duplicate row", ""];
	tui.requestRender();
	await terminal.waitForRender();

	// The task panel's run clock ticks while the subagent works.
	for (let s = 1; s <= 3; s++) {
		panel.lines = ["▎ subagents 0/1", `▎ ◐ ◇ [explore] Search for the duplicate row      ${s}s`];
		tui.requestRender();
		await terminal.waitForRender();
	}

	const screen = terminal.getScrollBuffer().filter((line) => line.trim().length > 0);
	tui.stop();
	pool.dispose();
	rmSync(cwd, { recursive: true, force: true });
	return screen;
}

describe("dispatch does not corrupt the TUI frame", () => {
	it("paints exactly one Agent row for one dispatched subagent", async () => {
		setTerminalOwnedByTui(true);
		const screen = await renderDispatchTurn();

		const agentRows = screen.filter((line) => line.includes("Agent [explore]"));
		expect(agentRows).toHaveLength(1);
		// No fragment of the dispatch line may reach the transcript.
		expect(screen.some((line) => line.includes("[DISPATCH]"))).toBe(false);
		expect(screen.some((line) => line.includes("task_id=dispatch-"))).toBe(false);
	});

	it("would duplicate the row if the dispatch wrote to the terminal (guards the fix)", async () => {
		// Same turn with suppression off: reproduces the original bug, so the test
		// above cannot pass for the wrong reason (e.g. the row never rendering).
		setTerminalOwnedByTui(false);
		const screen = await renderDispatchTurn();

		expect(screen.filter((line) => line.includes("Agent [explore]")).length).toBeGreaterThan(1);
	});
});
