/**
 * Tips, and the one place they are written down.
 *
 * ## Why this exists
 *
 * HooCode has four modes, ~40 slash commands, a rebindable key for most dials,
 * skills, plugins, canvases and a subagent runner. None of that is discoverable
 * by using the product: the prompt looks like a prompt, so people find `/help`,
 * find three things, and use those three things forever. The features that
 * would have saved them the most time are exactly the ones they never learn
 * exist.
 *
 * The fix is not a longer help page — it is putting one small thing in front of
 * someone at a moment when they are not busy. There are two such moments in a
 * session and the band above the prompt already owns both of them:
 *
 *   - **idle**: the prompt is up, nothing is streaming, and the user has not
 *     touched a key in a while. They are reading, or thinking, or away.
 *   - **streaming**: the agent has been working for long enough that the user is
 *     watching a spinner. That time is already spent; spending it on one line
 *     costs nothing.
 *
 * ## The rules a tip obeys
 *
 * A tip is the lowest-priority thing on the screen, and the rules below all come
 * from that one fact:
 *
 *   - It never interrupts. `TipsController` posts only when the band is empty,
 *     so a tip can never push aside, delay, or replace something the user
 *     actually caused.
 *   - It never repeats until everything else has been said. The rotation walks
 *     unseen tips first and remembers across sessions, so a returning user does
 *     not get taught `alt+a` five times.
 *   - It is short. Headline plus at most two rows, the same budget any glimpse
 *     gets, because it fades on the same clock.
 *   - It can be turned off, in one place, forever (`/settings`, or
 *     `tips.enabled: false`).
 *
 * ## Keys are resolved late
 *
 * A tip that names a key reads it out of the live keybinding manager at display
 * time rather than baking a string in here. Someone who rebound the mode dial
 * should be taught *their* key, and a tip that teaches the wrong one is worse
 * than no tip.
 *
 * ## Adding a tip
 *
 * Add a row to {@link TIPS}. Give it an id that will never be reused (the id is
 * what "already seen" is stored against, so renaming one re-teaches it to
 * everybody). Keep the headline under ~50 columns so it survives a narrow
 * terminal, and put anything longer in `body`.
 */

import { keyText } from "./components/keybinding-hints.js";

/** Where a tip is allowed to appear. Most are fine in both. */
export type TipMoment = "idle" | "streaming";

export interface Tip {
	/**
	 * Stable forever. "Seen" is recorded against this, so changing an id
	 * re-teaches the tip to every existing user.
	 */
	id: string;
	/** One line. Keep it short enough to survive a narrow terminal. */
	title: string;
	/**
	 * Rows under the headline. A function when the text names a key or anything
	 * else that is read from live config — it is called at display time.
	 */
	body?: string[] | (() => string[]);
	/** Right-aligned afterword, usually the key or command being taught. */
	note?: string;
	/**
	 * Which moments this tip suits. Defaults to both. A tip about interrupting a
	 * turn is only useful while a turn is running; one about starting a session
	 * is only useful when nothing is.
	 */
	moments?: readonly TipMoment[];
}

/**
 * The tips.
 *
 * Ordered roughly by how soon a new user benefits: the dials and the two
 * commands that answer "what else is there" first, then session shape, then the
 * extension surfaces, then the things you only want once you are living in it.
 * The rotation walks this order for anyone who has seen nothing.
 */
export const TIPS: readonly Tip[] = [
	{
		id: "modes",
		title: "Four modes, one key",
		body: () => [
			`${keyText("app.mode.cycleForward")} cycles ask → plan → build → debug.`,
			"Ask never edits. Plan writes a plan, not code.",
		],
		note: "/mode",
	},
	{
		id: "thinking",
		title: "Turn the thinking dial up for hard problems",
		body: () => [`${keyText("app.thinking.cycleForward")} steps off → minimal → low → medium → high.`],
		note: "/settings",
	},
	{
		id: "hotkeys",
		title: "Every shortcut, on one screen",
		body: () => [`${keyText("app.hotkeys.open")} or /hotkeys. They are all rebindable.`],
		note: "/hotkeys",
	},
	{
		id: "settings",
		title: "Settings live behind one key",
		body: () => [`${keyText("app.settings.open")} or /settings — models, chrome, search, voice, tools.`],
		note: "/settings",
	},
	{
		id: "at-files",
		title: "Type @ to point at a file",
		body: ["Fuzzy path completion, so you never paste a path again."],
		note: "@",
	},
	{
		id: "interrupt",
		title: "Going the wrong way? Stop it.",
		body: () => [`${keyText("app.interrupt")} aborts the turn. What it already did stays.`],
		moments: ["streaming"],
	},
	{
		id: "queue",
		title: "You can type while it works",
		body: ["Messages you send mid-turn queue up and go next."],
		moments: ["streaming"],
	},
	{
		id: "hoo-alias",
		title: "`hoo` is the same thing as `hoocode`",
		body: ["Four fewer keys, several times a day."],
	},
	{
		id: "fork",
		title: "Fork rather than re-explain",
		body: ["/fork rewinds to an earlier message and branches from there."],
		note: "/fork",
	},
	{
		id: "tree",
		title: "A session is a tree, not a line",
		body: ["/tree walks the branches you have made and switches between them."],
		note: "/tree",
	},
	{
		id: "resume",
		title: "Yesterday's session is still there",
		body: () => [`${keyText("app.session.resume")} or /resume picks it back up where you left it.`],
		note: "/resume",
	},
	{
		id: "compact",
		title: "Long session slowing down?",
		body: ["/compact summarises the context and keeps going."],
		note: "/compact",
		moments: ["idle"],
	},
	{
		id: "search",
		title: "Search ranks by meaning, not just by keyword",
		body: ["The search tool fuses lexical and semantic hits when embsearch is installed."],
	},
	{
		id: "agents-md",
		title: "Teach it your project once",
		body: ["An AGENTS.md at the repo root is read every session. Rules, conventions, gotchas."],
		note: "AGENTS.md",
	},
	{
		id: "learn",
		title: "It can write its own AGENTS.md",
		body: ["/learn reads your past sessions and proposes rules from what actually kept happening."],
		note: "/learn",
	},
	{
		id: "plugins",
		title: "One-click plugins",
		body: ["/plugin browses the marketplaces and installs into this project or your user config."],
		note: "/plugin",
	},
	{
		id: "skills",
		title: "Skills are just a folder and a SKILL.md",
		body: ["/new-skill scaffolds one. /reload picks it up without restarting."],
		note: "/new-skill",
	},
	{
		id: "subagent",
		title: "Hand a side-quest to a subagent",
		body: ["/subagent <mode> <task> runs it in its own context and reports back."],
		note: "/subagent",
	},
	{
		id: "canvas",
		title: "Canvases put a UI in the terminal",
		body: ["/canvas opens one. /new-canvas starts your own."],
		note: "/canvas",
	},
	{
		id: "export",
		title: "Take the session with you",
		body: ["/export writes styled HTML (or .jsonl). /share puts it in a secret gist."],
		note: "/export",
	},
	{
		id: "copy",
		title: "Copy the reply, not a picture of it",
		body: ["/copy gives you real markdown — /copy all, or /copy <turns>."],
		note: "/copy",
	},
	{
		id: "models",
		title: "Swap models mid-session",
		body: ["/model picks one. /scoped-models chooses which ones the cycle key walks."],
		note: "/model",
	},
	{
		id: "cd",
		title: "Move without leaving",
		body: ["/cd <path> starts a session there. Bare /cd goes home, /cd - goes back."],
		note: "/cd",
	},
	{
		id: "chrome",
		title: "Small terminal? Take the chrome back",
		body: ["/chrome compact, or /chrome bare to get every row for the conversation."],
		note: "/chrome",
	},
	{
		id: "offline",
		title: "It works with no network",
		body: ["HOOCODE_OFFLINE=1 skips every startup fetch; search and completion fall back to pure JS."],
	},
	{
		id: "external-tools",
		title: "fd and rg make everything faster",
		body: ["Already on your PATH? HooCode uses them. Otherwise it fetches them once, quietly."],
	},
	{
		id: "reload",
		title: "No need to restart",
		body: ["/reload re-reads keybindings, extensions, skills, prompts and themes."],
		note: "/reload",
	},
	{
		id: "cost",
		title: "Know what a turn cost",
		body: ["/cost breaks down tokens and spend for this session."],
		note: "/cost",
	},
	{
		id: "issues",
		title: "Something broken or missing?",
		body: [
			"Open an issue — bug reports and feature ideas are both welcome.",
			"github.com/kolisachint/hoocode/issues",
		],
		moments: ["idle"],
	},
];

/**
 * The star nudge.
 *
 * Not in {@link TIPS} because it is not a tip and must not be rationed like
 * one: it is an ask, it is the only line here that wants something rather than
 * gives something, and so it gets its own budget — rare, capped, and never
 * twice in a session. Treating it as one more row in the rotation would have it
 * come round as often as `/copy`, which is how a nudge becomes an advert.
 */
export const STAR_NUDGE: Tip = {
	id: "star",
	title: "★ Enjoying HooCode? Star it.",
	body: ["It is the cheapest way to help people find it.", "github.com/kolisachint/hoocode"],
	moments: ["idle"],
};

/** How many times, ever, the star nudge may be shown. */
export const STAR_NUDGE_LIMIT = 3;

/**
 * How many tips go by between star nudges.
 *
 * Together with the per-session cap of one, this is what keeps the ask on the
 * right side of the line: someone who uses HooCode daily sees it three times in
 * their life, and someone who tries it once sees it at most once.
 */
const STAR_NUDGE_EVERY = 6;

export interface TipRotationOptions {
	/** Overridable for tests. */
	tips?: readonly Tip[];
	/** Tip ids already shown, across every session. */
	seen: () => readonly string[];
	/** Record that a tip has now been shown. */
	markSeen: (id: string) => void;
	/** How many times the star nudge has been shown, across every session. */
	starNudgeCount: () => number;
	/** Record one more star nudge. */
	markStarNudge: () => void;
}

/**
 * Picks what to say next, and remembers what it has said.
 *
 * Kept apart from the controller that schedules it because *what* to show and
 * *when* to show it are different problems with different tests: this half is
 * pure, has no timers, and can be walked end to end in a unit test.
 */
export class TipRotation {
	private readonly tips: readonly Tip[];
	private readonly opts: TipRotationOptions;
	/** Shown since this process started — the rotation never repeats within a run. */
	private readonly shownThisSession = new Set<string>();
	/** Tips emitted since the last star nudge, for the cadence above. */
	private sinceStarNudge = 0;
	/** The ask is once per session, whatever the lifetime budget says. */
	private starNudgedThisSession = false;

	constructor(options: TipRotationOptions) {
		this.opts = options;
		this.tips = options.tips ?? TIPS;
	}

	/**
	 * The next thing to show at this moment, or undefined when there is nothing
	 * left worth saying.
	 *
	 * Unseen tips come first and in declaration order, which is roughly "what
	 * helps a new user soonest". Once everything has been seen the rotation
	 * starts over, skipping only what this run has already shown — a session
	 * long enough to exhaust the list has earned a repeat, and silence would
	 * read as the feature having broken.
	 */
	next(moment: TipMoment): Tip | undefined {
		if (this.shouldNudgeStar(moment)) {
			this.sinceStarNudge = 0;
			this.starNudgedThisSession = true;
			this.opts.markStarNudge();
			return STAR_NUDGE;
		}

		const tip = this.pick(moment);
		if (!tip) return undefined;

		this.shownThisSession.add(tip.id);
		this.sinceStarNudge += 1;
		this.opts.markSeen(tip.id);
		return tip;
	}

	private pick(moment: TipMoment): Tip | undefined {
		const eligible = this.tips.filter((tip) => suitsMoment(tip, moment) && !this.shownThisSession.has(tip.id));
		if (eligible.length === 0) return undefined;

		const seen = new Set(this.opts.seen());
		return eligible.find((tip) => !seen.has(tip.id)) ?? eligible[0];
	}

	private shouldNudgeStar(moment: TipMoment): boolean {
		if (this.starNudgedThisSession) return false;
		if (!suitsMoment(STAR_NUDGE, moment)) return false;
		if (this.opts.starNudgeCount() >= STAR_NUDGE_LIMIT) return false;
		// Never the first thing someone sees. Asking for a star before having
		// been useful once is the whole reason these prompts are resented.
		return this.sinceStarNudge >= STAR_NUDGE_EVERY;
	}
}

function suitsMoment(tip: Tip, moment: TipMoment): boolean {
	return tip.moments === undefined || tip.moments.includes(moment);
}

/** Resolve a tip's late-bound parts for display. */
export function renderTip(tip: Tip): { title: string; body: string[]; note?: string } {
	const body = typeof tip.body === "function" ? tip.body() : (tip.body ?? []);
	return { title: tip.title, body, note: tip.note };
}
