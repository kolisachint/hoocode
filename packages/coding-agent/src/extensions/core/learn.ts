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
import { CONFIG_DIR_NAME, getHooCodeDir } from "../../config.js";
import { loadProjectContextFiles } from "../../core/context-files.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { isEmptyDigest, renderLearnDigest } from "../../core/learn/digest.js";
import {
	buildCoverageIndex,
	extractLearnDigest,
	type LearnDigest,
	matchCoverage,
	type SessionScanReport,
	scanSessions,
} from "../../core/learn/extract.js";
import {
	getLearnStatePath,
	readLearnState,
	recordSurfaced,
	summarizeLearnState,
	writeLearnState,
} from "../../core/learn/state.js";
import { getSessionDirPath } from "../../core/session-manager.js";
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

/** The settings keys `/learn` reads, paired with the values in force right now. */
type LearnWindow = ReturnType<SettingsManager["getLearnSettings"]>;

const SETTING_KEYS: Array<{ key: keyof LearnWindow; setting: string; note: string }> = [
	{ key: "maxSessions", setting: "learnMaxSessions", note: "recent sessions scanned" },
	{ key: "maxAgeDays", setting: "learnMaxAgeDays", note: "ignore sessions older than this, in days" },
	{ key: "minRepeats", setting: "learnMinRepeats", note: "times a directive must recur to be proposed" },
	{
		key: "minWorkflowRepeats",
		setting: "learnMinWorkflowRepeats",
		note: "repeats before a tool sequence is proposed",
	},
	{ key: "maxProposals", setting: "learnMaxProposals", note: "cap on each list in the digest" },
];

/**
 * Where the knobs live, and what they are set to.
 *
 * `/learn` has five settings and no UI, so until this existed the only way to
 * find them was to already know they were in `settings.json`. Every message that
 * reports a disappointing result names a threshold, so every one of them ends
 * with these lines.
 */
function settingsPathLines(ctx: ExtensionCommandContext, agentDir: string): string[] {
	return [
		"Settings — edit either file, no restart needed",
		`  user     ${displayPath(join(agentDir, "settings.json"))}`,
		`  project  ${displayPath(join(ctx.cwd, CONFIG_DIR_NAME, "settings.json"))}  (wins where both set a key)`,
	];
}

function settingsLines(ctx: ExtensionCommandContext, agentDir: string, window: LearnWindow): string[] {
	const lines = settingsPathLines(ctx, agentDir);
	for (const { key, setting, note } of SETTING_KEYS) {
		lines.push(`  ${setting.padEnd(24)} ${String(window[key]).padStart(3)}   ${note}`);
	}
	return lines;
}

/**
 * The directory whose name keys this cwd's bookmark.
 *
 * Derived from the cwd, never from the live session manager. An in-memory
 * session (`--no-session`) reports an empty session directory, which used to key
 * every such run to the same nameless state file, and a shared custom
 * `sessionDir` used to make two unrelated projects share one bookmark. The cwd
 * is what "per directory" means here, so the cwd is what it is keyed on.
 */
function stateKeyDir(ctx: ExtensionCommandContext, agentDir: string): string {
	return getSessionDirPath(ctx.cwd, agentDir);
}

/** Run the directory scan without ranking anything, for the reports that only need counts. */
function sessionScanPreview(ctx: ExtensionCommandContext, agentDir: string, window: LearnWindow): SessionScanReport {
	return scanSessions({
		cwd: ctx.cwd,
		agentDir,
		sessionDir: ctx.sessionManager.getSessionDir(),
		maxSessions: window.maxSessions,
		maxAgeDays: window.maxAgeDays,
	});
}

/** Where sessions were looked for, and what was passed over — the "why nothing?" answer. */
function scanLines(scan: SessionScanReport, window: LearnWindow): string[] {
	const lines: string[] = ["Looked in"];
	for (const dir of scan.dirs) {
		const missing = scan.missingDirs.includes(dir) ? "  (does not exist)" : "";
		lines.push(`  ${displayPath(dir)}${missing}`);
	}
	lines.push(`Found ${scan.files} session file(s)`);

	const skips: string[] = [];
	if (scan.tooOld > 0) skips.push(`${scan.tooOld} older than ${window.maxAgeDays} days (learnMaxAgeDays)`);
	if (scan.otherCwd > 0) skips.push(`${scan.otherCwd} recorded a different working directory`);
	if (scan.overLimit > 0) skips.push(`${scan.overLimit} beyond the newest ${window.maxSessions} (learnMaxSessions)`);
	if (scan.unreadable > 0) skips.push(`${scan.unreadable} empty or unreadable`);
	for (const skip of skips) lines.push(`  skipped: ${skip}`);
	return lines;
}

/**
 * Explain an empty scan rather than asserting there is no history.
 *
 * The old single sentence was wrong as often as it was right: sessions existed,
 * they were simply all outside the window or recorded under another path. Naming
 * the directory searched and the reason each file was passed over turns a dead
 * end into something the reader can fix.
 */
function reportNoSessions(ctx: ExtensionCommandContext, agentDir: string, digest: LearnDigest, window: LearnWindow) {
	const lines: string[] = [];
	lines.push(
		digest.scan.files === 0
			? "/learn found no session transcripts for this directory."
			: "/learn found session transcripts, but none inside the current window.",
	);
	lines.push("");
	lines.push(...scanLines(digest.scan, window));
	lines.push("");
	lines.push(...settingsLines(ctx, agentDir, window));
	ctx.ui.notify(lines.join("\n"), "warning");
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
	const window = SettingsManager.create(ctx.cwd, agentDir).getLearnSettings();
	const statePath = getLearnStatePath(agentDir, stateKeyDir(ctx, agentDir));
	const state = readLearnState(statePath);

	if (Object.keys(state.surfaced).length === 0) {
		// Nothing on record means `/learn` has never proposed anything here — which
		// is as likely to be "it never found any sessions" as "you never ran it", so
		// point at both the sessions it can see and the knobs that gate them.
		const lines = ["No /learn history for this directory yet — nothing has been proposed here."];
		lines.push(`  State file  ${displayPath(statePath)}  (not created yet)`);
		lines.push("");
		lines.push(...scanLines(sessionScanPreview(ctx, agentDir, window), window));
		lines.push("");
		lines.push(...settingsPathLines(ctx, agentDir));
		lines.push("  Run /learn settings for the thresholds in force.");
		ctx.ui.notify(lines.join("\n"), "info");
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
	lines.push(`State file          ${displayPath(statePath)}`);
	lines.push("");
	lines.push(...settingsPathLines(ctx, agentDir));
	lines.push("  Run /learn settings for the thresholds in force.");

	ctx.ui.notify(lines.join("\n"), "info");
}

/** `/learn settings` — the knobs, their current values, and the files to set them in. */
function reportSettings(ctx: ExtensionCommandContext): void {
	const agentDir = getHooCodeDir();
	const window = SettingsManager.create(ctx.cwd, agentDir).getLearnSettings();
	const lines = settingsLines(ctx, agentDir, window);
	lines.push("");
	lines.push(...scanLines(sessionScanPreview(ctx, agentDir, window), window));
	lines.push(`State file  ${displayPath(getLearnStatePath(agentDir, stateKeyDir(ctx, agentDir)))}`);
	ctx.ui.notify(lines.join("\n"), "info");
}

export function setupLearn(pi: ExtensionAPI): void {
	const guarded = pi as unknown as Record<symbol, boolean>;
	if (guarded[REGISTERED]) return;
	guarded[REGISTERED] = true;

	pi.registerCommand("learn", {
		description: "Mine recent sessions for durable rules and skills. Usage: /learn [all|stats|settings]",
		getArgumentCompletions: (prefix: string) =>
			(
				[
					{ value: "all", label: "re-propose everything" },
					{ value: "stats", label: "what happened to past proposals" },
					{ value: "settings", label: "where sessions are read from, and the knobs" },
				] as const
			)
				.filter((option) => option.value.startsWith(prefix))
				.map((option) => ({ value: option.value, label: option.label })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const argument = args.trim().toLowerCase();
			if (argument && argument !== "all" && argument !== "stats" && argument !== "settings") {
				ctx.ui.notify("Usage: /learn [all|stats|settings]", "warning");
				return;
			}
			if (argument === "stats") {
				reportStats(ctx);
				return;
			}
			if (argument === "settings") {
				reportSettings(ctx);
				return;
			}
			const ignoreState = argument === "all";

			// Read per-invocation so a settings edit takes effect without a reload,
			// and so a project settings.json can narrow the window for one repo.
			const agentDir = getHooCodeDir();
			const window = SettingsManager.create(ctx.cwd, agentDir).getLearnSettings();
			const statePath = getLearnStatePath(agentDir, stateKeyDir(ctx, agentDir));

			let digest: LearnDigest;
			try {
				digest = extractLearnDigest({
					cwd: ctx.cwd,
					agentDir,
					// Searched in addition to the per-cwd default directory, so a session
					// manager pointing elsewhere (`--session`, a custom `sessionDir`, or
					// an in-memory session reporting none at all) cannot hide the history.
					sessionDir: ctx.sessionManager.getSessionDir(),
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
				reportNoSessions(ctx, agentDir, digest, window);
				return;
			}

			if (isEmptyDigest(digest)) {
				const lines: string[] = [];
				lines.push(
					digest.suppressed > 0
						? `Scanned ${digest.scannedSessions} session(s) — nothing new since last time (${digest.suppressed} already shown). Run /learn all to see them again.`
						: `Scanned ${digest.scannedSessions} session(s) — nothing repeated often enough to be worth a rule yet.`,
				);
				if (digest.suppressed === 0) {
					// The thresholds are the reason a scan with real sessions in it came
					// back empty, so this is the moment they are worth knowing about.
					lines.push("");
					lines.push(...settingsLines(ctx, agentDir, window));
				}
				ctx.ui.notify(lines.join("\n"), "info");
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
