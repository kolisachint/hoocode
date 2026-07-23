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

import { accessSync, constants, existsSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import chalk from "chalk";
import ignore from "ignore";
import { getAgentDir } from "../../config.js";
import { ensureTool } from "../../utils/tools-manager.js";
import { execCommand } from "../exec.js";

type IgnoreMatcher = ReturnType<typeof ignore>;

/** Default request timeout (seconds) passed to the binary. */
export const WEBTOOLS_DEFAULT_TIMEOUT_SECS = 15;

/** Lower/upper bounds on the effective request timeout (seconds). */
const WEBTOOLS_MIN_TIMEOUT_SECS = 1;
const WEBTOOLS_MAX_TIMEOUT_SECS = 120;

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
 * TLS plumbing forwarded to the `webtools` binary for `webfetch`/`websearch`.
 * Kept separate from hoocode's own app-level TLS trust (utils/tls-ca.ts): the
 * binary has its own TLS stack, so it needs the CA / insecure flag passed in.
 */
export interface WebtoolsTLSConfig {
	/** Path to a PEM CA bundle forwarded as `--ca-cert <path>` (validated readable). */
	caCertPath?: string;
	/** Forward `--insecure` (disables TLS verification in the binary). Strictly opt-in. */
	insecure?: boolean;
}

function isTruthyEnv(value: string | undefined): boolean {
	if (!value) return false;
	const normalized = value.trim().toLowerCase();
	return normalized === "1" || normalized === "true" || normalized === "yes";
}

/**
 * Resolve the webtools TLS config from explicit overrides (e.g. settings.json
 * passed down from the tool factories) falling back to the environment
 * (`HOOCODE_WEBTOOLS_CA_CERT`, `HOOCODE_WEBTOOLS_INSECURE`). Never hardcoded.
 */
export function resolveWebtoolsTLSConfig(overrides?: WebtoolsTLSConfig): WebtoolsTLSConfig {
	const envCaCert = process.env.HOOCODE_WEBTOOLS_CA_CERT?.trim();
	const caCertPath = overrides?.caCertPath ?? (envCaCert && envCaCert.length > 0 ? envCaCert : undefined);
	const insecure = overrides?.insecure ?? isTruthyEnv(process.env.HOOCODE_WEBTOOLS_INSECURE);
	return { caCertPath, insecure };
}

/** Clamp a request timeout to the supported range, flooring to whole seconds. */
function clampTimeoutSecs(secs: number): number {
	return Math.min(WEBTOOLS_MAX_TIMEOUT_SECS, Math.max(WEBTOOLS_MIN_TIMEOUT_SECS, Math.floor(secs)));
}

/**
 * Resolve the effective webtools request timeout (seconds) from an explicit
 * override (e.g. settings.json passed down from the tool factories) falling back
 * to the environment (`HOOCODE_WEBTOOLS_TIMEOUT`) and finally the default. Mirrors
 * {@link resolveWebtoolsTLSConfig}: resolve once, thread in, never hardcode. A
 * malformed or out-of-range env value falls back to the default; every result is
 * clamped to [1, 120].
 */
export function resolveWebtoolsTimeoutSecs(override?: number): number {
	if (override !== undefined && Number.isFinite(override)) {
		return clampTimeoutSecs(override);
	}
	const envRaw = process.env.HOOCODE_WEBTOOLS_TIMEOUT?.trim();
	if (envRaw) {
		const envValue = Number(envRaw);
		if (Number.isFinite(envValue) && envValue > 0) {
			return clampTimeoutSecs(envValue);
		}
	}
	return WEBTOOLS_DEFAULT_TIMEOUT_SECS;
}

// Warn at most once per distinct message for the life of the process.
const warnedWebtoolsMessages = new Set<string>();
function warnOnce(message: string): void {
	if (warnedWebtoolsMessages.has(message)) return;
	warnedWebtoolsMessages.add(message);
	console.warn(chalk.yellow(`[webtools] ${message}`));
}

/** True only when `path` is a readable regular file; warns once and returns false otherwise. */
function isReadableFile(path: string): boolean {
	try {
		if (!statSync(path).isFile()) {
			warnOnce(`--ca-cert path is not a regular file, ignoring: ${path}`);
			return false;
		}
		accessSync(path, constants.R_OK);
		return true;
	} catch (error) {
		const reason = error instanceof Error ? error.message : String(error);
		warnOnce(`--ca-cert path is not readable, ignoring: ${path} (${reason})`);
		return false;
	}
}

/** Build the TLS-related argv flags forwarded to the binary (argv array, no shell). */
function buildTLSArgs(config: WebtoolsTLSConfig | undefined): string[] {
	const flags: string[] = [];
	if (!config) return flags;
	if (config.caCertPath && isReadableFile(config.caCertPath)) {
		flags.push("--ca-cert", config.caCertPath);
	}
	if (config.insecure) {
		warnOnce(
			"webtools running with --insecure: TLS verification is DISABLED for webfetch/websearch. " +
				"Prefer HOOCODE_WEBTOOLS_CA_CERT to trust your proxy's CA with verification kept on.",
		);
		flags.push("--insecure");
	}
	return flags;
}

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
	tlsConfig?: WebtoolsTLSConfig,
): Promise<T> {
	if (signal?.aborted) throw new Error("Operation aborted");

	const binaryPath = await ensureTool("webtools", true);
	if (!binaryPath) throw new Error(BINARY_MISSING_MESSAGE);

	// Give the spawn a little headroom over the binary's own request timeout so
	// the binary reports the timeout itself rather than being killed mid-flight.
	const spawnTimeoutMs = (timeoutSecs + 5) * 1000;
	const tlsArgs = buildTLSArgs(tlsConfig);
	const result = await execCommand(binaryPath, [subcommand, ...args, ...tlsArgs, "--json"], cwd, {
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

/**
 * A computation shared by every caller that requested the same key while it was
 * still running. The subprocess is only aborted once *all* joined callers have
 * aborted, tracked by {@link refCount} against a shared {@link controller}.
 */
interface InFlightEntry<T> {
	promise: Promise<T>;
	controller: AbortController;
	refCount: number;
}

export class WebToolsCache<T> {
	private readonly entries = new Map<string, CacheEntry<T>>();
	private readonly inflight = new Map<string, InFlightEntry<T>>();

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

	/**
	 * Return a cached value, join an identical in-flight computation, or start a
	 * new one — collapsing concurrent duplicate fetch/search calls onto a single
	 * subprocess. Successful results are cached; failures are not.
	 *
	 * Cancellation is shared safely: a caller whose own `signal` aborts rejects
	 * promptly and releases its reference, but the underlying work keeps running
	 * for the remaining callers and is only cancelled once none are left.
	 */
	async getOrCompute(
		key: string,
		signal: AbortSignal | undefined,
		compute: (signal: AbortSignal) => Promise<T>,
	): Promise<T> {
		const cached = this.get(key);
		if (cached !== undefined) return cached;
		if (signal?.aborted) throw new Error("Operation aborted");

		let entry = this.inflight.get(key);
		if (!entry) {
			const controller = new AbortController();
			const promise = (async () => {
				try {
					const value = await compute(controller.signal);
					this.set(key, value);
					return value;
				} finally {
					this.inflight.delete(key);
				}
			})();
			entry = { promise, controller, refCount: 0 };
			this.inflight.set(key, entry);
		}

		const joined = entry;
		// Every joined caller (signalled or not) holds a reference; the shared work
		// is cancelled only when an abort drops the count back to zero.
		joined.refCount++;

		if (!signal) {
			return joined.promise;
		}

		const onAbort = () => {
			if (joined.refCount > 0) joined.refCount--;
			if (joined.refCount === 0) joined.controller.abort();
		};
		signal.addEventListener("abort", onAbort, { once: true });
		try {
			return await Promise.race([
				joined.promise,
				new Promise<never>((_, reject) => {
					signal.addEventListener("abort", () => reject(new Error("Operation aborted")), { once: true });
				}),
			]);
		} finally {
			signal.removeEventListener("abort", onAbort);
		}
	}
}

// ============================================================================
// .webtoolsignore policy matcher
// ============================================================================

/**
 * Memoize the parsed matcher per cwd. The policy is consulted on every webfetch
 * (twice: permission gate + tool execute) and every websearch, so re-reading and
 * re-parsing three files each time is wasted sync I/O on the hot path. The cache
 * is invalidated by a cheap stat signature (existence + mtime + size) so an
 * edited `.webtoolsignore` still takes effect immediately — correctness matters
 * here because this gate enforces host policy.
 */
interface IgnoreCacheEntry {
	signature: string;
	matcher: IgnoreMatcher | undefined;
}
const ignoreCacheByCwd = new Map<string, IgnoreCacheEntry>();

function ignoreSignature(files: string[]): string {
	return files
		.map((file) => {
			try {
				const st = statSync(file);
				return `${file}:${st.mtimeMs}:${st.size}`;
			} catch {
				return `${file}:absent`;
			}
		})
		.join("|");
}

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

	const signature = ignoreSignature(files);
	const cached = ignoreCacheByCwd.get(cwd);
	if (cached && cached.signature === signature) {
		return cached.matcher;
	}

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
	const matcher = found ? ig : undefined;
	ignoreCacheByCwd.set(cwd, { signature, matcher });
	return matcher;
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
