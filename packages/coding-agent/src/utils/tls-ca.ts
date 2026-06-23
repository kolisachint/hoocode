/**
 * App-level TLS CA trust for hoocode's own outbound traffic (provider calls,
 * GitHub API, tool downloads). This lets hoocode work behind corporate
 * TLS-intercepting proxies WITH certificate validation kept ON, replacing the
 * insecure `NODE_TLS_REJECT_UNAUTHORIZED=0` workaround.
 *
 * Invariants:
 * - Verification is NEVER disabled (we never set `rejectUnauthorized: false`).
 * - Trust is ADDITIVE to Node's bundled roots — a custom CA extends the trust
 *   set, it does not replace it.
 * - Fail closed: a missing/invalid CA source warns once and is skipped; we never
 *   fall back to trusting everything, and there is no trust-on-first-use.
 *
 * This does NOT cover the `webfetch`/`websearch` tools — those shell out to a
 * separate `webtools` binary with its own TLS stack.
 */
import chalk from "chalk";
import { readFileSync, statSync } from "fs";
import { globalAgent } from "https";
import { getCACertificates, rootCertificates } from "tls";

// Warnings are deduplicated by message so a given problem is reported at most
// once for the life of the process ("warn once").
const warnedMessages = new Set<string>();
function warnOnce(message: string): void {
	if (warnedMessages.has(message)) return;
	warnedMessages.add(message);
	console.warn(chalk.yellow(`[tls] ${message}`));
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** Scan `process.argv` for `--flag value` or `--flag=value`, returning the value. */
function readArgValue(flag: string): string | undefined {
	const argv = process.argv;
	for (let i = 0; i < argv.length; i++) {
		const current = argv[i];
		if (current === flag) {
			const next = argv[i + 1];
			if (next !== undefined && !next.startsWith("-")) {
				const trimmed = next.trim();
				return trimmed.length > 0 ? trimmed : undefined;
			}
			return undefined;
		}
		if (current.startsWith(`${flag}=`)) {
			const trimmed = current.slice(flag.length + 1).trim();
			return trimmed.length > 0 ? trimmed : undefined;
		}
	}
	return undefined;
}

/** Scan `process.argv` for a boolean `--flag`. */
function hasArgFlag(flag: string): boolean {
	return process.argv.includes(flag);
}

/**
 * Resolve the path to an explicit PEM CA bundle from the first configured
 * source, in precedence order: `--ca-cert <path>` > `HOOCODE_CA_CERT` >
 * `NODE_EXTRA_CA_CERTS`.
 */
function resolveExplicitCAPath(): string | undefined {
	const fromFlag = readArgValue("--ca-cert");
	if (fromFlag) return fromFlag;

	const fromHoocodeEnv = process.env.HOOCODE_CA_CERT?.trim();
	if (fromHoocodeEnv) return fromHoocodeEnv;

	const fromNodeEnv = process.env.NODE_EXTRA_CA_CERTS?.trim();
	if (fromNodeEnv) return fromNodeEnv;

	return undefined;
}

/** Read a PEM bundle from a readable regular file, warning and skipping on failure. */
function readCABundle(path: string): string | undefined {
	try {
		if (!statSync(path).isFile()) {
			warnOnce(`CA certificate path is not a regular file, skipping: ${path}`);
			return undefined;
		}
		return readFileSync(path, "utf8");
	} catch (error) {
		warnOnce(`Could not read CA certificate file, skipping: ${path} (${errorMessage(error)})`);
		return undefined;
	}
}

/** True when the OS trust store has been explicitly opted into. */
function isSystemStoreOptedIn(): boolean {
	if (hasArgFlag("--use-system-ca")) return true;
	const value = process.env.HOOCODE_USE_SYSTEM_CA?.trim().toLowerCase();
	return value === "1" || value === "true" || value === "yes";
}

/** De-duplicate certificate strings while preserving insertion order. */
function dedupe(certs: string[]): string[] {
	const seen = new Set<string>();
	const result: string[] = [];
	for (const cert of certs) {
		const key = cert.trim();
		if (key.length === 0 || seen.has(key)) continue;
		seen.add(key);
		result.push(cert);
	}
	return result;
}

/**
 * Build the additive set of trusted CA certificates:
 *   (a) Node's bundled root certificates (always),
 *   (b) an explicit PEM bundle from the first configured source (always, when set),
 *   (c) the OS trust store, ONLY when explicitly opted in.
 *
 * Never throws: every source that fails is warned-once and skipped, and the
 * bundled defaults are always retained.
 */
export function resolveTrustedCAs(): string[] {
	const certs: string[] = [];

	// (a) Bundled defaults — always present so trust stays additive.
	for (const cert of rootCertificates) {
		certs.push(cert);
	}

	// (b) Explicit PEM bundle (first configured source wins).
	const explicitPath = resolveExplicitCAPath();
	if (explicitPath) {
		const bundle = readCABundle(explicitPath);
		if (bundle) certs.push(bundle);
	}

	// (c) OS trust store — opt-in only, and only if the runtime supports it.
	if (isSystemStoreOptedIn()) {
		if (typeof getCACertificates === "function") {
			try {
				for (const cert of getCACertificates("system")) {
					certs.push(cert);
				}
			} catch (error) {
				warnOnce(`Could not read the system CA store, skipping: ${errorMessage(error)}`);
			}
		} else {
			warnOnce("System CA store requested, but this Node runtime does not support tls.getCACertificates().");
		}
	}

	return dedupe(certs);
}

/**
 * Install the resolved CA set on the global HTTPS agent and return it so the
 * caller can thread the same trust set into other dispatchers (e.g. undici).
 * Warns once if `NODE_TLS_REJECT_UNAUTHORIZED=0` is set, since that disables
 * verification globally and defeats the purpose of trusting a specific CA.
 */
export function configureGlobalTLS(): string[] {
	const ca = resolveTrustedCAs();
	globalAgent.options.ca = ca;

	if (process.env.NODE_TLS_REJECT_UNAUTHORIZED === "0") {
		warnOnce(
			"NODE_TLS_REJECT_UNAUTHORIZED=0 disables all TLS certificate verification and is insecure. " +
				"Prefer --ca-cert <path> (or --use-system-ca) to trust your proxy's CA with verification kept on.",
		);
	}

	return ca;
}
