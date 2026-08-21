/**
 * Authoring a canvas: the request a person types, the files it writes, and the
 * brief the model builds from.
 *
 * Design: `docs/canvas-extensions-design.md` §9 Phase 3, §13.
 *
 * This lives in `core/` rather than beside the other `/new-*` scaffolds because
 * `/new-canvas` stopped being a file-writing command. Copilot's `/create-canvas`
 * takes a sentence, has the agent write the extension, and opens it in a panel to
 * iterate on; matching that means the command has to reach the canvas session to
 * open, and the agent loop to build. Every decision that does not need either —
 * what the name is, where the file goes, what goes in it, what the model is told —
 * is here, testable without a terminal, a fork or a model.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { MarketplacePlatform } from "../extensions/plugins/formats/types.js";
import { CANVAS_ENTRY_FILE } from "./discovery.js";

/**
 * Where a canvas extension lives, per platform.
 *
 * This does not go through `WorkspaceLayout` like the other scaffolds, and the
 * reason is that a canvas has no Claude convention to emit into: the surface
 * exists in Copilot and in hoocode's own `.agents/` tree and nowhere else. A
 * layout method returning nothing for one adapter would be a worse lie than
 * naming the two real homes here — these are exactly the roots
 * `core/canvas/discovery.ts` searches, which is what makes a scaffold live on the
 * next `/canvas`.
 */
export const CANVAS_HOMES: Partial<Record<MarketplacePlatform, string[]>> = {
	agents: [".agents", "extensions"],
	github: [".github", "extensions"],
};

/** Validates a canvas name: lowercase a-z, 0-9, hyphens, no leading/trailing/double hyphens. */
export function validateCanvasName(name: string): string | null {
	if (!name) return "name is required";
	if (!/^[a-z0-9-]+$/.test(name)) return "name must be lowercase a-z, 0-9, and hyphens only";
	if (name.startsWith("-") || name.endsWith("-")) return "name must not start or end with a hyphen";
	if (name.includes("--")) return "name must not contain consecutive hyphens";
	return null;
}

/**
 * Words dropped when a directory name is derived from a sentence.
 *
 * Only grammar, never subject matter: "a board for tracking the release checklist"
 * should become `board-tracking-release`, and a list that reached further would
 * start deciding which of someone's nouns mattered.
 */
const FILLER = new Set([
	"a",
	"an",
	"and",
	"are",
	"as",
	"at",
	"be",
	"can",
	"for",
	"from",
	"i",
	"in",
	"is",
	"it",
	"its",
	"me",
	"my",
	"of",
	"on",
	"one",
	"or",
	"our",
	"so",
	"that",
	"the",
	"their",
	"them",
	"then",
	"this",
	"to",
	"us",
	"we",
	"which",
	"with",
	"you",
	"your",
]);

/** How many words a derived name keeps. Enough to be recognisable, short enough to type. */
const DERIVED_NAME_WORDS = 3;

/**
 * Turn a sentence into a directory name.
 *
 * Returns undefined when nothing usable survives — an all-punctuation request, or
 * a sentence of nothing but filler — because inventing `canvas-1` would hide from
 * the person that we did not understand them.
 */
export function canvasNameFromDescription(description: string): string | undefined {
	const words = description
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, " ")
		.trim()
		.split(" ")
		.filter((word) => word.length > 0);
	// "canvas" is dropped only when something else remains: "/new-canvas a canvas"
	// should still produce `canvas` rather than nothing.
	const meaningful = words.filter((word) => !FILLER.has(word));
	const named = meaningful.filter((word) => word !== "canvas");
	const kept = (named.length > 0 ? named : meaningful).slice(0, DERIVED_NAME_WORDS);
	const name = kept.join("-");
	return validateCanvasName(name) === null ? name : undefined;
}

/** What a person asked `/new-canvas` for. */
export interface CanvasRequest {
	/** Directory and default canvas id. */
	name: string;
	/**
	 * What they want it to do, in their words, or undefined when they only named
	 * one. Present means the model is expected to build it (Copilot's
	 * `/create-canvas` shape); absent means they want the template to edit by hand.
	 */
	description: string | undefined;
}

/**
 * Parse `/new-canvas`'s argument.
 *
 * Three shapes, in the order they are tested:
 *
 * - `my-board` — a bare name. Unchanged from before descriptions existed, and
 *   tested first so it can never be re-read as a one-word description.
 * - `my-board: a kanban board for the release checklist` — both, when someone
 *   cares what the directory is called. A colon rather than a flag because the
 *   rest of the line is prose and a flag parser would have to guess where it ends.
 * - `a kanban board for the release checklist` — a description, the shape
 *   `/create-canvas` uses. The name is derived and reported.
 *
 * Returns a string when the input cannot become a canvas; the caller shows it.
 */
export function parseCanvasRequest(input: string): CanvasRequest | string {
	const trimmed = input.trim();
	if (trimmed.length === 0) return "name or description is required";

	if (validateCanvasName(trimmed) === null) return { name: trimmed, description: undefined };

	const colon = trimmed.indexOf(":");
	if (colon > 0) {
		const name = trimmed.slice(0, colon).trim();
		const description = trimmed.slice(colon + 1).trim();
		if (validateCanvasName(name) === null && description.length > 0) return { name, description };
	}

	const derived = canvasNameFromDescription(trimmed);
	if (!derived) {
		return `could not derive a directory name from that. Give one explicitly: /new-canvas <name>: ${trimmed}`;
	}
	return { name: derived, description: trimmed };
}

/**
 * A working single-canvas extension.
 *
 * Deliberately complete rather than a stub: a canvas has no passive half — its
 * name, its actions and its UI all come from running its code — so a scaffold
 * that does not run teaches nothing and cannot be checked with `/canvas open`.
 * This one opens, serves a page, answers an action, and closes cleanly, which is
 * the whole contract; everything past that is the author's.
 *
 * Shaped like the catalog extensions hoocode already hosts: the only import is
 * `@github/copilot-sdk/extension` (host-resolved — never installed, see
 * `docs/canvas-extensions-design.md` §4.1) plus `node:` builtins, and the UI is
 * served from an ephemeral loopback port behind a per-instance token so nothing
 * else on the machine can read it.
 */
export const CANVAS_ENTRY_TEMPLATE = (name: string): string => `\
// ${name} — a canvas extension. Run it with: /canvas open ${name}
//
// The "@github/copilot-sdk/extension" import is resolved by the host at fork
// time. Do not install it, and do not add a node_modules here.
import { createCanvas, CanvasError, joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

/** Per-instance state. A canvas can be opened more than once at a time. */
const instances = new Map();

const session = await joinSession({
	canvases: [
		createCanvas({
			id: "${name}",
			displayName: "${name}",
			description: "TODO: one sentence — the agent reads this to decide whether to open it.",
			actions: [
				{
					name: "add_note",
					description: "TODO: describe what the agent can do to this canvas.",
					inputSchema: {
						type: "object",
						properties: { text: { type: "string" } },
						required: ["text"],
					},
					handler: (ctx) => {
						const entry = instances.get(ctx.instanceId);
						// CanvasError carries a code the host shows verbatim; a bare
						// throw arrives as an opaque internal error instead.
						if (!entry) throw new CanvasError("no_instance", \`Instance "\${ctx.instanceId}" is not open.\`);
						entry.notes.push(ctx.input.text);
						return { notes: entry.notes.length };
					},
				},
			],
			open: async (ctx) => {
				const token = randomBytes(32).toString("base64url");
				const notes = [];
				const server = createServer((req, res) => {
					const url = new URL(req.url ?? "/", "http://127.0.0.1");
					if (url.searchParams.get("token") !== token) {
						res.writeHead(403, { "Content-Type": "text/plain" });
						res.end("forbidden");
						return;
					}
					res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
					res.end(
						\`<!doctype html><meta charset="utf-8"><title>${name}</title>\` +
							\`<p>\${notes.length} note(s). TODO: build the UI.\`,
					);
				});
				// Port 0 on 127.0.0.1: an ephemeral port, reachable only from this machine.
				await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
				const { port } = server.address();
				instances.set(ctx.instanceId, { server, notes });
				return {
					url: \`http://127.0.0.1:\${port}/?token=\${token}\`,
					title: "${name}",
				};
			},
			onClose: async (ctx) => {
				const entry = instances.get(ctx.instanceId);
				// Called for an abandoned open too, so the instance may be unknown.
				if (!entry) return;
				instances.delete(ctx.instanceId);
				await new Promise((resolve) => entry.server.close(resolve));
			},
		}),
	],
});

// stdout is the protocol channel. Use session.log, never console.log.
await session.log("${name} ready");
`;

/** What {@link scaffoldCanvas} wrote. */
export interface CanvasScaffoldResult {
	/** Workspace-relative entry files created, one per platform target. */
	created: string[];
	/** Workspace-relative entry files that already existed and were left alone. */
	skipped: string[];
}

/**
 * Write the template into every platform target that has a canvas home.
 *
 * Existing files are never clobbered — they are reported and skipped, so running
 * `/new-canvas` twice on a canvas you have been editing cannot lose it.
 */
export function scaffoldCanvas(cwd: string, name: string, platforms: MarketplacePlatform[]): CanvasScaffoldResult {
	const created: string[] = [];
	const skipped: string[] = [];
	for (const platform of platforms) {
		const home = CANVAS_HOMES[platform];
		if (!home) continue;
		const relative = join(...home, name, CANVAS_ENTRY_FILE);
		const absolute = join(cwd, relative);
		if (existsSync(absolute)) {
			skipped.push(relative);
			continue;
		}
		mkdirSync(dirname(absolute), { recursive: true });
		writeFileSync(absolute, CANVAS_ENTRY_TEMPLATE(name), "utf8");
		created.push(relative);
	}
	return { created, skipped };
}

/** Where a build brief's canvas is, if opening it worked. */
export interface CanvasBriefTarget {
	instanceId: string;
	url: string | undefined;
}

/**
 * The message the model builds from — hoocode's half of `/create-canvas`.
 *
 * A scaffold plus a sentence is not a canvas, and the gap between them is the
 * agent's work. This is what turns "a kanban board for the release checklist"
 * into a build task with the contract attached, so the model does not have to
 * infer the rules of a surface it cannot see from a template it has not read.
 *
 * Four of those rules are stated because getting them wrong fails in ways whose
 * symptom does not name the cause: an installed dependency (the resolver already
 * provides the SDK, and a `node_modules` here is a §4.1 violation), a
 * `console.log` (corrupts the JSON-RPC channel and surfaces as "non-protocol
 * stdout"), a write without a reload (changes nothing at all, because the running
 * child was forked from the old bytes), and a renamed canvas id.
 *
 * That last one is the newest and was found the hard way, by building a canvas
 * with this command: the scaffold names the canvas after the directory, a model
 * that thinks of a better name renames the `id`, and the next reload drops the
 * instance the person is looking at — correctly, since the canvas it was opened
 * against no longer exists. `displayName` is the half they actually see, so it
 * is the half to change.
 */
export function canvasBuildBrief(
	name: string,
	description: string,
	entryPath: string,
	target: CanvasBriefTarget | undefined,
): string {
	const lines = [
		`/new-canvas: build a canvas extension named "${name}".`,
		"",
		"What the person asked for, in their words:",
		`  ${description}`,
		"",
		`A working template is already at ${entryPath}. Edit it until it does what was asked.`,
	];

	if (target) {
		lines.push(
			"",
			target.url
				? `It is already open at ${target.url} (instance ${target.instanceId}), so the person is watching it as you work.`
				: `It is already open as instance ${target.instanceId}.`,
			`After every edit, call reload_canvas with extensionId "${name}". A write to the file changes nothing on its own — the running process was forked from the old code. Reloading hands back a NEW url; give it to the person, because the tab they have open dies with the old process.`,
		);
	} else {
		lines.push(
			"",
			`It is not open — the person will run /canvas open ${name} when they want to look. Build it anyway; you can check it runs by reading it carefully rather than by opening it yourself.`,
		);
	}

	lines.push(
		"",
		"The contract this surface runs under, which is not yours to change:",
		'- The only non-`node:` import allowed is "@github/copilot-sdk/extension". The host resolves it when it forks the extension. Do not install anything, and do not add a package.json or node_modules in the extension directory.',
		"- stdout is the JSON-RPC channel. Use `session.log(...)`; a `console.log` corrupts the protocol.",
		"- Do not change the canvas's `id`. An open instance is bound to it, so renaming it drops the canvas the person is watching on your next reload. Give it a nicer `displayName` instead — that is the one they see.",
		"- Serve the UI from the loopback server the template already starts. Keep the per-instance token check, and keep `onClose` shutting the server down — that is what stops a port outliving the session.",
		"- Everything in `actions: [...]` becomes callable by you through invoke_canvas_action once it is open, so give each action a real description and inputSchema. That list is how you drive the canvas; the person drives the same state through the page.",
		"",
		"Build the thing that was asked for, not a stub: leave no TODO behind, and make the page render real state rather than a placeholder.",
	);
	return lines.join("\n");
}
