import { beforeEach, describe, expect, test, vi } from "vitest";
import { taskStore } from "../src/core/task-store.js";
import { connectTeamView, type TeamViewEvent, TeamViewMapper } from "../src/core/team-view.js";

describe("TeamViewMapper", () => {
	let mapper: TeamViewMapper;

	beforeEach(() => {
		taskStore.clear();
		mapper = new TeamViewMapper(taskStore);
	});

	function agent(role: string) {
		return taskStore.agents().find((a) => a.id === `team:${role}`);
	}

	function roleTask(role: string) {
		return taskStore.list().find((task) => task.agent === `team:${role}`);
	}

	test("applyStatus registers each role as a kind=role agent; only busy roles get a task", () => {
		mapper.applyStatus({
			planner: { status: "thinking", lastEventType: "turn_start" },
			coder: { status: "idle" },
		});

		const planner = agent("planner");
		expect(planner).toBeDefined();
		expect(planner?.kind).toBe("role");
		expect(planner?.name).toBe("planner");
		expect(planner?.state).toBe("active");
		expect(roleTask("planner")?.status).toBe("in_progress");

		expect(agent("coder")?.state).toBe("idle");
		expect(roleTask("coder")).toBeUndefined();
	});

	test("an all-idle team leaves the task list empty so the pane stays collapsed", () => {
		mapper.applyStatus({ planner: { status: "idle" }, coder: { status: "idle" } });
		expect(taskStore.agents()).toHaveLength(2);
		expect(taskStore.list()).toHaveLength(0);
	});

	test("mirrored tasks never outlive reset() as pending work", () => {
		mapper.applyEvent({ type: "agent_start", role: "coder" });
		mapper.applyEvent({ type: "agent_end", role: "coder" });
		mapper.applyStatus({ planner: { status: "idle" } });
		// New user turn: nothing the mirror created may survive as active work.
		taskStore.reset();
		expect(taskStore.list()).toHaveLength(0);
	});

	test("maps the TeamEvent lifecycle onto agent state and task patches", () => {
		mapper.applyEvent({ type: "agent_start", role: "coder" });
		expect(agent("coder")?.state).toBe("active");
		expect(roleTask("coder")?.status).toBe("in_progress");
		expect(roleTask("coder")?.title).toBe("thinking");

		mapper.applyEvent({ type: "tool_execution_start", role: "coder", toolName: "bash" });
		expect(agent("coder")?.state).toBe("running");
		expect(roleTask("coder")?.title).toBe("tool: bash");

		mapper.applyEvent({ type: "tool_execution_end", role: "coder", toolName: "bash" });
		expect(agent("coder")?.state).toBe("active");

		mapper.applyEvent({ type: "agent_end", role: "coder" });
		expect(agent("coder")?.state).toBe("done");
		expect(roleTask("coder")?.status).toBe("done");
	});

	test("a turn_end with an assistant errorMessage marks the role failed", () => {
		mapper.applyEvent({ type: "agent_start", role: "tester" });
		mapper.applyEvent({
			type: "turn_end",
			role: "tester",
			message: { role: "assistant", errorMessage: "boom" },
		});
		expect(agent("tester")?.state).toBe("failed");
		expect(roleTask("tester")?.status).toBe("failed");
	});

	test("agent_end after a failed turn keeps the role failed", () => {
		mapper.applyEvent({ type: "agent_start", role: "tester" });
		mapper.applyEvent({
			type: "turn_end",
			role: "tester",
			message: { role: "assistant", errorMessage: "boom" },
		});
		mapper.applyEvent({ type: "agent_end", role: "tester" });
		expect(agent("tester")?.state).toBe("failed");
		expect(roleTask("tester")?.status).toBe("failed");
	});

	test("a clean turn_end leaves the role state untouched", () => {
		mapper.applyEvent({ type: "agent_start", role: "tester" });
		mapper.applyEvent({ type: "turn_end", role: "tester", message: { role: "assistant" } });
		expect(agent("tester")?.state).toBe("active");
	});

	test("re-creates the role's task after the store was reset between turns", () => {
		mapper.applyEvent({ type: "agent_start", role: "coder" });
		mapper.applyEvent({ type: "agent_end", role: "coder" });
		// New user turn: finished tasks and idle agents are wiped.
		taskStore.reset();
		expect(taskStore.list()).toHaveLength(0);

		mapper.applyEvent({ type: "turn_start", role: "coder" });
		expect(agent("coder")?.state).toBe("active");
		expect(roleTask("coder")?.status).toBe("in_progress");
	});

	test("each role owns exactly one live task across many events", () => {
		mapper.applyStatus({ coder: { status: "idle" } });
		mapper.applyEvent({ type: "agent_start", role: "coder" });
		mapper.applyEvent({ type: "message_update", role: "coder" });
		mapper.applyEvent({ type: "tool_execution_start", role: "coder", toolName: "read" });
		expect(taskStore.list().filter((task) => task.agent === "team:coder")).toHaveLength(1);
	});

	test("events without a role are ignored", () => {
		mapper.applyEvent({ type: "agent_start" } as any);
		expect(taskStore.agents()).toHaveLength(0);
		expect(taskStore.list()).toHaveLength(0);
	});

	test("maps the orchestrator task lifecycle: started → paused (waiting) → resumed → finished", () => {
		mapper.applyEvent({ type: "task_started", role: "ops", taskId: "deploy" });
		expect(agent("ops")?.state).toBe("active");
		expect(roleTask("ops")?.title).toBe("task: deploy");

		mapper.applyEvent({
			type: "task_paused",
			role: "ops",
			taskId: "deploy",
			question: "Ship it?",
			options: ["yes", "no"],
		});
		expect(agent("ops")?.state).toBe("waiting");
		expect(roleTask("ops")?.title).toBe("awaiting approval: Ship it?");
		expect(roleTask("ops")?.status).toBe("in_progress");

		mapper.applyEvent({ type: "task_resumed", role: "ops", taskId: "deploy", chosenOption: "yes" });
		expect(agent("ops")?.state).toBe("active");

		mapper.applyEvent({ type: "task_finished", role: "ops", taskId: "deploy", status: "done" });
		expect(agent("ops")?.state).toBe("done");
		expect(roleTask("ops")?.status).toBe("done");
	});

	test("task_finished with status error marks the role failed", () => {
		mapper.applyEvent({ type: "task_started", role: "ops", taskId: "deploy" });
		mapper.applyEvent({ type: "task_finished", role: "ops", taskId: "deploy", status: "error" });
		expect(agent("ops")?.state).toBe("failed");
		expect(roleTask("ops")?.status).toBe("failed");
	});

	test("agent_end after task_paused keeps the role waiting (gate stays open, not idle)", () => {
		// hooteams mirrors agent_end right after task_paused (the run ends on the
		// approval gate). Without the guard this flipped the role to done/idle while
		// the AskOptions pane was still open.
		mapper.applyEvent({ type: "task_started", role: "ops", taskId: "deploy" });
		mapper.applyEvent({
			type: "task_paused",
			role: "ops",
			taskId: "deploy",
			question: "Ship it?",
			options: ["yes", "no"],
		});
		expect(agent("ops")?.state).toBe("waiting");

		mapper.applyEvent({ type: "agent_end", role: "ops" });
		expect(agent("ops")?.state).toBe("waiting");
		expect(roleTask("ops")?.title).toBe("awaiting approval: Ship it?");

		// Answering resumes the role; a later agent_end then settles it done.
		mapper.applyEvent({ type: "task_resumed", role: "ops", taskId: "deploy", chosenOption: "yes" });
		expect(agent("ops")?.state).toBe("active");
		mapper.applyEvent({ type: "agent_end", role: "ops" });
		expect(agent("ops")?.state).toBe("done");
	});

	test("a paused role opens a waiting task even from cold (gate pending before attach)", () => {
		mapper.applyEvent({ type: "task_paused", role: "ops", taskId: "deploy", question: "Ship it?", options: [] });
		expect(agent("ops")?.state).toBe("waiting");
		expect(roleTask("ops")?.status).toBe("in_progress");
	});

	test("applyStatus maps the paused status word to waiting", () => {
		mapper.applyStatus({ ops: { status: "paused" } });
		expect(agent("ops")?.state).toBe("waiting");
	});

	test("dag settlement events never create an orchestrator roster entry", () => {
		mapper.applyEvent({ type: "dag_complete", role: "orchestrator", runId: "run-1" });
		mapper.applyEvent({ type: "dag_failed", role: "orchestrator", runId: "run-1" });
		expect(taskStore.agents()).toHaveLength(0);
	});
});

describe("connectTeamView", () => {
	beforeEach(() => {
		taskStore.clear();
	});

	function sseResponse(frames: string[]): Response {
		return new Response(
			new ReadableStream<Uint8Array>({
				start(controller) {
					for (const frame of frames) controller.enqueue(new TextEncoder().encode(frame));
					controller.close();
				},
			}),
		);
	}

	async function until(condition: () => boolean, timeoutMs = 2000): Promise<void> {
		const start = Date.now();
		while (!condition()) {
			if (Date.now() - start > timeoutMs) throw new Error("condition not met in time");
			await new Promise((resolve) => setTimeout(resolve, 5));
		}
	}

	test("a flapping /events stream warns once; only delivered data re-arms the warning", async () => {
		const warnings: string[] = [];
		let eventsCalls = 0;
		let dataStreams = 0;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request): Promise<Response> => {
				if (String(input).endsWith("/status")) return Response.json({});
				eventsCalls++;
				if (dataStreams > 0) {
					dataStreams--;
					return sseResponse(['data: {"type":"agent_start","role":"coder"}\n\n']);
				}
				return sseResponse([]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: (message) => warnings.push(message),
				retryDelayMs: 1,
				store: taskStore,
			});
			const dropWarnings = () => warnings.filter((w) => w.includes("lost connection")).length;

			// A 200 that closes without ever streaming used to warn on every retry.
			await until(() => eventsCalls >= 3);
			expect(dropWarnings()).toBe(1);

			// Once data actually flows the stream counts as recovered, so the next
			// drop is announced again — exactly once, because the streams after the
			// single data-bearing one are empty again.
			dataStreams = 1;
			await until(() => taskStore.agents().some((a) => a.id === "team:coder"));
			await until(() => dropWarnings() === 2);
			view.stop();
		} finally {
			vi.unstubAllGlobals();
		}
	});

	test("subscribers receive every event from the shared stream; unsubscribe detaches", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request): Promise<Response> => {
				if (String(input).endsWith("/status")) return Response.json({});
				return sseResponse([
					'data: {"type":"agent_start","role":"coder"}\n\n',
					'data: {"type":"agent_start","role":"planner"}\n\n',
				]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: () => {},
				retryDelayMs: 60_000,
				store: taskStore,
			});
			const seen: TeamViewEvent[] = [];
			const unsubscribe = view.subscribe((event) => seen.push(event));
			expect(view.subscriberCount()).toBe(1);

			await until(() => seen.length >= 2);
			// Unfiltered fan-out: events for all roles arrive (the attach panel filters).
			expect(seen.map((event) => event.role)).toEqual(["coder", "planner"]);
			// The task-store mirror keeps working alongside subscribers.
			expect(taskStore.agents().some((a) => a.id === "team:coder")).toBe(true);

			unsubscribe();
			expect(view.subscriberCount()).toBe(0);
			view.stop();
		} finally {
			vi.unstubAllGlobals();
		}
	});

	test("steer POSTs { role, message } to /steer and rejects on HTTP errors", async () => {
		const steerBodies: unknown[] = [];
		let steerStatus = 200;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
				const url = String(input);
				if (url.endsWith("/status")) return Response.json({});
				if (url.endsWith("/steer")) {
					expect(init?.method).toBe("POST");
					steerBodies.push(JSON.parse(String(init?.body)));
					return new Response(null, { status: steerStatus });
				}
				return sseResponse([]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: () => {},
				retryDelayMs: 60_000,
				store: taskStore,
			});
			await view.steer("coder", "focus on the failing test");
			expect(steerBodies).toEqual([{ role: "coder", message: "focus on the failing test" }]);

			steerStatus = 500;
			await expect(view.steer("coder", "again")).rejects.toThrow("HTTP 500");
			view.stop();
		} finally {
			vi.unstubAllGlobals();
		}
	});

	test("resume POSTs { option, feedback } to /tasks/:id/resume; 409 reads as answered elsewhere", async () => {
		const resumeCalls: Array<{ url: string; body: unknown }> = [];
		let resumeStatus = 200;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
				const url = String(input);
				if (url.endsWith("/status")) return Response.json({});
				if (url.includes("/tasks/")) {
					expect(init?.method).toBe("POST");
					resumeCalls.push({ url, body: JSON.parse(String(init?.body)) });
					return new Response(null, { status: resumeStatus });
				}
				return sseResponse([]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: () => {},
				retryDelayMs: 60_000,
				store: taskStore,
			});
			await view.resume("deploy", "yes", "ship it");
			expect(resumeCalls).toEqual([
				{ url: "http://localhost:9/tasks/deploy/resume", body: { option: "yes", feedback: "ship it" } },
			]);

			resumeStatus = 409;
			await expect(view.resume("deploy", "no")).rejects.toThrow("answered elsewhere");
			expect(resumeCalls[1]?.body).toEqual({ option: "no" });

			resumeStatus = 500;
			await expect(view.resume("deploy", "no")).rejects.toThrow("HTTP 500");
			view.stop();
		} finally {
			vi.unstubAllGlobals();
		}
	});

	test("pendingApprovals returns the open gates; a 404 (no active run) reads as none", async () => {
		let pendingStatus = 200;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request): Promise<Response> => {
				const url = String(input);
				if (url.endsWith("/status")) return Response.json({});
				if (url.endsWith("/tasks/pending")) {
					if (pendingStatus !== 200) return new Response(null, { status: pendingStatus });
					return Response.json({
						runId: "run-1",
						pending: [{ taskId: "deploy", question: "Ship it?", options: ["yes", "no"] }],
					});
				}
				return sseResponse([]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: () => {},
				retryDelayMs: 60_000,
				store: taskStore,
			});
			expect(await view.pendingApprovals()).toEqual([
				{ taskId: "deploy", question: "Ship it?", options: ["yes", "no"] },
			]);

			pendingStatus = 404;
			expect(await view.pendingApprovals()).toEqual([]);

			pendingStatus = 500;
			await expect(view.pendingApprovals()).rejects.toThrow("HTTP 500");
			view.stop();
		} finally {
			vi.unstubAllGlobals();
		}
	});

	test("stop() clears subscribers", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string | URL | Request): Promise<Response> => {
				if (String(input).endsWith("/status")) return Response.json({});
				return sseResponse([]);
			}),
		);
		try {
			const view = connectTeamView("http://localhost:9", {
				warn: () => {},
				retryDelayMs: 60_000,
				store: taskStore,
			});
			view.subscribe(() => {});
			view.subscribe(() => {});
			expect(view.subscriberCount()).toBe(2);
			view.stop();
			expect(view.subscriberCount()).toBe(0);
		} finally {
			vi.unstubAllGlobals();
		}
	});
});
