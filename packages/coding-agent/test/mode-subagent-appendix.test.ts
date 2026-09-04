import { afterEach, describe, expect, it } from "vitest";
import { SUBAGENT_DEPTH_ENV } from "../src/core/subagent-depth.js";
import { setupMode } from "../src/extensions/core/modes.js";
import { createHarnessWithExtensions, type Harness } from "./test-harness.js";

/**
 * A spawned subagent already carries its own system prompt (from its agent
 * definition) and its own tool allowlist. Appending the parent's active-mode
 * prompt on top contradicts it — a read-only `explore` child would be told to
 * "read before editing" and "run tests after every change" — and spends tokens
 * on rules for tools the child does not have.
 *
 * The guard mirrors the light-mode one in setupMode; these lock in both
 * directions so a future refactor cannot silently drop it.
 */
describe("mode appendix in spawned subagents", () => {
	let harness: Harness | undefined;
	const savedDepth = process.env[SUBAGENT_DEPTH_ENV];

	afterEach(() => {
		harness?.cleanup();
		harness = undefined;
		if (savedDepth === undefined) delete process.env[SUBAGENT_DEPTH_ENV];
		else process.env[SUBAGENT_DEPTH_ENV] = savedDepth;
	});

	it("omits the mode appendix when the process is a spawned subagent", async () => {
		process.env[SUBAGENT_DEPTH_ENV] = "1";

		harness = await createHarnessWithExtensions({
			responses: ["ok"],
			extensionFactories: [(pi) => setupMode(pi)],
		});
		await harness.session.bindExtensions({});
		await harness.session.prompt("hi");

		expect(harness.faux.contexts[0].systemPrompt).not.toContain("<!-- hoo-core: mode=");
	});

	it("keeps the mode appendix in a top-level session", async () => {
		delete process.env[SUBAGENT_DEPTH_ENV];

		harness = await createHarnessWithExtensions({
			responses: ["ok"],
			extensionFactories: [(pi) => setupMode(pi)],
		});
		await harness.session.bindExtensions({});
		await harness.session.prompt("hi");

		expect(harness.faux.contexts[0].systemPrompt).toContain("<!-- hoo-core: mode=");
	});
});
