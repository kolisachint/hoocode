import { stripVTControlCharacters } from "node:util";
import { beforeAll, describe, expect, it } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import { sessionSlugFor } from "../src/core/session-identity.js";
import type { SessionInfo } from "../src/core/session-manager.js";
import { SessionSelectorComponent } from "../src/modes/interactive/components/session-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

function makeSession(overrides: Partial<SessionInfo> & { id: string }): SessionInfo {
	return {
		path: overrides.path ?? `/tmp/${overrides.id}.jsonl`,
		id: overrides.id,
		cwd: overrides.cwd ?? "",
		name: overrides.name,
		color: overrides.color,
		branch: overrides.branch,
		parentSessionPath: overrides.parentSessionPath,
		created: overrides.created ?? new Date(0),
		modified: overrides.modified ?? new Date(0),
		messageCount: overrides.messageCount ?? 1,
		firstMessage: overrides.firstMessage ?? "look at this",
		allMessagesText: overrides.allMessagesText ?? "look at this",
	};
}

const flushPromises = () => new Promise((resolve) => setTimeout(resolve, 0));

async function render(sessions: SessionInfo[], width = 120): Promise<string> {
	const selector = new SessionSelectorComponent(
		async () => sessions,
		async () => [],
		() => {},
		() => {},
		() => {},
		() => {},
		{ keybindings: KeybindingsManager.create() },
	);
	await flushPromises();
	return stripVTControlCharacters(selector.render(width).join("\n"));
}

describe("session selector branch column", () => {
	beforeAll(() => {
		initTheme(undefined, false);
	});

	// The case the list could not answer before: nobody named it, the opening
	// message says nothing, and the branch says exactly what it was for.
	it("shows the branch for an unnamed session whose first message is vague", async () => {
		const output = await render([makeSession({ id: "a", branch: "refactor-auth" })]);

		expect(output).toContain("refactor-auth");
		expect(output).toContain("look at this");
	});

	it("stays quiet for a session that already says what it is", async () => {
		const output = await render([makeSession({ id: "a", name: "parser spike", branch: "refactor-auth" })]);

		expect(output).toContain("parser spike");
		expect(output).not.toContain("refactor-auth");
	});

	it("spends no width on a branch that names no work", async () => {
		for (const branch of ["main", "master", "trunk", "develop"]) {
			const output = await render([makeSession({ id: "a", branch })]);
			expect(output, branch).not.toContain(branch);
		}
	});

	it("says nothing for sessions recorded before the branch was kept", async () => {
		const output = await render([makeSession({ id: "a" })]);

		expect(output).toContain("look at this");
		// Still identifiable by the slug its chip was showing.
		expect(output).toContain(sessionSlugFor("a"));
	});

	it("keeps the row inside the terminal width", async () => {
		const sessions = [makeSession({ id: "a", branch: "a-very-long-feature-branch-name-indeed" })];
		for (const width of [40, 60, 80, 120]) {
			const output = await render(sessions, width);
			for (const line of output.split("\n")) {
				expect(line.length, `width ${width}: ${line}`).toBeLessThanOrEqual(width);
			}
		}
	});
});
