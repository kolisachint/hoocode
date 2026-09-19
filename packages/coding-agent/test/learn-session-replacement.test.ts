/**
 * `/learn` when the session it was started in is replaced mid-run.
 *
 * Mining runs for minutes on end, and nothing stops the user starting a new
 * session, resuming another, or switching mode while it does. What replaces the
 * session disposes the old one, and disposal invalidates the command ctx the run
 * captured: every accessor on it throws from that point. The run used to keep
 * going and write its result to that ctx, so a `/learn` the user had waited
 * minutes for ended as `Extension "command:learn" error: This extension ctx is
 * stale after session replacement or reload`, printed under the banner of the
 * session that had just replaced it.
 *
 * The ctx here throws the same way the real one does, so a run that fails to
 * notice would fail these tests the same way it failed on screen.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../src/core/extensions/types.js";
import { setupLearn } from "../src/extensions/core/learn.js";

const STALE = "This extension ctx is stale after session replacement or reload.";

/** A registered command and the events the extension subscribed to. */
type Harness = {
	hoo: ExtensionAPI;
	run: (args: string) => Promise<void>;
	shutdown: () => Promise<void>;
	sent: string[];
	notified: string[];
};

/**
 * A command ctx that goes stale on demand.
 *
 * Every member is a getter so `invalidate()` reaches the ones already handed
 * out, which is exactly how the runner's guarded context behaves.
 */
function makeCtx(cwd: string, auth: Promise<{ ok: false; error: string }>) {
	let stale = false;
	const guard = <T>(value: T): T => {
		if (stale) throw new Error(STALE);
		return value;
	};
	const notified: string[] = [];
	const ctx = {
		get cwd() {
			return guard(cwd);
		},
		get ui() {
			return guard({
				notify: (message: string) => notified.push(message),
				confirm: async () => true,
				onTerminalInput: () => () => {},
			});
		},
		get model() {
			return guard({ provider: "test", id: "test-model", contextWindow: 100_000 });
		},
		get modelRegistry() {
			return guard({
				getAvailable: () => [],
				find: () => undefined,
				getApiKeyAndHeaders: () => auth,
			});
		},
		get sessionManager() {
			return guard({ getSessionDir: () => join(cwd, "sessions") });
		},
	} as unknown as ExtensionCommandContext;
	const invalidate = () => {
		stale = true;
	};
	return { ctx, notified, invalidate };
}

function makeHarness(cwd: string, auth: Promise<{ ok: false; error: string }>): Harness & { invalidate: () => void } {
	const commands = new Map<string, { handler: (args: string, ctx: ExtensionCommandContext) => Promise<void> }>();
	const handlers = new Map<string, Array<(event: unknown, ctx: unknown) => unknown>>();
	const sent: string[] = [];
	const hoo = {
		registerCommand: (name: string, command: never) => commands.set(name, command),
		on: (event: string, handler: never) => {
			const list = handlers.get(event) ?? [];
			list.push(handler);
			handlers.set(event, list);
		},
		getCommands: () => [],
		sendUserMessage: (content: string) => sent.push(content),
	} as unknown as ExtensionAPI;

	setupLearn(hoo);
	const { ctx, notified, invalidate } = makeCtx(cwd, auth);
	return {
		hoo,
		sent,
		notified,
		invalidate,
		run: (args: string) => commands.get("learn")!.handler(args, ctx),
		shutdown: async () => {
			for (const handler of handlers.get("session_shutdown") ?? []) {
				await handler({ type: "session_shutdown", reason: "new" }, {});
			}
		},
	};
}

let tempDir = "";
let previousAgentDir: string | undefined;

beforeEach(() => {
	tempDir = mkdtempSync(join(tmpdir(), "learn-replacement-"));
	previousAgentDir = process.env[ENV_AGENT_DIR];
	process.env[ENV_AGENT_DIR] = join(tempDir, "agent");
});

afterEach(() => {
	if (previousAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
	else process.env[ENV_AGENT_DIR] = previousAgentDir;
	rmSync(tempDir, { recursive: true, force: true });
	tempDir = "";
});

describe("/learn when its session is replaced mid-run", () => {
	it("says nothing rather than writing to a ctx that no longer exists", async () => {
		let release: (value: { ok: false; error: string }) => void = () => {};
		const auth = new Promise<{ ok: false; error: string }>((resolve) => {
			release = resolve;
		});
		const harness = makeHarness(tempDir, auth);

		const running = harness.run("");
		// The replacement lands while the run is still waiting on the model.
		// `session_shutdown` is emitted before the old session is disposed, which
		// is the whole reason a run can still be told.
		await harness.shutdown();
		harness.invalidate();
		release({ ok: false, error: "no key" });

		await expect(running).resolves.toBeUndefined();
		expect(harness.notified).toEqual([]);
		expect(harness.sent).toEqual([]);
	});

	it("still reports to a session that is not going anywhere", async () => {
		// The control: the same run, the same failure, no replacement. Without
		// this the test above would pass just as well against a `/learn` that
		// had stopped reporting anything at all.
		const harness = makeHarness(tempDir, Promise.resolve({ ok: false as const, error: "no key" }));
		await harness.run("");
		expect(harness.notified).toHaveLength(1);
		expect(harness.notified[0]).toContain("could not authenticate");
	});
});
