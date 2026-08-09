/**
 * G3 — behavioral smoke for a plugin's executable capabilities.
 *
 * G1 and G2 read files. G3 is the first gate that finds out whether the thing
 * actually runs, which is the most common real failure: a plugin that installs
 * cleanly and breaks the session at the next tool call.
 *
 * **What this is and is not.** Hooks run with `cwd`, `HOME`, `TMPDIR` and the
 * plugin data dir redirected into a throwaway directory, and with a hard
 * timeout. That reduces blast radius; it is *not* containment. Without OS-level
 * sandboxing a shell command can still write wherever it likes, and claiming
 * otherwise in a confirmation prompt would be worse than saying nothing — the
 * whole point of showing gate results to a human is that they are true.
 *
 * **Why it runs before the human confirms.** It does execute not-yet-approved
 * code, which is a real cost. Against it: the code was authored in this session
 * from the user's own request rather than fetched from anywhere, G2 has already
 * screened it for destructive shapes, and absent the smoke test the very same
 * command runs moments later anyway — unscreened, unredirected, and in the real
 * working directory. Running it once under redirection to find out whether it
 * even works is the smaller risk. G3 is therefore authored-only: it is never
 * applied to a marketplace plugin, where the code is someone else's and
 * executing it pre-consent would not be defensible.
 */

import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import * as readline from "node:readline";
import type { GateFinding } from "./gates.js";
import { pluginVariables } from "./index.js";
import type { NormalizedPlugin } from "./manifest.js";

/** Per-capability budget. Long enough for a real server to boot, short enough not to stall a turn. */
const SMOKE_TIMEOUT_MS = 10_000;

/** Env override, so tests can exercise the timeout path without waiting for it. */
function smokeTimeoutMs(): number {
	const raw = Number(process.env.HOOCODE_PLUGIN_SMOKE_TIMEOUT_MS);
	return Number.isFinite(raw) && raw > 0 ? raw : SMOKE_TIMEOUT_MS;
}

/** Synthetic payloads, one per hook event, matching what the bridge really sends. */
const SYNTHETIC_PAYLOADS: Record<string, unknown> = {
	PreToolUse: { hook_event_name: "PreToolUse", tool_name: "read", tool_input: { file_path: "smoke.txt" } },
	PostToolUse: { hook_event_name: "PostToolUse", tool_name: "read", tool_input: {}, tool_response: "" },
	UserPromptSubmit: { hook_event_name: "UserPromptSubmit", prompt: "smoke test" },
	SessionStart: { hook_event_name: "SessionStart", source: "startup" },
	Stop: { hook_event_name: "Stop" },
};

export interface SmokeOptions {
	/** Skip the whole gate (no UI to report into, or an explicit opt-out). */
	skip?: boolean;
}

/** Environment for a smoke run: the plugin's own variables, redirected at a scratch dir. */
function smokeEnv(plugin: NormalizedPlugin, sandbox: string): NodeJS.ProcessEnv {
	return {
		...process.env,
		...pluginVariables(plugin.root, path.join(sandbox, "data")),
		HOME: sandbox,
		TMPDIR: sandbox,
	};
}

function runOnce(
	command: string,
	payload: unknown,
	cwd: string,
	env: NodeJS.ProcessEnv,
): Promise<{ code: number | null; timedOut: boolean; output: string; spawnError?: string }> {
	return new Promise((resolve) => {
		let settled = false;
		let output = "";
		const child = spawn(command, { shell: true, cwd, env });
		const timer = setTimeout(() => {
			if (settled) return;
			settled = true;
			child.kill("SIGKILL");
			resolve({ code: null, timedOut: true, output });
		}, smokeTimeoutMs());
		timer.unref?.();

		child.stdout?.on("data", (d) => {
			output += d.toString().slice(0, 2000);
		});
		child.stderr?.on("data", (d) => {
			output += d.toString().slice(0, 2000);
		});
		child.on("error", (e) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			resolve({ code: null, timedOut: false, output, spawnError: String(e) });
		});
		child.on("close", (code) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			resolve({ code, timedOut: false, output });
		});

		// A command that ignores stdin (`exit 0`) closes the pipe before we write,
		// and the resulting EPIPE surfaces asynchronously — a try/catch around the
		// write does not see it. Swallow it on the stream instead; not reading the
		// payload is a legitimate thing for a hook to do.
		child.stdin?.on("error", () => {});
		child.stdin?.end(`${JSON.stringify(payload)}\n`);
	});
}

/**
 * Complete an MCP handshake against a candidate server, then kill it.
 *
 * Hand-rolled rather than reusing `connectMcpServer`: that registers the
 * connection in a module-level map and terminates any existing entry with the
 * same name, so smoke-testing a draft server would tear down a live one the
 * session is using.
 */
function probeMcpServer(
	command: string,
	args: string[],
	env: NodeJS.ProcessEnv,
	cwd: string,
): Promise<{ ok: boolean; detail: string; startFailed?: boolean }> {
	return new Promise((resolve) => {
		let settled = false;
		let stderr = "";
		const child = spawn(command, args, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
		const finish = (ok: boolean, detail: string, startFailed = false) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			child.kill("SIGKILL");
			resolve({ ok, detail, startFailed });
		};
		const timer = setTimeout(() => finish(false, "no response to initialize in time"), smokeTimeoutMs());
		timer.unref?.();

		child.on("error", (e) => finish(false, `could not start: ${e}`, true));
		child.on("close", (code) =>
			finish(false, `exited (${code}) before completing the handshake${stderr ? `: ${stderr.slice(0, 300)}` : ""}`),
		);
		child.stderr?.on("data", (d) => {
			stderr += d.toString().slice(0, 1000);
		});

		child.stdin?.on("error", () => {});
		const rl = readline.createInterface({ input: child.stdout! });
		rl.on("line", (line) => {
			let msg: { id?: number; result?: unknown; error?: { message?: string } };
			try {
				msg = JSON.parse(line);
			} catch {
				return; // servers sometimes log plain text on stdout
			}
			if (msg.error) return finish(false, `server error: ${msg.error.message ?? "unknown"}`);
			if (msg.id === 1) {
				// Per the spec the client acknowledges initialize before anything else;
				// strict servers refuse tools/list without it.
				child.stdin?.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
				child.stdin?.write(`${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} })}\n`);
				return;
			}
			if (msg.id === 2) {
				const tools = (msg.result as { tools?: unknown[] } | undefined)?.tools ?? [];
				finish(true, `handshake ok, ${tools.length} tool(s)`);
			}
		});

		child.stdin?.write(
			`${JSON.stringify({
				jsonrpc: "2.0",
				id: 1,
				method: "initialize",
				params: {
					protocolVersion: "2024-11-05",
					capabilities: { tools: {} },
					clientInfo: { name: "hoocode-smoke", version: "1.0.0" },
				},
			})}\n`,
		);
	});
}

/** Run G3 over a plugin's executable capabilities. Returns findings; empty means nothing to test. */
export async function runSmokeGate(plugin: NormalizedPlugin, opts: SmokeOptions = {}): Promise<GateFinding[]> {
	if (opts.skip) return [];
	const findings: GateFinding[] = [];
	const hasExecutable = !!plugin.hooks || !!plugin.mcpServers;
	if (!hasExecutable) return findings;

	const sandbox = mkdtempSync(path.join(tmpdir(), "hoo-smoke-"));
	const env = smokeEnv(plugin, sandbox);
	try {
		for (const [event, groups] of Object.entries(plugin.hooks ?? {})) {
			const payload = SYNTHETIC_PAYLOADS[event] ?? { hook_event_name: event };
			for (const group of groups) {
				for (const cmd of group.hooks) {
					const res = await runOnce(cmd.command, payload, sandbox, env);
					const label = `hook ${event}`;
					if (res.spawnError) {
						// Consistent with G2: a missing binary is a documented-prerequisite
						// problem, not a broken plugin. Erroring here would contradict the
						// warning G2 already issues for the very same condition.
						findings.push({
							gate: "G3",
							severity: "warning",
							message: `${label} could not start: ${res.spawnError}`,
						});
					} else if (res.timedOut) {
						findings.push({
							gate: "G3",
							severity: "error",
							message: `${label} did not finish in time — it would stall every matching tool call.`,
						});
					} else if (res.code === 0 || (event === "PreToolUse" && res.code === 2)) {
						// Exit 2 from PreToolUse is the documented "block" decision, not a
						// failure: the hook ran and made a call.
						findings.push({ gate: "G3", severity: "info", message: `${label} ran cleanly (exit ${res.code}).` });
					} else {
						// A non-zero exit may just mean the synthetic payload is not one this
						// hook handles, so it is reported rather than fatal.
						findings.push({
							gate: "G3",
							severity: "warning",
							message: `${label} exited ${res.code} on a synthetic ${event} payload${res.output.trim() ? `: ${res.output.trim().slice(0, 200)}` : ""}`,
						});
					}
				}
			}
		}

		for (const [name, raw] of Object.entries(plugin.mcpServers ?? {})) {
			const cfg = raw as { command?: unknown; args?: unknown; env?: unknown };
			if (typeof cfg.command !== "string") continue; // remote transports are not spawned
			const res = await probeMcpServer(
				cfg.command,
				Array.isArray(cfg.args) ? cfg.args.map(String) : [],
				{ ...env, ...(cfg.env && typeof cfg.env === "object" ? (cfg.env as NodeJS.ProcessEnv) : {}) },
				plugin.root,
			);
			findings.push({
				gate: "G3",
				// A server that never starts is the missing-binary case again — warn.
				// One that starts and then fails the handshake is genuinely broken and
				// would take the session down at connect time.
				severity: res.ok ? "info" : res.startFailed ? "warning" : "error",
				message: `mcp server "${name}": ${res.detail}`,
			});
		}
	} finally {
		rmSync(sandbox, { recursive: true, force: true });
	}

	return findings;
}
