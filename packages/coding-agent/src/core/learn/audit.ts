/**
 * The subtractive half of `/learn`: which lines in a context file describe
 * things that no longer exist?
 *
 * The mining pipeline can only ever propose additions. Nothing in it moves the
 * always-loaded token surface down, so a context file accumulates: a rule
 * naming a deleted workflow, a command that was removed, a file that moved two
 * refactors ago. Those lines cost tokens on every request forever and are worse
 * than useless, because the agent believes them.
 *
 * This is deliberately deterministic — no model call, no cache, no state. It
 * reads the context files already in force, pulls out the referents they name
 * in backticks, and asks the filesystem. That makes it instant and free, which
 * is what lets it be the half you run most often.
 *
 * Precision is bought with exclusions rather than cleverness, because a noisy
 * audit is one nobody reads. Three rules do most of the work:
 *
 * - **Only path-like referents with a separator.** A bare `auth.json` could be
 *   anywhere or nowhere; `docs/providers.md` is a claim about this repo.
 * - **Resolve against every package root, not just the repo root.** A monorepo
 *   names `src/cli/args.ts` relative to the package being discussed, and
 *   checking only the repo root reports the entire contributing guide as stale.
 * - **Skip lines that assert absence.** "these are all gone", "e.g.
 *   `bedrock-utils.ts`", "create `foo.ts`" legitimately name files that do not
 *   exist. Deciding this in general is a judgement call; a short vocabulary of
 *   assertive forms catches the cases that occur in practice.
 *
 * What is left is a short list where a wrong entry costs one glance and a right
 * one costs a line of always-loaded context. That asymmetry is the reason the
 * remaining false positives are acceptable and silent misses are not.
 */

import { type Dirent, existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";

/** One referent that did not resolve, with the line that claimed it. */
export interface StaleReference {
	/** Context file the claim lives in. */
	file: string;
	/** 1-based line number. */
	line: number;
	/** The line, trimmed — what the reader would delete or fix. */
	lineText: string;
	/** The referent that could not be found. */
	referent: string;
	kind: "path" | "script";
	/** Rough token cost of the line, so the value of deleting it is visible. */
	tokens: number;
}

/** Why a referent was not checked. Reported as counts so the audit's reach is visible. */
export interface AuditSkips {
	/** Contains a placeholder or a glob, e.g. an angle-bracket stand-in or a star. */
	placeholder: number;
	/** Home-relative or absolute: a runtime location, not a repo artifact. */
	runtime: number;
	/** A URL, or a git ref like `origin/main`. */
	external: number;
	/** No path separator, so the claim is not about a specific location. */
	ambiguous: number;
	/** The line asserts the referent is absent, optional, or to be created. */
	assertsAbsence: number;
}

export interface AuditReport {
	/** Context files audited, with their recurring cost. */
	files: Array<{ path: string; tokens: number }>;
	/** Context files skipped because they live outside the working tree. */
	skippedFiles: string[];
	/** Referents actually resolved against the filesystem. */
	checked: number;
	skipped: AuditSkips;
	stale: StaleReference[];
	/** Directories referents were resolved against, nearest first. */
	roots: string[];
}

/** Directories never descended into when discovering package roots. */
const IGNORED_DIRS = new Set(["node_modules", ".git", "dist", "build", "coverage", "out", ".next", ".turbo"]);

/** How deep to look for package roots. Deep enough for a monorepo, shallow enough to stay instant. */
const MAX_ROOT_DEPTH = 3;

/** Extensions that make a token a file reference even without an obvious path shape. */
const FILE_EXTENSIONS =
	/\.(md|markdown|ts|tsx|js|jsx|mjs|cjs|json|jsonc|ya?ml|toml|sh|bash|zsh|py|rs|go|lock|txt|css|html)$/i;

/**
 * Forms that legitimately name something absent.
 *
 * Every one of these was a false positive on a real context file before it was
 * excluded. `removed`/`gone` describe a deletion; `e.g.`/`for example` name a
 * pattern rather than a file; `create`/`scaffold` describe an artifact the
 * reader is being told to write.
 *
 * The list is deliberately short. Broad prohibitions ("never edit X", "do not
 * run Y") name things that exist and are exactly the claims worth checking, so
 * matching on those words would trade the audit's whole purpose for a little
 * precision.
 */
const ABSENCE_MARKERS = [
	"removed",
	"deleted",
	" gone",
	"no longer",
	"used to",
	"if present",
	"if it exists",
	"optional",
	"e.g.",
	"for example",
	"create ",
	"scaffold",
];

/** Referents that look like a git ref rather than a path. */
const GIT_REF_PREFIXES = ["origin/", "upstream/", "refs/", "HEAD"];

/** Rough token estimate, matching the convention used elsewhere for context files. */
function estimateTokens(text: string): number {
	return Math.round(Buffer.byteLength(text, "utf-8") / 4);
}

/**
 * Directories a relative referent may be resolved against.
 *
 * The repo root alone is not enough: a monorepo's contributing notes name
 * `src/cli/args.ts` meaning "inside the package under discussion", and resolving
 * that only from the root reports every such line as stale. Every directory
 * holding a `package.json` is therefore a root, nearest-shallowest first.
 */
export function resolutionRoots(base: string): string[] {
	const roots: string[] = [resolve(base)];

	const walk = (dir: string, depth: number): void => {
		if (depth > MAX_ROOT_DEPTH) return;
		let entries: Dirent[];
		try {
			entries = readdirSync(dir, { withFileTypes: true });
		} catch {
			return;
		}
		for (const entry of entries) {
			if (!entry.isDirectory() || IGNORED_DIRS.has(entry.name)) continue;
			const child = join(dir, entry.name);
			if (existsSync(join(child, "package.json"))) roots.push(child);
			walk(child, depth + 1);
		}
	};
	walk(resolve(base), 1);

	return roots;
}

/** Every backticked span in a line, in order. */
function backtickedSpans(line: string): string[] {
	const out: string[] = [];
	const pattern = /`([^`\n]+)`/g;
	let match = pattern.exec(line);
	while (match) {
		if (match[1]) out.push(match[1]);
		match = pattern.exec(line);
	}
	return out;
}

/** True when the line reads as a statement about something absent, optional, or yet to be written. */
function assertsAbsence(line: string): boolean {
	const lower = line.toLowerCase();
	return ABSENCE_MARKERS.some((marker) => lower.includes(marker));
}

/** The script name a `bun run x` / `npm run x` reference names, if it is one. */
function scriptReference(token: string): string | undefined {
	const match = /^(?:bun|npm|pnpm|yarn)\s+run\s+([A-Za-z0-9:_-]+)$/.exec(token.trim());
	return match?.[1];
}

type Classification =
	| { kind: "path" }
	| { kind: "script"; script: string }
	| { kind: "skip"; reason: keyof AuditSkips };

/**
 * Decide whether a backticked token is a checkable claim about this repo.
 *
 * Ordering matters: the skip reasons are reported as counts, and a token that
 * matches several should be attributed to the most specific one, so a
 * placeholder is a placeholder rather than "ambiguous".
 */
export function classifyReferent(token: string): Classification {
	const value = token.trim();
	const script = scriptReference(value);
	if (script) return { kind: "script", script };

	if (/[<>*{}$|]/.test(value)) return { kind: "skip", reason: "placeholder" };
	if (/^https?:\/\//i.test(value)) return { kind: "skip", reason: "external" };
	if (GIT_REF_PREFIXES.some((prefix) => value.startsWith(prefix))) return { kind: "skip", reason: "external" };
	// A shell command, a prose fragment, or a flag list — not a path.
	if (/\s/.test(value)) return { kind: "skip", reason: "ambiguous" };
	if (value.startsWith("~") || value.startsWith(sep) || /^[A-Za-z]:[\\/]/.test(value)) {
		return { kind: "skip", reason: "runtime" };
	}
	if (value === "." || value === "..") return { kind: "skip", reason: "ambiguous" };
	// The load-bearing precision rule: without a separator the token names a
	// filename that could be anywhere, and "anywhere" is not a claim worth
	// contradicting. A bare `stream.test.ts` in a monorepo is not stale, it is
	// under-specified.
	if (!value.includes("/")) return { kind: "skip", reason: "ambiguous" };
	// Past the separator rule, something that is neither a known file type nor an
	// obvious directory is more likely prose than a path.
	if (!FILE_EXTENSIONS.test(value) && !value.endsWith("/")) return { kind: "skip", reason: "ambiguous" };

	return { kind: "path" };
}

/** True when the referent resolves against any root. Directories count. */
function pathResolves(referent: string, roots: string[]): boolean {
	const relative = referent.replace(/\/+$/, "");
	if (!relative) return false;
	return roots.some((root) => existsSync(join(root, relative)));
}

/** Script names declared by any `package.json` at any resolution root. */
function declaredScripts(roots: string[]): Set<string> {
	const names = new Set<string>();
	for (const root of roots) {
		const manifest = join(root, "package.json");
		if (!existsSync(manifest)) continue;
		try {
			const parsed = JSON.parse(readFileSync(manifest, "utf-8")) as { scripts?: Record<string, unknown> };
			for (const name of Object.keys(parsed.scripts ?? {})) names.add(name);
		} catch {
			// A malformed manifest is not this command's problem; treat it as
			// declaring nothing rather than failing the audit.
		}
	}
	return names;
}

/**
 * The project a referent is resolved against: the nearest ancestor holding a
 * `.git`, or `cwd` when there is none.
 *
 * Not `cwd` itself. Context files are collected by walking up from `cwd`, so in
 * a monorepo the repo's `AGENTS.md` sits *above* the package you are working
 * in — and running from a package root is the normal case, not the exception.
 * Anchoring on `cwd` meant the file with all the claims in it was declared "not
 * in this working tree" and skipped, so the audit passed by checking nothing.
 */
export function findProjectRoot(cwd: string): string {
	let dir = resolve(cwd);
	while (true) {
		if (existsSync(join(dir, ".git"))) return dir;
		const parent = resolve(dir, "..");
		if (parent === dir) return resolve(cwd);
		dir = parent;
	}
}

/** True when the path is inside the project, so its claims are about this repo. */
function insideTree(path: string, root: string): boolean {
	const base = resolve(root);
	const target = resolve(path);
	return target === base || target.startsWith(base + sep);
}

export interface AuditOptions {
	cwd: string;
	/** Context files in force, as loaded for the system prompt. */
	files: Array<{ path: string; tokens?: number }>;
}

/**
 * Check every referent named by the repo-scope context files.
 *
 * Files outside the project are listed but not audited: a rule in
 * `~/.agents/AGENTS.md` naming `src/index.ts` is a claim about whichever repo
 * it was written for, and resolving it here would report another project's
 * rules as broken.
 *
 * File contents are re-read from disk rather than taken from the loader, which
 * truncates oversized files for the prompt — auditing the truncation would
 * silently stop checking exactly the files most likely to have gone stale.
 */
export function auditContextFiles(options: AuditOptions): AuditReport {
	// Rooted at the project, not at `cwd`: a path in the repo's context file is
	// written relative to the repo, and is being read from wherever you happen to
	// be working.
	const projectRoot = findProjectRoot(options.cwd);
	const roots = resolutionRoots(projectRoot);
	const scripts = declaredScripts(roots);

	const report: AuditReport = {
		files: [],
		skippedFiles: [],
		checked: 0,
		skipped: { placeholder: 0, runtime: 0, external: 0, ambiguous: 0, assertsAbsence: 0 },
		stale: [],
		roots,
	};

	for (const file of options.files) {
		if (!insideTree(file.path, projectRoot)) {
			report.skippedFiles.push(file.path);
			continue;
		}

		let content: string;
		try {
			content = readFileSync(file.path, "utf-8");
		} catch {
			report.skippedFiles.push(file.path);
			continue;
		}
		report.files.push({ path: file.path, tokens: file.tokens ?? estimateTokens(content) });

		const lines = content.split("\n");
		let inFence = false;
		for (const [index, raw] of lines.entries()) {
			// A fenced block is example code, not a claim about the repo. Its
			// contents are also where most of a context file's plausible-looking
			// paths live, so auditing it is almost pure noise.
			if (raw.trimStart().startsWith("```")) {
				inFence = !inFence;
				continue;
			}
			if (inFence) continue;

			const spans = backtickedSpans(raw);
			if (spans.length === 0) continue;

			const absence = assertsAbsence(raw);
			for (const span of spans) {
				const classification = classifyReferent(span);
				if (classification.kind === "skip") {
					report.skipped[classification.reason]++;
					continue;
				}
				if (absence) {
					report.skipped.assertsAbsence++;
					continue;
				}

				report.checked++;
				const resolved =
					classification.kind === "script" ? scripts.has(classification.script) : pathResolves(span, roots);
				if (resolved) continue;

				report.stale.push({
					file: file.path,
					line: index + 1,
					lineText: raw.trim(),
					referent: span,
					kind: classification.kind,
					tokens: estimateTokens(raw),
				});
			}
		}
	}

	return report;
}

/** Total recurring cost of the lines the audit flagged. */
export function staleTokens(report: AuditReport): number {
	const seen = new Set<string>();
	let total = 0;
	for (const item of report.stale) {
		const key = `${item.file}:${item.line}`;
		if (seen.has(key)) continue;
		seen.add(key);
		total += item.tokens;
	}
	return total;
}
