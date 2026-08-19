/**
 * Availability: can canvases run in this process, and how do we fork one?
 *
 * Design: `docs/canvas-extensions-design.md` §11.1. Both of the traps this module
 * exists to avoid fail *silently* if the check is wrong, so each has a test naming
 * it: Bun exports `module.register` but ignores resolve hooks, and Bun reports a
 * `process.versions.node` that would satisfy a naive version check.
 */

import { describe, expect, it } from "vitest";
import {
	CANVAS_MIN_NODE_MAJOR,
	CANVAS_MIN_NODE_MINOR,
	type CanvasHostProbe,
	currentCanvasHostProbe,
	resolveCanvasRuntime,
} from "../src/core/canvas/launch.js";

const SHIM = "file:///app/dist/core/canvas/sdk-shim/index.js";

function probe(overrides: Partial<CanvasHostProbe> = {}): CanvasHostProbe {
	return {
		bunVersion: undefined,
		nodeVersion: "22.22.2",
		execPath: "/usr/bin/node",
		shimCandidates: [SHIM],
		exists: () => true,
		...overrides,
	};
}

describe("resolveCanvasRuntime", () => {
	it("produces a runtime that forks this Node with no extra argv", () => {
		const result = resolveCanvasRuntime(probe());
		expect(result).toEqual({
			available: true,
			runtime: { execPath: "/usr/bin/node", execArgv: [], shimUrl: SHIM },
		});
	});

	it("refuses under Bun, whose module.register ignores resolve hooks", () => {
		// The trap: Bun exports register() as a function, so the call succeeds and
		// nothing warns — the child then resolves the real SDK instead of the shim.
		const result = resolveCanvasRuntime(probe({ bunVersion: "1.3.11" }));
		expect(result.available).toBe(false);
		expect(result.available === false && result.reason).toMatch(/self-contained build \(Bun 1\.3\.11\)/);
	});

	it("refuses under Bun even though Bun reports a satisfying node version", () => {
		// The second trap: Bun reports process.versions.node = "24.3.0". A version
		// check alone passes, so detection must key on bunVersion being absent.
		const result = resolveCanvasRuntime(probe({ bunVersion: "1.3.11", nodeVersion: "24.3.0" }));
		expect(result.available).toBe(false);
	});

	it("refuses a Node older than module.register", () => {
		const result = resolveCanvasRuntime(probe({ nodeVersion: "20.5.1" }));
		expect(result.available).toBe(false);
		expect(result.available === false && result.reason).toMatch(/module\.register/);
	});

	it("accepts the exact minimum", () => {
		expect(
			resolveCanvasRuntime(probe({ nodeVersion: `${CANVAS_MIN_NODE_MAJOR}.${CANVAS_MIN_NODE_MINOR}.0` })).available,
		).toBe(true);
	});

	it("accepts a newer major", () => {
		expect(resolveCanvasRuntime(probe({ nodeVersion: "24.0.0" })).available).toBe(true);
	});

	it("refuses an unparseable node version rather than guessing", () => {
		expect(resolveCanvasRuntime(probe({ nodeVersion: "not-a-version" })).available).toBe(false);
	});

	it("prefers the built shim over the TypeScript source", () => {
		const result = resolveCanvasRuntime(
			probe({ shimCandidates: [SHIM, "file:///app/src/core/canvas/sdk-shim/index.ts"] }),
		);
		expect(result.available === true && result.runtime.shimUrl).toBe(SHIM);
	});

	it("falls through to the next candidate when the first is missing", () => {
		const source = "file:///app/src/core/canvas/sdk-shim/index.ts";
		const result = resolveCanvasRuntime(probe({ shimCandidates: [SHIM, source], exists: (url) => url === source }));
		expect(result.available === true && result.runtime.shimUrl).toBe(source);
	});

	it("refuses when no shim is on disk, and says the install looks incomplete", () => {
		const result = resolveCanvasRuntime(probe({ exists: () => false }));
		expect(result.available).toBe(false);
		expect(result.available === false && result.reason).toMatch(/incomplete install/);
	});

	it("never throws — an unavailable host is a state, not a failure", () => {
		expect(() => resolveCanvasRuntime(probe({ nodeVersion: "", exists: () => false }))).not.toThrow();
	});
});

describe("currentCanvasHostProbe", () => {
	it("reads this process honestly", () => {
		const actual = currentCanvasHostProbe();
		expect(actual.nodeVersion).toBe(process.versions.node);
		expect(actual.bunVersion).toBe(process.versions.bun);
		expect(actual.execPath).toBe(process.execPath);
		expect(actual.shimCandidates.length).toBeGreaterThan(0);
	});

	it("offers only the built shim, never the TypeScript source", () => {
		// A wrong "available" is worse than an honest "no": a forked child runs with
		// execArgv: [] and cannot import .ts, so a source checkout must report
		// unavailable rather than fail later at fork time.
		const actual = currentCanvasHostProbe();
		expect(actual.shimCandidates).toHaveLength(1);
		expect(actual.shimCandidates[0]?.endsWith("/sdk-shim/index.js")).toBe(true);
	});

	it("reports unavailable in this source checkout, because no built shim exists here", () => {
		const result = resolveCanvasRuntime();
		expect(result.available).toBe(false);
		expect(result.available === false && result.reason).toMatch(/built canvas shim/);
	});
});
