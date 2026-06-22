/**
 * Shared plumbing for the `webfetch` and `websearch` tools.
 *
 * Both tools shell out to the `webtools` binary (fetch / search subcommands,
 * resolved/downloaded via {@link ensureTool}) and parse its `--json` output.
 * This module owns:
 * - the spawn-and-parse runner,
 * - the locked JSON result types,
 * - a short-lived in-process result cache, and
 * - the `.webtoolsignore` policy matcher (gitignore semantics) used to block
 *   hosts both before a fetch and when filtering search result links.
 */

import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import ignore from "ignore";
import { getAgentDir } from "../../config.js";
import { ensureTool } from "../../utils/tools-manager.js";
import { execCommand } from "../exec.js";

type IgnoreMatcher = ReturnType<typeof ignore>;

/** Default request timeout (seconds) passed to the binary. */
export const WEBTOOLS_DEFAULT_TIMEOUT_SECS = 15;

/** How long a successful result stays cached, mirroring the documented 15-min TTL. */
const CACHE_TTL_MS = 15 * 60 * 1000;

// ============================================================================
// Result types (locked against `webtools <cmd> --json`)
// ============================================================================

export interface WebFetchReference {
	index: number;
	url: string;
	text?: string;
}

export interface WebFetchMetadata {
	description?: string;
	author?: string;
	published?: string;
	lang?: string;
	site_name?: string;
}

export interface WebFetchResult {
	title?: string;
	final_url: string;
	content: string;
	content_type: string;
	media: string;
	token_estimate: number;
	references: WebFetchReference[];
	metadata?: WebFetchMetadata;
	source: string;
}

export interface WebSearchResultItem {
	title: string;
	snippet: string;
	url: string;
	ref_index: number;
}

export interface WebSearchReference {
	index: number;
	url: string;
}

export interface WebSearchOutput {
	query: string;
	results: WebSearchResultItem[];
	references: WebSearchReference[];
	token_estimate: number;
	result_count: number;
}

// ============================================================================
// Binary runner
// ============================================================================

const BINARY_MISSING_MESSAGE =
	"webtools binary unavailable and could not be downloaded — web tools require the `webtools` CLI on PATH or a published release for this platform";

/**
 * Run a `webtools` subcommand with `--json` and return parsed stdout.
 *
 * Throws on missing binary, non-zero exit (surfacing the binary's stderr), or
 * unparseable output. Callers convert thrown errors into tool error results.
 */
export async function runWebtools<T>(
	subcommand: "fetch" | "search",
	args: string[],
	cwd: string,
	signal?: AbortSignal,
	timeoutSecs: number = WEBTOOLS_DEFAULT_TIMEOUT_SECS,
): Promise<T> {
	if (signal?.aborted) throw new Error("Operation aborted");

	const binaryPath = await ensureTool("webtools", true);
	if (!binaryPath) throw new Error(BINARY_MISSING_MESSAGE);

	// Give the spawn a little headroom over the binary's own request timeout so
	// the binary reports the timeout itself rather than being killed mid-flight.
	const spawnTimeoutMs = (timeoutSecs + 5) * 1000;
	const result = await execCommand(binaryPath, [subcommand, ...args, "--json"], cwd, {
		signal,
		timeout: spawnTimeoutMs,
	});

	if (signal?.aborted) throw new Error("Operation aborted");
	if (result.killed) throw new Error(`webtools ${subcommand} timed out after ${timeoutSecs}s`);
	if (result.code !== 0) {
		const stderr = result.stderr.trim();
		throw new Error(stderr || `webtools ${subcommand} exited with code ${result.code}`);
	}

	try {
		return JSON.parse(result.stdout) as T;
	} catch {
		throw new Error(`webtools ${subcommand} returned malformed JSON`);
	}
}

// ============================================================================
// Result cache (per-process, short TTL)
// ============================================================================

interface CacheEntry<T> {
	value: T;
	expiresAt: number;
}

export class WebToolsCache<T> {
	private readonly entries = new Map<string, CacheEntry<T>>();

	get(key: string): T | undefined {
		const entry = this.entries.get(key);
		if (!entry) return undefined;
		if (Date.now() >= entry.expiresAt) {
			this.entries.delete(key);
			return undefined;
		}
		return entry.value;
	}

	set(key: string, value: T): void {
		this.entries.set(key, { value, expiresAt: Date.now() + CACHE_TTL_MS });
	}
}

// ============================================================================
// .webtoolsignore policy matcher
// ============================================================================

/**
 * Build an {@link Ignore} matcher from `.webtoolsignore` policy files.
 *
 * Precedence is project-after-user so a project file can re-allow (`!host`)
 * something the user blocked, matching gitignore layering. Returns undefined
 * when no policy files exist (the common case: everything allowed).
 *
 * Hosts are matched as single path components, so subdomains need an explicit
 * wildcard (`*.example.com`), exactly like gitignore directory matching.
 */
export function loadWebtoolsIgnore(cwd: string): IgnoreMatcher | undefined {
	const files = [
		join(getAgentDir(), "webtoolsignore"),
		join(homedir(), ".webtoolsignore"),
		join(cwd, ".webtoolsignore"),
	];
	let found = false;
	const ig = ignore();
	for (const file of files) {
		if (!existsSync(file)) continue;
		try {
			ig.add(readFileSync(file, "utf8"));
			found = true;
		} catch {
			// Unreadable policy file: ignore it rather than failing the tool call.
		}
	}
	return found ? ig : undefined;
}

/** Extract the lowercased hostname from a URL, or undefined if it cannot be parsed. */
export function hostnameOf(url: string): string | undefined {
	try {
		const host = new URL(url).hostname.toLowerCase();
		return host || undefined;
	} catch {
		return undefined;
	}
}

/**
 * Whether a host is blocked by policy. A matcher is required; with no policy
 * files present callers treat every host as allowed.
 */
export function isHostBlocked(matcher: IgnoreMatcher, host: string): boolean {
	if (!host) return false;
	return matcher.ignores(host);
}

/**
 * Convenience used by the permission gate: returns the blocked host for a URL,
 * or undefined when the URL is allowed (or there is no policy / unparseable URL).
 */
export function blockedHostForUrl(cwd: string, url: string): string | undefined {
	const matcher = loadWebtoolsIgnore(cwd);
	if (!matcher) return undefined;
	const host = hostnameOf(url);
	if (!host) return undefined;
	return isHostBlocked(matcher, host) ? host : undefined;
}
