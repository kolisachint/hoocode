/**
 * Renders the extractor's output into the message `/learn` injects.
 *
 * The digest is evidence plus instructions, and the split matters: the numbers
 * come from {@link extractLearnDigest} and are not negotiable, while everything
 * the model does with them — phrasing, routing, deciding a pattern is not worth
 * a rule — is judgement it has to exercise. Counts are printed on every item
 * because "said in 5 of your last 12 sessions" is a decision the reader can
 * make in one keystroke, where "extracted from your session" is not.
 */

import type { LearnDigest } from "./extract.js";
import { LEARN_DIGEST_MARKER } from "./extract.js";

function shortDate(iso: string | undefined): string {
	if (!iso) return "unknown";
	const date = new Date(iso);
	return Number.isNaN(date.getTime()) ? "unknown" : date.toISOString().slice(0, 10);
}

function evidence(count: number, sessions: number, lastSeen: string): string {
	const times = count === 1 ? "once" : `${count}x`;
	const where = sessions === 1 ? "1 session" : `${sessions} sessions`;
	return `${times} across ${where}, last ${shortDate(lastSeen)}`;
}

/** True when there is nothing worth asking the model to look at. */
export function isEmptyDigest(digest: LearnDigest): boolean {
	return digest.directives.length === 0 && digest.fixes.length === 0 && digest.workflows.length === 0;
}

export function renderLearnDigest(digest: LearnDigest, options: { userScopePath: string }): string {
	const lines: string[] = [];

	lines.push(
		`${LEARN_DIGEST_MARKER} Mined ${digest.scannedSessions} session(s) in this directory` +
			(digest.skippedSessions > 0 ? ` (${digest.skippedSessions} skipped: out of window or unreadable)` : "") +
			(digest.oldestSession ? `, ${shortDate(digest.oldestSession)} to ${shortDate(digest.newestSession)}` : "") +
			".",
	);
	lines.push("");
	lines.push(
		"The counts below are computed from session transcripts on disk, not from this conversation. " +
			"Treat them as evidence, not conclusions — your job is to decide what deserves to be written down, " +
			"phrase it, and put it in the right place.",
	);
	lines.push("");

	// ── Directives ───────────────────────────────────────────────────────────
	if (digest.directives.length > 0) {
		lines.push("## Directives you have repeated");
		lines.push("");
		for (const cluster of digest.directives) {
			lines.push(`- **${cluster.status}** — "${cluster.text.replace(/\s+/g, " ").trim()}"`);
			lines.push(`  - ${evidence(cluster.count, cluster.sessions, cluster.lastSeen)}`);
			if (cluster.existingRule) {
				lines.push(`  - already covered by: "${cluster.existingRule.slice(0, 160)}"`);
			}
		}
		lines.push("");
	}

	// ── Fixes ────────────────────────────────────────────────────────────────
	if (digest.fixes.length > 0) {
		lines.push("## Failures you resolved");
		lines.push("");
		lines.push(
			"Each is a command that failed, then later succeeded unchanged after intervening work — " +
				"so something in between was the fix.",
		);
		lines.push("");
		for (const fix of digest.fixes) {
			lines.push(`- \`${fix.command}\` — ${evidence(fix.count, fix.sessions, fix.lastSeen)}`);
			lines.push(`  - error: ${fix.errorExcerpt}`);
			if (fix.interveningCommands.length > 0) {
				lines.push(`  - commands in between: ${fix.interveningCommands.map((c) => `\`${c}\``).join(", ")}`);
			}
			if (fix.editedFiles.length > 0) {
				lines.push(`  - files edited: ${fix.editedFiles.join(", ")}`);
			}
		}
		lines.push("");
	}

	// ── Workflows ────────────────────────────────────────────────────────────
	if (digest.workflows.length > 0) {
		lines.push("## Repeated tool sequences");
		lines.push("");
		for (const workflow of digest.workflows) {
			lines.push(
				`- \`${workflow.steps.join(" → ")}\` — ${evidence(workflow.count, workflow.sessions, workflow.lastSeen)}`,
			);
		}
		lines.push("");
	}

	// ── Instructions ─────────────────────────────────────────────────────────
	lines.push("## What to do");
	lines.push("");
	lines.push("Work through the items above and propose concrete edits. For each one, decide:");
	lines.push("");
	lines.push(
		"1. **Is it durable?** A rule that will still be true next month belongs somewhere. A one-off preference " +
			"about the task you happened to be doing does not. When in doubt, drop it — a wrong rule costs more than " +
			"a missing one, because it is paid on every request forever.",
	);
	lines.push(
		"2. **Rule or skill?** This is the most important call. A context file is loaded on **every** turn; a skill " +
			"is loaded **on demand**. So: short, always-true, unconditional → a one-line rule. Long, procedural, " +
			'or conditional (a sequence of steps, a runbook, anything starting "when X, do Y") → a skill, not a rule. ' +
			"Repeated tool sequences are almost always skills.",
	);
	lines.push(
		`3. **Which scope?** Project-specific (this repo's tests, build, architecture, conventions) → the repo ` +
			`\`AGENTS.md\`. Personal habits that travel with you across every repo (style preferences, how you like ` +
			`commits written) → \`${options.userScopePath}\`. If it names this repo's files or commands, it is not a ` +
			`user-scope rule.`,
	);
	lines.push(
		"4. **Restated items are rewrites, not additions.** An item marked `restated` is already covered by a rule " +
			"that is not working — too vague, buried, or contradicted elsewhere. Rewrite the existing line or delete " +
			"it in favour of a sharper one. Do not add a second rule saying the same thing.",
	);
	lines.push("");
	lines.push("Then, while you have the file open, audit it:");
	lines.push("");
	lines.push(
		"- **Delete rules that no longer match the code.** Check a sample against the repo before trusting them.",
	);
	lines.push("- **Delete rules that restate default behaviour.** Guidance the agent already follows is pure cost.");
	lines.push(
		"- **Collapse duplicates**, including any rule stated at both repo and user scope — that one is paid twice.",
	);
	lines.push(
		"- **One line per rule.** No rationale, no examples, no preamble, unless the example *is* the rule. Prose is " +
			"the single biggest source of context-file bloat.",
	);
	lines.push("");

	if (digest.agentsFilePath) {
		lines.push(
			`The repo context file is \`${digest.agentsFilePath}\`` +
				(digest.agentsFileTokens ? ` (~${digest.agentsFileTokens} tokens, re-sent every request)` : "") +
				". Report the token delta of your proposed changes before applying them; a net reduction is a good outcome.",
		);
	} else {
		lines.push(
			"No repo context file exists yet. Create one only if at least one durable project rule survives step 1.",
		);
	}
	lines.push("");
	lines.push(
		"Show what you propose, then apply it with edits — do not ask a separate approval question first, the edit " +
			"prompt is the approval. If nothing here is worth writing down, say so plainly and change nothing.",
	);

	return lines.join("\n");
}
