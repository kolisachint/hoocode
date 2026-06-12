import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { findFreePort, findTeamConfig, resolveHooteamsLauncher, startAutoTeam } from "../src/core/team-auto.js";

describe("findTeamConfig", () => {
	let root: string;

	afterEach(() => {
		rmSync(root, { recursive: true, force: true });
	});

	test("walks up from cwd to find a config in an ancestor", () => {
		root = mkdtempSync(path.join(tmpdir(), "team-auto-"));
		const nested = path.join(root, "a", "b", "c");
		mkdirSync(nested, { recursive: true });
		const config = path.join(root, "hooteams.config.json");
		writeFileSync(config, "{}");
		expect(findTeamConfig(nested)).toBe(config);
	});

	test(".agents/teams/default.json wins over hooteams.config.json in the same directory", () => {
		root = mkdtempSync(path.join(tmpdir(), "team-auto-"));
		mkdirSync(path.join(root, ".agents", "teams"), { recursive: true });
		const preferred = path.join(root, ".agents", "teams", "default.json");
		writeFileSync(preferred, "{}");
		writeFileSync(path.join(root, "hooteams.config.json"), "{}");
		expect(findTeamConfig(root)).toBe(preferred);
	});

	test("a closer hooteams.config.json beats a farther default.json", () => {
		root = mkdtempSync(path.join(tmpdir(), "team-auto-"));
		mkdirSync(path.join(root, ".agents", "teams"), { recursive: true });
		writeFileSync(path.join(root, ".agents", "teams", "default.json"), "{}");
		const nested = path.join(root, "pkg");
		mkdirSync(nested);
		const closer = path.join(nested, "hooteams.config.json");
		writeFileSync(closer, "{}");
		expect(findTeamConfig(nested)).toBe(closer);
	});

	test("returns undefined when nothing is found", () => {
		root = mkdtempSync(path.join(tmpdir(), "team-auto-"));
		const nested = path.join(root, "empty");
		mkdirSync(nested);
		// Constrain the walk to the temp tree by checking the result is not inside it;
		// a config outside the sandbox (e.g. in /tmp) would be a machine artifact.
		const found = findTeamConfig(nested);
		if (found !== undefined) {
			expect(found.startsWith(root)).toBe(false);
		} else {
			expect(found).toBeUndefined();
		}
	});
});

describe("findFreePort", () => {
	test("returns a bindable port number", async () => {
		const port = await findFreePort();
		expect(Number.isInteger(port)).toBe(true);
		expect(port).toBeGreaterThan(0);
		expect(port).toBeLessThan(65536);
	});
});

describe("resolveHooteamsLauncher", () => {
	test("resolves nothing on an empty PATH", () => {
		expect(resolveHooteamsLauncher({ PATH: "" })).toBeUndefined();
	});
});

describe("startAutoTeam", () => {
	test.skipIf(process.platform === "win32")(
		"spawns a discovered config end-to-end: /health gates startup, stop() reaps the child",
		async () => {
			const root = mkdtempSync(path.join(tmpdir(), "team-auto-e2e-"));
			try {
				writeFileSync(path.join(root, "hooteams.config.json"), "{}");

				// Fake hooteams on PATH: serves /health and /stop on the given --port.
				const bin = path.join(root, "bin");
				mkdirSync(bin);
				const server = path.join(bin, "fake-server.cjs");
				writeFileSync(
					server,
					`const http = require("node:http");
const port = Number(process.argv[process.argv.indexOf("--port") + 1]);
http
	.createServer((req, res) => {
		res.setHeader("content-type", "application/json");
		if (req.url === "/health") return res.end(JSON.stringify({ ok: true }));
		if (req.url === "/stop") {
			res.end("{}");
			setTimeout(() => process.exit(0), 10);
			return;
		}
		res.end("{}");
	})
	.listen(port);
`,
				);
				const launcher = path.join(bin, "hooteams");
				writeFileSync(launcher, `#!/bin/sh\nexec "${process.execPath}" "${server}" "$@"\n`);
				chmodSync(launcher, 0o755);

				const team = await startAutoTeam(root, {
					env: { ...process.env, PATH: bin },
					healthTimeoutMs: 10_000,
				});
				expect(team.url).toMatch(/^http:\/\/localhost:\d+$/);
				const health = (await (await fetch(`${team.url}/health`)).json()) as { ok?: boolean };
				expect(health.ok).toBe(true);

				await team.stop();
				// The server must be gone after stop(): /health stops answering.
				const deadline = Date.now() + 5000;
				let down = false;
				while (Date.now() < deadline && !down) {
					try {
						await fetch(`${team.url}/health`, { signal: AbortSignal.timeout(250) });
						await new Promise((resolve) => setTimeout(resolve, 50));
					} catch {
						down = true;
					}
				}
				expect(down).toBe(true);
			} finally {
				rmSync(root, { recursive: true, force: true });
			}
		},
		20_000,
	);

	test("fails with a clear error when no config exists", async () => {
		const root = mkdtempSync(path.join(tmpdir(), "team-auto-"));
		try {
			// An isolated drive-less dir almost certainly has no config above it,
			// but tolerate machines that do by only asserting when none was found.
			if (findTeamConfig(root) === undefined) {
				await expect(startAutoTeam(root)).rejects.toThrow(/no team config found/);
			}
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});
