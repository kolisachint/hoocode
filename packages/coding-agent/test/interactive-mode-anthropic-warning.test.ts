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
function warn(controller: any, model: unknown): Promise<void> {
	return (ModelController as any).prototype.maybeWarnAboutAnthropicSubscriptionAuth.call(controller, model);
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
