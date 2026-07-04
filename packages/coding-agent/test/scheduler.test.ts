import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { matchesCron, TaskScheduler } from "../src/core/scheduler.js";

describe("matchesCron", () => {
	it("matches a wildcard expression at any time", () => {
		expect(matchesCron("* * * * *", new Date(2026, 0, 5, 14, 30))).toBe(true);
	});

	it("matches step minutes", () => {
		expect(matchesCron("*/5 * * * *", new Date(2026, 0, 5, 14, 10))).toBe(true);
		expect(matchesCron("*/5 * * * *", new Date(2026, 0, 5, 14, 12))).toBe(false);
	});

	it("matches an exact time", () => {
		expect(matchesCron("30 14 * * *", new Date(2026, 0, 5, 14, 30))).toBe(true);
		expect(matchesCron("30 14 * * *", new Date(2026, 0, 5, 14, 31))).toBe(false);
	});

	it("matches weekday ranges (Jan 5 2026 is a Monday)", () => {
		expect(matchesCron("0 9 * * 1-5", new Date(2026, 0, 5, 9, 0))).toBe(true);
		// Jan 4 2026 is a Sunday
		expect(matchesCron("0 9 * * 1-5", new Date(2026, 0, 4, 9, 0))).toBe(false);
	});

	it("rejects malformed expressions", () => {
		expect(matchesCron("* * * *", new Date())).toBe(false);
		expect(matchesCron("nonsense", new Date())).toBe(false);
	});
});

describe("TaskScheduler", () => {
	let tempDir: string;
	let storePath: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-sched-"));
		storePath = path.join(tempDir, "scheduled_tasks.json");
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("creates, persists, lists, and deletes tasks", () => {
		const s = new TaskScheduler({ storePath, fire: () => {} });
		const task = s.create({ cron: "*/5 * * * *", prompt: "hi" });
		expect(s.list()).toHaveLength(1);
		expect(fs.existsSync(storePath)).toBe(true);

		// A fresh scheduler reads the persisted task.
		const reloaded = new TaskScheduler({ storePath, fire: () => {} });
		expect(reloaded.list()).toHaveLength(1);

		expect(s.delete(task.id)).toBe(true);
		expect(s.list()).toHaveLength(0);
	});

	it("fires due tasks once per minute and respects idle gating", () => {
		const fired: string[] = [];
		let idle = true;
		const s = new TaskScheduler({ storePath, fire: (p) => fired.push(p), isIdle: () => idle });
		s.create({ cron: "30 14 * * *", prompt: "run" });

		const due = new Date(2026, 0, 5, 14, 30, 0);
		idle = false;
		s.tick(due);
		expect(fired).toHaveLength(0); // busy → skipped

		idle = true;
		s.tick(due);
		expect(fired).toEqual(["run"]);

		// Same minute → no double fire.
		s.tick(new Date(2026, 0, 5, 14, 30, 45));
		expect(fired).toHaveLength(1);
	});

	it("reads the legacy store when the primary one is absent, then migrates forward", () => {
		const legacyStorePath = path.join(tempDir, "legacy_tasks.json");
		fs.writeFileSync(
			legacyStorePath,
			JSON.stringify({
				tasks: [{ id: "old", cron: "*/5 * * * *", prompt: "legacy", recurring: true, createdAt: 1 }],
			}),
		);

		// Primary absent → falls back to the legacy store.
		const s = new TaskScheduler({ storePath, legacyStorePath, fire: () => {} });
		expect(s.list().map((t) => t.prompt)).toEqual(["legacy"]);
		expect(fs.existsSync(storePath)).toBe(false);

		// First mutation persists to the primary path (migration forward).
		s.create({ cron: "0 9 * * *", prompt: "new" });
		expect(fs.existsSync(storePath)).toBe(true);

		// A fresh scheduler now prefers the primary store and ignores the legacy one.
		const reloaded = new TaskScheduler({ storePath, legacyStorePath, fire: () => {} });
		expect(
			reloaded
				.list()
				.map((t) => t.prompt)
				.sort(),
		).toEqual(["legacy", "new"]);
	});

	it("deletes one-shot tasks after firing", () => {
		const fired: string[] = [];
		const s = new TaskScheduler({ storePath, fire: (p) => fired.push(p) });
		s.create({ cron: "30 14 * * *", prompt: "once", recurring: false });
		s.tick(new Date(2026, 0, 5, 14, 30));
		expect(fired).toEqual(["once"]);
		expect(s.list()).toHaveLength(0);
	});
});
