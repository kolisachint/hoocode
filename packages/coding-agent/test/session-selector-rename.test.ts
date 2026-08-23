import { setKeybindings } from "@kolisachint/hoocode-tui";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import type { SessionInfo } from "../src/core/session-manager.js";
import { SessionSelectorComponent } from "../src/modes/interactive/components/session-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

async function flushPromises(): Promise<void> {
	await new Promise<void>((resolve) => {
		setImmediate(resolve);
	});
}

function makeSession(overrides: Partial<SessionInfo> & { id: string }): SessionInfo {
	return {
		path: overrides.path ?? `/tmp/${overrides.id}.jsonl`,
		id: overrides.id,
		cwd: overrides.cwd ?? "",
		name: overrides.name,
		created: overrides.created ?? new Date(0),
		modified: overrides.modified ?? new Date(0),
		messageCount: overrides.messageCount ?? 1,
		firstMessage: overrides.firstMessage ?? "hello",
		allMessagesText: overrides.allMessagesText ?? "hello",
	};
}

/**
 * The rename verb's raw input and its display text, both read from the
 * keybinding. The picker's verbs live on alt so the query line keeps its emacs
 * editing keys; asserting on the resolved key rather than a literal keeps this
 * test honest if the binding moves again.
 */
const RENAME_KEY = new KeybindingsManager().getKeys("app.session.rename")[0]!;
const RENAME_INPUT = (() => {
	const match = /^alt\+(.)$/.exec(RENAME_KEY);
	if (!match) throw new Error(`Expected an alt+<char> binding for app.session.rename, got ${RENAME_KEY}`);
	return `\x1b${match[1]}`;
})();

describe("session selector rename", () => {
	beforeAll(() => {
		initTheme("dark");
	});

	beforeEach(() => {
		// Ensure test isolation: keybindings are a global singleton
		setKeybindings(new KeybindingsManager());
	});

	it("shows rename hint in interactive /resume picker configuration", async () => {
		const sessions = [makeSession({ id: "a" })];
		const keybindings = new KeybindingsManager();
		const selector = new SessionSelectorComponent(
			async () => sessions,
			async () => [],
			() => {},
			() => {},
			() => {},
			() => {},
			{ showRenameHint: true, keybindings },
		);
		await flushPromises();

		const output = selector.render(120).join("\n");
		expect(output).toContain(RENAME_KEY);
		expect(output).toContain("rename");
	});

	it("does not show rename hint in --resume picker configuration", async () => {
		const sessions = [makeSession({ id: "a" })];
		const keybindings = new KeybindingsManager();
		const selector = new SessionSelectorComponent(
			async () => sessions,
			async () => [],
			() => {},
			() => {},
			() => {},
			() => {},
			{ showRenameHint: false, keybindings },
		);
		await flushPromises();

		const output = selector.render(120).join("\n");
		expect(output).not.toContain(RENAME_KEY);
		expect(output).not.toContain("rename");
	});

	it("enters rename mode on the rename key and submits with Enter", async () => {
		const sessions = [makeSession({ id: "a", name: "Old" })];
		const renameSession = vi.fn(async () => {});

		const keybindings = new KeybindingsManager();
		const selector = new SessionSelectorComponent(
			async () => sessions,
			async () => [],
			() => {},
			() => {},
			() => {},
			() => {},
			{ renameSession, showRenameHint: true, keybindings },
		);
		await flushPromises();

		selector.getSessionList().handleInput(RENAME_INPUT);
		await flushPromises();

		// Rename mode layout
		const output = selector.render(120).join("\n");
		expect(output).toContain("Rename Session");
		expect(output).not.toContain("Resume Session");

		// Type and submit
		selector.handleInput("X");
		selector.handleInput("\r");
		await flushPromises();

		expect(renameSession).toHaveBeenCalledTimes(1);
		expect(renameSession).toHaveBeenCalledWith(sessions[0]!.path, "XOld");
	});
});
