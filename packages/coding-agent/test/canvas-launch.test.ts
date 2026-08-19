/**
 * Availability: can canvases run, and what do we fork?
 *
 * Design: `docs/canvas-extensions-design.md` §11.1. The requirement is on the child
 * — Node ≥ 20.6, for `module.register` — not on hoocode, so a Bun-compiled parent
 * forking a Node child is fine. Both traps this module exists to avoid fail
 * *silently*, so each has a test naming it: Bun exports `module.register` but ignores
 * resolve hooks, and Bun reports a `process.versions.node` that satisfies a naive
 * version check.
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
		probePathNode: async () => undefined,
		...overrides,
	};
}

describe("resolveCanvasRuntime", () => {
	describe("running under Node", () => {
		it("forks this very Node, with no extra argv and no PATH lookup", async () => {
			let probed = false;
			const result = await resolveCanvasRuntime(
				probe({
					probePathNode: async () => {
						probed = true;
						return undefined;
					},
				}),
			);
			expect(result).toEqual({
				available: true,
				runtime: { execPath: "/usr/bin/node", execArgv: [], shimUrl: SHIM },
			});
			expect(probed, "should not spawn node --version when we are already Node").toBe(false);
		});

		it("accepts the exact minimum", async () => {
			const nodeVersion = `${CANVAS_MIN_NODE_MAJOR}.${CANVAS_MIN_NODE_MINOR}.0`;
			expect((await resolveCanvasRuntime(probe({ nodeVersion }))).available).toBe(true);
		});

		it("falls back to PATH when this Node is too old", async () => {
			const result = await resolveCanvasRuntime(
				probe({
					nodeVersion: "20.5.1",
					probePathNode: async () => ({ execPath: "node", version: "22.9.0" }),
				}),
			);
			expect(result).toMatchObject({ available: true, runtime: { execPath: "node" } });
		});
	});

	describe("running under the standalone Bun build", () => {
		it("forks a Node found on PATH rather than refusing outright", async () => {
			// The correction that made binary support cheap: the Node requirement is on
			// the child, so a Bun-compiled parent forking a Node child is fine.
			const result = await resolveCanvasRuntime(
				probe({
					bunVersion: "1.3.11",
					execPath: "/usr/local/bin/hoocode",
					probePathNode: async () => ({ execPath: "node", version: "22.9.0" }),
				}),
			);
			expect(result).toEqual({
				available: true,
				runtime: { execPath: "node", execArgv: [], shimUrl: SHIM },
			});
		});

		it("never forks itself, even though Bun reports a satisfying node version", async () => {
			// The trap: Bun reports process.versions.node = "24.3.0" and exports
			// module.register as a function, so both naive checks pass — and the child
			// then resolves the real SDK from Bun's cache instead of our shim.
			const result = await resolveCanvasRuntime(
				probe({
					bunVersion: "1.3.11",
					nodeVersion: "24.3.0",
					execPath: "/usr/local/bin/hoocode",
					probePathNode: async () => ({ execPath: "node", version: "22.9.0" }),
				}),
			);
			expect(result.available === true && result.runtime.execPath).toBe("node");
		});

		it("says to install Node when PATH has none", async () => {
			const result = await resolveCanvasRuntime(probe({ bunVersion: "1.3.11" }));
			expect(result.available).toBe(false);
			expect(result.available === false && result.reason).toMatch(/no `node` was found on PATH/);
		});

		it("names the version it found when PATH's Node is too old", async () => {
			const result = await resolveCanvasRuntime(
				probe({ bunVersion: "1.3.11", probePathNode: async () => ({ execPath: "node", version: "18.20.0" }) }),
			);
			expect(result.available).toBe(false);
			expect(result.available === false && result.reason).toMatch(/on PATH is 18\.20\.0/);
		});
	});

	describe("the shim", () => {
		it("refuses when none is on disk, and says the install looks incomplete", async () => {
			const result = await resolveCanvasRuntime(probe({ exists: () => false }));
			expect(result.available).toBe(false);
			expect(result.available === false && result.reason).toMatch(/incomplete install/);
		});

		it("checks the shim before probing PATH, so the actionable reason wins", async () => {
			let probed = false;
			const result = await resolveCanvasRuntime(
				probe({
					bunVersion: "1.3.11",
					exists: () => false,
					probePathNode: async () => {
						probed = true;
						return { execPath: "node", version: "22.9.0" };
					},
				}),
			);
			expect(result.available === false && result.reason).toMatch(/incomplete install/);
			expect(probed).toBe(false);
		});

		it("falls through to the next candidate when the first is missing", async () => {
			const second = "file:///app/canvas/sdk-shim/index.js";
			const result = await resolveCanvasRuntime(
				probe({ shimCandidates: [SHIM, second], exists: (url) => url === second }),
			);
			expect(result.available === true && result.runtime.shimUrl).toBe(second);
		});
	});

	it("refuses an unparseable node version rather than guessing", async () => {
		expect((await resolveCanvasRuntime(probe({ nodeVersion: "not-a-version" }))).available).toBe(false);
	});

	it("never throws — an unavailable host is a state, not a failure", async () => {
		await expect(resolveCanvasRuntime(probe({ nodeVersion: "", exists: () => false }))).resolves.toMatchObject({
			available: false,
		});
	});
});

describe("currentCanvasHostProbe", () => {
	it("reads this process honestly", () => {
		const actual = currentCanvasHostProbe();
		expect(actual.nodeVersion).toBe(process.versions.node);
		expect(actual.bunVersion).toBe(process.versions.bun);
		expect(actual.execPath).toBe(process.execPath);
	});

	it("offers only the built shim, never the TypeScript source", () => {
		// A wrong "available" is worse than an honest no: a forked child runs with
		// execArgv: [] and cannot import .ts.
		const actual = currentCanvasHostProbe();
		expect(actual.shimCandidates).toHaveLength(1);
		expect(actual.shimCandidates[0]?.endsWith("/sdk-shim/index.js")).toBe(true);
	});

	it("really can find a Node on PATH in this environment", async () => {
		const found = await currentCanvasHostProbe().probePathNode();
		expect(found?.execPath).toBe("node");
		expect(found?.version).toMatch(/^\d+\.\d+\.\d+/);
	});
});
