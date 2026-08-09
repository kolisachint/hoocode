/**
 * Step 11: G4 trigger eval, and the Tier 2 drift check.
 *
 * Both are tested offline and deterministically — the trigger eval with an
 * injected judge, the drift check with synthetic reference text. That is the
 * reason both were built with a seam: an eval you can only run by spending a
 * model call, and a drift check you can only run with network, are checks that
 * get exercised once and then rot.
 */

import { describe, expect, it } from "vitest";
import {
	analyzeDrift,
	type DriftSource,
	formatDriftReport,
	manifestKeysIn,
	pathsIn,
} from "../src/core/extensions/plugins/drift.js";
import { declaredVocabulary } from "../src/core/extensions/plugins/formats/index.js";
import {
	distractorCandidates,
	runTriggerEval,
	type TriggerCandidate,
	type TriggerCase,
	type TriggerJudge,
	triggerFindings,
} from "../src/core/extensions/plugins/trigger-eval.js";

// ── G4 trigger eval ───────────────────────────────────────────────────────────

const candidates: TriggerCandidate[] = [
	{ name: "pdf-extract", description: "Extract text and tables from a PDF.", own: true },
	{ name: "mail-send", description: "Send an email.", own: false },
];

const cases: TriggerCase[] = [
	{ prompt: "pull the tables out of this PDF", expect: "pdf-extract" },
	{ prompt: "get the text from report.pdf", expect: "pdf-extract" },
	{ prompt: "email the summary to Dana", expect: "mail-send" },
	{ prompt: "what is the capital of France", expect: null },
];

/** A judge that returns exactly what it is told to, in order. */
function scriptedJudge(verdicts: Array<string | null>): TriggerJudge {
	return async () => verdicts;
}

describe("G4 trigger eval", () => {
	it("scores a description that fires correctly and stays quiet otherwise", async () => {
		const outcome = await runTriggerEval(
			"pdf",
			candidates,
			cases,
			scriptedJudge(["pdf-extract", "pdf-extract", "mail-send", null]),
		);
		expect(outcome.status).toBe("ran");
		if (outcome.status !== "ran") return;
		expect(outcome.record.recall).toBe(1);
		expect(outcome.record.specificity).toBe(1);
		expect(triggerFindings(outcome).every((f) => f.severity === "info")).toBe(true);
	});

	it("errors when the skill does not fire on the prompts it is for", async () => {
		const outcome = await runTriggerEval("pdf", candidates, cases, scriptedJudge([null, null, "mail-send", null]));
		expect(outcome.status).toBe("ran");
		if (outcome.status !== "ran") return;
		expect(outcome.record.recall).toBe(0);
		const findings = triggerFindings(outcome);
		expect(findings.some((f) => f.gate === "G4" && f.severity === "error")).toBe(true);
	});

	it("warns — not errors — when the skill also claims prompts that are not its own", async () => {
		// Over-firing costs context on every request; it does not break anything.
		const outcome = await runTriggerEval(
			"pdf",
			candidates,
			cases,
			scriptedJudge(["pdf-extract", "pdf-extract", "pdf-extract", "pdf-extract"]),
		);
		expect(outcome.status).toBe("ran");
		if (outcome.status !== "ran") return;
		expect(outcome.record.recall).toBe(1);
		expect(outcome.record.specificity).toBe(0);
		const findings = triggerFindings(outcome);
		expect(findings.some((f) => f.severity === "warning")).toBe(true);
		expect(findings.some((f) => f.severity === "error")).toBe(false);
	});

	it("counts a sibling's prompt as a negative, not a miss", async () => {
		// "email the summary" expects `mail-send`, which is a distractor. Getting it
		// right is us staying quiet — it must not count against our recall.
		const outcome = await runTriggerEval(
			"pdf",
			candidates,
			[cases[0], cases[2]],
			scriptedJudge(["pdf-extract", "mail-send"]),
		);
		if (outcome.status !== "ran") throw new Error("expected a run");
		expect(outcome.record.recall).toBe(1);
		expect(outcome.record.specificity).toBe(1);
	});

	describe("never fabricates a result", () => {
		it("reports not-run without a judge, and does not pass", async () => {
			const outcome = await runTriggerEval("pdf", candidates, cases, undefined);
			expect(outcome.status).toBe("not-run");
			const findings = triggerFindings(outcome);
			// `info`, so a machine with no model is not blocked — but the record says
			// the check did not happen rather than leaving a silence that reads green.
			expect(findings[0].severity).toBe("info");
			expect(findings[0].message).toContain("did not run");
		});

		it("reports not-run without a gold set", async () => {
			const outcome = await runTriggerEval("pdf", candidates, undefined, scriptedJudge([]));
			expect(outcome.status).toBe("not-run");
			if (outcome.status !== "not-run") return;
			expect(outcome.reason).toContain("eval");
		});

		it("refuses a judge that answers the wrong number of prompts", async () => {
			// Scoring a partial alignment would silently compare the wrong pairs.
			const outcome = await runTriggerEval("pdf", candidates, cases, scriptedJudge(["pdf-extract"]));
			expect(outcome.status).toBe("not-run");
			if (outcome.status !== "not-run") return;
			expect(outcome.reason).toContain("1 verdict(s) for 4 prompt(s)");
		});

		it("reports not-run when the judge throws", async () => {
			const outcome = await runTriggerEval("pdf", candidates, cases, async () => {
				throw new Error("no API key");
			});
			expect(outcome.status).toBe("not-run");
			if (outcome.status !== "not-run") return;
			expect(outcome.reason).toContain("no API key");
		});
	});

	it("pins what was judged, so a later number is comparable", async () => {
		const a = await runTriggerEval(
			"pdf",
			candidates,
			cases,
			scriptedJudge(["pdf-extract", "pdf-extract", "mail-send", null]),
		);
		const b = await runTriggerEval(
			"pdf",
			candidates,
			cases,
			scriptedJudge(["pdf-extract", "pdf-extract", "mail-send", null]),
		);
		if (a.status !== "ran" || b.status !== "ran") throw new Error("expected runs");
		expect(a.record.corpusHash).toBe(b.record.corpusHash);

		// An edited description is a different corpus, and the record must say so.
		const edited = [{ ...candidates[0], description: "Something else." }, candidates[1]];
		const c = await runTriggerEval(
			"pdf",
			edited,
			cases,
			scriptedJudge(["pdf-extract", "pdf-extract", "mail-send", null]),
		);
		if (c.status !== "ran") throw new Error("expected a run");
		expect(c.record.corpusHash).not.toBe(a.record.corpusHash);
	});

	it("draws distractors from other sources only", () => {
		const docs = [
			{ id: "a", kind: "skill" as const, name: "mine", description: "x", source: "pdf", deferred: false },
			{ id: "b", kind: "skill" as const, name: "theirs", description: "y", source: "other", deferred: false },
			{ id: "c", kind: "command" as const, name: "cmd", description: "z", source: "other", deferred: false },
		];
		const out = distractorCandidates("pdf", docs);
		// Own capabilities excluded; commands excluded (invoked by name, not chosen).
		expect(out.map((c) => c.name)).toEqual(["theirs"]);
		expect(out.every((c) => !c.own)).toBe(true);
	});
});

// ── Tier 2 drift check ────────────────────────────────────────────────────────

const REFERENCE = `
# Plugin reference

Put your manifest at \`.claude-plugin/plugin.json\`:

\`\`\`json
{
  "name": "example",
  "version": "1.0.0",
  "brandNewField": true
}
\`\`\`

Hooks live at \`my-plugin/hooks/hooks.json\`. LSP servers at \`lsp-config/servers.json\`.
See \`README.md\` and the \`anthropics/claude-plugins-community\` marketplace, or
call \`roots/list\`. Downloads are a \`.zip\`.
`;

describe("Tier 2 drift check", () => {
	it("takes manifest keys from parsed JSON, not from prose", () => {
		const keys = manifestKeysIn(REFERENCE);
		expect(keys.has("brandNewField")).toBe(true);
		expect(keys.has("name")).toBe(true);
		// Backticked prose is not a manifest key.
		expect(keys.has("hooks.json")).toBe(false);
	});

	it("ignores an unparseable fence rather than guessing at its keys", () => {
		expect(manifestKeysIn("```json\n{ not json\n```").size).toBe(0);
	});

	it("rejects the path shapes that made the first version unusable", () => {
		const paths = pathsIn(REFERENCE);
		// Protocol methods, repo slugs, bare extensions and generic files: all the
		// noise that drowned the two real findings on the first live run.
		expect(paths.has("roots/list")).toBe(false);
		expect(paths.has("anthropics/claude-plugins-community")).toBe(false);
		expect(paths.has(".zip")).toBe(false);
		expect(paths.has("README.md")).toBe(false);
		expect(paths.has("lsp-config/servers.json")).toBe(true);
	});

	it("reports what the vendor documents and we do not declare", () => {
		const report = analyzeDrift([{ label: "ref", url: "u", text: REFERENCE }]);
		expect(report.clean).toBe(false);
		expect(report.findings.some((f) => f.kind === "manifest-key" && f.value === "brandNewField")).toBe(true);
		expect(report.findings.some((f) => f.kind === "path" && f.value === "lsp-config/servers.json")).toBe(true);
		// Already declared, in three different ways: modelled key, example-rooted
		// path, and a directory whose contents we declare.
		expect(report.findings.some((f) => f.value === "name")).toBe(false);
		expect(report.findings.some((f) => f.value === "my-plugin/hooks/hooks.json")).toBe(false);
	});

	it("never calls an incomplete sweep clean", () => {
		const sources: DriftSource[] = [
			{ label: "ok", url: "u", text: "```json\n{}\n```" },
			{ label: "down", url: "u2", error: "ECONNREFUSED" },
		];
		const report = analyzeDrift(sources);
		expect(report.findings).toEqual([]);
		// No findings, but a source was unreadable — the day the docs move behind a
		// redirect, this must not read as "no drift".
		expect(report.clean).toBe(false);
		expect(formatDriftReport(report)).toContain("UNREACHABLE down");
		expect(formatDriftReport(report)).toContain("incomplete");
	});

	it("is clean only when everything was read and nothing was new", () => {
		const report = analyzeDrift([{ label: "ok", url: "u", text: '```json\n{ "name": "x" }\n```' }]);
		expect(report.clean).toBe(true);
		expect(formatDriftReport(report)).toContain("No drift.");
	});

	it("declares a non-empty vocabulary for the check to diff against", () => {
		const declared = declaredVocabulary();
		expect(declared.manifestKeys).toContain("mcpServers");
		expect(declared.marketplaceKeys).toContain("source");
		expect(declared.readPaths).toContain("skills");
		expect(declared.marketplaceFiles.length).toBeGreaterThan(0);
		// Real relative paths, not display labels — the labels once caused a known
		// surface to be reported as drift.
		expect(declared.unsupportedSurfaces).toContain("monitors/monitors.json");
	});
});
