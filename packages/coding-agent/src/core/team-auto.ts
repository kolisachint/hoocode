/**
 * `--team auto`: discover a team config, spawn a local hooteams server as a
 * child process, and hand back its URL so the rest of the pipeline behaves
 * exactly as if `--team http://localhost:<port>` had been passed.
 *
 * hooteams is intentionally not bundled — the launcher is resolved from PATH
 * (`hooteams`, falling back to `bunx hooteams`) and missing pieces fail with
 * a clear, actionable error. The child is reaped on hoocode exit, clean or
 * signalled, via a process "exit" hook (the interactive shutdown path calls
 * process.exit directly, so an async cleanup would never run).
 */

import { type ChildProcess, spawn } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { createServer } from "node:net";
import path from "node:path";

/** Config locations probed at each directory level, in priority order. */
export const TEAM_CONFIG_CANDIDATES = [path.join(".agents", "teams", "default.json"), "hooteams.config.json"];

/**
 * Walk up from startDir to the filesystem root, returning the first config
 * found. Both candidates are probed per level (.agents/teams/default.json
 * wins over hooteams.config.json in the same directory).
 */
export function findTeamConfig(startDir: string): string | undefined {
	let dir = path.resolve(startDir);
	while (true) {
		for (const candidate of TEAM_CONFIG_CANDIDATES) {
			const candidatePath = path.join(dir, candidate);
			if (existsSync(candidatePath)) return candidatePath;
		}
		const parent = path.dirname(dir);
		if (parent === dir) return undefined;
		dir = parent;
	}
}

/** Ask the OS for a free port by binding port 0 and reading the assignment. */
export function findFreePort(): Promise<number> {
	return new Promise((resolve, reject) => {
		const server = createServer();
		server.once("error", reject);
		server.listen(0, "127.0.0.1", () => {
			const address = server.address();
			if (address === null || typeof address === "string") {
				server.close(() => reject(new Error("could not determine a free port")));
				return;
			}
			const { port } = address;
			server.close(() => resolve(port));
		});
	});
}

function isExecutableOnPath(name: string, env: NodeJS.ProcessEnv): boolean {
	const pathVar = env.PATH ?? "";
	const extensions = process.platform === "win32" ? (env.PATHEXT ?? ".EXE;.CMD;.BAT;.COM").split(";") : [""];
	for (const dir of pathVar.split(path.delimiter)) {
		if (!dir) continue;
		for (const extension of extensions) {
			try {
				accessSync(path.join(dir, name + extension.toLowerCase()), constants.X_OK);
				return true;
			} catch {
				// keep probing
			}
		}
	}
	return false;
}

/** How to launch hooteams: directly from PATH, or through bunx. */
export function resolveHooteamsLauncher(
	env: NodeJS.ProcessEnv = process.env,
): { command: string; prefixArgs: string[] } | undefined {
	if (isExecutableOnPath("hooteams", env)) return { command: "hooteams", prefixArgs: [] };
	if (isExecutableOnPath("bunx", env)) return { command: "bunx", prefixArgs: ["hooteams"] };
	return undefined;
}

export interface AutoTeam {
	/** Base URL of the spawned hooteams server. */
	url: string;
	/** Graceful shutdown: POST /stop, then kill the child's process group. */
	stop(): Promise<void>;
}

export interface AutoTeamOptions {
	/** Startup progress sink (pre-TUI, so console is fine). */
	log?: (message: string) => void;
	/** How long to wait for GET /health (default 15s). */
	healthTimeoutMs?: number;
	env?: NodeJS.ProcessEnv;
}

function killChild(child: ChildProcess): void {
	if (child.pid === undefined || child.exitCode !== null || child.signalCode !== null) return;
	try {
		// POSIX: the child leads its own process group (detached), so a negative
		// pid reaches hooteams even when launched through a bunx wrapper.
		if (process.platform !== "win32") process.kill(-child.pid, "SIGTERM");
		else child.kill();
	} catch {
		// Already gone.
	}
}

async function waitForHealth(url: string, child: ChildProcess, timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (child.exitCode !== null || child.signalCode !== null) {
			throw new Error(`--team auto: hooteams exited (code ${child.exitCode ?? "signal"}) before becoming healthy`);
		}
		try {
			const response = await fetch(`${url}/health`, { signal: AbortSignal.timeout(1000) });
			if (response.ok) {
				const body = (await response.json()) as { ok?: boolean };
				if (body.ok === true) return;
			}
		} catch {
			// Not up yet; keep polling.
		}
		await new Promise((resolve) => setTimeout(resolve, 150));
	}
	throw new Error(`--team auto: hooteams did not report healthy at ${url}/health within ${timeoutMs}ms`);
}

/**
 * Resolve the config, spawn hooteams on a free port, and wait for /health.
 * Throws (with a message ready for the terminal) when no config is found, no
 * launcher resolves, or the server never becomes healthy.
 */
export async function startAutoTeam(cwd: string, options: AutoTeamOptions = {}): Promise<AutoTeam> {
	const env = options.env ?? process.env;
	const config = findTeamConfig(cwd);
	if (!config) {
		throw new Error(
			`--team auto: no team config found. Looked for ${TEAM_CONFIG_CANDIDATES.join(" or ")} in ${cwd} and every parent directory.`,
		);
	}
	const launcher = resolveHooteamsLauncher(env);
	if (!launcher) {
		throw new Error(
			"--team auto: hooteams is not on PATH and bunx is unavailable. Install hooteams (or bun) or pass --team <url> to use a running server.",
		);
	}

	const port = await findFreePort();
	const url = `http://localhost:${port}`;
	options.log?.(`Starting hooteams (config ${config}) on port ${port}…`);

	const child = spawn(
		launcher.command,
		[...launcher.prefixArgs, "start", "--config", config, "--port", String(port)],
		{
			stdio: "ignore",
			env,
			detached: process.platform !== "win32",
		},
	);
	child.unref();
	const reapOnExit = () => killChild(child);
	process.on("exit", reapOnExit);

	try {
		await waitForHealth(url, child, options.healthTimeoutMs ?? 15000);
	} catch (error) {
		process.off("exit", reapOnExit);
		killChild(child);
		throw error;
	}

	return {
		url,
		async stop() {
			process.off("exit", reapOnExit);
			try {
				await fetch(`${url}/stop`, { method: "POST", signal: AbortSignal.timeout(2000) });
			} catch {
				// Graceful stop is best-effort; the kill below is the guarantee.
			}
			killChild(child);
		},
	};
}
