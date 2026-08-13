/**
 * `/learn` — promote what recent sessions actually taught into durable rules
 * and skills.
 *
 * The command is a thin shell on purpose. It runs the deterministic extractor
 * over session transcripts on disk, renders the ranked result, and injects it
 * as a follow-up message; every judgement after that belongs to the model,
 * which can read the repo and phrase a rule far better than a heuristic can.
 *
 * Reading transcripts from disk rather than the live context is what makes this
 * work: the on-disk history survives compaction, and it spans past sessions, so
 * "you have said this in five separate sessions" is available as a number
 * instead of a guess. That number is the whole reason the command exists.
 *
 * Follows /grill in modes.ts: no session switch, no mode change, no config
 * write — just a follow-up message. Writes to AGENTS.md happen through ordinary
 * edit tools, so the existing permission prompt is the approval step and no
 * separate picker is needed.
 */

import { homedir } from "node:os";
import { join } from "node:path";
import { getHooCodeDir } from "../../config.js";
import { loadProjectContextFiles } from "../../core/context-files.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { isEmptyDigest, renderLearnDigest } from "../../core/learn/digest.js";
import { buildCoverageIndex, extractLearnDigest, matchCoverage } from "../../core/learn/extract.js";
import {
	getLearnStatePath,
	readLearnState,
	recordSurfaced,
	summarizeLearnState,
	writeLearnState,
} from "../../core/learn/state.js";
import { SettingsManager } from "../../core/settings-manager.js";

/** Guards against double-registration when default extensions load more than once. */
const REGISTERED = Symbol.for("hoocode.learn.registered");

/** User-scope destination offered for personal rules that travel across repos. */
const USER_SCOPE_PATH = join(homedir(), ".agents", "AGENTS.md");

/** Render a home-relative path the way the user would type it. */
function displayPath(path: string): string {
	const home = homedir();
	return path.startsWith(home) ? `~${path.slice(home.length)}` : path;
}

function shortDate(iso: string | undefined): string {
	if (!iso) return "unknown";
	const date = new Date(iso);
	return Number.isNaN(date.getTime()) ? "unknown" : date.toISOString().slice(0, 10);
}

/**
 * `/learn stats` — what became of past proposals.
 *
 * Reads the state file and recomputes coverage; it does not re-mine sessions,
 * so it is instant and answers a different question than a normal run: not
 * "what should I write down" but "is this command earning its place".
 */
function reportStats(ctx: ExtensionCommandContext): void {
	const agentDir = getHooCodeDir();
	const statePath = getLearnStatePath(agentDir, ctx.sessionManager.getSessionDir());
	const state = readLearnState(statePath);

	if (Object.keys(state.surfaced).length === 0) {
		ctx.ui.notify("No /learn history for this directory yet.", "info");
		return;
	}

	const coverage = buildCoverageIndex({ cwd: ctx.cwd, agentDir });
	const stats = summarizeLearnState(state, (normalized) => {
		const match = matchCoverage(normalized, coverage);
		return !!(match.rule || match.skill);
	});

	const contextTokens = loadProjectContextFiles({ cwd: ctx.cwd, agentDir }).agentsFiles.reduce(
		(sum, file) => sum + (file.tokens ?? 0),
		0,
	);

	const lines: string[] = [];
	lines.push(`/learn history for this directory — ${shortDate(stats.earliest)} to ${shortDate(stats.latest)}`);
	lines.push(
		`  Proposals shown   ${stats.total}  (${stats.directives} directive, ${stats.fixes} fix, ${stats.workflows} workflow)`,
	);
	if (stats.lastRun) lines.push(`  Last run          ${shortDate(stats.lastRun)}`);
	lines.push("");

	if (stats.open === 0) {
		lines.push("No directive proposals yet, so there is nothing to measure adoption against.");
	} else {
		const rate = Math.round((stats.adopted / stats.open) * 100);
		lines.push("Directive adoption — the only category with a coverage signal");
		lines.push(`  Written down      ${stats.adopted} of ${stats.open}  (${rate}%)`);
		lines.push(`  Passed over       ${stats.declined}`);
		lines.push("");
		// Without this the number invites the wrong conclusion. Adoption is a proxy
		// for usefulness, and a proposal correctly rejected as not durable counts
		// against it exactly like a junk one — so near-100% means the bar is too
		// low, not that the extractor is perfect.
		lines.push("  A very high rate means the bar is too low, not that every proposal was good.");
		lines.push("  Near zero means the extractor is proposing the wrong things.");
	}

	lines.push("");
	lines.push(`Context files       ~${contextTokens} tokens, re-sent every request`);

	ctx.ui.notify(lines.join("\n"), "info");
}

export function setupLearn(pi: ExtensionAPI): void {
	const guarded = pi as unknown as Record<symbol, boolean>;
	if (guarded[REGISTERED]) return;
	guarded[REGISTERED] = true;

	pi.registerCommand("learn", {
		description: "Mine recent sessions for durable rules and skills. Usage: /learn [all|stats]",
		getArgumentCompletions: (prefix: string) =>
			(
				[
					{ value: "all", label: "re-propose everything" },
					{ value: "stats", label: "what happened to past proposals" },
				] as const
			)
				.filter((option) => option.value.startsWith(prefix))
				.map((option) => ({ value: option.value, label: option.label })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const argument = args.trim().toLowerCase();
			if (argument && argument !== "all" && argument !== "stats") {
				ctx.ui.notify("Usage: /learn [all|stats]", "warning");
				return;
			}
			if (argument === "stats") {
				reportStats(ctx);
				return;
			}
			const ignoreState = argument === "all";

			// Read per-invocation so a settings edit takes effect without a reload,
			// and so a project settings.json can narrow the window for one repo.
			const agentDir = getHooCodeDir();
			const window = SettingsManager.create(ctx.cwd, agentDir).getLearnSettings();
			const sessionDir = ctx.sessionManager.getSessionDir();
			const statePath = getLearnStatePath(agentDir, sessionDir);

			let digest: ReturnType<typeof extractLearnDigest>;
			try {
				digest = extractLearnDigest({
					cwd: ctx.cwd,
					agentDir,
					// The live session manager already knows where this cwd's sessions
					// live, which avoids re-deriving (and re-creating) the directory.
					sessionDir,
					maxSessions: window.maxSessions,
					maxAgeDays: window.maxAgeDays,
					minRepeats: window.minRepeats,
					minWorkflowRepeats: window.minWorkflowRepeats,
					maxProposals: window.maxProposals,
					state: readLearnState(statePath),
					ignoreState,
				});
			} catch (error) {
				ctx.ui.notify(`/learn could not read session history: ${error}`, "error");
				return;
			}

			if (digest.scannedSessions === 0) {
				ctx.ui.notify("No recent sessions in this directory to learn from.", "warning");
				return;
			}

			if (isEmptyDigest(digest)) {
				ctx.ui.notify(
					digest.suppressed > 0
						? `Scanned ${digest.scannedSessions} session(s) — nothing new since last time (${digest.suppressed} already shown). Run /learn all to see them again.`
						: `Scanned ${digest.scannedSessions} session(s) — nothing repeated often enough to be worth a rule yet.`,
					"info",
				);
				return;
			}

			const counts = [
				digest.directives.length > 0 ? `${digest.directives.length} directive(s)` : undefined,
				digest.fixes.length > 0 ? `${digest.fixes.length} fix(es)` : undefined,
				digest.workflows.length > 0 ? `${digest.workflows.length} workflow(s)` : undefined,
			].filter((part): part is string => !!part);
			const held = digest.suppressed > 0 ? `, ${digest.suppressed} held back` : "";
			ctx.ui.notify(`Mined ${digest.scannedSessions} session(s): ${counts.join(", ")}${held}.`, "info");

			// Record before delivering: what matters is that these were put in front
			// of the user, which is true whether or not they act on the digest.
			writeLearnState(statePath, recordSurfaced(readLearnState(statePath), digest.surfaced));

			pi.sendUserMessage(renderLearnDigest(digest, { userScopePath: displayPath(USER_SCOPE_PATH) }), {
				deliverAs: "followUp",
			});
		},
	});
}
