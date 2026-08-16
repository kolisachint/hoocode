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

import type { AuditReport } from "./audit.js";
import { staleTokens } from "./audit.js";
import type { LearnDigest } from "./extract.js";
import { LEARN_DIGEST_MARKER } from "./extract.js";

/**
 * Quote lengths. A directive is a sentence; a request is a whole task message,
 * and can be a slash-command body running to thousands of characters.
 */
const DIRECTIVE_QUOTE_CHARS = 400;
const REQUEST_QUOTE_CHARS = 200;

/** One line, bounded — a quote has to survive being rendered inside a list item. */
function quote(text: string, limit: number): string {
	const flat = text.replace(/\s+/g, " ").trim();
	return flat.length > limit ? `${flat.slice(0, limit)}…` : flat;
}

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
	return digest.directives.length === 0 && digest.fixes.length === 0 && digest.requests.length === 0;
}

/**
 * Render the audit findings as a message the model can act on.
 *
 * Deliberately framed as questions rather than verdicts. The checker is
 * deterministic and therefore confident, but "this path does not resolve" is
 * not the same claim as "this line is wrong" — a context file may name a
 * location the tool reads at runtime, or one that belongs to another checkout.
 * Roughly a third of findings on a real file are of that kind, so the message
 * that carries them has to ask for verification, not authorise a sweep.
 */
export function renderAuditReport(report: AuditReport): string {
	const lines: string[] = [];
	const cost = staleTokens(report);

	lines.push(
		`${LEARN_DIGEST_MARKER} Audited ${report.files.length} context file(s) — ${report.checked} referent(s) checked ` +
			`against the filesystem, ${report.stale.length} did not resolve (~${cost} tokens of always-loaded context).`,
	);
	lines.push("");
	lines.push(
		"This check is deterministic: it resolved every backticked path and `run` script named by the context files " +
			"below against this working tree and every package root in it. It costs no model calls and knows nothing " +
			"about intent.",
	);
	lines.push("");

	for (const item of report.stale) {
		lines.push(`- \`${item.referent}\` — ${item.file}:${item.line}, ~${item.tokens} tokens`);
		lines.push(`  - line: ${item.lineText.slice(0, 200)}`);
	}
	lines.push("");

	lines.push("## What to do");
	lines.push("");
	lines.push(
		"Each entry is a candidate, not a verdict. For each one, check the repo before touching the line — " +
			"`git log` for a file that moved or was deleted is usually enough to tell which case you are in:",
	);
	lines.push("");
	lines.push("1. **The referent moved.** Fix the path in place. Do not delete the rule; it is still true.");
	lines.push(
		"2. **The referent is gone and the rule went with it.** Delete the line, and the surrounding section if " +
			"nothing in it survives. This is the case worth the most — it is always-loaded context describing " +
			"something that cannot happen.",
	);
	lines.push(
		"3. **The path is a runtime or optional location** the tool reads if it happens to exist, or a file in " +
			"another checkout. Nothing is wrong; leave it alone and say so.",
	);
	lines.push("");
	lines.push(
		"Report the token delta of what you remove. Do not add anything — this pass is subtractive, and it is the " +
			"only one that moves the per-request cost down.",
	);

	return lines.join("\n");
}

export function renderLearnDigest(
	digest: LearnDigest,
	options: { userScopePath: string; mode?: "incremental" | "all" },
): string {
	const lines: string[] = [];

	lines.push(
		`${LEARN_DIGEST_MARKER} Mined ${digest.scannedSessions} session(s) in this directory` +
			(digest.skippedSessions > 0 ? ` (${digest.skippedSessions} skipped: out of window or unreadable)` : "") +
			(digest.oldestSession ? `, ${shortDate(digest.oldestSession)} to ${shortDate(digest.newestSession)}` : "") +
			(digest.suppressed > 0 ? `. ${digest.suppressed} item(s) held back — already shown and unchanged since` : "") +
			// A cut item cleared every bar and lost on rank. Saying so is the
			// difference between "this is everything" and "this is the top of a list".
			(digest.cut > 0 ? `. ${digest.cut} more cleared the bar but were cut to fit the per-run cap` : "") +
			".",
	);
	lines.push(
		`Read ${digest.funnel.candidates} occurrence(s), which named ${digest.funnel.points} distinct point(s); ` +
			`${digest.funnel.belowThreshold} did not recur in enough separate sessions to be proposed.`,
	);
	// Naming the mode keeps two very different empty results from reading alike:
	// "nothing new since last time" and "nothing here at all" are not the same
	// answer, and the reader cannot tell them apart from the counts.
	if (options.mode === "all") {
		lines.push("Mode: all — suppression is off, so items you have already seen and decided on are included.");
	}
	// The model reads every transcript in full, which costs real tokens. Saying
	// what was re-read versus reused keeps that price visible rather than hidden.
	lines.push(
		`Read by the model this run: ${digest.mining.mined}; reused from cache: ${digest.mining.cached}` +
			(digest.mining.failed > 0
				? `; failed: ${digest.mining.failed} (their signals are missing from the counts below)`
				: "") +
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
			lines.push(`- **${cluster.status}** — "${quote(cluster.text, DIRECTIVE_QUOTE_CHARS)}"`);
			lines.push(`  - ${evidence(cluster.count, cluster.sessions, cluster.lastSeen)}`);
			// Occurrences were grouped by meaning, not by wording, so the quote above
			// is one phrasing of several. Naming the shared point keeps a count of 5
			// from looking like five copies of one sentence.
			lines.push(`  - grouped as: ${cluster.label}`);
			if (cluster.rationale) {
				lines.push(`  - why it may be durable: ${cluster.rationale}`);
			}
			if (cluster.existingRule) {
				lines.push(`  - already covered by: "${cluster.existingRule.slice(0, 160)}"`);
			}
			if (cluster.existingSkill) {
				lines.push(`  - already covered by the \`${cluster.existingSkill}\` skill`);
			}
			if (cluster.previouslyDeclined) {
				lines.push("  - proposed before and not written down — you have already passed on this once");
			}
		}
		lines.push("");
	}

	// ── Fixes ────────────────────────────────────────────────────────────────
	if (digest.fixes.length > 0) {
		lines.push("## Failures you resolved");
		lines.push("");
		lines.push(
			"Each is a command that failed and later succeeded, where something done in between was the fix. " +
				"Recurring ones are worth writing down; a one-off is not.",
		);
		lines.push("");
		for (const fix of digest.fixes) {
			lines.push(`- \`${fix.command}\` — ${evidence(fix.count, fix.sessions, fix.lastSeen)}`);
			lines.push(`  - grouped as: ${fix.label}`);
			// The excerpt comes from the model now, which may not have quoted one.
			if (fix.errorExcerpt) {
				lines.push(`  - error: ${fix.errorExcerpt}`);
			}
			if (fix.interveningCommands.length > 0) {
				lines.push(`  - commands in between: ${fix.interveningCommands.map((c) => `\`${c}\``).join(", ")}`);
			}
			if (fix.editedFiles.length > 0) {
				lines.push(`  - files edited: ${fix.editedFiles.join(", ")}`);
			}
		}
		lines.push("");
	}

	// ── Requests ────────────────────────────────────────────────────────────
	if (digest.requests.length > 0) {
		lines.push("## Work you keep asking for by name");
		lines.push("");
		for (const request of digest.requests) {
			// Flattened and capped, unlike a directive quote. A request *is* a whole
			// task message — a slash-command body runs to thousands of characters — so
			// eight of them rendered raw would swamp the digest and a multi-line one
			// would break the list it sits in.
			lines.push(`- **${request.label}** — "${quote(request.text, REQUEST_QUOTE_CHARS)}"`);
			lines.push(`  - ${evidence(request.count, request.sessions, request.lastSeen)}`);
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
		"2. **Rule, skill, or slash command?** This is the most important call, and it is a cost question. A " +
			"context file is loaded on **every** turn; a skill's description is always loaded but its body only on " +
			"demand; a slash command costs nothing until it is invoked.",
	);
	lines.push(
		"   - **Rule** — short, always true, unconditional. One line in a context file. Highest bar, because it is " +
			"paid on every request forever whether or not it is relevant.",
	);
	lines.push(
		'   - **Skill** — long, procedural, or conditional; anything shaped "when X, do Y"; a runbook or a sequence ' +
			"of steps. Write `.agents/skills/<name>/SKILL.md`, and spend the effort on the `description` " +
			"frontmatter: it is the only part always in context, and it decides whether the skill ever fires.",
	);
	lines.push(
		"   - **Slash command** — a *job you keep asking for*, not a rule about how work is done. The items under " +
			'"Work you keep asking for by name" are these. Write `.agents/commands/<name>.md`, with `$1`/`$ARGUMENTS` ' +
			"where the request varies. The cheapest artifact there is: nothing is loaded until you type it.",
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
	lines.push(
		"5. **`has-skill` items are a triggering problem, not a missing rule.** A skill already covers it and you " +
			"asked by hand anyway, which usually means the skill's `description` frontmatter does not describe the " +
			"situation you were in. Sharpen that description so it matches, rather than adding a rule that duplicates " +
			"what the skill already does.",
	);
	lines.push(
		"6. **Write local files, not a plugin.** Skills and commands proposed from this evidence are local habits: " +
			"write them under `.agents/`. `ProposePlugin` packages something already proven useful into a portable, " +
			"publishable artifact — a later step for a skill that has earned it, not the way to create one. Never " +
			"propose a hook or an MCP server from this evidence: it records what was said and what failed, which is " +
			"far too weak a warrant for anything that executes.",
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
