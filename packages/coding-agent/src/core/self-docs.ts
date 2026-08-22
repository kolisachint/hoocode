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
	cachedSections = undefined;
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
 * Deliberately just filenames. An earlier version carried a one-line summary
 * per doc and cost ~860 tokens on every single turn, which is a poor trade for
 * something most turns never use — and it stopped being necessary once
 * SearchHooCode could retrieve at the heading level. Filenames alone still let
 * the model go straight to `themes.md` or `keybindings.md` for the obvious
 * cases, and anything less obvious is one search away. That is ~180 tokens.
 *
 * Directories are printed once rather than repeated per entry, for the same
 * reason: the path was the single largest term on every line.
 */
export function formatSelfDocsForPrompt(docs: readonly SelfDoc[] = listSelfDocs()): string {
	if (docs.length === 0) return "";

	// Insertion order is already meaningful (overview first, then alphabetical,
	// then README/CHANGELOG), so group without re-sorting.
	const groups = new Map<string, string[]>();
	for (const doc of docs) {
		const root = dirname(doc.path);
		const bucket = groups.get(root);
		if (bucket) bucket.push(doc.id);
		else groups.set(root, [doc.id]);
	}

	const sections = [...groups].map(([root, files]) => `${root}/: ${files.join(", ")}`);

	return `

# About hoocode itself

You are running inside hoocode. Its own docs ship with the install, listed below; hoocode is actively developed, so answer questions about it from these files rather than from memory. They sit outside the working directory, so searching the project will not find them. Use SearchHooCode to locate a specific heading, or read a file directly.

${sections.join("\n")}`;
}

// ---------------------------------------------------------------------------
// Section index
// ---------------------------------------------------------------------------

/**
 * A single heading's worth of a doc.
 *
 * Doc-level retrieval would add nothing the prompt listing above does not
 * already give: thirty files with a summary each are cheap enough to list in
 * full, so a search that answers "read extensions.md" is a round trip for
 * information the model already had. The questions that actually need
 * retrieval are the ones inside a 1,100-line file — "how do I register a
 * tool?" should land on `extensions.md § Custom tools` with a line number, not
 * on the file.
 */
export interface SelfDocSection {
	/** `<file>#<slug>`, unique across the corpus. */
	id: string;
	/** Filename, e.g. `extensions.md`. */
	file: string;
	/** Absolute path to the file. */
	path: string;
	/** Heading trail from the document title down, e.g. `["Extensions", "Custom tools"]`. */
	headings: string[];
	/** 1-based line of the heading, so a reader can jump straight to it. */
	line: number;
	/** Start of the section body, for ranking and for showing why a hit matched. */
	excerpt: string;
}

/**
 * How much section body to keep.
 *
 * Every character past this is invisible to retrieval, so the cap is a recall
 * limit, not just a size one: at 240 a question about `/grill` missed the
 * section that documents it, because the term sat in the fourth sentence. 400
 * covers the opening of essentially every section here for about 95KB more
 * index across the corpus, which buys back that class of miss.
 */
const MAX_EXCERPT = 400;

/** `Custom tools` → `custom-tools`, so ids stay stable and readable. */
function slugify(heading: string): string {
	return (
		heading
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-+|-+$/g, "") || "section"
	);
}

/** `extensions.md § Extensions › Custom tools` — what a search result is labelled with. */
export function sectionLabel(section: SelfDocSection): string {
	return section.headings.length > 0 ? `${section.file} § ${section.headings.join(" › ")}` : section.file;
}

/**
 * Split one markdown file into sections at its headings.
 *
 * Fenced code is tracked so a `#` comment inside a bash block cannot be
 * mistaken for a heading — which would otherwise split docs at every shell
 * comment. Code *content* still lands in the excerpt: the exact identifiers
 * someone searches for (`pi.registerTool`) usually live in the examples, and
 * dropping them would blind the lexical leg to the best terms in the file.
 */
export function splitIntoSections(markdown: string, file: string, path: string): SelfDocSection[] {
	const lines = markdown.split(/\r?\n/);
	const sections: SelfDocSection[] = [];
	const trail: Array<{ depth: number; text: string }> = [];
	const usedIds = new Set<string>();

	let current: SelfDocSection | undefined;
	let body: string[] = [];
	let inFence = false;

	const flush = (): void => {
		if (!current) return;
		current.excerpt = truncate(stripInlineMarkdown(body.join(" ")), MAX_EXCERPT);
		sections.push(current);
		body = [];
	};

	for (let i = 0; i < lines.length; i++) {
		const raw = lines[i] ?? "";
		if (raw.trimStart().startsWith("```")) {
			inFence = !inFence;
			continue;
		}
		const heading = inFence ? null : /^(#{1,6})\s+(.+?)\s*$/.exec(raw);
		if (!heading) {
			if (raw.trim() !== "") body.push(raw.trim());
			continue;
		}

		flush();

		const depth = heading[1]?.length ?? 1;
		const text = stripInlineMarkdown(heading[2] ?? "");
		while (trail.length > 0 && (trail[trail.length - 1]?.depth ?? 0) >= depth) trail.pop();
		trail.push({ depth, text });

		// Disambiguate repeated headings ("Example" appears eleven times in
		// extensions.md) so ids stay unique and the registry does not collapse them.
		let id = `${file}#${slugify(trail.map((t) => t.text).join("-"))}`;
		if (usedIds.has(id)) {
			let n = 2;
			while (usedIds.has(`${id}-${n}`)) n++;
			id = `${id}-${n}`;
		}
		usedIds.add(id);

		current = { id, file, path, headings: trail.map((t) => t.text), line: i + 1, excerpt: "" };
	}
	flush();

	return sections;
}

/**
 * Files kept out of the section index.
 *
 * The changelog is 40% of the corpus by section count and none of it answers
 * "how does X work": it is hundreds of near-identical `Added`/`Fixed`/`Changed`
 * headings under version numbers, which crowd real documentation out of the
 * ranking while matching almost any query about a feature by name.
 *
 * `index.md` is excluded for the mirror-image reason: it is a table of contents,
 * so its "sections" are lists of links whose text is every other doc's title and
 * summary. That makes it match any query those docs would match, while carrying
 * none of the content — a guaranteed false attractor that displaces the page it
 * is pointing at.
 *
 * Both stay in the prompt's filename listing, one read away.
 */
const SECTION_INDEX_EXCLUDED = new Set(["CHANGELOG.md", "index.md"]);

let cachedSections: SelfDocSection[] | undefined;

/** Drop the cached section index. Tests, and anything that relocates the package root. */
export function resetSelfDocSections(): void {
	cachedSections = undefined;
}

/**
 * Every section of every shipped doc.
 *
 * Reads each file once per session and caches; the docs are read-only install
 * content, so there is nothing to invalidate on.
 */
export function listSelfDocSections(): SelfDocSection[] {
	if (cachedSections) return cachedSections;

	const sections: SelfDocSection[] = [];
	for (const doc of listSelfDocs()) {
		if (SECTION_INDEX_EXCLUDED.has(doc.id)) continue;
		let content: string;
		try {
			content = readFileSync(doc.path, "utf-8");
		} catch {
			continue;
		}
		sections.push(...splitIntoSections(content, doc.id, doc.path));
	}

	cachedSections = sections;
	return sections;
}
