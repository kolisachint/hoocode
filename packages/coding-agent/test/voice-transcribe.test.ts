import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { startVoiceTranscribe, type VoiceStatus } from "../src/modes/interactive/voice-transcribe.js";

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
