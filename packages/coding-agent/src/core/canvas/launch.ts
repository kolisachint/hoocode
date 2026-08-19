/**
 * Whether canvases can run here, and how to fork one.
 *
 * Design: `docs/canvas-extensions-design.md` §11.1. Canvas support requires hoocode
 * to be running under **Node ≥ 20.6** — the version that introduced
 * `module.register`, which `resolver.ts` uses to make the SDK specifier resolvable
 * inside the child without writing into the extension directory.
 *
 * That is the npm install path (`npm install -g @kolisachint/hoocode-agent`), where
 * two things hold at once: `process.execPath` is a Node we can fork, and the built
 * `dist` is on disk for the child to import the shim from. Under the self-contained
 * binary, canvases are unavailable and this module says why.
 *
 * Two traps found by running it rather than reasoning about it, both of which fail
 * *silently* if you get them wrong:
 *
 *  1. **Bun exports `module.register` but ignores resolve hooks.** The call
 *     succeeds, nothing warns, and the child then resolves the real
 *     `@github/copilot-sdk` out of Bun's global install cache instead of the shim.
 *  2. **`process.versions.node` does not identify Node.** Bun reports
 *     `process.versions.node = "24.3.0"` next to `process.versions.bun`. Detection
 *     must therefore key on `process.versions.bun` being *absent*, never on the
 *     Node version alone.
 */

import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { CanvasRuntime } from "./runner.js";

/** Minimum Node that supports `module.register`, which `resolver.ts` depends on. */
export const CANVAS_MIN_NODE_MAJOR = 20;
/** Minimum minor within {@link CANVAS_MIN_NODE_MAJOR}. */
export const CANVAS_MIN_NODE_MINOR = 6;

/** Either a runtime that can fork canvases, or the reason none is available. */
export type CanvasAvailability = { available: true; runtime: CanvasRuntime } | { available: false; reason: string };

/** Host facts {@link resolveCanvasRuntime} reads. Injectable so tests need no subprocess. */
export interface CanvasHostProbe {
	/** `process.versions.bun`, or undefined on Node. */
	bunVersion: string | undefined;
	/** `process.versions.node`. */
	nodeVersion: string;
	/** `process.execPath`. */
	execPath: string;
	/** Candidate shim module URLs, highest precedence first. */
	shimCandidates: string[];
	/** Whether a `file:` URL exists on disk. */
	exists: (fileUrl: string) => boolean;
}

/**
 * Where the child looks for the shim.
 *
 * The built layout only — `dist/core/canvas/sdk-shim/index.js`, beside the compiled
 * copy of this module.
 *
 * The TypeScript source is deliberately **not** a candidate. A forked child runs
 * with `execArgv: []` and cannot import `.ts`, so offering `index.ts` would make
 * this function answer "available" in a source checkout and then fail at fork time
 * — a wrong yes is worse than an honest no. Running canvases from source therefore
 * means supplying a `CanvasRuntime` directly with a TypeScript loader in `execArgv`,
 * which is what `test/canvas-test-runtime.ts` does.
 */
function defaultShimCandidates(): string[] {
	return [new URL("./sdk-shim/index.js", import.meta.url).href];
}

function fileUrlExists(fileUrl: string): boolean {
	try {
		return existsSync(fileURLToPath(fileUrl));
	} catch {
		return false;
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
	};
}

function meetsMinimum(nodeVersion: string): boolean {
	const [major, minor] = nodeVersion.split(".").map((part) => Number.parseInt(part, 10));
	if (!Number.isInteger(major) || !Number.isInteger(minor)) return false;
	if (major > CANVAS_MIN_NODE_MAJOR) return true;
	return major === CANVAS_MIN_NODE_MAJOR && minor >= CANVAS_MIN_NODE_MINOR;
}

/**
 * Decide whether canvases can run in this process, and produce the runtime if so.
 *
 * Never throws and never guesses: an unavailable result carries a sentence a person
 * can act on, because "canvas support is off" is a legitimate state under the
 * packaged binary rather than a failure.
 */
export function resolveCanvasRuntime(probe: CanvasHostProbe = currentCanvasHostProbe()): CanvasAvailability {
	if (probe.bunVersion !== undefined) {
		return {
			available: false,
			reason:
				`Canvas extensions need hoocode running under Node ${CANVAS_MIN_NODE_MAJOR}.${CANVAS_MIN_NODE_MINOR} or newer; ` +
				`this is the self-contained build (Bun ${probe.bunVersion}), whose module resolver cannot redirect the ` +
				`Copilot SDK import a canvas relies on. Install hoocode from npm to use canvases.`,
		};
	}

	if (!meetsMinimum(probe.nodeVersion)) {
		return {
			available: false,
			reason:
				`Canvas extensions need Node ${CANVAS_MIN_NODE_MAJOR}.${CANVAS_MIN_NODE_MINOR} or newer for ` +
				`module.register(); this process is Node ${probe.nodeVersion}.`,
		};
	}

	const shimUrl = probe.shimCandidates.find((candidate) => probe.exists(candidate));
	if (shimUrl === undefined) {
		return {
			available: false,
			reason:
				"Canvas extensions need the built canvas shim, which was not found next to this module. " +
				"This usually means an incomplete install; reinstalling hoocode should restore it.",
		};
	}

	return { available: true, runtime: { execPath: probe.execPath, execArgv: [], shimUrl } };
}
