/**
 * The agent's index of hoocode's *own* documentation.
 *
 * The startup banner promises "hoocode can explain its own features and look up
 * its docs", and the docs really do ship with the install (`package.json`
 * `files` includes `docs`, and `copy-binary-assets` copies them into `dist/` for
 * the pkg binaries). What was missing is the only part that makes the promise
 * true: telling the model they exist. `getDocsPath()` had exactly one consumer —
 * `auth-guidance.ts`, which prints paths to the *human* — so nothing ever put a
 * docs path into model context.
 *
 * That gap is not one the model can close by itself. Its cwd is the user's
 * project, so `grep`/`find` there discover the user's docs, never hoocode's,
 * which live in an install directory whose path it cannot derive.
 *
 * Descriptions come from `docs/index.md` rather than being duplicated here.
 * That file is a curated, human-maintained table of contents, and a second
 * hand-written list is how an index goes stale the first week nobody updates
 * it. The directory listing stays the source of truth for *what exists*, so a
 * new doc still shows up (described from its own first paragraph) on the day it
 * lands, with or without an index entry.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { getChangelogPath, getDocsPath, getReadmePath } from "../config.js";

export interface SelfDoc {
	/** Stable id: the filename, e.g. `skills.md`. Also how the model refers to it. */
	id: string;
	/** Absolute path, ready to hand to the read tool verbatim. */
	path: string;
	/** Human title, e.g. "Skills". */
	title: string;
	/** One line on what the doc covers. May be empty if nothing could be derived. */
	description: string;
}

/** How much of a doc to read when deriving a fallback description. */
const HEAD_BYTES = 2048;

/** Cap on a derived description, so one run-on opening line cannot bloat the prompt. */
const MAX_DESCRIPTION = 110;

function truncate(text: string, max = MAX_DESCRIPTION): string {
	const clean = text.replace(/\s+/g, " ").trim();
	if (clean.length <= max) return clean;
	return `${clean.slice(0, max - 1).trimEnd()}…`;
}

/** Strip inline markdown that adds noise but no meaning in a prompt listing. */
function stripInlineMarkdown(text: string): string {
	return text
		.replace(/\[([^\]]+)\]\([^)]*\)/g, "$1") // links → their text
		.replace(/[`*_]/g, "")
		.trim();
}

function readHead(path: string): string {
	try {
		// Whole-file read: these are small, and slicing bytes off a UTF-8 file can
		// split a multi-byte character. Truncate after decoding instead.
		return readFileSync(path, "utf-8").slice(0, HEAD_BYTES);
	} catch {
		return "";
	}
}

/** First `# ` heading, or undefined. */
function firstHeading(markdown: string): string | undefined {
	for (const line of markdown.split(/\r?\n/)) {
		const match = /^#\s+(.+)$/.exec(line.trim());
		if (match?.[1]) return stripInlineMarkdown(match[1]);
	}
	return undefined;
}

/**
 * First real prose line: not a heading, blockquote, list item, fence, or table
 * row. Used only for docs the curated index does not describe.
 */
function firstParagraph(markdown: string): string | undefined {
	let inFence = false;
	for (const raw of markdown.split(/\r?\n/)) {
		const line = raw.trim();
		if (line.startsWith("```")) {
			inFence = !inFence;
			continue;
		}
		if (inFence || line === "") continue;
		if (/^[#>|-]/.test(line) || /^\d+\./.test(line)) continue;
		return stripInlineMarkdown(line);
	}
	return undefined;
}

/**
 * Titles and descriptions the docs maintain about themselves, keyed by filename.
 *
 * Matches list entries of the form `- [Title](file.md) - description`, which is
 * how every section of `index.md` is written. Anything that does not match is
 * skipped rather than guessed at.
 */
function parseCuratedIndex(docsRoot: string): Map<string, { title: string; description: string }> {
	const curated = new Map<string, { title: string; description: string }>();
	const indexPath = join(docsRoot, "index.md");
	if (!existsSync(indexPath)) return curated;

	let content: string;
	try {
		content = readFileSync(indexPath, "utf-8");
	} catch {
		return curated;
	}

	// `[Title](file.md)` followed by a dash of any width and the description.
	const entry = /^\s*[-*]\s*\[([^\]]+)\]\(([^)#]+\.md)\)\s*[-–—:]\s*(.+?)\s*$/;
	for (const line of content.split(/\r?\n/)) {
		const match = entry.exec(line);
		if (!match) continue;
		const [, title, target, description] = match;
		const file = basename(target);
		if (curated.has(file)) continue; // first mention wins
		curated.set(file, { title: stripInlineMarkdown(title), description: truncate(stripInlineMarkdown(description)) });
	}
	return curated;
}

function describe(path: string, file: string, curated: Map<string, { title: string; description: string }>): SelfDoc {
	const fromIndex = curated.get(file);
	if (fromIndex) {
		return { id: file, path, title: fromIndex.title, description: fromIndex.description };
	}
	// Not in the curated index — derive from the doc itself so new files are
	// still usable the day they land.
	const head = readHead(path);
	return {
		id: file,
		path,
		title: firstHeading(head) ?? file.replace(/\.md$/, ""),
		description: truncate(firstParagraph(head) ?? ""),
	};
}

let cached: SelfDoc[] | undefined;

/** Drop the cached listing. Tests, and anything that relocates the package root. */
export function resetSelfDocs(): void {
	cached = undefined;
}

/**
 * Every shipped doc, sorted with the overview first and the rest alphabetical.
 *
 * Returns `[]` when the docs directory is absent rather than throwing: a source
 * checkout, an odd packaging, or a trimmed container should degrade to "no docs
 * section in the prompt", never to a failed session start.
 */
export function listSelfDocs(): SelfDoc[] {
	if (cached) return cached;

	const docsRoot = getDocsPath();
	const docs: SelfDoc[] = [];

	if (existsSync(docsRoot)) {
		const curated = parseCuratedIndex(docsRoot);
		let files: string[];
		try {
			files = readdirSync(docsRoot).filter((f) => f.endsWith(".md"));
		} catch {
			files = [];
		}
		// Overview first: it is the doc that explains the others.
		files.sort((a, b) => (a === "index.md" ? -1 : b === "index.md" ? 1 : a.localeCompare(b)));
		for (const file of files) {
			const path = join(docsRoot, file);
			try {
				if (!statSync(path).isFile()) continue;
			} catch {
				continue;
			}
			docs.push(describe(path, file, curated));
		}
	}

	// README and CHANGELOG sit beside the docs directory, not inside it, but the
	// model needs them for the two questions the docs do not answer: what
	// hoocode is, and what changed in this version.
	const extras: Array<{ path: string; title: string; description: string }> = [
		{ path: getReadmePath(), title: "README", description: "What hoocode is, install, and a feature overview." },
		{
			path: getChangelogPath(),
			title: "Changelog",
			description: "Released versions and what changed in each.",
		},
	];
	for (const extra of extras) {
		if (!existsSync(extra.path)) continue;
		docs.push({ id: basename(extra.path), path: extra.path, title: extra.title, description: extra.description });
	}

	cached = docs;
	return docs;
}

/**
 * The system-prompt section, or `""` when there is nothing to point at.
 *
 * Groups by directory and prints each root once rather than repeating a ~50
 * character absolute path on all thirty lines — that repetition alone cost more
 * tokens than every description combined, and the model can join a root and a
 * filename perfectly well.
 */
export function formatSelfDocsForPrompt(docs: readonly SelfDoc[] = listSelfDocs()): string {
	if (docs.length === 0) return "";

	// Insertion order is already meaningful (overview first, then alphabetical,
	// then README/CHANGELOG), so group without re-sorting.
	const groups = new Map<string, SelfDoc[]>();
	for (const doc of docs) {
		const root = dirname(doc.path);
		const bucket = groups.get(root);
		if (bucket) bucket.push(doc);
		else groups.set(root, [doc]);
	}

	const sections: string[] = [];
	for (const [root, entries] of groups) {
		const lines = entries.map((doc) => {
			const summary = doc.description ? ` — ${doc.description}` : "";
			return `- ${doc.id} (${doc.title})${summary}`;
		});
		sections.push(`In ${root}/\n${lines.join("\n")}`);
	}

	return `

# About hoocode itself

You are running inside hoocode, and its own documentation ships with the install. When the user asks what hoocode can do, how one of its features works, or how to configure, extend, or troubleshoot it, read the relevant file below and answer from it. Do not answer such questions from memory — hoocode is actively developed and your training data does not cover it.

These files live outside the working directory, so searching the project will not find them. Read them by joining the directory and the filename.

${sections.join("\n\n")}`;
}
