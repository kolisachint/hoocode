/**
 * Sink for the agent's operational log lines — subagent dispatch, warm-worker
 * fallback, lifeguard kills. These describe what the agent is doing to its own
 * machinery, not what the user asked for.
 *
 * They used to go straight to `console.error`. stdout is reserved for the JSON
 * event stream, so stderr looked free — but when the interactive TUI owns the
 * screen it is not. The TUI is a differential renderer: it tracks the frame it
 * painted (`previousLines`) and repositions by *relative* row deltas from its
 * own bookkeeping (`hardwareCursorRow`). A write it did not make scrolls the
 * terminal underneath it, and every later partial repaint lands one row off —
 * the visible result being duplicated rows (two `Agent [explore]` lines for one
 * dispatch) plus fragments of the log line wedged into the transcript, which
 * survive until something forces a full repaint. The TUI cannot defend itself
 * here; it has no way to learn that another writer touched the terminal. So the
 * writer has to hold back.
 *
 * Hence: silent while a TUI is attached, unchanged everywhere else (`--print`,
 * RPC, CI), where these lines are the only trace of a dispatch and where
 * `docs/routing.md`'s `[DISPATCH] … depth=2 …` check still has to work.
 *
 * Nothing is lost in the interactive case. A dispatch is already on screen as
 * the `Agent [explore]` tool row and its task-panel row, and every field of the
 * `[DISPATCH]` line is persisted to `dispatch-log.json`; warm-worker and
 * inherited-model fallbacks already surface as a `⚠` note on the task row.
 * Set `HOOCODE_DEBUG_AGENT_LOG=1` to tee the suppressed lines to
 * `~/.hoocode/agent/hoocode-debug.log` (mirrors HOOCODE_DEBUG_REDRAW).
 */

import { appendFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** True once an interactive TUI owns the terminal (see InteractiveMode.start/stop). */
let ownedByTui = false;

/**
 * Declare whether a TUI currently owns the terminal. Interactive mode sets this
 * around the UI's lifetime; every other run mode leaves it false.
 */
export function setTerminalOwnedByTui(owned: boolean): void {
	ownedByTui = owned;
}

/** Whether operational logging is currently being withheld from the terminal. */
export function isTerminalOwnedByTui(): boolean {
	return ownedByTui;
}

/**
 * Emit one operational log line, unless a TUI owns the terminal.
 *
 * Callers must not assume this reaches anyone: it is diagnostics, not a user
 * surface. Anything a user needs to see belongs in the transcript or the task
 * panel.
 */
export function agentLog(message: string): void {
	if (ownedByTui) {
		debugTee(message);
		return;
	}
	// Straight to fd 2 rather than console.error: this is explicitly a write to
	// the terminal, and test/extension hosts that swap `console` out from under
	// us would otherwise hide exactly the behavior that matters here.
	process.stderr.write(`${message}\n`);
}

/** Best-effort file tee for suppressed lines, off unless explicitly enabled. */
function debugTee(message: string): void {
	if (process.env.HOOCODE_DEBUG_AGENT_LOG !== "1") return;
	try {
		const agentDir = process.env.HOOCODE_CODING_AGENT_DIR ?? join(homedir(), ".hoocode", "agent");
		appendFileSync(join(agentDir, "hoocode-debug.log"), `[${new Date().toISOString()}] ${message}\n`);
	} catch {
		// Diagnostics only — never let logging break a dispatch.
	}
}
