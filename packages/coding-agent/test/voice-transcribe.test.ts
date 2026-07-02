import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	startVoiceTranscribe,
	VoiceDaemon,
	type VoiceDaemonHandlers,
	type VoiceStatus,
} from "../src/modes/interactive/voice/voice-transcribe.js";

/**
 * Write a fake `voicetools` executable (a Node script) that emits the given
 * stdout lines, one per array entry, then exits with `exitCode`.
 */
function writeFakeBin(dir: string, lines: string[], exitCode = 0): string {
	const path = join(dir, "voicetools.mjs");
	const body = `${lines.map((l) => `process.stdout.write(${JSON.stringify(`${l}\n`)});`).join("\n")}
process.exit(${exitCode});
`;
	writeFileSync(path, `#!/usr/bin/env node\n${body}`);
	chmodSync(path, 0o755);
	return path;
}

interface Collected {
	segments: string[];
	statuses: VoiceStatus[];
	errors: string[];
}

function run(bin: string, args: string[] = []): Promise<Collected> {
	const collected: Collected = { segments: [], statuses: [], errors: [] };
	return new Promise((resolve) => {
		let settled = false;
		const settle = () => {
			if (settled) return;
			settled = true;
			// Give the child a tick to fully close before resolving.
			setTimeout(() => resolve(collected), 20);
		};
		// The module spawns `bin transcribe`; we point bin at `node` and rely on
		// NODE_OPTIONS-free invocation by wrapping the script path as the arg.
		startVoiceTranscribe(bin, {
			onSegment: (t) => collected.segments.push(t),
			onStatus: (s) => {
				collected.statuses.push(s);
				if (s === "done") settle();
			},
			onError: (e) => {
				collected.errors.push(e);
				settle();
			},
		});
		void args;
	});
}

describe("voice-transcribe", () => {
	let dir: string;

	beforeEach(() => {
		dir = mkdtempSync(join(tmpdir(), "voice-transcribe-"));
	});

	afterEach(() => {
		rmSync(dir, { recursive: true, force: true });
	});

	it("parses STATUS, SEGMENT, and DONE lines", async () => {
		const script = writeFakeBin(dir, [
			"STATUS recording",
			"STATUS transcribing",
			"SEGMENT hello",
			"SEGMENT world",
			"DONE",
		]);
		// Wrap node as the binary so `bin transcribe` runs the script; the script
		// ignores the extra `transcribe` arg.
		const wrapper = join(dir, "voicetools");
		writeFileSync(wrapper, `#!/bin/sh\nexec node ${JSON.stringify(script)} "$@"\n`);
		chmodSync(wrapper, 0o755);

		const result = await run(wrapper);
		expect(result.statuses).toEqual(["recording", "transcribing", "done"]);
		expect(result.segments).toEqual(["hello", "world"]);
		expect(result.errors).toEqual([]);
	});

	it("surfaces ERROR lines", async () => {
		const script = writeFakeBin(dir, ["STATUS recording", "ERROR no model found"], 1);
		const wrapper = join(dir, "voicetools");
		writeFileSync(wrapper, `#!/bin/sh\nexec node ${JSON.stringify(script)} "$@"\n`);
		chmodSync(wrapper, 0o755);

		const result = await run(wrapper);
		expect(result.statuses).toEqual(["recording"]);
		expect(result.errors).toEqual(["no model found"]);
	});

	it("reports an error when the binary cannot be spawned", async () => {
		const result = await run(join(dir, "does-not-exist"));
		expect(result.errors.length).toBe(1);
		expect(result.errors[0]).toContain("voicetools binary not found");
		expect(result.errors[0]).toContain("VOICETOOLS_BIN");
	});

	it("reports non-zero exit without an explicit ERROR line", async () => {
		const script = writeFakeBin(dir, ["STATUS recording"], 3);
		const wrapper = join(dir, "voicetools");
		writeFileSync(wrapper, `#!/bin/sh\nexec node ${JSON.stringify(script)} "$@"\n`);
		chmodSync(wrapper, 0o755);

		const result = await run(wrapper);
		expect(result.errors.length).toBe(1);
		expect(result.errors[0]).toContain("code 3");
	});
});

/** Write a fake `voicetools` wrapper that execs a Node script for `serve`. */
function writeFakeWrapper(dir: string, script: string): string {
	const wrapper = join(dir, "voicetools");
	writeFileSync(wrapper, `#!/bin/sh\nexec node ${JSON.stringify(script)} "$@"\n`);
	chmodSync(wrapper, 0o755);
	return wrapper;
}

interface DaemonCollected {
	ready: number;
	segments: string[];
	partials: string[];
	finals: string[];
	statuses: VoiceStatus[];
	levels: number[];
	phases: string[];
	errors: string[];
	crashes: string[];
	idle: number;
}

function emptyCollected(): DaemonCollected {
	return {
		ready: 0,
		segments: [],
		partials: [],
		finals: [],
		statuses: [],
		levels: [],
		phases: [],
		errors: [],
		crashes: [],
		idle: 0,
	};
}

function collectingHandlers(collected: DaemonCollected): VoiceDaemonHandlers {
	return {
		onReady: () => {
			collected.ready += 1;
		},
		onSegment: (t) => collected.segments.push(t),
		onPartial: (t) => collected.partials.push(t),
		onFinal: (t) => collected.finals.push(t),
		onStatus: (s) => collected.statuses.push(s),
		onLevel: (rms) => collected.levels.push(rms),
		onPhase: (p) => collected.phases.push(p),
		onError: (e) => collected.errors.push(e),
		onCrash: (m) => collected.crashes.push(m),
		onIdle: () => {
			collected.idle += 1;
		},
	};
}

describe("VoiceDaemon", () => {
	let dir: string;

	beforeEach(() => {
		dir = mkdtempSync(join(tmpdir(), "voice-daemon-"));
	});

	afterEach(() => {
		rmSync(dir, { recursive: true, force: true });
	});

	it("resolves reason: unsupported when the binary doesn't support `serve` (probe fallback)", async () => {
		// Old binaries reject the unrecognized `serve` subcommand and exit
		// immediately without ever printing READY or any stdout line at all.
		const script = join(dir, "old.mjs");
		writeFileSync(script, "#!/usr/bin/env node\nprocess.exit(2);\n");
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected));
		expect(result).toEqual({ ok: false, reason: "unsupported" });
		expect(collected.ready).toBe(0);
		expect(collected.errors).toEqual([]);
	});

	// Reproduces a real behavior observed from the actual voicetools binary:
	// `voicetools serve` with no model installed prints an ERROR line and
	// exits 1, without ever printing READY. This must NOT be misread as an
	// "unsupported binary" (which would silently retry with `transcribe` and
	// just hit the same error with a worse message) — it's a real, actionable
	// error the user needs to see.
	it("resolves reason: error (not unsupported) for a genuine pre-READY ERROR line", async () => {
		const script = join(dir, "no-model.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
process.stdout.write("ERROR no model found for 'parakeet-v3' \\u2014 run: voicetools setup --model parakeet-v3\\n");
process.exit(1);
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected));
		expect(result).toEqual({ ok: false, reason: "error" });
		expect(collected.errors).toEqual([
			"no model found for 'parakeet-v3' — run: voicetools setup --model parakeet-v3",
		]);
	});

	it("loads once, then streams LEVEL/PHASE/SEGMENT/DONE for a capture", async () => {
		// A minimal fake daemon: prints READY immediately, then on receiving
		// START streams a listening -> silence -> segment -> done sequence.
		const script = join(dir, "serve.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { createInterface } from "node:readline";
process.stdout.write("READY\\n");
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	if (line === "START") {
		process.stdout.write("STATUS listening\\n");
		process.stdout.write("LEVEL 0.05\\n");
		process.stdout.write("PHASE silence\\n");
		process.stdout.write("LEVEL 0.001\\n");
		process.stdout.write("SEGMENT hello world\\n");
		process.stdout.write("DONE\\n");
	} else if (line === "SHUTDOWN") {
		process.exit(0);
	}
});
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const doneSeen = new Promise<void>((resolve) => {
			const handlers = collectingHandlers(collected);
			handlers.onStatus = (s) => {
				collected.statuses.push(s);
				if (s === "done") resolve();
			};
			void (async () => {
				const result = await VoiceDaemon.spawn(wrapper, handlers);
				expect(result.ok).toBe(true);
				if (result.ok) result.daemon.startCapture();
			})();
		});

		await doneSeen;
		expect(collected.ready).toBe(1);
		expect(collected.statuses).toEqual(["listening", "done"]);
		expect(collected.levels).toEqual([0.05, 0.001]);
		expect(collected.phases).toEqual(["silence"]);
		expect(collected.segments).toEqual(["hello world"]);
	});

	// Mirrors the real v0.1.4 serve streaming protocol: repeated PARTIAL lines
	// carrying the FULL growing hypothesis (each supersedes the previous, never
	// committed), then a single FINAL with the complete text, then DONE.
	it("streams PARTIAL (growing) then a single FINAL for a v0.1.4 capture", async () => {
		const script = join(dir, "serve-stream.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { createInterface } from "node:readline";
process.stdout.write("READY\\n");
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	if (line === "START") {
		process.stdout.write("STATUS listening\\n");
		process.stdout.write("LEVEL 0.12\\n");
		process.stdout.write("PARTIAL and so\\n");
		process.stdout.write("PARTIAL and so my fellow\\n");
		process.stdout.write("PARTIAL and so my fellow americans\\n");
		process.stdout.write("PHASE silence\\n");
		process.stdout.write("STATUS transcribing\\n");
		process.stdout.write("FINAL And so, my fellow Americans.\\n");
		process.stdout.write("DONE\\n");
	} else if (line === "SHUTDOWN") {
		process.exit(0);
	}
});
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const doneSeen = new Promise<void>((resolve) => {
			const handlers = collectingHandlers(collected);
			handlers.onStatus = (s) => {
				collected.statuses.push(s);
				if (s === "done") resolve();
			};
			void (async () => {
				const result = await VoiceDaemon.spawn(wrapper, handlers);
				expect(result.ok).toBe(true);
				if (result.ok) result.daemon.startCapture();
			})();
		});

		await doneSeen;
		expect(collected.statuses).toEqual(["listening", "transcribing", "done"]);
		// Each PARTIAL is the full text-so-far, not a per-word delta.
		expect(collected.partials).toEqual(["and so", "and so my fellow", "and so my fellow americans"]);
		// Exactly one FINAL, carrying the complete committed utterance.
		expect(collected.finals).toEqual(["And so, my fellow Americans."]);
		// Streaming mode does not use SEGMENT.
		expect(collected.segments).toEqual([]);
		expect(collected.phases).toEqual(["silence"]);
	});

	it("forwards silenceMs to the binary as --silence-ms", async () => {
		const argsPath = join(dir, "args.json");
		const script = join(dir, "args-echo.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { writeFileSync } from "node:fs";
writeFileSync(${JSON.stringify(argsPath)}, JSON.stringify(process.argv.slice(2)));
process.stdout.write("READY\\n");
setTimeout(() => process.exit(0), 20);
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected), { silenceMs: 3000 });
		expect(result.ok).toBe(true);
		if (result.ok) result.daemon.shutdown();

		const args = JSON.parse(readFileSync(argsPath, "utf8")) as string[];
		expect(args).toEqual(["serve", "--silence-ms", "3000"]);
	});

	it("fires onIdle after idleTimeoutMs when no new capture starts", async () => {
		// Fake daemon: prints READY, then immediately responds to START with STATUS+DONE.
		const script = join(dir, "idle-test.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { createInterface } from "node:readline";
process.stdout.write("READY\\n");
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	if (line === "START") {
		process.stdout.write("STATUS listening\\n");
		process.stdout.write("DONE\\n");
	} else if (line === "SHUTDOWN") {
		process.exit(0);
	}
});
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected), { idleTimeoutMs: 100 });
		expect(result.ok).toBe(true);
		if (!result.ok) return;

		result.daemon.startCapture();
		// Wait for the capture to complete (DONE) + idle timer to fire
		await new Promise((r) => setTimeout(r, 200));

		expect(collected.statuses).toContain("listening");
		expect(collected.idle).toBe(1);
	});

	it("does not fire onIdle when a new capture starts before the timeout", async () => {
		const script = join(dir, "idle-cancel.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { createInterface } from "node:readline";
process.stdout.write("READY\\n");
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	if (line === "START") {
		process.stdout.write("STATUS listening\\n");
		process.stdout.write("DONE\\n");
	} else if (line === "SHUTDOWN") {
		process.exit(0);
	}
});
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected), { idleTimeoutMs: 200 });
		expect(result.ok).toBe(true);
		if (!result.ok) return;

		// First capture — starts the idle timer
		result.daemon.startCapture();
		await new Promise((r) => setTimeout(r, 50));
		// Second capture before idle fires — should reset the timer
		result.daemon.startCapture();
		await new Promise((r) => setTimeout(r, 150));

		// onIdle should NOT have fired yet (timer was reset)
		expect(collected.idle).toBe(0);

		result.daemon.shutdown();
	});

	it("does not fire onIdle when idleTimeoutMs is 0 (disabled)", async () => {
		const script = join(dir, "idle-disabled.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
import { createInterface } from "node:readline";
process.stdout.write("READY\\n");
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
	if (line === "START") {
		process.stdout.write("STATUS listening\\n");
		process.stdout.write("DONE\\n");
	} else if (line === "SHUTDOWN") {
		process.exit(0);
	}
});
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected), { idleTimeoutMs: 0 });
		expect(result.ok).toBe(true);
		if (!result.ok) return;

		result.daemon.startCapture();
		await new Promise((r) => setTimeout(r, 150));

		expect(collected.idle).toBe(0);
		result.daemon.shutdown();
	});

	it("reports onCrash when the daemon exits unexpectedly after READY", async () => {
		const script = join(dir, "crashy.mjs");
		writeFileSync(
			script,
			`#!/usr/bin/env node
process.stdout.write("READY\\n");
setTimeout(() => process.exit(1), 20);
`,
		);
		chmodSync(script, 0o755);
		const wrapper = writeFakeWrapper(dir, script);

		const collected: DaemonCollected = emptyCollected();
		const result = await VoiceDaemon.spawn(wrapper, collectingHandlers(collected));
		expect(result.ok).toBe(true);

		await new Promise<void>((resolve) => {
			const check = setInterval(() => {
				if (collected.crashes.length > 0) {
					clearInterval(check);
					resolve();
				}
			}, 10);
		});
		expect(collected.crashes[0]).toContain("code 1");
	});
});
