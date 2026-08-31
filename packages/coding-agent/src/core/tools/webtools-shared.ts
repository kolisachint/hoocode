/**
 * Shared plumbing for the `webfetch` and `websearch` tools.
 *
 * Both tools shell out to the `webtools` binary (fetch / search subcommands,
 * resolved/downloaded via {@link ensureTool}) and parse its `--json` output.
 * This module owns:
 * - the spawn-and-parse runner,
 * - the locked JSON result types,
 * - a short-lived in-process result cache,
 * - the `.webtoolsignore` policy matcher (gitignore semantics) used to block
 *   hosts both before a fetch and when filtering search result links, and
 * - the read-only check for whether `websearch` has a keyed backend configured.
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
const WEBTOOLS_DEFAULT_TIMEOUT_SECS = 15;

/** Lower/upper bounds on the effective request timeout (seconds). */
const WEBTOOLS_MIN_TIMEOUT_SECS = 1;
const WEBTOOLS_MAX_TIMEOUT_SECS = 120;

/** How long a successful result stays cached, mirroring the documented 15-min TTL. */
const CACHE_TTL_MS = 15 * 60 * 1000;

// ============================================================================
// Result types (locked against `webtools <cmd> --json`)
// ============================================================================

/**
 * Whether the binary actually extracted content (`FetchResult.status`).
 *
 * Optional here because an older `webtools` on PATH predates the field; absent
 * is treated as `ok`. Without this, a JavaScript-rendered shell and a genuinely
 * blank page are both "empty content, exit 0" and the model reads either as
 * "this page has nothing to say".
 */
export type WebFetchContentStatus = "ok" | "empty" | "needs_js" | "too_complex";

/** Whether the search answered (`SearchOutput.status`). See {@link WebFetchContentStatus} on optionality. */
export type WebSearchStatus = "ok" | "empty" | "blocked";

/**
 * The elision marker the binary appends when `--max-tokens` cuts a body
 * (`compress::TRUNCATION_MARKER`). Both output formats hoocode asks for — text
 * and markdown — route their budgeting through `truncate_to_tokens`, so its
 * presence is what tells us a page continued past what we were handed. The
 * binary reports no structured flag today; when it grows one, prefer that and
 * keep this as the fallback for older binaries.
 */
export const WEBTOOLS_TRUNCATION_MARKER = "…[truncated]";

/**
 * Whether a fetch came back cut off. Substring rather than suffix: the marker
 * lands at the end of the *body*, and the reference block is assembled after
 * it.
 */
export function isTruncatedContent(content: string | undefined): boolean {
	return typeof content === "string" && content.includes(WEBTOOLS_TRUNCATION_MARKER);
}

/** One-line explanation for a non-`ok` fetch status, mirroring the binary's own note. */
export function fetchStatusNote(status: WebFetchContentStatus | undefined): string | undefined {
	switch (status) {
		case "empty":
			return "the page parsed but contains no text";
		case "needs_js":
			return "no text content: the page renders its body with JavaScript, which webtools does not execute";
		case "too_complex":
			return "the document is too deeply nested to parse safely and was refused";
		default:
			return undefined;
	}
}

interface WebFetchReference {
	index: number;
	url: string;
	text?: string;
}

interface WebFetchMetadata {
	description?: string;
	author?: string;
	published?: string;
	lang?: string;
	site_name?: string;
}

/** One heading in a page's outline, and the span of text it opens. */
export interface WebFetchOutlineSection {
	level: number;
	title: string;
	/** Byte offset in the extracted text — the same space `offset` addresses. */
	offset: number;
	bytes: number;
	token_estimate: number;
}

export interface WebFetchResult {
	title?: string;
	final_url: string;
	/**
	 * Where this window sits in the extracted document. Absent on binaries
	 * older than the paging fields, which is why every consumer falls back to
	 * the elision marker (see {@link isTruncatedContent}) rather than treating
	 * a missing `next_offset` as "the page ended here".
	 */
	offset?: number;
	/** Byte offset to resume at, absent when the document ended in this window. */
	next_offset?: number;
	/** Size of the whole extracted body, the space offsets index into. */
	total_bytes?: number;
	/** Estimated tokens of the whole extracted body, before budget and window. */
	total_token_estimate?: number;
	/** The binary's own truncation flag; authoritative when present. */
	truncated?: boolean;
	/** The page's headings, when an outline was requested. */
	outline?: WebFetchOutlineSection[];
	content: string;
	content_type: string;
	media: string;
	token_estimate: number;
	/** Absent on binaries older than the status field; treated as "ok". */
	status?: WebFetchContentStatus;
	references: WebFetchReference[];
	metadata?: WebFetchMetadata;
	/** The URL that was requested, before any redirect (`final_url` is post-redirect). */
	source: string;
}

export interface WebSearchResultItem {
	title: string;
	snippet: string;
	url: string;
	ref_index: number;
}

interface WebSearchReference {
	index: number;
	url: string;
}

export interface WebSearchOutput {
	query: string;
	results: WebSearchResultItem[];
	references: WebSearchReference[];
	token_estimate: number;
	result_count: number;
	/** Absent on binaries older than the status field; treated as "ok". */
	status?: WebSearchStatus;
	/** Which backend answered, so a silent fallback to DuckDuckGo stays visible. */
	provider?: string;
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

// ============================================================================
// Search provider credentials
// ============================================================================

/**
 * The `webtools.search` block of `~/.hoocode/settings.json`.
 *
 * hoocode and the binary share that file: the binary reads its own `webtools`
 * key (snake_case, per its own schema) and ignores everything else, so these
 * keys are mirrored verbatim rather than camelCased. hoocode never writes them
 * — it only reads them to tell whether `websearch` has a keyed backend.
 */
export interface WebtoolsSearchSettings {
	/** Primary backend: "duckduckgo" | "brave" | "tavily" | "searxng". */
	provider?: string;
	/** Backend tried when the primary fails; "none" disables the fallback. */
	fallback?: string;
	providers?: {
		brave?: { api_key?: string };
		tavily?: { api_key?: string };
		searxng?: { base_url?: string; api_key?: string };
	};
}

/** A search backend that answers over an API contract instead of scraped HTML. */
export type KeyedSearchProvider = "brave" | "tavily" | "searxng";

export interface WebSearchCredentialStatus {
	/** A keyed backend is reachable, so search does not depend on scraped DuckDuckGo. */
	configured: boolean;
	/** Which backend the credential belongs to, when one is configured. */
	provider?: KeyedSearchProvider;
	/** Where the credential came from — env wins over the settings file. */
	source?: "env" | "settings";
	/** The user explicitly asked for the keyless backend, so nothing is missing. */
	explicitKeyless?: boolean;
}

/** Env var names per keyed provider, in the precedence the binary applies. */
const SEARCH_CREDENTIAL_ENV: ReadonlyArray<{ provider: KeyedSearchProvider; vars: readonly string[] }> = [
	{ provider: "brave", vars: ["WEBTOOLS_BRAVE_API_KEY", "BRAVE_API_KEY"] },
	{ provider: "tavily", vars: ["WEBTOOLS_TAVILY_API_KEY", "TAVILY_API_KEY"] },
	// SearXNG is self-hosted: the endpoint is the credential, its key optional.
	{ provider: "searxng", vars: ["WEBTOOLS_SEARXNG_URL"] },
];

function hasText(value: string | undefined): boolean {
	return typeof value === "string" && value.trim().length > 0;
}

/**
 * Whether `websearch` has a keyed backend configured, and where it came from.
 *
 * Mirrors the binary's own resolution order (env over settings file) for the
 * three keyed backends. This is a read-only check used to decide whether to
 * tell the user that search is running on keyless DuckDuckGo — it never
 * returns the credential itself, so a key cannot leak into the UI or a log.
 *
 * A provider pinned to `duckduckgo` (env or settings) is reported as
 * `explicitKeyless`: the user chose the scraped backend, so nothing is missing.
 */
export function resolveWebSearchCredentials(search?: WebtoolsSearchSettings): WebSearchCredentialStatus {
	const pinned = (process.env.WEBTOOLS_SEARCH_PROVIDER ?? search?.provider)?.trim().toLowerCase();
	if (pinned === "duckduckgo") {
		return { configured: false, explicitKeyless: true };
	}

	for (const { provider, vars } of SEARCH_CREDENTIAL_ENV) {
		if (vars.some((name) => hasText(process.env[name]))) {
			return { configured: true, provider, source: "env" };
		}
	}

	const providers = search?.providers;
	if (hasText(providers?.brave?.api_key)) return { configured: true, provider: "brave", source: "settings" };
	if (hasText(providers?.tavily?.api_key)) return { configured: true, provider: "tavily", source: "settings" };
	if (hasText(providers?.searxng?.base_url)) return { configured: true, provider: "searxng", source: "settings" };

	return { configured: false };
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
 * The binary bounds a whole fetch (redirects + retries) at this multiple of the
 * per-request `--timeout`, so the spawn must outlive that or we kill a fetch the
 * binary would have finished. Search has no such budget, but it may try a
 * fallback provider after the primary fails, so it gets two requests' worth.
 */
const WHOLE_RUN_TIMEOUT_MULTIPLIER: Record<"fetch" | "search", number> = {
	fetch: 3,
	search: 2,
};

/** Extra wall-clock headroom (seconds) so the binary reports its own timeout before we kill it. */
const SPAWN_TIMEOUT_HEADROOM_SECS = 5;

/**
 * Turn a non-zero exit into the most specific message available. `search` exits
 * non-zero on a blocked provider but writes its JSON (carrying `status`) to
 * stdout with nothing on stderr, so the generic "exited with code 1" would throw
 * away the only useful detail.
 */
function describeFailedRun(subcommand: "fetch" | "search", stdout: string, stderr: string, code: number): string {
	const trimmedStderr = stderr.trim();
	if (trimmedStderr) return trimmedStderr;

	try {
		const parsed = JSON.parse(stdout) as { status?: string };
		if (parsed?.status === "blocked") {
			return (
				"web search was blocked by the provider (bot challenge or rate limit) rather than returning no results — " +
				"retry later, or configure a different search provider"
			);
		}
	} catch {
		// Not JSON: fall through to the generic message.
	}
	return `webtools ${subcommand} exited with code ${code}`;
}

/**
 * Run a `webtools` subcommand with `--json` and return parsed stdout.
 *
 * Throws on missing binary, non-zero exit (surfacing the binary's stderr, or the
 * status carried on stdout when stderr is empty), or unparseable output. Callers
 * convert thrown errors into tool error results.
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

	// Give the spawn headroom over the binary's own worst-case runtime so the
	// binary reports the timeout itself rather than being killed mid-flight.
	const wholeRunSecs = timeoutSecs * WHOLE_RUN_TIMEOUT_MULTIPLIER[subcommand];
	const spawnTimeoutMs = (wholeRunSecs + SPAWN_TIMEOUT_HEADROOM_SECS) * 1000;
	const tlsArgs = buildTLSArgs(tlsConfig);
	// `--timeout` must be forwarded: without it the binary falls back to its own
	// default and the resolved setting/env value would never reach the request.
	const result = await execCommand(
		binaryPath,
		[subcommand, ...args, "--timeout", String(timeoutSecs), ...tlsArgs, "--json"],
		cwd,
		{
			signal,
			timeout: spawnTimeoutMs,
		},
	);

	if (signal?.aborted) throw new Error("Operation aborted");
	if (result.killed) throw new Error(`webtools ${subcommand} timed out after ${wholeRunSecs}s`);
	if (result.code !== 0) {
		throw new Error(describeFailedRun(subcommand, result.stdout, result.stderr, result.code));
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
