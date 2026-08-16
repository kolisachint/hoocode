import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { getLearnCacheDir, hashSessionFile, readCachedMining, writeCachedMining } from "../src/core/learn/cache.js";
import type { Clusterer } from "../src/core/learn/cluster.js";
import type { CoverageIndex, CoverageJudge, CoverageMatch } from "../src/core/learn/coverage.js";
import { parseVerdicts } from "../src/core/learn/coverage.js";
import { isEmptyDigest, renderLearnDigest } from "../src/core/learn/digest.js";
import {
	buildCoverageIndex,
	type MineOptions,
	mineLearnDigest,
	planMining,
	scanSessions,
} from "../src/core/learn/extract.js";
import type { LabelledCandidate, MinableSession, MinedCandidate, Miner } from "../src/core/learn/mine.js";
import {
	chunkCharsForModel,
	chunkTranscript,
	isReplayedTurn,
	LEARN_DIGEST_MARKER,
	parseCandidates,
	renderTranscript,
	replayFingerprints,
	verifyCandidates,
} from "../src/core/learn/mine.js";
import { reduceDirectives, reduceFixes, reduceRequests } from "../src/core/learn/reduce.js";
import {
	getLearnStatePath,
	readLearnState,
	recordSurfaced,
	summarizeLearnState,
	writeLearnState,
} from "../src/core/learn/state.js";
import { getSessionDirPath } from "../src/core/session-manager.js";

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

// ── Test doubles for the two model-backed stages ────────────────────────────

/** Words too common to distinguish one directive from another, for the fake judge. */
const NOISE = new Set(["the", "with", "a", "an", "and", "to", "for", "always", "run", "use", "before"]);

function significantWords(text: string): string[] {
	return text
		.toLowerCase()
		.replace(/[^a-z0-9\s-]/g, " ")
		.split(/\s+/)
		.filter((w) => w.length > 2 && !NOISE.has(w));
}

/** Stable slug for a directive, standing in for the model's semantic label. */
function slugify(text: string): string {
	const slug = significantWords(text).slice(0, 4).join("-").replace(/-{2,}/g, "-").replace(/^-|-$/g, "");
	return slug || "unlabelled";
}

/**
 * A miner that reports one directive per user turn.
 *
 * It reads `renderTranscript` output rather than the raw entries, so every test
 * that goes through it also exercises the real rendering path — including the
 * exclusion of `/learn`'s own injected digest, which is what stops proposals
 * compounding their own counts.
 */
function fakeMiner(extra?: (session: MinableSession) => MinedCandidate[]): Miner {
	return async (session) => {
		const directives = renderTranscript(session)
			.split("\n")
			.filter((line) => line.startsWith("USER: "))
			.map((line) => line.slice("USER: ".length))
			.map<MinedCandidate>((text) => ({ kind: "directive", text }));
		return [...directives, ...(extra?.(session) ?? [])];
	};
}

/** A miner that reports nothing, for tests about discovery rather than content. */
const emptyMiner: Miner = async () => [];

/**
 * A clusterer that names each item after its significant words.
 *
 * Crude on purpose, and crude in the right direction: two differently-worded
 * lines about the same thing land on one label, which is the property the real
 * pass exists to provide. It also honours the known-label vocabulary, so tests
 * about state stability exercise the same path the model is asked to follow.
 */
function fakeClusterer(): Clusterer {
	return async (inputs, knownLabels) => {
		const out = new Map<number, string>();
		for (const input of inputs) {
			const slug = slugify(input.text);
			const known = knownLabels.find((label) => label === slug);
			out.set(input.id, known ?? slug);
		}
		return out;
	};
}

/**
 * A coverage judge that matches on shared significant words.
 *
 * Deliberately crude — the point is that the pipeline routes verdicts
 * correctly, not that this reproduces a model's judgement. The real judge is a
 * model call and is covered by `parseVerdicts` tests instead.
 */
function fakeCoverageJudge(): CoverageJudge {
	return async (queries, index) => {
		const out = new Map<string, CoverageMatch>();
		for (const query of queries) {
			const words = significantWords(query.text);
			if (words.length === 0) continue;

			const rule = index.ruleLines.find((line) => {
				const hay = significantWords(line);
				return words.every((w) => hay.includes(w));
			});
			if (rule) {
				out.set(query.label, { rule });
				continue;
			}
			const skill = index.skills.find((s) => {
				const hay = significantWords(`${s.name} ${s.description}`);
				return words.every((w) => hay.includes(w));
			});
			if (skill) out.set(query.label, { skill: skill.name });
		}
		return out;
	};
}

/** Run the pipeline against a fixture with the fake stages wired in. */
function mine(fixture: Fixture, overrides: Partial<MineOptions> = {}) {
	return mineLearnDigest({
		cwd: fixture.cwd,
		agentDir: fixture.agentDir,
		sessionDir: fixture.sessionDir,
		// Skills are injected empty by default: the real loader reads ~/.claude/skills,
		// so a developer's own skills would otherwise leak into coverage assertions.
		skills: [],
		miner: fakeMiner(),
		clusterer: fakeClusterer(),
		coverageJudge: fakeCoverageJudge(),
		...overrides,
	});
}

// ── Transcript rendering ────────────────────────────────────────────────────

describe("renderTranscript", () => {
	function session(entries: Entry[]): MinableSession {
		return { id: "s1", timestamp: "2026-08-01T00:00:00.000Z", entries: entries as MinableSession["entries"] };
	}

	it("keeps every user turn whole, whatever its phrasing", () => {
		// None of these contain an imperative keyword; the old regex gate dropped
		// all three, which is the recall failure this pipeline exists to fix.
		const rendered = renderTranscript(
			session([userEntry("we're on bun now"), userEntry("that's not how our error handling works")]),
		);
		expect(rendered).toContain("USER: we're on bun now");
		expect(rendered).toContain("USER: that's not how our error handling works");
	});

	it("does not truncate a long directive", () => {
		const long = `we do it this way because ${"reasons ".repeat(200)}`.trim();
		expect(renderTranscript(session([userEntry(long)]))).toContain(long);
	});

	it("excludes its own injected digest, so proposals cannot compound", () => {
		const rendered = renderTranscript(session([userEntry(`${LEARN_DIGEST_MARKER} always use bun`)]));
		expect(rendered).not.toContain("always use bun");
	});

	it("keeps tool calls and marks errors, which is where fixes come from", () => {
		const rendered = renderTranscript(session(bash("npm install", "gyp ERR! boom", true)));
		expect(rendered).toContain("TOOL: bash(command=npm install)");
		expect(rendered).toContain("ERROR: gyp ERR! boom");
	});

	it("drops successful tool output, which is a file talking rather than the user", () => {
		// Kept, this is where "directives" lifted out of plan files come from.
		const rendered = renderTranscript(session(bash("cat plan.md", "Set the tagline to 'coding agent'", false)));
		expect(rendered).toContain("TOOL: bash(command=cat plan.md)");
		expect(rendered).not.toContain("tagline");
	});

	it("drops a user turn that is a slash-command body replaying itself", () => {
		const template = {
			content: "Open a release PR with bump type: **$1**. Stage only the files you changed in this session.",
		};
		const fingerprints = replayFingerprints([template]);
		const expansion =
			"Open a release PR with bump type: **patch**. Stage only the files you changed in this session.";
		const rendered = renderTranscript(session([userEntry(expansion), userEntry("use tabs")]), fingerprints);
		expect(rendered).not.toContain("Stage only");
		expect(rendered).toContain("USER: use tabs");
	});

	it("keeps a turn that merely mentions a command, which the user did type", () => {
		const fingerprints = replayFingerprints([
			{ content: "Open a release PR with bump type: **$1**. Stage only the files you changed in this session." },
		]);
		const rendered = renderTranscript(session([userEntry("run /pr patch when you are done")]), fingerprints);
		expect(rendered).toContain("USER: run /pr patch when you are done");
	});
});

describe("replayFingerprints", () => {
	it("ignores a template with no literal run long enough to identify it", () => {
		// A short body cannot be told from a sentence someone would type, and a
		// false positive here deletes real evidence rather than noise.
		expect(replayFingerprints([{ content: "$1 $2" }])).toEqual([]);
		expect(replayFingerprints([{ content: "commit my work" }])).toEqual([]);
		expect(replayFingerprints([{ content: "x".repeat(39) }])).toEqual([]);
		expect(replayFingerprints([{ content: "x".repeat(40) }])).toHaveLength(1);
	});

	it("survives the wrapping a markdown body imposes on a sentence", () => {
		const fingerprints = replayFingerprints([
			{ content: "Identify ONLY the files you changed in this session. Do NOT use\n   `git add -A`." },
		]);
		expect(
			isReplayedTurn("Identify ONLY the files you changed in this session. Do NOT use `git add -A`.", fingerprints),
		).toBe(true);
	});
});

describe("verifyCandidates", () => {
	function directive(text: string): MinedCandidate {
		return { kind: "directive", text };
	}

	it("keeps a quote that can be found in what the user said", () => {
		const kept = verifyCandidates([directive("always use bun")], "we're on bun now. always use bun, never npm");
		expect(kept).toHaveLength(1);
	});

	it("matches across the wrapping the user's own message had", () => {
		expect(verifyCandidates([directive("always use bun")], "always   use\nbun")).toHaveLength(1);
	});

	it("drops a quote that appears nowhere, since it cannot be shown", () => {
		expect(verifyCandidates([directive("never force push")], "use tabs everywhere")).toHaveLength(0);
	});

	it("holds fixes to no such test, since their evidence is not something said", () => {
		const fix: MinedCandidate = { kind: "fix", text: "install failed, then passed" };
		expect(verifyCandidates([fix], "unrelated")).toHaveLength(1);
	});
});

describe("chunkTranscript", () => {
	it("splits on line boundaries so a user turn is never cut in half", () => {
		const text = ["USER: alpha", "USER: beta", "USER: gamma"].join("\n");
		const chunks = chunkTranscript(text, 20);
		expect(chunks.length).toBeGreaterThan(1);
		for (const chunk of chunks) {
			for (const line of chunk.split("\n")) expect(["USER: alpha", "USER: beta", "USER: gamma"]).toContain(line);
		}
	});

	it("keeps a single oversized turn intact rather than splitting it", () => {
		const long = `USER: ${"x".repeat(500)}`;
		expect(chunkTranscript(long, 100)).toEqual([long]);
	});

	it("returns nothing for an empty transcript", () => {
		expect(chunkTranscript("")).toEqual([]);
	});
});

describe("chunkCharsForModel", () => {
	it("sizes from the window, so a typical session is one call on a large model", () => {
		// The two real transcripts in this repo render to 183 KB and 266 KB. A
		// 200k-token window must swallow either whole — chunking them was costing
		// calls and putting blind spots inside a single session.
		const budget = chunkCharsForModel({ contextWindow: 200_000 });
		expect(budget).toBeGreaterThan(280_000);
	});

	it("leaves headroom rather than filling the window", () => {
		// Instructions, the response and tokenizer variance all have to fit
		// alongside; overshooting costs a context-overflow error, not a worse answer.
		const budget = chunkCharsForModel({ contextWindow: 200_000 });
		expect(budget).toBeLessThan(200_000 * 4);
	});

	it("still chunks a small window, but never into slivers", () => {
		expect(chunkCharsForModel({ contextWindow: 8_000 })).toBe(40_000);
	});

	it("falls back when a model reports no usable window", () => {
		expect(chunkCharsForModel({ contextWindow: 0 })).toBe(120_000);
		expect(chunkCharsForModel({ contextWindow: Number.NaN })).toBe(120_000);
	});
});

// ── Model response parsing ──────────────────────────────────────────────────

describe("parseCandidates", () => {
	it("reads a well-formed response", () => {
		const parsed = parseCandidates(JSON.stringify({ candidates: [{ kind: "directive", text: "we're on bun now" }] }));
		expect(parsed).toHaveLength(1);
		expect(parsed[0]!.text).toBe("we're on bun now");
	});

	it("ignores a label, which this stage is no longer asked for", () => {
		const parsed = parseCandidates(
			JSON.stringify({ candidates: [{ kind: "directive", label: "use-bun", text: "we're on bun now" }] }),
		);
		expect(parsed).toHaveLength(1);
		expect("label" in parsed[0]!).toBe(false);
	});

	it("tolerates a fenced or prefixed response", () => {
		const parsed = parseCandidates(
			'Here you go:\n```json\n{"candidates":[{"kind":"fix","text":"npm install"}]}\n```',
		);
		expect(parsed).toHaveLength(1);
		expect(parsed[0]!.kind).toBe("fix");
	});

	it("drops rows that cannot be quoted or typed", () => {
		const parsed = parseCandidates(
			JSON.stringify({
				candidates: [
					{ kind: "directive", text: "" },
					{ kind: "nonsense", text: "x" },
					{ kind: "directive", text: "keep me" },
				],
			}),
		);
		expect(parsed.map((c) => c.text)).toEqual(["keep me"]);
	});

	it("returns nothing rather than throwing on unparseable output", () => {
		expect(parseCandidates("the model refused")).toEqual([]);
		expect(parseCandidates("{ not json")).toEqual([]);
	});
});

describe("parseVerdicts", () => {
	const index: CoverageIndex = {
		ruleLines: ["- always use bun", "- write tests first"],
		skills: [{ name: "scaffold-route", description: "scaffolds a route" }],
	};
	const queries = [{ label: "use-bun", text: "we're on bun" }];

	it("resolves a rule verdict to the line it names", () => {
		const out = parseVerdicts(
			JSON.stringify({ verdicts: [{ label: "use-bun", verdict: "rule", ruleIndex: 0 }] }),
			queries,
			index,
		);
		expect(out.get("use-bun")).toEqual({ rule: "- always use bun" });
	});

	it("ignores a verdict for a label that was never asked about", () => {
		const out = parseVerdicts(
			JSON.stringify({ verdicts: [{ label: "invented", verdict: "rule", ruleIndex: 0 }] }),
			queries,
			index,
		);
		expect(out.size).toBe(0);
	});

	it("treats an out-of-range rule index as not covered", () => {
		// "Covered, but I cannot show you by what" is not something the reader can
		// act on, so it must not become a `restated` label.
		const out = parseVerdicts(
			JSON.stringify({ verdicts: [{ label: "use-bun", verdict: "rule", ruleIndex: 99 }] }),
			queries,
			index,
		);
		expect(out.has("use-bun")).toBe(false);
	});
});

// ── Reduction ───────────────────────────────────────────────────────────────

describe("reduce", () => {
	const session = (id: string, lastActivity: string, candidates: LabelledCandidate[]) => ({
		sessionId: id,
		lastActivity,
		candidates,
	});

	it("groups differently-worded occurrences that share a label", () => {
		const clusters = reduceDirectives(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{ kind: "directive", label: "use-bun-not-npm", text: "we're on bun now" },
				]),
				session("s2", "2026-08-02T00:00:00.000Z", [
					{ kind: "directive", label: "use-bun-not-npm", text: "stop using npm install here please" },
				]),
			],
			2,
		);

		expect(clusters).toHaveLength(1);
		expect(clusters[0]!.count).toBe(2);
		expect(clusters[0]!.sessions).toBe(2);
		// The longest quote is kept, since it carries the most of the reasoning.
		expect(clusters[0]!.text).toBe("stop using npm install here please");
		expect(clusters[0]!.lastSeen).toBe("2026-08-02T00:00:00.000Z");
	});

	it("counts sessions, not lines: saying it twice in one sitting is not a habit", () => {
		// Restating something in one session usually means the agent ignored it the
		// first time, which is evidence about that afternoon, not about how you work.
		const clusters = reduceDirectives(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{ kind: "directive", label: "loud", text: "a" },
					{ kind: "directive", label: "loud", text: "a" },
					{ kind: "directive", label: "loud", text: "a" },
				]),
			],
			2,
		);
		expect(clusters).toEqual([]);
	});

	it("drops a label that has not reached the repeat threshold", () => {
		const one = [session("s1", "2026-08-01T00:00:00.000Z", [{ kind: "directive" as const, label: "x", text: "x" }])];
		expect(reduceDirectives(one, 2)).toEqual([]);
	});

	it("ranks by distinct sessions before raw count", () => {
		const clusters = reduceDirectives(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{ kind: "directive", label: "loud", text: "a" },
					{ kind: "directive", label: "loud", text: "a" },
					{ kind: "directive", label: "loud", text: "a" },
					{ kind: "directive", label: "spread", text: "b" },
				]),
				session("s2", "2026-08-02T00:00:00.000Z", [{ kind: "directive", label: "spread", text: "b" }]),
			],
			2,
		);
		// "spread" was said in two sessions; "loud" three times in one. Two sessions
		// is the stronger evidence that something is durable rather than situational.
		expect(clusters[0]!.label).toBe("spread");
	});

	it("merges two labels carrying the same sentence, which is the same proposal twice", () => {
		// Observed on a real corpus: the same quote arrived under `stage-only-own-files`
		// and `stage-only-changed-files` and was proposed twice, word for word.
		const quote = "Do NOT use `git add -A` or `git add .`";
		const clusters = reduceDirectives(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{ kind: "directive", label: "stage-only-own-files", text: quote },
				]),
				session("s2", "2026-08-02T00:00:00.000Z", [
					{ kind: "directive", label: "stage-only-changed-files", text: `  ${quote.toUpperCase()}  ` },
				]),
			],
			2,
		);
		// Merging before the threshold, not after: each label alone is below it.
		expect(clusters).toHaveLength(1);
		expect(clusters[0]!.sessions).toBe(2);
		expect(clusters[0]!.count).toBe(2);
	});

	it("gives a merged cluster the same name whichever session came first", () => {
		// The name is the state key. If it followed session order it would change
		// whenever a session was added, and every item the reader had already
		// decided on would come back as new.
		const quote = "stage only your own files";
		const a = { kind: "directive" as const, label: "stage-only-own-files", text: quote };
		const b = { kind: "directive" as const, label: "only-stage-your-files", text: quote };
		const forwards = reduceDirectives(
			[session("s1", "2026-08-01T00:00:00.000Z", [a]), session("s2", "2026-08-02T00:00:00.000Z", [b])],
			2,
		);
		const backwards = reduceDirectives(
			[session("s2", "2026-08-02T00:00:00.000Z", [b]), session("s1", "2026-08-01T00:00:00.000Z", [a])],
			2,
		);
		expect(forwards[0]!.key).toBe(backwards[0]!.key);
	});

	it("merges fix detail across occurrences without duplicating it", () => {
		const fixes = reduceFixes(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{
						kind: "fix",
						label: "py-missing",
						text: "npm install",
						command: "npm install",
						errorExcerpt: "gyp ERR!",
						editedFiles: ["package.json"],
					},
				]),
				session("s2", "2026-08-02T00:00:00.000Z", [
					{ kind: "fix", label: "py-missing", text: "npm install", editedFiles: ["package.json", ".npmrc"] },
				]),
			],
			2,
		);
		expect(fixes[0]!.editedFiles).toEqual(["package.json", ".npmrc"]);
		expect(fixes[0]!.errorExcerpt).toBe("gyp ERR!");
	});

	it("carries a request's wording through, so the proposal can quote it", () => {
		const requests = reduceRequests(
			[
				session("s1", "2026-08-01T00:00:00.000Z", [
					{ kind: "request", label: "open-release-pr", text: "open a release PR" },
				]),
				session("s2", "2026-08-02T00:00:00.000Z", [
					{ kind: "request", label: "open-release-pr", text: "open a release PR" },
				]),
			],
			2,
		);
		expect(requests[0]!.text).toBe("open a release PR");
		expect(requests[0]!.sessions).toBe(2);
	});
});

// ── Pipeline ────────────────────────────────────────────────────────────────

describe("directive mining", () => {
	it("reports a directive repeated across sessions, with its counts", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture);
		expect(digest.scannedSessions).toBe(2);
		expect(digest.directives).toHaveLength(1);
		expect(digest.directives[0]!.count).toBe(2);
		expect(digest.directives[0]!.sessions).toBe(2);
		expect(digest.directives[0]!.status).toBe("new");
	});

	it("groups two phrasings of one point across sessions, which one session cannot do", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("we're on bun now")]);
		writeSession(fixture, "s2", [userEntry("stop using npm install")]);

		// Nothing in either sentence overlaps the other, so this only works because
		// naming happens once with both in view.
		const clusterer: Clusterer = async (items) =>
			new Map(items.map((item) => [item.id, /bun|npm/.test(item.text) ? "use-bun-not-npm" : "other"]));

		const digest = await mine(fixture, { clusterer });
		expect(digest.directives).toHaveLength(1);
		expect(digest.directives[0]!.label).toBe("use-bun-not-npm");
		expect(digest.directives[0]!.sessions).toBe(2);
	});

	it("offers the labels already on record, so a bookmarked item keeps its key", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		let offered: string[] = [];
		const clusterer: Clusterer = async (items, knownLabels) => {
			offered = knownLabels;
			return new Map(items.map((item) => [item.id, "run-tests-with-coverage"]));
		};

		const first = await mine(fixture, { clusterer });
		const state = recordSurfaced(readLearnState("missing"), first.surfaced);
		await mine(fixture, { clusterer, state });

		// Without this the second run can rename the cluster, the bookmark stops
		// matching, and everything already decided on comes back forever.
		expect(offered).toContain("run-tests-with-coverage");
	});

	it("still proposes when clustering fails, grouping identical wording only", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture, {
			clusterer: async () => {
				throw new Error("provider down");
			},
		});
		// The floor: worse recall than a working pass, but not a lost run.
		expect(digest.directives).toHaveLength(1);
		expect(digest.directives[0]!.label).toBe("always-run-the-tests-with-coverage");
	});

	it("drops a directive said only once", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);

		expect((await mine(fixture)).directives).toEqual([]);
	});

	it("marks a repeated directive as restated when AGENTS.md already covers it", async () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage in CI\n");
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture);
		expect(digest.directives[0]!.status).toBe("restated");
		expect(digest.directives[0]!.existingRule).toContain("--coverage");
	});

	it("ignores its own injected digest, so proposals cannot compound", async () => {
		const fixture = createFixture();
		const digestText = `${LEARN_DIGEST_MARKER} always run the tests with --coverage`;
		writeSession(fixture, "s1", [userEntry(digestText)]);
		writeSession(fixture, "s2", [userEntry(digestText)]);

		expect((await mine(fixture)).directives).toEqual([]);
	});

	it("leaves everything new when the coverage judge fails", async () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage\n");
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		// Over-proposing is the right way to fail: a duplicate can be rejected, but
		// a proposal wrongly withheld is never seen again.
		const digest = await mine(fixture, {
			coverageJudge: async () => {
				throw new Error("provider down");
			},
		});
		expect(digest.directives[0]!.status).toBe("new");
		// Flagged so the caller does not bookmark a guess as a reading: `new` here
		// means "not known", and storing it as "not covered" would have a later run
		// report a proposal as passed over that was never put in front of anyone.
		expect(digest.coverageFailed).toBe(true);
	});

	it("reports the funnel, so an empty run can be told from an over-filtered one", async () => {
		const fixture = createFixture();
		// Two points, one of them said in both sessions and one only in the first.
		writeSession(fixture, "s1", [
			userEntry("always run the tests with --coverage"),
			userEntry("never commit to main"),
		]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture);
		expect(digest.funnel.candidates).toBe(3);
		expect(digest.funnel.points).toBe(2);
		// Exactly what the threshold cost, not an estimate of it.
		expect(digest.funnel.belowThreshold).toBe(1);
		expect(digest.directives).toHaveLength(1);
	});

	it("reports what the per-run cap cut, so the top of a list does not read as the list", async () => {
		const fixture = createFixture();
		for (const id of ["s1", "s2"]) {
			writeSession(fixture, id, [
				userEntry("always run the tests with --coverage"),
				userEntry("never commit to main"),
			]);
		}

		const digest = await mine(fixture, { maxProposals: 1 });
		expect(digest.directives).toHaveLength(1);
		expect(digest.cut).toBe(1);
	});
});

describe("fix and request mining", () => {
	/** Reports a fix whenever the transcript shows a failed command. */
	const toolMiner: Miner = async (session) => {
		const rendered = renderTranscript(session);
		if (!rendered.includes("ERROR:")) return [];
		return [
			{
				kind: "fix",
				text: "npm install",
				command: "npm install",
				errorExcerpt: "gyp ERR!",
				editedFiles: ["package.json"],
			},
		];
	};

	it("carries fix detail through the pipeline into the digest", async () => {
		const fixture = createFixture();
		const entries = [...bash("npm install", "gyp ERR! no Python", true), ...edit("package.json")];
		writeSession(fixture, "s1", entries);
		writeSession(fixture, "s2", entries);

		const digest = await mine(fixture, { miner: toolMiner });
		expect(digest.fixes).toHaveLength(1);
		expect(digest.fixes[0]!.command).toBe("npm install");
		expect(digest.fixes[0]!.editedFiles).toEqual(["package.json"]);
		expect(digest.fixes[0]!.sessions).toBe(2);
	});

	it("proposes a repeated request as a slash command, on its own higher threshold", async () => {
		const fixture = createFixture();
		// The signal the miner used to be told to throw away: a job asked for by
		// name, in three different sessions, in slightly different words.
		const requestMiner: Miner = async (session) =>
			renderTranscript(session)
				.split("\n")
				.filter((line) => line.startsWith("USER: "))
				.map((line) => ({ kind: "request" as const, text: line.slice("USER: ".length) }));
		const clusterer: Clusterer = async (items) => new Map(items.map((item) => [item.id, "open-release-pr"]));

		writeSession(fixture, "s1", [userEntry("open a release PR")]);
		writeSession(fixture, "s2", [userEntry("raise the release PR please")]);

		// Two sessions is under the request bar, which is higher than the directive
		// one: a job that came up twice may just be a job that came up twice.
		expect((await mine(fixture, { miner: requestMiner, clusterer })).requests).toEqual([]);

		writeSession(fixture, "s3", [userEntry("cut a release PR")]);
		const digest = await mine(fixture, { miner: requestMiner, clusterer });
		expect(digest.requests).toHaveLength(1);
		expect(digest.requests[0]!.sessions).toBe(3);
	});
});

describe("mining failures", () => {
	it("reports a failed session instead of losing the whole run", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		let calls = 0;
		const flaky: Miner = async (session) => {
			calls++;
			if (calls === 1) throw new Error("rate limited");
			return fakeMiner()(session);
		};

		const digest = await mine(fixture, { miner: flaky });
		expect(digest.mining.failed).toBe(1);
		expect(digest.mining.mined).toBe(1);
		expect(digest.scannedSessions).toBe(2);
	});

	it("reports progress as it goes, so a cold run is not a silent wait", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("a directive")]);
		writeSession(fixture, "s2", [userEntry("a directive")]);

		const seen: Array<{ done: number; total: number }> = [];
		await mine(fixture, { onProgress: ({ done, total }) => seen.push({ done, total }) });
		expect(seen).toEqual([
			{ done: 1, total: 2 },
			{ done: 2, total: 2 },
		]);
	});
});

describe("cancellation", () => {
	it("stops reading and reports it, rather than presenting a part-counted window", async () => {
		const fixture = createFixture();
		for (const name of ["s1", "s2", "s3"]) {
			writeSession(fixture, name, [userEntry("always run the tests with --coverage")]);
		}

		const controller = new AbortController();
		let read = 0;
		const stopsAfterOne: Miner = async (session) => {
			read++;
			if (read === 1) controller.abort();
			return fakeMiner()(session);
		};

		const digest = await mine(fixture, { miner: stopsAfterOne, signal: controller.signal });
		expect(digest.aborted).toBe(true);
		// Cancelling is not the provider failing on a transcript.
		expect(digest.mining.failed).toBe(0);
		expect(digest.mining.mined).toBe(1);
	});

	it("keeps what it already read, so resuming is cheap", async () => {
		const fixture = createFixture();
		for (const name of ["s1", "s2", "s3"]) {
			writeSession(fixture, name, [userEntry("always run the tests with --coverage")]);
		}

		const controller = new AbortController();
		let read = 0;
		const stopsAfterTwo: Miner = async (session) => {
			read++;
			if (read === 2) controller.abort();
			return fakeMiner()(session);
		};

		await mine(fixture, { miner: stopsAfterTwo, signal: controller.signal });
		// The resumed run pays only for what the cancelled one did not reach.
		const resumed = await mine(fixture);
		expect(resumed.aborted).toBe(false);
		expect(resumed.mining.cached).toBe(2);
		expect(resumed.mining.mined).toBe(1);
		expect(resumed.directives[0]!.sessions).toBe(3);
	});
});

describe("planMining", () => {
	it("counts what the run will actually read, not every file on disk", async () => {
		const fixture = createFixture();
		writeSession(fixture, "mine", [userEntry("always run the tests with --coverage")]);
		// Belongs to another checkout: found on disk, never mined.
		writeFileSync(
			join(fixture.sessionDir, "other.jsonl"),
			`${JSON.stringify({ type: "session", version: 3, id: "other", timestamp: "2026-08-01T00:00:00.000Z", cwd: "/elsewhere" })}\n${JSON.stringify(userEntry("x"))}\n`,
		);

		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };
		// Two .jsonl files exist, but only one is this directory's.
		expect(scanSessions(base).files).toBe(2);
		expect(planMining(base)).toEqual({ total: 1, cached: 0, pending: 1 });
	});

	it("honours the session cap the run will use", () => {
		const fixture = createFixture();
		for (const name of ["s1", "s2", "s3"]) writeSession(fixture, name, [userEntry("a directive")]);

		const base = { cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir };
		expect(planMining({ ...base, maxSessions: 2 })).toEqual({ total: 2, cached: 0, pending: 2 });
	});

	it("reports nothing pending once the window is mined", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("a directive")]);
		await mine(fixture);

		expect(planMining({ cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir })).toEqual({
			total: 1,
			cached: 1,
			pending: 0,
		});
	});
});

// ── Cache ───────────────────────────────────────────────────────────────────

describe("mining cache", () => {
	it("round-trips through disk", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("x")]);
		const hash = hashSessionFile(join(fixture.sessionDir, "s1.jsonl"))!;

		expect(readCachedMining(fixture.agentDir, hash)).toBeUndefined();
		writeCachedMining(fixture.agentDir, hash, {
			sessionId: "s1",
			timestamp: "2026-08-01T00:00:00.000Z",
			candidates: [{ kind: "directive", text: "x" }],
			minedAt: "2026-08-01T00:00:00.000Z",
		});
		expect(readCachedMining(fixture.agentDir, hash)!.candidates).toHaveLength(1);
	});

	it("reuses a mined session rather than reading it twice", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const first = await mine(fixture);
		expect(first.mining.mined).toBe(2);
		expect(first.mining.cached).toBe(0);

		const second = await mine(fixture);
		expect(second.mining.mined).toBe(0);
		expect(second.mining.cached).toBe(2);
		// The counts are recomputed over every cached session, so caching the
		// expensive step never costs the cross-session evidence.
		expect(second.directives[0]!.count).toBe(2);
	});

	it("re-reads a session whose content changed", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		await mine(fixture);

		// A live session is appended to between runs; its bytes change, so the
		// memo must miss. Keying on mtime alone would also re-read an untouched
		// file that was merely resumed or copied.
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage"), userEntry("and use bun")]);
		expect((await mine(fixture)).mining.mined).toBe(1);
	});

	it("treats a corrupt entry as a miss rather than failing", () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("x")]);
		const hash = hashSessionFile(join(fixture.sessionDir, "s1.jsonl"))!;
		mkdirSync(getLearnCacheDir(fixture.agentDir), { recursive: true });
		writeFileSync(join(getLearnCacheDir(fixture.agentDir), `v1-${hash}.json`), "{ not json");

		expect(readCachedMining(fixture.agentDir, hash)).toBeUndefined();
	});

	it("does not write an entry for a session that failed to mine", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("x")]);

		await mine(fixture, {
			miner: async () => {
				throw new Error("nope");
			},
		});
		const dir = getLearnCacheDir(fixture.agentDir);
		expect(existsSync(dir) ? readdirSync(dir) : []).toEqual([]);
	});
});

// ── Session selection ───────────────────────────────────────────────────────

describe("session selection", () => {
	it("walks the active branch, ignoring abandoned forks", async () => {
		const fixture = createFixture();
		// A fork: the abandoned branch carries a directive, the taken one does not.
		const entries: Entry[] = [
			{ ...userEntry("first turn here"), id: "f0", parentId: null },
			{ ...userEntry("abandoned directive about pnpm"), id: "abandoned", parentId: "f0" },
			{ ...userEntry("moving on"), id: "taken", parentId: "f0" },
		];
		writeSession(fixture, "s1", entries, { withIds: true });

		const digest = await mine(fixture, { minRepeats: 1 });
		const labels = digest.directives.map((d) => d.text);
		expect(labels).toContain("moving on");
		expect(labels).not.toContain("abandoned directive about pnpm");
	});

	it("treats a legacy session without entry ids as flat", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("alpha directive"), userEntry("beta directive")]);

		expect((await mine(fixture, { minRepeats: 1 })).directives).toHaveLength(2);
	});

	it("skips sessions recorded for a different directory", async () => {
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

		const digest = await mine(fixture);
		expect(digest.scannedSessions).toBe(0);
		expect(digest.skippedSessions).toBe(1);
	});

	it("honours a configured repeat threshold", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		expect((await mine(fixture, { minRepeats: 2 })).directives).toHaveLength(1);
		expect((await mine(fixture, { minRepeats: 3 })).directives).toEqual([]);
	});

	it("honours a configured session cap", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const capped = await mine(fixture, { maxSessions: 1 });
		expect(capped.scannedSessions).toBe(1);
		expect(capped.skippedSessions).toBe(1);
		// One session left in the window no longer clears the default threshold.
		expect(capped.directives).toEqual([]);
	});

	it("ignores sessions older than the window", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture, { now: new Date(Date.now() + 90 * 24 * 60 * 60 * 1000) });
		expect(digest.scannedSessions).toBe(0);
	});

	it("returns an empty digest when there is nothing to mine", async () => {
		const fixture = createFixture();
		expect(isEmptyDigest(await mine(fixture, { miner: emptyMiner }))).toBe(true);
	});
});

// ── Session discovery ───────────────────────────────────────────────────────

/**
 * Where the sessions are found at all, which is upstream of every other
 * behaviour here: each case below used to report "no recent sessions in this
 * directory" while a full history sat on disk.
 */
describe("session discovery", () => {
	/** Write a session into the per-cwd default directory rather than the fixture's. */
	function writeDefaultDirSession(fixture: Fixture, name: string, headerCwd = fixture.cwd): string {
		const dir = getSessionDirPath(fixture.cwd, fixture.agentDir);
		mkdirSync(dir, { recursive: true });
		const lines = [
			JSON.stringify({
				type: "session",
				version: 3,
				id: name,
				timestamp: "2026-08-01T00:00:00.000Z",
				cwd: headerCwd,
			}),
			JSON.stringify(userEntry("always run the tests with --coverage")),
		];
		writeFileSync(join(dir, `${name}.jsonl`), `${lines.join("\n")}\n`);
		return dir;
	}

	it("finds the cwd's own sessions when the session manager reports no directory", async () => {
		const fixture = createFixture();
		writeDefaultDirSession(fixture, "s1");
		writeDefaultDirSession(fixture, "s2");

		// An in-memory session (`--no-session`) reports "", which is not nullish and
		// so used to be taken literally as the directory to scan.
		const digest = await mine(fixture, { sessionDir: "" });
		expect(digest.scannedSessions).toBe(2);
		expect(digest.directives).toHaveLength(1);
	});

	it("finds the cwd's own sessions when the session manager points elsewhere", async () => {
		const fixture = createFixture();
		writeDefaultDirSession(fixture, "s1");
		writeDefaultDirSession(fixture, "s2");

		// `--session <path>` or a custom `sessionDir` setting: the live directory is
		// real but is not where this directory's history lives.
		const digest = await mine(fixture);
		expect(digest.scannedSessions).toBe(2);
		expect(digest.scan.dirs).toHaveLength(2);
	});

	it("counts a session reachable from both directories once", async () => {
		const fixture = createFixture();
		const dir = writeDefaultDirSession(fixture, "s1");
		writeDefaultDirSession(fixture, "s2");

		const digest = await mine(fixture, { sessionDir: dir });
		expect(digest.scan.dirs).toHaveLength(1);
		expect(digest.scannedSessions).toBe(2);
	});

	it("accepts a header cwd spelled through a symlink", async () => {
		const fixture = createFixture();
		const link = join(tempDir, "link-to-repo");
		symlinkSync(fixture.cwd, link);
		writeDefaultDirSession(fixture, "s1", link);
		writeDefaultDirSession(fixture, "s2", link);

		const digest = await mine(fixture);
		expect(digest.scannedSessions).toBe(2);
		expect(digest.scan.otherCwd).toBe(0);
	});

	it("reports why each file was passed over", () => {
		const fixture = createFixture();
		writeSession(fixture, "mine", [userEntry("always run the tests with --coverage")]);
		writeFileSync(
			join(fixture.sessionDir, "other.jsonl"),
			`${JSON.stringify({ type: "session", version: 3, id: "other", timestamp: "2026-08-01T00:00:00.000Z", cwd: "/elsewhere" })}\n${JSON.stringify(userEntry("always run the tests with --coverage"))}\n`,
		);

		const scan = scanSessions({ cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: fixture.sessionDir });
		expect(scan.files).toBe(2);
		expect(scan.otherCwd).toBe(1);
		expect(scan.dirs).toContain(fixture.sessionDir);
	});

	it("names the missing directory when there is no history at all", () => {
		const fixture = createFixture();
		const scan = scanSessions({ cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: "" });
		expect(scan.files).toBe(0);
		expect(scan.missingDirs).toEqual([getSessionDirPath(fixture.cwd, fixture.agentDir)]);
	});

	it("separates stale history from missing history", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);

		const digest = await mine(fixture, { now: new Date(Date.now() + 90 * 24 * 60 * 60 * 1000) });
		expect(digest.scannedSessions).toBe(0);
		expect(digest.scan.files).toBe(1);
		expect(digest.scan.tooOld).toBe(1);
	});

	it("does not create the session directory just by looking for it", () => {
		const fixture = createFixture();
		const dir = getSessionDirPath(fixture.cwd, fixture.agentDir);
		scanSessions({ cwd: fixture.cwd, agentDir: fixture.agentDir, sessionDir: "" });
		expect(existsSync(dir)).toBe(false);
	});
});

describe("buildCoverageIndex", () => {
	function index(agents: string): string[] {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), agents);
		return buildCoverageIndex({ cwd: fixture.cwd, agentDir: fixture.agentDir, skills: [] }).ruleLines;
	}

	it("carries the heading path, so a rule is judged with its subject attached", () => {
		// "Stage only your own files" is unanswerable without knowing it is a git
		// rule, and the heading is where that lives.
		const lines = index("# Rules\n\n## Git\n\n- Stage only your own files\n");
		expect(lines).toContain("[repo] Rules > Git > - Stage only your own files");
	});

	it("pops back out of a subsection rather than nesting forever", () => {
		const lines = index("## Git\n\n### Committing\n\n- one\n\n## Testing\n\n- two\n");
		expect(lines).toContain("[repo] Git > Committing > - one");
		expect(lines).toContain("[repo] Testing > - two");
	});

	it("drops fenced code, which illustrates a rule rather than being one", () => {
		const lines = index("## Commands\n\n```bash\ngit reset --hard\n```\n\n- Never reset\n");
		expect(lines.some((line) => line.includes("git reset --hard"))).toBe(false);
		expect(lines).toContain("[repo] Commands > - Never reset");
	});

	it("names the scope, since a rule's home decides who it binds", () => {
		expect(index("- use tabs\n")).toContain("[repo] - use tabs");
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

	const TIMES = ["2026-08-01T00:00:00.000Z", "2026-08-02T00:00:00.000Z"];

	it("holds back an item already shown that has not recurred since", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const first = await mine(fixture);
		expect(first.directives).toHaveLength(1);

		// Record the run as happening after the newest session.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));
		const second = await mine(fixture, { state });

		expect(second.directives).toEqual([]);
		expect(second.suppressed).toBe(1);
	});

	it("dates an item by when it was said, not by when the session was opened", async () => {
		const fixture = createFixture();
		// Both sessions were opened before the last run and worked in after it — the
		// shape of any long-running session. Dating them by the header would call
		// today's words old news and hold them back.
		["s0", "s1"].forEach((id) => {
			const lines = [
				JSON.stringify({
					type: "session",
					version: 3,
					id,
					timestamp: "2026-08-01T00:00:00.000Z",
					cwd: fixture.cwd,
				}),
				JSON.stringify({
					...userEntry("always run the tests with --coverage"),
					timestamp: "2026-08-10T00:00:00.000Z",
				}),
			];
			writeFileSync(join(fixture.sessionDir, `${id}.jsonl`), `${lines.join("\n")}\n`);
		});

		const first = await mine(fixture);
		expect(first.directives).toHaveLength(1);

		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-05T00:00:00.000Z"));
		expect((await mine(fixture, { state })).directives).toHaveLength(1);
	});

	it("shows it again once it recurs after being shown", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const first = await mine(fixture);
		// Surfaced before the second session, so that session counts as new.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-01T12:00:00.000Z"));

		expect((await mine(fixture, { state })).directives).toHaveLength(1);
	});

	it("does not flag a rule accepted from a previous run as restated", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const first = await mine(fixture);
		expect(first.directives[0]!.status).toBe("new");

		// The user accepts it: the rule lands in AGENTS.md, nothing else changes.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage\n");

		// Without suppression this would come back as "restated" — accusing a rule
		// that is working of not working.
		expect((await mine(fixture, { state })).directives).toEqual([]);
	});

	it("marks an item shown before and still unwritten as previously declined", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const first = await mine(fixture);
		// Surfaced before the last session, so it recurs and comes back.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-01T12:00:00.000Z"));

		expect((await mine(fixture, { state })).directives[0]!.previouslyDeclined).toBe(true);
	});

	it("re-proposes everything under /learn all", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const first = await mine(fixture);
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-03T00:00:00.000Z"));

		expect((await mine(fixture, { state })).directives).toEqual([]);
		expect((await mine(fixture, { state, ignoreState: true })).directives).toHaveLength(1);
	});

	it("counts an existing skill as coverage, so one routed to a skill is not called declined", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);
		const skills = [{ name: "coverage-tests", description: "Always run the tests with --coverage before pushing." }];

		const first = await mine(fixture, { skills });
		expect(first.directives[0]!.status).toBe("has-skill");
		expect(first.directives[0]!.existingSkill).toBe("coverage-tests");

		// Surfaced before the last session, so it recurs and comes back — but
		// covered by the skill, so it must not be reported as passed over.
		const state = recordSurfaced(readLearnState("missing"), first.surfaced, new Date("2026-08-01T12:00:00.000Z"));
		expect((await mine(fixture, { skills, state })).directives[0]!.previouslyDeclined).toBe(false);
	});

	it("prefers a matching rule over a matching skill", async () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "- always run the tests with --coverage\n");
		repeatedDirective(fixture, TIMES);

		const digest = await mine(fixture, {
			skills: [{ name: "coverage-tests", description: "Always run the tests with --coverage before pushing." }],
		});
		expect(digest.directives[0]!.status).toBe("restated");
		expect(digest.directives[0]!.existingSkill).toBeUndefined();
	});

	it("does not call an unrelated skill coverage", async () => {
		const fixture = createFixture();
		repeatedDirective(fixture, TIMES);

		const digest = await mine(fixture, {
			skills: [{ name: "pdf-export", description: "Create and edit PDF documents, fill forms, extract pages." }],
		});
		expect(digest.directives[0]!.status).toBe("new");
	});

	it("counts coverage from the user scope, so a rule routed there is not called declined", async () => {
		const fixture = createFixture();
		const userAgentsDir = join(tempDir, ".agents");
		mkdirSync(userAgentsDir, { recursive: true });
		writeFileSync(join(userAgentsDir, "AGENTS.md"), "- always run the tests with --coverage\n");
		const prior = process.env.HOOCODE_USER_AGENTS_DIR;
		process.env.HOOCODE_USER_AGENTS_DIR = userAgentsDir;
		try {
			repeatedDirective(fixture, TIMES);
			expect((await mine(fixture)).directives[0]!.status).toBe("restated");
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

	it("discards a v1 file, whose keys are normalized text and can never match a label", () => {
		const fixture = createFixture();
		const path = getLearnStatePath(fixture.agentDir, fixture.sessionDir);
		mkdirSync(join(path, ".."), { recursive: true });

		// Left in place, every one of these would be counted by /learn stats as a
		// proposal that was never adopted — the rate would read low forever.
		writeFileSync(
			path,
			JSON.stringify({
				version: 1,
				surfaced: {
					"directive:always run the tests with coverage": {
						surfacedAt: "2026-08-01T00:00:00.000Z",
						lastOccurrence: "2026-08-01T00:00:00.000Z",
						coveredWhenSurfaced: false,
					},
				},
			}),
		);
		expect(readLearnState(path).surfaced).toEqual({});
	});

	it("keeps the wording, so stats can ask about coverage with the sentence not the slug", () => {
		const fixture = createFixture();
		const path = getLearnStatePath(fixture.agentDir, fixture.sessionDir);

		writeLearnState(
			path,
			recordSurfaced(readLearnState(path), [
				{
					key: "directive:use-bun-not-npm",
					lastSeen: "2026-08-01T00:00:00.000Z",
					covered: false,
					text: "we're on bun now",
				},
			]),
		);
		expect(readLearnState(path).surfaced["directive:use-bun-not-npm"]!.text).toBe("we're on bun now");
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

describe("learn stats", () => {
	function stateWith(keys: string[]) {
		return recordSurfaced(
			readLearnState("missing"),
			keys.map((key) => ({ key, lastSeen: "2026-08-01T00:00:00.000Z", covered: false })),
			new Date("2026-08-02T00:00:00.000Z"),
		);
	}

	it("counts what has been proposed, by category", () => {
		const stats = summarizeLearnState(
			stateWith(["directive:coverage-tests", "fix:npm-install-gyp", "request:open-release-pr"]),
		);

		expect(stats.total).toBe(3);
		expect(stats.directives).toBe(1);
		expect(stats.fixes).toBe(1);
		expect(stats.requests).toBe(1);
	});

	it("reports no adoption rate, which was never a number anyone could act on", () => {
		// It re-judged coverage with a model call and called the delta "adopted",
		// so a failed judge at either end moved it, and a proposal correctly
		// rejected as junk counted exactly like one ignored. What replaced it is
		// the always-loaded token count, which the filesystem answers exactly.
		const stats = summarizeLearnState(stateWith(["directive:coverage-tests"]));
		expect("adopted" in stats).toBe(false);
		expect("declined" in stats).toBe(false);
	});

	it("dates the record from what it holds", () => {
		const stats = summarizeLearnState(stateWith(["directive:coverage-tests"]));
		expect(stats.earliest).toBe("2026-08-02T00:00:00.000Z");
		expect(stats.lastRun).toBe("2026-08-02T00:00:00.000Z");
	});
});

describe("digest rendering", () => {
	it("carries evidence and the routing rules the model needs", async () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "# Rules\n");
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const rendered = renderLearnDigest(await mine(fixture), { userScopePath: "~/.agents/AGENTS.md" });
		expect(rendered.startsWith(LEARN_DIGEST_MARKER)).toBe(true);
		expect(rendered).toContain("2x across 2 sessions");
		expect(rendered).toContain("~/.agents/AGENTS.md");
		expect(rendered).toContain("loaded on **every** turn");
		expect(rendered).toContain("token delta");
	});

	it("states what the run cost, so the price is visible rather than inferred", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const rendered = renderLearnDigest(await mine(fixture), { userScopePath: "~/.agents/AGENTS.md" });
		expect(rendered).toContain("Read by the model this run: 2");
		expect(rendered).toContain("reused from cache: 0");
	});

	it("names the mode, so two very different empty results do not read alike", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);
		const digest = await mine(fixture);

		expect(renderLearnDigest(digest, { userScopePath: "~", mode: "all" })).toContain("Mode: all");
		expect(renderLearnDigest(digest, { userScopePath: "~", mode: "incremental" })).not.toContain("Mode: all");
	});

	it("omits detail the model did not supply, rather than rendering empty fields", async () => {
		const fixture = createFixture();
		const bare: Miner = async () => [
			{ kind: "fix", text: "npm install", command: "npm install" },
			{ kind: "request", text: "a routine job" },
		];
		writeSession(fixture, "s1", [userEntry("x")]);
		writeSession(fixture, "s2", [userEntry("x")]);

		const rendered = renderLearnDigest(await mine(fixture, { miner: bare, minRequestRepeats: 2 }), {
			userScopePath: "~",
		});
		expect(rendered).not.toContain("- error: \n");
		expect(rendered).not.toContain("``");
		expect(rendered).toContain("a routine job");
	});

	it("flattens and caps a request quote, which is a whole task message", async () => {
		const fixture = createFixture();
		const long = `open a release PR\nand ${"then do the next thing ".repeat(40)}`;
		const requestMiner: Miner = async () => [{ kind: "request", text: long }];
		for (const id of ["s1", "s2"]) writeSession(fixture, id, [userEntry(long)]);

		const rendered = renderLearnDigest(await mine(fixture, { miner: requestMiner, minRequestRepeats: 2 }), {
			userScopePath: "~",
		});
		const line = rendered.split("\n").find((l) => l.includes("open a release PR")) ?? "";
		// One bullet, not a paragraph that breaks the list it sits in.
		expect(line.startsWith("- ")).toBe(true);
		expect(line.length).toBeLessThan(300);
		expect(rendered).toContain("…");
	});

	it("names the shared label, so a count does not look like repeated copies of one sentence", async () => {
		const fixture = createFixture();
		writeSession(fixture, "s1", [userEntry("always run the tests with --coverage")]);
		writeSession(fixture, "s2", [userEntry("always run the tests with --coverage")]);

		const rendered = renderLearnDigest(await mine(fixture), { userScopePath: "~" });
		expect(rendered).toContain("grouped as: tests-coverage");
	});
});
