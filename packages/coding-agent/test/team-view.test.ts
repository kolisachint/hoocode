import { beforeEach, describe, expect, test, vi } from "vitest";
import { taskStore } from "../src/core/task-store.js";
import { connectTeamView, TeamViewMapper } from "../src/core/team-view.js";

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
			vi.fn(async (input: RequestInfo | URL): Promise<Response> => {
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
});
