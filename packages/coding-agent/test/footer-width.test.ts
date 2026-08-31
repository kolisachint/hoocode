import { stripVTControlCharacters } from "node:util";
import { visibleWidth } from "@kolisachint/hoocode-tui";
import { beforeAll, describe, expect, it } from "vitest";
import type { AgentSession } from "../src/core/agent-session.js";
import type { ReadonlyFooterDataProvider } from "../src/core/footer-data-provider.js";
import { FooterComponent } from "../src/modes/interactive/components/footer.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

type AssistantUsage = {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cost: { total: number };
};

function createSession(options: {
	sessionName: string;
	modelId?: string;
	provider?: string;
	reasoning?: boolean;
	thinkingLevel?: string;
	usage?: AssistantUsage;
}): AgentSession {
	const usage = options.usage;
	const entries =
		usage === undefined
			? []
			: [
					{
						type: "message",
						message: {
							role: "assistant",
							usage,
						},
					},
				];

	const session = {
		state: {
			model: {
				id: options.modelId ?? "test-model",
				provider: options.provider ?? "test",
				contextWindow: 200_000,
				reasoning: options.reasoning ?? false,
			},
			thinkingLevel: options.thinkingLevel ?? "off",
		},
		sessionManager: {
			getEntries: () => entries,
			getSessionName: () => options.sessionName,
			// Every session has something to show: the chosen name if there is one,
			// else the slug it was auto-assigned.
			getDisplayName: () => options.sessionName || "amber-harbor",
			getCwd: () => "/tmp/project",
		},
		settingsManager: {
			getCompactionSettings: () => ({ enabled: true, reserveTokens: 16_384, keepRecentTokens: 20_000 }),
		},
		getContextUsage: () => ({ contextWindow: 200_000, percent: 12.3 }),
		modelRegistry: {
			isUsingOAuth: () => false,
		},
	};

	return session as unknown as AgentSession;
}

function createFooterData(providerCount: number): ReadonlyFooterDataProvider {
	const provider = {
		getGitBranch: () => "main",
		getExtensionStatuses: () => new Map<string, string>(),
		getAvailableProviderCount: () => providerCount,
		onBranchChange: (callback: () => void) => {
			void callback;
			return () => {};
		},
		getActiveMode: () => "build",
		getSubagentEnabled: () => false,
	};

	return provider;
}

describe("FooterComponent width handling", () => {
	beforeAll(() => {
		initTheme(undefined, false);
	});

	it("keeps all lines within width for wide session names", () => {
		const width = 93;
		const session = createSession({ sessionName: "한글".repeat(30) });
		const footer = new FooterComponent(session, createFooterData(1));

		const lines = footer.render(width);
		for (const line of lines) {
			expect(visibleWidth(line)).toBeLessThanOrEqual(width);
		}
	});

	it("hands the session name to the chip once the box is wide enough to carry one", () => {
		const session = createSession({ sessionName: "refactor-auth" });
		const footer = new FooterComponent(session, createFooterData(1));
		footer.setSessionChipShown(true);

		// Wide: the chip has the name, so line 1 must not repeat it.
		expect(stripVTControlCharacters(footer.render(120)[0]!)).not.toContain("refactor-auth");
		// Narrow: no chip is drawn there, so the footer is the only place the name
		// can appear and has to take it back — clipped by the width clamp, as
		// every other segment on this line is.
		expect(stripVTControlCharacters(footer.render(40)[0]!)).toContain("• refactor");
	});

	it("shows the auto-assigned name when no chip is set", () => {
		const session = createSession({ sessionName: "" });
		const footer = new FooterComponent(session, createFooterData(1));

		expect(stripVTControlCharacters(footer.render(120)[0]!)).toContain("amber-harbor");
	});

	it("keeps stats line within width for wide model and provider names", () => {
		const width = 60;
		const session = createSession({
			sessionName: "",
			modelId: "模".repeat(30),
			provider: "공급자",
			reasoning: true,
			thinkingLevel: "high",
			usage: {
				input: 12_345,
				output: 6_789,
				cacheRead: 0,
				cacheWrite: 0,
				cost: { total: 1.234 },
			},
		});
		const footer = new FooterComponent(session, createFooterData(2));

		const lines = footer.render(width);
		for (const line of lines) {
			expect(visibleWidth(line)).toBeLessThanOrEqual(width);
		}
	});
});
