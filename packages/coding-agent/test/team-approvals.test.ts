import { describe, expect, test } from "vitest";
import { type TeamApproval, TeamApprovalCoordinator, type TeamApprovalHost } from "../src/core/team-approvals.js";

/** Host whose present() hands control to the test: resolve/skip/abort per gate. */
function manualHost() {
	const presented: Array<{ approval: TeamApproval; signal: AbortSignal; answer: (option?: string) => void }> = [];
	const resumed: Array<{ taskId: string; option: string }> = [];
	const infos: string[] = [];
	const warnings: string[] = [];
	let resumeError: Error | undefined;
	const host: TeamApprovalHost = {
		present(approval, signal) {
			return new Promise((resolve) => {
				presented.push({ approval, signal, answer: resolve });
				signal.addEventListener("abort", () => resolve(undefined), { once: true });
			});
		},
		async resume(taskId, option) {
			if (resumeError) throw resumeError;
			resumed.push({ taskId, option });
		},
		info: (message) => infos.push(message),
		warn: (message) => warnings.push(message),
	};
	return {
		host,
		presented,
		resumed,
		infos,
		warnings,
		failResumesWith: (error: Error) => {
			resumeError = error;
		},
	};
}

function paused(taskId: string, question = "Ship it?", options = ["yes", "no"]) {
	return { type: "task_paused", role: "ops", taskId, question, options };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("TeamApprovalCoordinator", () => {
	test("presents gates one at a time and resumes with the chosen option", async () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);

		coordinator.handleEvent(paused("a"));
		coordinator.handleEvent(paused("b"));
		expect(h.presented).toHaveLength(1);
		expect(coordinator.presentedTaskId()).toBe("a");
		expect(coordinator.queuedCount()).toBe(1);

		h.presented[0].answer("yes");
		await tick();
		expect(h.resumed).toEqual([{ taskId: "a", option: "yes" }]);
		// b comes up only after a settles
		expect(h.presented).toHaveLength(2);
		expect(coordinator.presentedTaskId()).toBe("b");
	});

	test("duplicate task_paused events do not re-queue a gate", () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent(paused("a"));
		coordinator.handleEvent(paused("a"));
		coordinator.handleEvent(paused("b"));
		coordinator.handleEvent(paused("b"));
		expect(h.presented).toHaveLength(1);
		expect(coordinator.queuedCount()).toBe(1);
	});

	test("task_resumed dismisses the on-screen gate without answering and tells the user", async () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent(paused("a"));
		expect(h.presented[0].signal.aborted).toBe(false);

		coordinator.handleEvent({ type: "task_resumed", role: "ops", taskId: "a", chosenOption: "yes" });
		expect(h.presented[0].signal.aborted).toBe(true);
		await tick();
		expect(h.resumed).toEqual([]);
		expect(h.infos.some((message) => message.includes("another surface"))).toBe(true);
	});

	test("task_resumed silently drops a gate that was still queued", async () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent(paused("a"));
		coordinator.handleEvent(paused("b"));
		coordinator.handleEvent({ type: "task_finished", role: "ops", taskId: "b", status: "done" });
		expect(coordinator.queuedCount()).toBe(0);

		h.presented[0].answer("yes");
		await tick();
		// b never reaches the screen
		expect(h.presented).toHaveLength(1);
	});

	test("a skipped gate stays pending server-side and the user is told", async () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent(paused("a"));
		h.presented[0].answer(undefined);
		await tick();
		expect(h.resumed).toEqual([]);
		expect(h.infos.some((message) => message.includes("stays pending"))).toBe(true);
	});

	test("a failed resume (e.g. 409 stale answer) surfaces as a warning", async () => {
		const h = manualHost();
		h.failResumesWith(new Error('task "a" was answered elsewhere'));
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent(paused("a"));
		h.presented[0].answer("yes");
		await tick();
		await tick();
		expect(h.warnings.some((message) => message.includes("answered elsewhere"))).toBe(true);
	});

	test("enqueuePending queues gates fetched from /tasks/pending", () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.enqueuePending({ taskId: "a", question: "Ship it?", options: ["yes", "no"] });
		expect(coordinator.presentedTaskId()).toBe("a");
		expect(h.presented[0].approval.role).toBeUndefined();
	});

	test("malformed task_paused events (no taskId/question) are ignored", () => {
		const h = manualHost();
		const coordinator = new TeamApprovalCoordinator(h.host);
		coordinator.handleEvent({ type: "task_paused", role: "ops" });
		coordinator.handleEvent({ type: "task_paused", role: "ops", taskId: "a" });
		expect(h.presented).toHaveLength(0);
	});
});
