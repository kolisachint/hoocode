/**
 * Hooks bridge — runs Claude Code / native-plugin shell hooks against hoocode events.
 *
 * Claude Code hooks are shell commands wired to named events and matched by tool
 * name. hoocode hooks are TypeScript handlers on the {@link ExtensionEvent} union.
 * This bridge registers handlers that shell out per the hook protocol and translate
 * stdin JSON + exit codes + stdout JSON back into hoocode result objects.
 *
 * Protocol (faithful to Claude Code):
 *  - Input: a JSON object on stdin describing the event.
 *  - Exit 0: success. stdout may carry a JSON decision; for prompt/session events
 *    plain stdout is treated as additional context.
 *  - Exit 2: blocking error. stderr (or JSON `reason`) is the block reason.
 *  - Other non-zero: non-blocking error (logged, not surfaced to the model).
 *  - Optional stdout JSON: `{ decision: "block"|"approve", reason, permissionDecision }`.
 */

import { spawn } from "node:child_process";
import type { ExtensionAPI, ToolCallEvent, ToolResultEvent } from "../types.js";
import type { PluginHookCommand, PluginHookMatcherGroup, PluginHooksConfig } from "./manifest.js";

const DEFAULT_TIMEOUT_MS = 60_000;

interface HookRunResult {
	exitCode: number;
	stdout: string;
	stderr: string;
	json: { decision?: string; reason?: string; permissionDecision?: string; continue?: boolean } | undefined;
}

/** Run one shell hook command, piping `input` as JSON on stdin. */
function runHookCommand(cmd: PluginHookCommand, input: unknown, root: string): Promise<HookRunResult> {
	return new Promise((resolve) => {
		const child = spawn(cmd.command, {
			shell: true,
			env: { ...process.env, CLAUDE_PLUGIN_ROOT: root, AGENTS_PLUGIN_ROOT: root },
		});

		let stdout = "";
		let stderr = "";
		const timer = setTimeout(() => child.kill("SIGTERM"), (cmd.timeout ?? DEFAULT_TIMEOUT_MS / 1000) * 1000);
		timer.unref?.();

		child.stdout.on("data", (d) => {
			stdout += d.toString();
		});
		child.stderr.on("data", (d) => {
			stderr += d.toString();
		});
		child.on("error", () => {
			clearTimeout(timer);
			resolve({ exitCode: 1, stdout, stderr, json: undefined });
		});
		child.on("close", (code) => {
			clearTimeout(timer);
			let json: HookRunResult["json"];
			const trimmed = stdout.trim();
			if (trimmed.startsWith("{")) {
				try {
					json = JSON.parse(trimmed);
				} catch {
					json = undefined;
				}
			}
			resolve({ exitCode: code ?? 0, stdout, stderr, json });
		});

		try {
			child.stdin.write(JSON.stringify(input));
			child.stdin.end();
		} catch {
			/* child may have already exited */
		}
	});
}

/** Empty / "*" matcher matches everything; otherwise treat as an anchored regex on the tool name. */
function matcherMatches(matcher: string | undefined, toolName: string): boolean {
	if (!matcher || matcher === "*") return true;
	try {
		return new RegExp(`^(?:${matcher})$`).test(toolName);
	} catch {
		return matcher === toolName;
	}
}

function groupsForTool(groups: PluginHookMatcherGroup[], toolName: string): PluginHookCommand[] {
	const cmds: PluginHookCommand[] = [];
	for (const g of groups) {
		if (matcherMatches(g.matcher, toolName)) cmds.push(...g.hooks);
	}
	return cmds;
}

function allCommands(groups: PluginHookMatcherGroup[]): PluginHookCommand[] {
	return groups.flatMap((g) => g.hooks);
}

/**
 * Register all hook events for one plugin against the ExtensionAPI.
 * `onError` reports non-blocking failures (kept off the model's path).
 */
export function installPluginHooks(
	pi: ExtensionAPI,
	hooks: PluginHooksConfig,
	root: string,
	onError: (message: string) => void,
): void {
	// ── PreToolUse → tool_call (blocking) ────────────────────────────────────
	const preGroups = hooks.PreToolUse;
	if (preGroups?.length) {
		pi.on("tool_call", async (event: ToolCallEvent) => {
			const cmds = groupsForTool(preGroups, event.toolName);
			for (const cmd of cmds) {
				const res = await runHookCommand(
					cmd,
					{ hook_event_name: "PreToolUse", tool_name: event.toolName, tool_input: event.input },
					root,
				);
				const decision = res.json?.decision ?? res.json?.permissionDecision;
				if (res.exitCode === 2 || decision === "block" || decision === "deny") {
					return { block: true, reason: res.json?.reason || res.stderr.trim() || "Blocked by plugin hook" };
				}
				if (res.exitCode !== 0) onError(`PreToolUse hook failed (${res.exitCode}): ${res.stderr.trim()}`);
			}
		});
	}

	// ── PostToolUse → tool_result (best-effort) ──────────────────────────────
	const postGroups = hooks.PostToolUse;
	if (postGroups?.length) {
		pi.on("tool_result", async (event: ToolResultEvent) => {
			const cmds = groupsForTool(postGroups, event.toolName);
			for (const cmd of cmds) {
				const res = await runHookCommand(
					cmd,
					{
						hook_event_name: "PostToolUse",
						tool_name: event.toolName,
						tool_input: event.input,
						tool_response: event.content,
					},
					root,
				);
				if (res.exitCode === 2 || res.json?.decision === "block") {
					const reason = res.json?.reason || res.stderr.trim() || "Flagged by plugin hook";
					return {
						content: [...event.content, { type: "text" as const, text: `\n[plugin hook] ${reason}` }],
						isError: true,
					};
				}
				if (res.exitCode !== 0) onError(`PostToolUse hook failed (${res.exitCode}): ${res.stderr.trim()}`);
			}
		});
	}

	// ── UserPromptSubmit → before_agent_start (adds context) ─────────────────
	const promptGroups = hooks.UserPromptSubmit;
	if (promptGroups?.length) {
		pi.on("before_agent_start", async (event) => {
			let systemPrompt = event.systemPrompt;
			for (const cmd of allCommands(promptGroups)) {
				const res = await runHookCommand(cmd, { hook_event_name: "UserPromptSubmit", prompt: event.prompt }, root);
				if (res.exitCode !== 0 && res.exitCode !== 2) {
					onError(`UserPromptSubmit hook failed (${res.exitCode}): ${res.stderr.trim()}`);
					continue;
				}
				const extra = res.exitCode === 2 ? res.stderr.trim() : res.json?.reason || res.stdout.trim();
				if (extra) systemPrompt = `${systemPrompt}\n\n<!-- plugin hook -->\n${extra}`;
			}
			return systemPrompt === event.systemPrompt ? undefined : { systemPrompt };
		});
	}

	// ── SessionStart → session_start (side effects) ──────────────────────────
	const sessionGroups = hooks.SessionStart;
	if (sessionGroups?.length) {
		pi.on("session_start", async (event) => {
			for (const cmd of allCommands(sessionGroups)) {
				const res = await runHookCommand(cmd, { hook_event_name: "SessionStart", source: event.reason }, root);
				if (res.exitCode !== 0) onError(`SessionStart hook failed (${res.exitCode}): ${res.stderr.trim()}`);
			}
		});
	}

	// ── Stop → agent_end (side effects) ──────────────────────────────────────
	const stopGroups = hooks.Stop;
	if (stopGroups?.length) {
		pi.on("agent_end", async () => {
			for (const cmd of allCommands(stopGroups)) {
				const res = await runHookCommand(cmd, { hook_event_name: "Stop" }, root);
				if (res.exitCode !== 0) onError(`Stop hook failed (${res.exitCode}): ${res.stderr.trim()}`);
			}
		});
	}
}
