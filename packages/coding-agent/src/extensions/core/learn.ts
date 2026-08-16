/**
 * `/learn` — promote what recent sessions actually taught into durable rules
 * and skills.
 *
 * The command is a thin shell on purpose. It runs the mining pipeline over
 * session transcripts on disk, renders the ranked result, and injects it as a
 * follow-up message; every judgement after that belongs to the model, which can
 * read the repo and phrase a rule far better than a heuristic can.
 *
 * Reading transcripts from disk rather than the live context is what makes this
 * work: the on-disk history survives compaction, and it spans past sessions, so
 * "you have said this in five separate sessions" is available as a number
 * instead of a guess. That number is the whole reason the command exists.
 *
 * The pipeline reads every transcript with a model rather than pre-filtering
 * with regexes, which costs real tokens on a cold cache. That price is stated
 * before it is paid, never inferred: a run with sessions to read asks first.
 *
 * Follows /grill in modes.ts: no session switch, no mode change, no config
 * write — just a follow-up message. Writes to AGENTS.md happen through ordinary
 * edit tools, so the existing permission prompt is the approval step and no
 * separate picker is needed.
 */

import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Api, Model } from "@kolisachint/hoocode-ai";
import { CONFIG_DIR_NAME, getHooCodeDir } from "../../config.js";
import { loadProjectContextFiles } from "../../core/context-files.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { auditContextFiles, staleTokens } from "../../core/learn/audit.js";
import type { Clusterer } from "../../core/learn/cluster.js";
import { createLlmClusterer } from "../../core/learn/cluster.js";
import type { CoverageJudge } from "../../core/learn/coverage.js";
import { createLlmCoverageJudge } from "../../core/learn/coverage.js";
import { isEmptyDigest, renderAuditReport, renderLearnDigest } from "../../core/learn/digest.js";
import {
	type LearnDigest,
	mineLearnDigest,
	planMining,
	type SessionScanReport,
	scanSessions,
} from "../../core/learn/extract.js";
import type { Miner } from "../../core/learn/mine.js";
import { chunkCharsForModel, createLlmMiner, replayFingerprints } from "../../core/learn/mine.js";
import {
	getLearnStatePath,
	readLearnState,
	recordSurfaced,
	summarizeLearnState,
	writeLearnState,
} from "../../core/learn/state.js";
import { resolveModelCategory } from "../../core/model-categories.js";

import { getSessionDirPath } from "../../core/session-manager.js";
import { SettingsManager } from "../../core/settings-manager.js";
import { startupProgress } from "../../core/startup-progress.js";

/** Guards against double-registration when default extensions load more than once. */
const REGISTERED = Symbol.for("hoocode.learn.registered");

/** User-scope destination offered for personal rules that travel across repos. */
const USER_SCOPE_PATH = join(homedir(), ".agents", "AGENTS.md");

/** Footer key for the mining progress bar. */
const PROGRESS_KEY = "learn-mining";

/** Escape, the way a raw terminal delivers it. */
const ESCAPE = "\x1b";

/**
 * Sessions that can be read without asking first.
 *
 * A run that has one or two new transcripts to read is the normal daily case
 * and interrupting it to confirm a trivial cost is noise. Beyond this the run
 * is a backfill — onboarding to an existing repo, or a first run — and the
 * reader should get to decide before it starts.
 */
const CONFIRM_ABOVE_PENDING = 3;

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
		key: "minRequestRepeats",
		setting: "learnMinRequestRepeats",
		note: "repeats before a tool sequence is proposed",
	},
	{ key: "maxProposals", setting: "learnMaxProposals", note: "cap on each list in the digest" },
];

/**
 * Where the knobs live, and what they are set to.
 *
 * `/learn` has its settings and no UI, so until this existed the only way to
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
		lines.push(`  ${setting.padEnd(24)} ${String(window[key] ?? "—").padStart(3)}   ${note}`);
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

/** Run the directory scan without mining anything, for the reports that only need counts. */
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
 * The model that reads transcripts.
 *
 * This is the one call in the pipeline that reads *everything*, so it wants the
 * cheapest capable model rather than the session's. That question already has an
 * answer in this codebase — the `fast` model category, which subagents use for
 * exactly this kind of bulk read — so it is reused rather than reinvented.
 * `settings.modelCategories.fast` wins when set; otherwise the tier is derived
 * from the user's available models, and nothing here is provider-specific.
 *
 * Falls back to the session model when the tier resolves to nothing or to a
 * model the registry cannot find, since a mis-set tier should not take the
 * command out entirely.
 */
function resolveMinerModel(ctx: ExtensionCommandContext, settings: SettingsManager): Model<Api> | undefined {
	const ref = resolveModelCategory(
		"fast",
		{
			modelCategories: settings.getModelCategories(),
			defaultProvider: settings.getDefaultProvider(),
			defaultModel: settings.getDefaultModel(),
		},
		ctx.modelRegistry.getAvailable(),
	);
	if (!ref) return ctx.model;

	const slash = ref.indexOf("/");
	const found = slash > 0 ? ctx.modelRegistry.find(ref.slice(0, slash), ref.slice(slash + 1)) : undefined;
	return found ?? ctx.model;
}

/**
 * Literal runs from the slash commands in force, so the miner can tell a
 * command body replaying itself from something the user typed.
 *
 * Read from the session's own command list rather than re-deriving the search
 * path: which directories are scanned, in which order, and which flags disable
 * them is a precedence list that lives in one place and would drift the moment
 * it lived in two.
 */
function loadReplayFingerprints(pi: ExtensionAPI): string[] {
	const bodies: Array<{ content: string }> = [];
	for (const command of pi.getCommands()) {
		const path = command.sourceInfo?.path;
		// A built-in has no file behind it, and nothing to replay.
		if (!path || !existsSync(path)) continue;
		try {
			bodies.push({ content: readFileSync(path, "utf-8") });
		} catch {
			// Unreadable command file: one fewer fingerprint, not a failed run.
		}
	}
	return replayFingerprints(bodies);
}

/** Build the two model-backed stages, or report why they cannot be built. */
async function buildPipeline(
	ctx: ExtensionCommandContext,
	settings: SettingsManager,
	/** Empty for callers that only need the coverage judge; mining wants the real set. */
	fingerprints: string[] = [],
): Promise<
	{ miner: Miner; clusterer: Clusterer; coverageJudge: CoverageJudge; model: Model<Api> } | { error: string }
> {
	const model = resolveMinerModel(ctx, settings);
	if (!model) {
		return {
			error: "/learn reads session transcripts with a model, and no model is selected. Pick one with /model, then run /learn again.",
		};
	}

	const auth = await ctx.modelRegistry.getApiKeyAndHeaders(model);
	if (!auth.ok) {
		return { error: `/learn could not authenticate ${model.provider}/${model.id}: ${auth.error}` };
	}

	const deps = {
		model,
		apiKey: auth.apiKey,
		headers: auth.headers,
		replayFingerprints: fingerprints,
	};
	return {
		miner: createLlmMiner(deps),
		clusterer: createLlmClusterer(deps),
		coverageJudge: createLlmCoverageJudge(deps),
		model,
	};
}

/**
 * What this run still owes the model.
 *
 * Delegated to `planMining` so the number quoted by the confirmation prompt
 * comes from the same session selection the run will use — same window, same
 * cwd check, same de-duplication.
 */
function pendingWork(ctx: ExtensionCommandContext, agentDir: string, window: LearnWindow) {
	return planMining({
		cwd: ctx.cwd,
		agentDir,
		sessionDir: ctx.sessionManager.getSessionDir(),
		maxSessions: window.maxSessions,
		maxAgeDays: window.maxAgeDays,
	});
}

/**
 * `/learn stats` — what has been proposed here, and what it costs.
 *
 * Reads the state file and the context files. No model call: this used to
 * re-judge coverage and report an "adoption rate", which was unreliable in both
 * directions and shipped with two disclaimers explaining how not to misread it.
 * The honest version of the question it was trying to answer — is the
 * always-loaded surface growing — is a number the filesystem can answer exactly.
 */
function reportStats(ctx: ExtensionCommandContext): void {
	const agentDir = getHooCodeDir();
	const settings = SettingsManager.create(ctx.cwd, agentDir);
	const window = settings.getLearnSettings();
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

	const stats = summarizeLearnState(state);

	const contextFiles = loadProjectContextFiles({ cwd: ctx.cwd, agentDir }).agentsFiles;
	const contextTokens = contextFiles.reduce((sum, file) => sum + (file.tokens ?? 0), 0);

	const lines: string[] = [];
	lines.push(`/learn history for this directory — ${shortDate(stats.earliest)} to ${shortDate(stats.latest)}`);
	lines.push(
		`  Proposals shown   ${stats.total}  (${stats.directives} directive, ${stats.fixes} fix, ${stats.requests} request)`,
	);
	if (stats.lastRun) lines.push(`  Last run          ${shortDate(stats.lastRun)}`);
	lines.push("");

	// The one number worth watching, and the only one here that is exact. Mining
	// can only push it up; `/learn stale` is what pushes it down.
	lines.push(`Always-loaded cost  ~${contextTokens} tokens across ${contextFiles.length} context file(s)`);
	for (const file of contextFiles) {
		lines.push(`  ~${file.tokens ?? 0}  ${displayPath(file.path)}`);
	}
	lines.push("  Run /learn stale to find lines naming something that no longer exists.");
	lines.push("");
	lines.push(`State file          ${displayPath(statePath)}`);
	lines.push("");
	lines.push(...settingsPathLines(ctx, agentDir));
	lines.push("  Run /learn settings for the thresholds in force.");

	ctx.ui.notify(lines.join("\n"), "info");
}

/**
 * `/learn stale` — which lines in the context files name something that is gone.
 *
 * The mining path can only propose additions, so this is the only half of the
 * command that moves the always-loaded token surface down. It is deterministic
 * and costs nothing, which is what makes it the half worth running often; the
 * findings go to the model only when there are some, so a clean audit is free.
 */
function reportAudit(pi: ExtensionAPI, ctx: ExtensionCommandContext): void {
	const agentDir = getHooCodeDir();
	const { agentsFiles } = loadProjectContextFiles({ cwd: ctx.cwd, agentDir });

	if (agentsFiles.length === 0) {
		ctx.ui.notify("/learn stale found no context files to check (no AGENTS.md or CLAUDE.md is in force).", "warning");
		return;
	}

	const report = auditContextFiles({ cwd: ctx.cwd, files: agentsFiles });

	if (report.files.length === 0) {
		const lines = ["/learn stale checked nothing — every context file in force is outside this working tree."];
		for (const path of report.skippedFiles) lines.push(`  ${displayPath(path)}`);
		lines.push("A rule written in a user-scope file names paths in whatever repo it was written for, not this one.");
		ctx.ui.notify(lines.join("\n"), "info");
		return;
	}

	const totalTokens = report.files.reduce((sum, file) => sum + file.tokens, 0);
	if (report.stale.length === 0) {
		ctx.ui.notify(
			`/learn stale — ${report.checked} referent(s) in ${report.files.length} context file(s) all resolve. ` +
				`~${totalTokens} tokens, re-sent every request.`,
			"info",
		);
		return;
	}

	ctx.ui.notify(
		`/learn stale — ${report.stale.length} of ${report.checked} referent(s) do not resolve ` +
			`(~${staleTokens(report)} of ~${totalTokens} always-loaded tokens).`,
		"info",
	);
	pi.sendUserMessage(renderAuditReport(report), { deliverAs: "followUp" });
}

/** `/learn settings` — the knobs, their current values, and the files to set them in. */
function reportSettings(ctx: ExtensionCommandContext): void {
	const agentDir = getHooCodeDir();
	const settings = SettingsManager.create(ctx.cwd, agentDir);
	const window = settings.getLearnSettings();
	const lines = settingsLines(ctx, agentDir, window);

	// The reading model is not a `/learn` setting — it is the shared `fast` tier,
	// so name it here rather than leaving the reader to guess which model is
	// about to read their history, and point at the setting that changes it.
	const model = resolveMinerModel(ctx, settings);
	lines.push(
		`  reads transcripts with  ${model ? `${model.provider}/${model.id}` : "no model selected"}` +
			`   (the \`fast\` tier — set modelCategories.fast to change it)`,
	);
	if (model) {
		lines.push(
			`  ${Math.round(chunkCharsForModel(model) / 1000)}k characters per call, from its ${model.contextWindow} token window`,
		);
	}

	lines.push("");
	lines.push(...scanLines(sessionScanPreview(ctx, agentDir, window), window));
	lines.push(`State file  ${displayPath(getLearnStatePath(agentDir, stateKeyDir(ctx, agentDir)))}`);
	const { pending } = pendingWork(ctx, agentDir, window);
	lines.push(
		pending === 0
			? "All sessions in the window are already mined; the next /learn costs one small coverage call."
			: `${pending} session(s) in the window still need reading, roughly one call each.`,
	);
	ctx.ui.notify(lines.join("\n"), "info");
}

export function setupLearn(pi: ExtensionAPI): void {
	const guarded = pi as unknown as Record<symbol, boolean>;
	if (guarded[REGISTERED]) return;
	guarded[REGISTERED] = true;

	pi.registerCommand("learn", {
		description: "Mine recent sessions for durable rules and skills. Usage: /learn [all|stale|stats|settings]",
		getArgumentCompletions: (prefix: string) =>
			(
				[
					{ value: "all", label: "re-propose everything" },
					{ value: "stale", label: "context-file lines naming something that is gone" },
					{ value: "stats", label: "what happened to past proposals" },
					{ value: "settings", label: "where sessions are read from, and the knobs" },
				] as const
			)
				.filter((option) => option.value.startsWith(prefix))
				.map((option) => ({ value: option.value, label: option.label })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const argument = args.trim().toLowerCase();
			if (argument && !["all", "stale", "stats", "settings"].includes(argument)) {
				ctx.ui.notify("Usage: /learn [all|stale|stats|settings]", "warning");
				return;
			}
			if (argument === "stale") {
				reportAudit(pi, ctx);
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
			const settings = SettingsManager.create(ctx.cwd, agentDir);
			const window = settings.getLearnSettings();
			const statePath = getLearnStatePath(agentDir, stateKeyDir(ctx, agentDir));

			const pipeline = await buildPipeline(ctx, settings, loadReplayFingerprints(pi));
			if ("error" in pipeline) {
				ctx.ui.notify(pipeline.error, "error");
				return;
			}

			// State the price before charging it. A first run in a busy repo reads
			// every transcript in the window, which is the expensive path by design
			// — but it should never be a surprise, and the cache means it is paid
			// once rather than on every run.
			const { pending } = pendingWork(ctx, agentDir, window);
			if (pending > CONFIRM_ABOVE_PENDING) {
				const proceed = await ctx.ui.confirm(
					"Read session transcripts?",
					`${pending} session(s) have not been read yet. /learn reads each one with a model ` +
						`(${pipeline.model.provider}/${pipeline.model.id}) and caches the result, so this cost is paid once ` +
						`per session. Later runs reuse it.`,
				);
				if (!proceed) {
					ctx.ui.notify("/learn cancelled — nothing was read.", "info");
					return;
				}
			}

			// A backfill can run for minutes across dozens of transcripts, and the
			// agent is idle throughout — so `ctx.signal` is undefined and there is no
			// ambient way out. Escape gets one.
			const controller = new AbortController();
			const unsubscribe = ctx.ui.onTerminalInput((data) => {
				if (data !== ESCAPE) return undefined;
				controller.abort();
				return { consume: true };
			});

			let digest: LearnDigest;
			try {
				digest = await mineLearnDigest({
					cwd: ctx.cwd,
					agentDir,
					// Searched in addition to the per-cwd default directory, so a session
					// manager pointing elsewhere (`--session`, a custom `sessionDir`, or
					// an in-memory session reporting none at all) cannot hide the history.
					sessionDir: ctx.sessionManager.getSessionDir(),
					maxSessions: window.maxSessions,
					maxAgeDays: window.maxAgeDays,
					minRepeats: window.minRepeats,
					minRequestRepeats: window.minRequestRepeats,
					maxProposals: window.maxProposals,
					state: readLearnState(statePath),
					ignoreState,
					miner: pipeline.miner,
					clusterer: pipeline.clusterer,
					coverageJudge: pipeline.coverageJudge,
					signal: controller.signal,
					onProgress: ({ done, total, cached }) => {
						// The same footer bar the semantic index uses. Cached sessions are
						// counted as done because they are: the bar measures progress
						// through the window, not money spent, and a run that is mostly
						// cache should look nearly finished from the start.
						startupProgress.set({
							key: PROGRESS_KEY,
							kind: "work",
							label:
								cached > 0
									? `Reading sessions (${cached} cached) — esc to stop`
									: "Reading sessions — esc to stop",
							done,
							total,
							unit: "sessions",
						});
					},
				});
			} catch (error) {
				ctx.ui.notify(`/learn could not read session history: ${error}`, "error");
				return;
			} finally {
				unsubscribe();
				startupProgress.remove(PROGRESS_KEY);
			}

			// A cancelled run counted only part of the window, so its numbers are not
			// merely incomplete — they are low. Showing them would be misleading and
			// bookmarking them would hide those items on the next, complete run.
			// Everything read so far is cached, so stopping costs nothing but time.
			if (digest.aborted) {
				ctx.ui.notify(
					`/learn stopped — ${digest.mining.mined} session(s) were read and cached, so resuming picks up where this left off.`,
					"info",
				);
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
						? `Read ${digest.scannedSessions} session(s) — nothing new since last time (${digest.suppressed} already shown). Run /learn all to see them again.`
						: `Read ${digest.scannedSessions} session(s) — nothing repeated often enough to be worth a rule yet.`,
				);
				if (digest.suppressed === 0) {
					// Which of the two empty results this is. "Nothing was said" and "a
					// lot was said and none of it repeated" read identically otherwise,
					// and they point at completely different knobs.
					lines.push(
						`  ${digest.funnel.candidates} occurrence(s) → ${digest.funnel.points} distinct point(s) → ` +
							`${digest.funnel.belowThreshold} below the repeat threshold`,
					);
					lines.push("");
					lines.push(...settingsLines(ctx, agentDir, window));
				}
				ctx.ui.notify(lines.join("\n"), "info");
				return;
			}

			const counts = [
				digest.directives.length > 0 ? `${digest.directives.length} directive(s)` : undefined,
				digest.fixes.length > 0 ? `${digest.fixes.length} fix(es)` : undefined,
				digest.requests.length > 0 ? `${digest.requests.length} request(s)` : undefined,
			].filter((part): part is string => !!part);
			const held = digest.suppressed > 0 ? `, ${digest.suppressed} held back` : "";
			const cut = digest.cut > 0 ? `, ${digest.cut} cut to fit the cap` : "";
			ctx.ui.notify(
				`Mined ${digest.scannedSessions} session(s) (${digest.mining.mined} read, ${digest.mining.cached} cached): ${counts.join(", ")}${held}${cut}.`,
				"info",
			);

			// Record before delivering: what matters is that these were put in front
			// of the user, which is true whether or not they act on the digest.
			//
			// Unless coverage could not be read. The bookmark stores whether an item
			// was already written down when it was shown, and that is what later tells
			// an adopted proposal from one passed over. Recording a guess as a reading
			// would have a later run tell the user they passed on something they were
			// never shown. Skipping costs one round of re-proposing.
			if (!digest.coverageFailed) {
				writeLearnState(statePath, recordSurfaced(readLearnState(statePath), digest.surfaced));
			}

			pi.sendUserMessage(
				renderLearnDigest(digest, {
					userScopePath: displayPath(USER_SCOPE_PATH),
					mode: ignoreState ? "all" : "incremental",
				}),
				{ deliverAs: "followUp" },
			);
		},
	});
}
