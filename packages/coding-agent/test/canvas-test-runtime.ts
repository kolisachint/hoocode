/**
 * Shared `CanvasRuntime` for canvas tests.
 *
 * Two things are deliberate here.
 *
 * The child runs the shim straight from `src/` under `tsx`, because the shim is
 * TypeScript and a forked child cannot import it otherwise. Production resolves
 * built JavaScript instead; `CanvasRuntime` takes the shim URL as a parameter
 * precisely so that swap needs no code change (see `docs/canvas-extensions-design.md`
 * §10, still-open item 2).
 *
 * `tsx` is resolved to an **absolute** path rather than passed as the bare
 * specifier `tsx/esm`. Node resolves a bare `--import` specifier against the
 * child's *working directory*, so `tsx/esm` silently works while an extension sits
 * inside this repository and fails with ERR_MODULE_NOT_FOUND the moment one lives
 * anywhere else — a temp directory in a trust-gate test, or a real user's
 * `~/.copilot/extensions`. Anything in `execArgv` should be absolute.
 */

import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import type { CanvasRuntime } from "../src/core/canvas/runner.js";

const require = createRequire(import.meta.url);

/** Runtime that forks children able to import the TypeScript shim. */
export function canvasTestRuntime(): CanvasRuntime {
	return {
		execPath: process.execPath,
		execArgv: ["--import", pathToFileURL(require.resolve("tsx/esm")).href],
		shimUrl: new URL("../src/core/canvas/sdk-shim/index.ts", import.meta.url).href,
	};
}
