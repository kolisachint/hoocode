import { describe, expect, test, vi } from "vitest";
import { ModelController } from "../src/modes/interactive/model-controller.js";

function createSettingsManager(warnings: { anthropicExtraUsage?: boolean } = {}) {
	return {
		getWarnings: vi.fn().mockReturnValue(warnings),
	};
}

// Drives the private method through `.call` on a fake `this`. The real
// controller resolves `this.session` via a prototype getter (returning
// `this.deps.session`); the fake supplies `session` as an own property, which
// the method reads the same way, plus `deps.showWarning` for the warning sink.
// The auth predicate is a sibling prototype method, so it has to be wired onto
// the fake explicitly.
function withPrototype(controller: any): any {
	controller.usesAnthropicSubscriptionAuth = (model: unknown) =>
		(ModelController as any).prototype.usesAnthropicSubscriptionAuth.call(controller, model);
	return controller;
}

function warn(controller: any, model: unknown): Promise<void> {
	return (ModelController as any).prototype.maybeWarnAboutAnthropicSubscriptionAuth.call(
		withPrototype(controller),
		model,
	);
}

function note(controller: any, model: unknown): Promise<string | undefined> {
	return (ModelController as any).prototype.getAnthropicSubscriptionNote.call(withPrototype(controller), model);
}

describe("ModelController.maybeWarnAboutAnthropicSubscriptionAuth", () => {
	test("warns once when Anthropic subscription auth is detected", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue(undefined),
				},
				getApiKeyForProvider: vi.fn().mockResolvedValue("sk-ant-oat01-test"),
			},
		};
		const showWarning = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning },
		};

		await warn(controller, { provider: "anthropic" });
		await warn(controller, { provider: "anthropic" });

		expect(showWarning).toHaveBeenCalledTimes(1);
		expect(session.modelRegistry.getApiKeyForProvider).toHaveBeenCalledTimes(1);
	});

	test("warns when Anthropic OAuth is stored even if token refresh lookup would fail", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue({ type: "oauth" }),
				},
				getApiKeyForProvider: vi.fn().mockResolvedValue(undefined),
			},
		};
		const showWarning = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning },
		};

		await warn(controller, { provider: "anthropic" });

		expect(showWarning).toHaveBeenCalledTimes(1);
		expect(session.modelRegistry.getApiKeyForProvider).not.toHaveBeenCalled();
	});

	test("does not warn for non-Anthropic models", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn(),
				},
				getApiKeyForProvider: vi.fn(),
			},
		};
		const showWarning = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning },
		};

		await warn(controller, { provider: "openai" });

		expect(showWarning).not.toHaveBeenCalled();
		expect(session.modelRegistry.getApiKeyForProvider).not.toHaveBeenCalled();
	});

	test("does not warn when Anthropic extra usage warning is disabled", async () => {
		const session = {
			settingsManager: createSettingsManager({ anthropicExtraUsage: false }),
			modelRegistry: {
				authStorage: {
					get: vi.fn(),
				},
				getApiKeyForProvider: vi.fn(),
			},
		};
		const showWarning = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning },
		};

		await warn(controller, { provider: "anthropic" });

		expect(showWarning).not.toHaveBeenCalled();
		expect(session.modelRegistry.authStorage.get).not.toHaveBeenCalled();
		expect(session.modelRegistry.getApiKeyForProvider).not.toHaveBeenCalled();
	});
});

describe("ModelController.getAnthropicSubscriptionNote", () => {
	test("returns the compact note and claims the once-per-session slot", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue({ type: "oauth" }),
				},
				getApiKeyForProvider: vi.fn(),
			},
		};
		const showWarning = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning },
		};

		expect(await note(controller, { provider: "anthropic" })).toContain("extra usage");
		// Claimed: neither a second note nor the long warning repeats it.
		expect(await note(controller, { provider: "anthropic" })).toBeUndefined();
		await warn(controller, { provider: "anthropic" });
		expect(showWarning).not.toHaveBeenCalled();
	});

	test("returns undefined for non-subscription auth", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue(undefined),
				},
				getApiKeyForProvider: vi.fn().mockResolvedValue("sk-ant-api03-test"),
			},
		};
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showWarning: vi.fn() },
		};

		expect(await note(controller, { provider: "anthropic" })).toBeUndefined();
		expect(controller.anthropicSubscriptionWarningShown).toBe(false);
	});
});
