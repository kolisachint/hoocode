/**
 * Warm subagent pool tests.
 *
 * Drive the real WarmSubagentWorker/WarmSubagentPool against a fake RPC child
 * (test/fixtures/fake-rpc-child.mjs) so the worker lifecycle — boot, run,
 * collect, reset, reuse, idle reclaim, and infra-failure handling — is verified
 * without a model. The fake echoes its pid/generation/prompt-count so reuse and
 * reset are observable from the returned answer text.
 */

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { WarmSubagentPool, WarmWorkerError } from "../src/core/warm-subagent-pool.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FAKE_CHILD = join(__dirname, "fixtures", "fake-rpc-child.mjs");

/** A spawn command that runs the fake RPC child under node, with optional extra args/env. */
function fakeSpawn(extraArgs: string[] = []): { executable: string; prefixArgs: string[] } {
	return { executable: process.execPath, prefixArgs: [FAKE_CHILD, ...extraArgs] };
}

const opts = { agentType: "explore", cwd: process.cwd() } as const;

describe("WarmSubagentPool", () => {
	const pools: WarmSubagentPool[] = [];
	const make = (...args: ConstructorParameters<typeof WarmSubagentPool>) => {
		const pool = new WarmSubagentPool(...args);
		pools.push(pool);
		return pool;
	};

	afterEach(async () => {
		await Promise.all(pools.splice(0).map((p) => p.dispose()));
	});

	it("runs a task on a warm worker and returns its answer + usage", async () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn());
		const result = await pool.dispatch("trace the bug", opts);

		expect(result.ok).toBe(true);
		expect(result.status).toBe("complete");
		expect(result.summary).toContain("for=trace the bug");
		expect(result.usage).toEqual({ input: 11, output: 7, cacheRead: 3, cacheWrite: 0, cost: 0.002 });
	});

	it("reuses the same worker across tasks and resets it between them", async () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn());

		const first = await pool.dispatch("task one", opts);
		// Released back to the pool as an idle worker.
		expect(pool.idleCount()).toBe(1);

		const second = await pool.dispatch("task two", opts);
		expect(pool.idleCount()).toBe(1);

		const pid1 = /pid=(\d+)/.exec(first.summary)?.[1];
		const pid2 = /pid=(\d+)/.exec(second.summary)?.[1];
		// Same process handled both tasks: the boot was paid once, then reused.
		expect(pid2).toBe(pid1);
		// new_session ran between tasks, so the second run sees a bumped generation
		// and its own prompt count — proof the conversation was reset, not appended.
		expect(first.summary).toContain("gen=0");
		expect(second.summary).toContain("gen=1");
		expect(second.summary).toContain("n=2");
	});

	it("reports live tool activity during a run and clears it at the end", async () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn());
		const activity: string[] = [];
		await pool.dispatch("trace the bug", opts, (a) => activity.push(a));

		// The child runs grep, so the callback sees the tool name then a clear ("").
		expect(activity).toContain("grep");
		// Always cleared at the end so the row doesn't linger on a stale tool.
		expect(activity[activity.length - 1]).toBe("");
	});

	it("reports a task failure (turn ended in error) without throwing", async () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn(["--fail-prompt"]));
		const result = await pool.dispatch("do the thing", opts);

		expect(result.ok).toBe(false);
		expect(result.status).toBe("failed");
		expect(result.error).toContain("boom");
	});

	it("surfaces an infra failure as WarmWorkerError and discards the dead worker", async () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn());
		// Child exits on the first prompt → the run rejects with WarmWorkerError so the
		// caller can fall back to the cold pool, and no dead worker is parked.
		process.env.FAKE_EXIT_ON_PROMPT = "1";
		try {
			await expect(pool.dispatch("will crash", opts)).rejects.toBeInstanceOf(WarmWorkerError);
		} finally {
			delete process.env.FAKE_EXIT_ON_PROMPT;
		}
		expect(pool.idleCount()).toBe(0);
	});

	it("treats explore as poolable and a fork-only agent as not", () => {
		const pool = make(process.cwd(), undefined, [], 2, 30_000, fakeSpawn());
		expect(pool.isPoolable("explore")).toBe(true);
		expect(pool.isPoolable("does-not-exist")).toBe(false);
	});
});
