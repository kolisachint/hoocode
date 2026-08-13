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
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";
import { isEmptyDigest, renderLearnDigest } from "../../core/learn/digest.js";
import { extractLearnDigest } from "../../core/learn/extract.js";

/** Guards against double-registration when default extensions load more than once. */
const REGISTERED = Symbol.for("hoocode.learn.registered");

/** User-scope destination offered for personal rules that travel across repos. */
const USER_SCOPE_PATH = join(homedir(), ".agents", "AGENTS.md");

/** Render a home-relative path the way the user would type it. */
function displayPath(path: string): string {
	const home = homedir();
	return path.startsWith(home) ? `~${path.slice(home.length)}` : path;
}

export function setupLearn(pi: ExtensionAPI): void {
	const guarded = pi as unknown as Record<symbol, boolean>;
	if (guarded[REGISTERED]) return;
	guarded[REGISTERED] = true;

	pi.registerCommand("learn", {
		description: "Mine recent sessions for durable rules and skills, and update AGENTS.md",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			let digest: ReturnType<typeof extractLearnDigest>;
			try {
				digest = extractLearnDigest({
					cwd: ctx.cwd,
					agentDir: getHooCodeDir(),
					// The live session manager already knows where this cwd's sessions
					// live, which avoids re-deriving (and re-creating) the directory.
					sessionDir: ctx.sessionManager.getSessionDir(),
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
					`Scanned ${digest.scannedSessions} session(s) — nothing repeated often enough to be worth a rule yet.`,
					"info",
				);
				return;
			}

			const counts = [
				digest.directives.length > 0 ? `${digest.directives.length} directive(s)` : undefined,
				digest.fixes.length > 0 ? `${digest.fixes.length} fix(es)` : undefined,
				digest.workflows.length > 0 ? `${digest.workflows.length} workflow(s)` : undefined,
			].filter((part): part is string => !!part);
			ctx.ui.notify(`Mined ${digest.scannedSessions} session(s): ${counts.join(", ")}.`, "info");

			pi.sendUserMessage(renderLearnDigest(digest, { userScopePath: displayPath(USER_SCOPE_PATH) }), {
				deliverAs: "followUp",
			});
		},
	});
}
