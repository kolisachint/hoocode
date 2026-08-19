/**
 * Whether canvases can run here, and how to fork one.
 *
 * Design: `docs/canvas-extensions-design.md` §11.1. A canvas child must run under
 * **Node ≥ 20.6** — the version that introduced `module.register`, which
 * `resolver.ts` uses to make the SDK specifier resolvable inside the child without
 * writing into the extension directory.
 *
 * The requirement is on the *child*, not on hoocode. An earlier version of this
 * module conflated the two and refused to run canvases at all under the
 * self-contained build; but a Bun-compiled parent forking a Node child is fine, so
 * the question is "is a usable Node reachable?", not "are we Node?". That makes
 * canvases available on every install path where Node exists — npm, bun, or the
 * standalone binary with Node on PATH.
 *
 * Two traps found by running it rather than reasoning about it, both of which fail
 * *silently* if you get them wrong:
 *
 *  1. **Bun exports `module.register` but ignores resolve hooks.** The call
 *     succeeds, nothing warns, and the child then resolves the real
 *     `@github/copilot-sdk` out of Bun's global install cache instead of the shim.
 *     So the child may never be Bun, however the parent was launched.
 *  2. **`process.versions.node` does not identify Node.** Bun reports
 *     `process.versions.node = "24.3.0"` next to `process.versions.bun`. Deciding
 *     "we can fork ourselves" must therefore require `process.versions.bun` to be
 *     absent, never just a satisfying Node version.
 *
 * Resolution is not cached here. It can spawn `node --version`, so the caller should
 * resolve once per session and hold the result rather than asking per open.
 */

import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import * as path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { getCanvasDir } from "../../config.js";
import type { CanvasRuntime } from "./runner.js";

const execFileAsync = promisify(execFile);

/** Minimum Node that supports `module.register`, which `resolver.ts` depends on. */
export const CANVAS_MIN_NODE_MAJOR = 20;
/** Minimum minor within {@link CANVAS_MIN_NODE_MAJOR}. */
export const CANVAS_MIN_NODE_MINOR = 6;

/** How long to wait for `node --version` before giving up on PATH discovery. */
export const CANVAS_NODE_PROBE_TIMEOUT_MS = 5_000;

/** Either a runtime that can fork canvases, or the reason none is available. */
export type CanvasAvailability = { available: true; runtime: CanvasRuntime } | { available: false; reason: string };

/** A Node executable found on PATH. */
export interface DiscoveredNode {
	/** Passed straight to `spawn`, so a bare name is fine — it resolves via PATH. */
	execPath: string;
	/** Version without the leading `v`. */
	version: string;
}

/** Host facts {@link resolveCanvasRuntime} reads. Injectable so tests need no subprocess. */
export interface CanvasHostProbe {
	/** `process.versions.bun`, or undefined on Node. */
	bunVersion: string | undefined;
	/** `process.versions.node` — meaningless as a Node check; see the module header. */
	nodeVersion: string;
	/** `process.execPath`. */
	execPath: string;
	/** Candidate shim module URLs, highest precedence first. */
	shimCandidates: string[];
	/** Whether a `file:` URL exists on disk. */
	exists: (fileUrl: string) => boolean;
	/** Locate a Node on PATH. Resolves undefined when there is none. */
	probePathNode: () => Promise<DiscoveredNode | undefined>;
}

/**
 * Where the child looks for the shim: the built layout, via `getCanvasDir()` so the
 * standalone binary's sidecar copy is found the same way themes and the HTML export
 * template are.
 *
 * The TypeScript source is deliberately **not** a candidate. A forked child runs
 * with `execArgv: []` and cannot import `.ts`, so offering `index.ts` would make
 * this function answer "available" in a source checkout and then fail at fork time
 * — a wrong yes is worse than an honest no. Running canvases from source means
 * supplying a `CanvasRuntime` directly with a TypeScript loader in `execArgv`, which
 * is what `test/canvas-test-runtime.ts` does.
 */
function defaultShimCandidates(): string[] {
	return [pathToFileURL(path.join(getCanvasDir(), "sdk-shim", "index.js")).href];
}

function fileUrlExists(fileUrl: string): boolean {
	try {
		return existsSync(fileURLToPath(fileUrl));
	} catch {
		return false;
	}
}

/**
 * Ask PATH for a Node.
 *
 * `spawn` resolves a bare command name through PATH (and PATHEXT on Windows)
 * without a shell, so there is no directory scanning to get wrong here — and unlike
 * `npx`/`npm`, `node` is a real executable rather than a `.cmd` shim, so the Windows
 * caveat in `extensions/core/mcp-loader.ts` does not apply.
 */
async function probeNodeOnPath(): Promise<DiscoveredNode | undefined> {
	try {
		const { stdout } = await execFileAsync("node", ["--version"], {
			timeout: CANVAS_NODE_PROBE_TIMEOUT_MS,
			windowsHide: true,
		});
		const version = stdout.trim().replace(/^v/, "");
		return version.length > 0 ? { execPath: "node", version } : undefined;
	} catch {
		return undefined;
	}
}

/** Read the current process. */
export function currentCanvasHostProbe(): CanvasHostProbe {
	return {
		bunVersion: process.versions.bun,
		nodeVersion: process.versions.node,
		execPath: process.execPath,
		shimCandidates: defaultShimCandidates(),
		exists: fileUrlExists,
		probePathNode: probeNodeOnPath,
	};
}

function meetsMinimum(nodeVersion: string): boolean {
	const [major, minor] = nodeVersion.split(".").map((part) => Number.parseInt(part, 10));
	if (!Number.isInteger(major) || !Number.isInteger(minor)) return false;
	if (major > CANVAS_MIN_NODE_MAJOR) return true;
	return major === CANVAS_MIN_NODE_MAJOR && minor >= CANVAS_MIN_NODE_MINOR;
}

const MIN_LABEL = `${CANVAS_MIN_NODE_MAJOR}.${CANVAS_MIN_NODE_MINOR}`;

/**
 * Decide whether canvases can run, and produce the runtime if so.
 *
 * Never throws and never guesses: an unavailable result carries a sentence a person
 * can act on, because "no Node here" is a legitimate state rather than a failure.
 */
export async function resolveCanvasRuntime(
	probe: CanvasHostProbe = currentCanvasHostProbe(),
): Promise<CanvasAvailability> {
	const shimUrl = probe.shimCandidates.find((candidate) => probe.exists(candidate));
	if (shimUrl === undefined) {
		return {
			available: false,
			reason:
				"Canvas extensions need the built canvas shim, which was not found where it ships. " +
				"This usually means an incomplete install; reinstalling hoocode should restore it.",
		};
	}

	// Forking ourselves is only sound when we are genuinely Node: Bun reports a
	// satisfying process.versions.node but ignores the resolve hook the child needs.
	if (probe.bunVersion === undefined && meetsMinimum(probe.nodeVersion)) {
		return { available: true, runtime: { execPath: probe.execPath, execArgv: [], shimUrl } };
	}

	const found = await probe.probePathNode();
	if (!found) {
		return {
			available: false,
			reason:
				`Canvas extensions run in a Node child process and no \`node\` was found on PATH. ` +
				`Install Node ${MIN_LABEL} or newer to use canvases.`,
		};
	}
	if (!meetsMinimum(found.version)) {
		return {
			available: false,
			reason:
				`Canvas extensions need Node ${MIN_LABEL} or newer for module.register(); ` +
				`the \`node\` on PATH is ${found.version}.`,
		};
	}
	return { available: true, runtime: { execPath: found.execPath, execArgv: [], shimUrl } };
}
