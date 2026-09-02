import type { AssistantMessage } from "@kolisachint/hoocode-ai";
import { describeProviderError } from "@kolisachint/hoocode-ai";
import { describe, expect, it } from "vitest";
import { AutoRetryController } from "../src/core/agent-session-retry.js";
import { sleep } from "../src/utils/sleep.js";

/**
 * A 429 says nothing about whether waiting will help. A burst rate limit clears
 * in seconds; an exhausted monthly quota does not clear at all inside this
 * session. Both arrive as "429", so the retry decision has to come from the
 * delay the provider asked for, not from the status.
 */

function errorMessage(text: string): AssistantMessage {
	return {
		role: "assistant",
		content: [],
		api: "anthropic-messages",
		provider: "github-copilot",
		model: "mock",
		usage: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		stopReason: "error",
		errorMessage: text,
		timestamp: Date.now(),
	};
}

/** A controller with just enough around it to answer the retry question. */
function controller(): AutoRetryController {
	return new AutoRetryController({
		getRetrySettings: () => ({ enabled: true, maxRetries: 3, baseDelayMs: 2000 }),
		getModel: () => undefined,
		getAgentMessages: () => [],
		setAgentMessages: () => {},
		continueAgent: () => {},
		waitForAgentIdle: async () => {},
		emit: () => {},
	});
}

/** The header a provider sends when the quota resets next month. */
function quotaError(): string {
	return describeProviderError(
		Object.assign(new Error("429 quota exceeded"), { headers: new Headers({ "retry-after": "2472352" }) }),
	);
}

describe("retry classification", () => {
	it("does not retry a quota that resets in weeks", () => {
		expect(controller().isRetryableError(errorMessage(quotaError()))).toBe(false);
	});

	it("still retries a burst rate limit", () => {
		const transient = describeProviderError(
			Object.assign(new Error("429 rate limit exceeded"), { headers: new Headers({ "retry-after": "30" }) }),
		);
		expect(controller().isRetryableError(errorMessage(transient))).toBe(true);
	});

	it("still retries the transient failures it always did", () => {
		for (const text of ["overloaded", "500 internal error", "fetch failed", "socket hang up"]) {
			expect(controller().isRetryableError(errorMessage(text))).toBe(true);
		}
	});

	it("still ignores a message that is not an error", () => {
		const ok = { ...errorMessage("429 quota exceeded"), stopReason: "stop" as const };
		expect(controller().isRetryableError(ok)).toBe(false);
	});
});

describe("sleep", () => {
	it("clamps a delay too large for a timer instead of firing immediately", async () => {
		// Unclamped, setTimeout replaces this with 1ms and resolves at once —
		// the hot-loop behaviour the clamp exists to prevent. Clamped, it is
		// still pending after a tick.
		const controllerSignal = new AbortController();
		let resolved = false;
		const pending = sleep(2_472_352_000, controllerSignal.signal).then(
			() => {
				resolved = true;
			},
			() => {},
		);

		await new Promise((resolve) => setTimeout(resolve, 25));
		expect(resolved).toBe(false);

		controllerSignal.abort();
		await pending;
	});

	it("still resolves a normal delay", async () => {
		await sleep(1);
	});
});
