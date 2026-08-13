import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { isEmptyDigest, renderLearnDigest } from "../src/core/learn/digest.js";
import { extractLearnDigest, LEARN_DIGEST_MARKER } from "../src/core/learn/extract.js";
import {
	isBenignFailure,
	isRuleShapedDirective,
	normalizeCommand,
	normalizeDirective,
	normalizeErrorSignature,
} from "../src/core/learn/normalize.js";
import { getLearnStatePath, readLearnState, recordSurfaced, writeLearnState } from "../src/core/learn/state.js";

let tempDir: string;

afterEach(() => {
	if (tempDir) {
		rmSync(tempDir, { recursive: true, force: true });
		tempDir = "";
	}
});

// ── Session fixture builders ────────────────────────────────────────────────

interface Entry {
	type: string;
	id?: string;
	parentId?: string | null;
	timestamp?: string;
	message?: unknown;
}

let nextId = 0;
function entryId(): string {
	nextId++;
	return `e${nextId}`;
}

function userEntry(text: string): Entry {
	return { type: "message", message: { role: "user", content: [{ type: "text", text }], timestamp: 0 } };
}

function toolCallEntry(name: string, args: Record<string, unknown>, callId: string): Entry {
	return {
		type: "message",
		message: { role: "assistant", content: [{ type: "toolCall", id: callId, name, arguments: args }], timestamp: 0 },
	};
}

function toolResultEntry(callId: string, output: string, isError: boolean): Entry {
	return {
		type: "message",
		message: {
			role: "toolResult",
			toolCallId: callId,
			toolName: "bash",
			content: [{ type: "text", text: output }],
			isError,
			timestamp: 0,
		},
	};
}

/** A bash call plus its result, as one sequential pair. */
function bash(command: string, output: string, isError: boolean): Entry[] {
	const callId = `c${++nextId}`;
	return [toolCallEntry("bash", { command }, callId), toolResultEntry(callId, output, isError)];
}

function edit(path: string): Entry[] {
	const callId = `c${++nextId}`;
	return [toolCallEntry("edit", { path }, callId), toolResultEntry(callId, "ok", false)];
}

interface Fixture {
	cwd: string;
	agentDir: string;
	sessionDir: string;
}

function createFixture(): Fixture {
	tempDir = mkdtempSync(join(tmpdir(), "hoo-learn-"));
	const fixture = {
		cwd: join(tempDir, "repo"),
		agentDir: join(tempDir, ".hoocode"),
		sessionDir: join(tempDir, "sessions"),
	};
	for (const dir of Object.values(fixture)) mkdirSync(dir, { recursive: true });
	return fixture;
}

/** Write a session file. `withIds` links entries into a tree; otherwise it is a flat legacy session. */
function writeSession(fixture: Fixture, name: string, entries: Entry[], options: { withIds?: boolean } = {}): void {
	const lines: string[] = [
		JSON.stringify({
			type: "session",
			version: 3,
			id: name,
			timestamp: "2026-08-01T00:00:00.000Z",
			cwd: fixture.cwd,
		}),
	];
	let parentId: string | null = null;
	const withIds = options.withIds ?? false;
	for (const entry of entries) {
		const linked: Entry = withIds
			? { ...entry, id: entry.id ?? entryId(), parentId: entry.parentId ?? parentId }
			: entry;
		if (withIds) parentId = linked.id ?? parentId;
		lines.push(JSON.stringify(linked));
	}
	writeFileSync(join(fixture.sessionDir, `${name}.jsonl`), `${lines.join("\n")}\n`);
}

function extract(fixture: Fixture) {
	return extractLearnDigest({ cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir });
}

// ── Normalization ───────────────────────────────────────────────────────────

describe("normalizeErrorSignature", () => {
	it("collapses paths, line numbers and hashes so two runs match", () => {
		const first = normalizeErrorSignature("error at /home/ada/proj/src/index.ts:12:4 (build a1b2c3d4e5f)");
		const second = normalizeErrorSignature("error at /home/grace/other/src/index.ts:88:1 (build f5e4d3c2b1a)");
		expect(first).toBe(second);
	});

	it("keeps compiler codes, which are the tokens worth matching on", () => {
		expect(normalizeErrorSignature("TS2307: Cannot find module 'foo'")).toContain("TS2307");
	});

	it("collapses timestamps and durations", () => {
		const a = normalizeErrorSignature("failed after 1200ms at 2026-08-01T10:00:00Z");
		const b = normalizeErrorSignature("failed after 3.5s at 2026-08-02T11:30:00Z");
		expect(a).toBe(b);
	});
});

describe("normalizeCommand", () => {
	it("keeps flags, which are frequently the fix itself", () => {
		expect(normalizeCommand("npm test --coverage")).not.toBe(normalizeCommand("npm test"));
	});

	it("collapses absolute paths so the same command matches across machines", () => {
		expect(normalizeCommand("node /home/ada/x/run.js")).toBe(normalizeCommand("node /home/grace/y/run.js"));
	});
});

describe("normalizeDirective", () => {
	it("strips stacked filler and punctuation", () => {
		expect(normalizeDirective("Ok so please always run the tests!")).toBe("always run the tests");
	});
});

describe("isRuleShapedDirective", () => {
	it("accepts corrective phrasing", () => {
		expect(isRuleShapedDirective("always run pnpm test with --coverage")).toBe(true);
		expect(isRuleShapedDirective("never commit directly to main")).toBe(true);
	});

	it("rejects acknowledgements, slash commands and pasted output", () => {
		expect(isRuleShapedDirective("go ahead")).toBe(false);
		expect(isRuleShapedDirective("/learn")).toBe(false);
		expect(isRuleShapedDirective("diff --git a/foo b/foo")).toBe(false);
	});

	it("rejects plain task statements", () => {
		expect(isRuleShapedDirective("add a button to the settings page")).toBe(false);
	});
});

describe("isBenignFailure", () => {
	it("treats no-match searches and test predicates as normal answers", () => {
		expect(isBenignFailure("rg needle src")).toBe(true);
		expect(isBenignFailure("test -f missing.txt")).toBe(true);
		expect(isBenignFailure("sudo grep foo bar")).toBe(true);
	});

	it("treats build and install failures as real", () => {
		expect(isBenignFailure("npm install")).toBe(false);
	});
});

// ── Extraction ──────────────────────────────────────────────────────────────

describe("directive extraction", () => {
	it("reports a directive repeated across sessions, with its counts", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = extract(fixture);
		expect(digest.scannedSessions).toBe(2);
		expect(digest.directives).toHaveLength(1);
		expect(digest.directives[0]!.count).toBe(2);
		expect(digest.directives[0]!.sessions).toBe(2);
		expect(digest.directives[0]!.status).toBe("new");
	});

	it("drops a directive said only once", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);

		expect(extract(fixture).directives).toEqual([]);
	});

	it("marks a repeated directive as restated when AGENTS.md already covers it", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage in CI\n");
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = extract(fixture);
		expect(digest.directives[0]!.status).toBe("restated");
		expect(digest.directives[0]!.existingRule).toContain("--coverage");
	});

	it("ignores its own injected digest, so proposals cannot compound", () => {
		const fixture = createFixture();
		const digestText = `${LEARN_DIGEST_MARKER} always run the tests with --coverage`;
		writeSession(fixture, "s1", [userEntry(digestText)]);
		writeSession(fixture, "s2", [userEntry(digestText)]);

		expect(extract(fixture).directives).toEqual([]);
	});
});

describe("fix extraction", () => {
	it("pairs a failure with the same command later passing", () => {
		const fixture = createFixture();
		const entries = [
			...bash("npm install", "gyp ERR! stack Error: Can't find Python executable", true),
			...edit("package.json"),
			...bash("npm install", "added 40 packages", false),
		];
		writeSession(fixture, "s1", entries);

		const digest = extract(fixture);
		expect(digest.fixes).toHaveLength(1);
		expect(digest.fixes[0]!.command).toBe("npm install");
		expect(digest.fixes[0]!.errorExcerpt).toContain("gyp ERR!");
		expect(digest.fixes[0]!.editedFiles).toEqual(["package.json"]);
	});

	it("ignores a failure that was never resolved", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [...bash("npm install", "gyp ERR! boom", true), ...edit("package.json")]);

		expect(extract(fixture).fixes).toEqual([]);
	});

	it("does not count a different command passing as a fix", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [
			...bash("npm install", "gyp ERR! boom", true),
			...bash("echo hello", "hello", false),
		]);

		expect(extract(fixture).fixes).toEqual([]);
	});

	it("ignores commands whose non-zero exit is a normal answer", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [
			...bash("rg needle src", "", true),
			...edit("src/x.ts"),
			...bash("rg needle src", "src/x.ts:1", false),
		]);

		expect(extract(fixture).fixes).toEqual([]);
	});

	it("counts the same failure signature across sessions", () => {
		const fixture = createFixture();
		const entries = () => [
			...bash("npm install", "gyp ERR! stack Error: Can't find Python executable", true),
			...bash("npm install", "ok", false),
		];
		writeSession(fixture, "s1", entries());
		writeSession(fixture, "s2", entries());

		const digest = extract(fixture);
		expect(digest.fixes).toHaveLength(1);
		expect(digest.fixes[0]!.count).toBe(2);
		expect(digest.fixes[0]!.sessions).toBe(2);
	});
});

describe("workflow extraction", () => {
	it("reports a tool sequence repeated enough to be a skill", () => {
		const fixture = createFixture();
		const cycle = () => [
			...edit("src/a.ts"),
			...bash("npm test", "ok", false),
			...bash("git commit -m x", "ok", false),
		];
		writeSession(fixture, "s1", [...cycle(), ...cycle(), ...cycle()]);

		const digest = extract(fixture);
		expect(digest.workflows.length).toBeGreaterThan(0);
		expect(digest.workflows[0]!.count).toBeGreaterThanOrEqual(3);
	});

	it("ignores a run of one repeated tool, which is a loop not a workflow", () => {
		const fixture = createFixture();
		const entries = Array.from({ length: 12 }, (_, i) => edit(`src/f${i}.ts`)).flat();
		writeSession(fixture, "s1", entries);

		expect(extract(fixture).workflows).toEqual([]);
	});
});

describe("session selection", () => {
	it("walks the active branch, ignoring abandoned forks", () => {
		const fixture = createFixture();
		// A fork: the abandoned branch contains a "fix", the taken one does not.
		const abandonedCall = toolCallEntry("bash", { command: "npm install" }, "abandoned");
		const entries: Entry[] = [
			...bash("npm install", "gyp ERR! boom", true).map((e, i) => ({
				...e,
				id: `f${i}`,
				parentId: i === 0 ? null : "f0",
			})),
			{ ...abandonedCall, id: "abandoned-call", parentId: "f1" },
			{ ...toolResultEntry("abandoned", "ok", false), id: "abandoned-result", parentId: "abandoned-call" },
			// The branch actually taken forks off f1 and never re-runs the command.
			{ ...userEntry("moving on"), id: "taken", parentId: "f1" },
		];
		writeSession(fixture, "s1", entries, { withIds: true });

		expect(extract(fixture).fixes).toEqual([]);
	});

	it("treats a legacy session without entry ids as flat", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [...bash("npm install", "gyp ERR! boom", true), ...bash("npm install", "ok", false)]);

		expect(extract(fixture).fixes).toHaveLength(1);
	});

	it("skips sessions recorded for a different directory", () => {
		const fixture = createFixture();
		const lines = [
			JSON.stringify({
				type: "session",
				version: 3,
				id: "other",
				timestamp: "2026-08-01T00:00:00.000Z",
				cwd: "/elsewhere",
			}),
			JSON.stringify(userEntry("always run the tests with --coverage")),
		];
		writeFileSync(join(fixture.sessionDir, "other.jsonl"), `${lines.join("\n")}\n`);

		const digest = extract(fixture);
		expect(digest.scannedSessions).toBe(0);
		expect(digest.skippedSessions).toBe(1);
	});

	it("honours a configured repeat threshold", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };
		expect(extractLearnDigest({ ...base, minRepeats: 2 }).directives).toHaveLength(1);
		expect(extractLearnDigest({ ...base, minRepeats: 3 }).directives).toEqual([]);
	});

	it("honours a configured session cap", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };
		const capped = extractLearnDigest({ ...base, maxSessions: 1 });
		expect(capped.scannedSessions).toBe(1);
		expect(capped.skippedSessions).toBe(1);
		// One session left in the window no longer clears the default threshold.
		expect(capped.directives).toEqual([]);
	});

	it("ignores sessions older than the window", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);

		const digest = extractLearnDigest({
			cwd: fixture.cwd,
			agentDir: fixture.agentDir,
			sessionDir: fixture.sessionDir,
			now: new Date(Date.now() + 90 * 24 * 60 * 60 * 1000),
		});
		expect(digest.scannedSessions).toBe(0);
	});

	it("returns an empty digest when there is nothing to mine", () => {
		const fixture = createFixture();
		expect(isEmptyDigest(extract(fixture))).toBe(true);
	});
});

describe("suppression across runs", () => {
	/** Two sessions repeating one directive, timestamped so recurrence can be simulated. */
	function repeatedDirective(fixture: Fixture, sessionTimes: string[]): void {
		sessionTimes.forEach((timestamp, index) => {
			const lines = [
				JSON.stringify({ type: "session", version: 3, id: `s${index}`, timestamp, cwd: fixture.cwd }),
				JSON.stringify(userEntry("always run the tests with --coverage")),
			];
			writeFileSync(join(fixture.sessionDir, `s${index}.jsonl`), `${lines.join("\n")}\n`);
		});
	}

	it("holds back an item already shown that has not recurred since", () => {
		const fixture = createFixture();
		repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };

		const first = extractLearnDigest(base);
		expect(first.directives).toHaveLength(1);

		// Record the run as happening after the newest session.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));
		const second = extractLearnDigest({ ...base, state });

		expect(second.directives).toEqual([]);
		expect(second.suppressed).toBe(1);
	});

	it("shows it again once it recurs after being shown", () => {
		const fixture = createFixture();
		repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };

		const first = extractLearnDigest(base);
		// Surfaced before the second session, so that session counts as new.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-01T12:00:00.000Z"));

		expect(extractLearnDigest({ ...base, state }).directives).toHaveLength(1);
	});

	it("does not flag a rule accepted from a previous run as restated", () => {
		const fixture = createFixture();
		repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };

		const first = extractLearnDigest(base);
		expect(first.directives[0]!.status).toBe("new");

		// The user accepts it: the rule lands in AGENTS.md, nothing else changes.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage\n");

		// Without suppression this would come back as "restated" — accusing a rule
		// that is working of not working.
		expect(extractLearnDigest({ ...base, state }).directives).toEqual([]);
	});

	it("marks an item shown before and still unwritten as previously declined", () => {
		const fixture = createFixture();
		repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };

		const first = extractLearnDigest(base);
		// Surfaced before the last session, so it recurs and comes back.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-01T12:00:00.000Z"));

		const second = extractLearnDigest({ ...base, state });
		expect(second.directives[0]!.previouslyDeclined).toBe(true);
	});

	it("re-proposes everything under /learn all", () => {
		const fixture = createFixture();
		repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };

		const first = extractLearnDigest(base);
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));

		expect(extractLearnDigest({ ...base, state }).directives).toEqual([]);
		expect(extractLearnDigest({ ...base, state, ignoreState: true }).directives).toHaveLength(1);
	});

	it("counts coverage from the user scope, so a rule routed there is not called declined", () => {
		const fixture = createFixture();
		const userAgentsDir = join(tempDir, ".agents");
		mkdirSync(userAgentsDir, { recursive: true });
		writeFileSync(join(userAgentsDir, "AGENTS.md"), "- always run the tests with --coverage\n");
		const prior = process.env.HOOCODE_USER_AGENTS_DIR;
		process.env.HOOCODE_USER_AGENTS_DIR = userAgentsDir;
		try {
			repeatedDirective(fixture, ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"]);
			const digest = extractLearnDigest({
				cwd: fixture.cwd,
				agentDir: fixture.agentDir,
				sessionDir: fixture.sessionDir,
			});
			expect(digest.directives[0]!.status).toBe("restated");
		} finally {
			if (prior === undefined) delete process.env.HOOCODE_USER_AGENTS_DIR;
			else process.env.HOOCODE_USER_AGENTS_DIR = prior;
		}
	});
});

describe("learn state file", () => {
	it("round-trips through disk", () => {
		const fixture = createFixture();
		const path = getLearnStatePath(fixture.agentDir, fixture.sessionDir);

		expect(readLearnState(path).surfaced).toEqual({});
		writeLearnState(
			path,
			recordSurfaced(readLearnState(path), [
				{ key: "directive:x", lastSeen: "2026-08-01T00:00:00.000Z", covered: false },
			]),
		);
		expect(readLearnState(path).surfaced["directive:x"]).toBeDefined();
	});

	it("treats a corrupt or unknown-version file as empty rather than failing", () => {
		const fixture = createFixture();
		const path = getLearnStatePath(fixture.agentDir, fixture.sessionDir);
		mkdirSync(join(path, ".."), { recursive: true });

		writeFileSync(path, "{ not json");
		expect(readLearnState(path).surfaced).toEqual({});

		writeFileSync(path, JSON.stringify({ version: 999, surfaced: { "directive:x": {} } }));
		expect(readLearnState(path).surfaced).toEqual({});
	});

	it("prunes entries nothing has referenced in a long time", () => {
		const fixture = createFixture();
		const path = getLearnStatePath(fixture.agentDir, fixture.sessionDir);
		const stale = recordSurfaced(
			readLearnState(path),
			[{ key: "directive:old", lastSeen: "2020-01-01T00:00:00.000Z", covered: false }],
			new Date("2020-01-01T00:00:00.000Z"),
		);

		writeLearnState(path, stale, new Date("2026-08-01T00:00:00.000Z"));
		expect(readLearnState(path).surfaced).toEqual({});
	});
});

describe("digest rendering", () => {
	it("carries evidence and the routing rules the model needs", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "# Rules\n");
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const rendered = renderLearnDigest(extract(fixture), { userScopePath: "~/.agents/AGENTS.md" });
		expect(rendered.startsWith(LEARN_DIGEST_MARKER)).toBe(true);
		expect(rendered).toContain("2x across 2 sessions");
		expect(rendered).toContain("~/.agents/AGENTS.md");
		expect(rendered).toContain("loaded on **every** turn");
		expect(rendered).toContain("token delta");
	});
});
