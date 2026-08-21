/**
 * Renaming and removing a canvas extension.
 *
 * Design: `docs/canvas-extensions-design.md` §13.6. These exist because a canvas
 * has more identity than a file does, and it is spread across four places: the
 * directory name (which *is* the extension id, since discovery keys off
 * position), the `id` a canvas declares, its `displayName`, and whatever the
 * header comment tells the reader to type. Renaming by hand means getting all
 * four right, and getting the `id` wrong is not a typo — it drops the canvas the
 * person is looking at on the next reload, because the instance was opened
 * against a canvas that no longer exists.
 *
 * So the rewriting is deliberately narrow. It changes the three places the
 * scaffold puts the name and nothing else, then **reports every other line the
 * old name still appears on** rather than guessing at prose. A rename that
 * silently edited a description would be worse than one that admits what it left
 * behind.
 */

import { existsSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import * as path from "node:path";
import type { CanvasSearchRoot, DiscoveredCanvasExtension } from "./discovery.js";
import { CANVAS_ENTRY_FILE } from "./discovery.js";
import { validateCanvasName } from "./scaffold.js";

/** One line the rename changed. */
export interface CanvasRewrite {
	/** 1-based, so it can be read straight off an editor gutter. */
	line: number;
	before: string;
	after: string;
}

/** What {@link renameCanvasExtension} did. */
export interface CanvasRenameResult {
	from: string;
	to: string;
	/** Workspace-relative where possible, for a message a person can act on. */
	dir: string;
	rewrites: CanvasRewrite[];
	/**
	 * Lines where the old name survives, because they are prose rather than
	 * identity. Surfaced, never silently edited.
	 */
	leftovers: number[];
}

/**
 * Why a canvas cannot be renamed or removed in place.
 *
 * `packaged` is the interesting one: a plugin's canvases are named by its
 * manifest rather than by where they sit, so moving the directory would either
 * do nothing or break the plugin. That is `/plugin`'s job, not this one's.
 */
export type CanvasLifecycleRefusal =
	| { reason: "invalid-name"; detail: string }
	| { reason: "exists"; detail: string }
	| { reason: "packaged"; detail: string }
	| { reason: "unwritable"; detail: string };

/**
 * Whether this extension is one we may move or delete.
 *
 * The test is positional and deliberately so: an extension is ours to edit when
 * its directory sits *directly* inside one of the search roots, which is exactly
 * the layout `discoverCanvasExtensions` walks. Anything else arrived inside a
 * package — resolved through a manifest by `plugin-canvases.ts` — and its
 * location is that package's business.
 */
export function canvasHomeRoot(
	extension: DiscoveredCanvasExtension,
	roots: readonly CanvasSearchRoot[],
): CanvasSearchRoot | undefined {
	const parent = path.dirname(path.resolve(extension.dir));
	return roots.find((root) => path.resolve(root.dir) === parent);
}

/**
 * Swap the old name for the new one where it is *identity* rather than prose.
 *
 * Two rules, and the first is the interesting one: **a string literal whose
 * entire content is the old name**. That covers `id`, `displayName`, `title` and
 * the scaffold's `const ID = "…"` without knowing any of their names, and it
 * does not touch a sentence that merely mentions the canvas — `"the board is the
 * point of the board"` is not the string `"board"`. A whole-word replacement
 * would have rewritten that sentence, which is worse than leaving it.
 *
 * The second rule keeps the instructions in a comment honest: `/canvas open
 * <old>` would otherwise tell the next reader to type a name that no longer
 * resolves.
 *
 * Anything else is left and reported. The scaffold names itself in exactly three
 * places, all covered here, so a canvas that has not been renamed by hand comes
 * through with nothing left over.
 */
function rewriteEntrySource(source: string, from: string, to: string): { source: string; rewrites: CanvasRewrite[] } {
	const name = escapeForRegExp(from);
	const substitutions: [RegExp, string][] = [
		// A quoted string that is *only* the old name. Backticks included, but only
		// without interpolation — `${x}-${from}` is a computed value, not a literal.
		[new RegExp(`(["'])${name}\\1`, "g"), `$1${to}$1`],
		[new RegExp("`" + name + "`", "g"), `\`${to}\``],
		// Instructions in a comment: /canvas open <old>, /canvas reload <old>, …
		[new RegExp(`(/canvas\\s+(?:open|reload|rename|remove|close)\\s+)${name}\\b`, "g"), `$1${to}`],
	];

	const before = source.split("\n");
	const after = before.map((line) =>
		substitutions.reduce((text, [pattern, replacement]) => text.replace(pattern, replacement), line),
	);

	const rewrites: CanvasRewrite[] = [];
	after.forEach((line, index) => {
		if (line !== before[index]) rewrites.push({ line: index + 1, before: before[index] as string, after: line });
	});
	return { source: after.join("\n"), rewrites };
}

/** Lines still mentioning `name` after the rewrite. */
function remainingMentions(source: string, name: string): number[] {
	const lines: number[] = [];
	source.split("\n").forEach((line, index) => {
		if (line.includes(name)) lines.push(index + 1);
	});
	return lines;
}

function escapeForRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Rename a canvas extension: its directory, and the name inside its code.
 *
 * The caller is responsible for closing open instances first. This does not do
 * it, because closing is the *session's* concern and this module has no session —
 * but skipping it leaves an instance pointing at a directory that has moved.
 */
export function renameCanvasExtension(
	extension: DiscoveredCanvasExtension,
	to: string,
	roots: readonly CanvasSearchRoot[],
): CanvasRenameResult | CanvasLifecycleRefusal {
	const invalid = validateCanvasName(to);
	if (invalid) return { reason: "invalid-name", detail: invalid };
	if (to === extension.id) return { reason: "invalid-name", detail: `it is already called "${to}"` };

	const root = canvasHomeRoot(extension, roots);
	if (!root) {
		return {
			reason: "packaged",
			detail: `"${extension.id}" came from an installed plugin, which names its canvases in its own manifest. Renaming it here would break the plugin; use /plugin instead.`,
		};
	}

	const target = path.join(root.dir, to);
	if (existsSync(target)) return { reason: "exists", detail: `${target} already exists` };

	const entry = path.join(extension.dir, CANVAS_ENTRY_FILE);
	let source: string;
	try {
		source = readFileSync(entry, "utf8");
	} catch (cause) {
		return {
			reason: "unwritable",
			detail: `could not read ${entry}: ${cause instanceof Error ? cause.message : String(cause)}`,
		};
	}

	const rewritten = rewriteEntrySource(source, extension.id, to);
	try {
		// Write before moving: if the write fails the directory has not moved and
		// nothing is half-renamed.
		if (rewritten.source !== source) writeFileSync(entry, rewritten.source, "utf8");
		renameSync(extension.dir, target);
	} catch (cause) {
		return { reason: "unwritable", detail: cause instanceof Error ? cause.message : String(cause) };
	}

	return {
		from: extension.id,
		to,
		dir: target,
		rewrites: rewritten.rewrites,
		leftovers: remainingMentions(rewritten.source, extension.id),
	};
}

/** What {@link removeCanvasExtension} deleted. */
export interface CanvasRemoveResult {
	id: string;
	dir: string;
}

/**
 * Delete a canvas extension's directory.
 *
 * As with rename, the caller closes open instances first — a deleted directory
 * whose child is still forked leaves a process serving code that no longer
 * exists on disk, which is the most confusing state of all.
 */
export function removeCanvasExtension(
	extension: DiscoveredCanvasExtension,
	roots: readonly CanvasSearchRoot[],
): CanvasRemoveResult | CanvasLifecycleRefusal {
	const root = canvasHomeRoot(extension, roots);
	if (!root) {
		return {
			reason: "packaged",
			detail: `"${extension.id}" came from an installed plugin. Deleting its directory would leave the plugin broken rather than uninstalled; use /plugin instead.`,
		};
	}
	try {
		rmSync(extension.dir, { recursive: true, force: true });
	} catch (cause) {
		return { reason: "unwritable", detail: cause instanceof Error ? cause.message : String(cause) };
	}
	return { id: extension.id, dir: extension.dir };
}

/** Narrow a lifecycle return value to its refusal case. */
export function isCanvasRefusal(value: object): value is CanvasLifecycleRefusal {
	return "reason" in value;
}
