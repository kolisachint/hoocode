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
// the method reads the same way, plus `deps.showNotice` for the notice sink.
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
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "anthropic" });
		await warn(controller, { provider: "anthropic" });

		expect(showNotice).toHaveBeenCalledTimes(1);
		expect(session.modelRegistry.getApiKeyForProvider).toHaveBeenCalledTimes(1);
	});

	test("states the billing consequence and the way to turn it off", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue({ type: "oauth" }),
				},
				getApiKeyForProvider: vi.fn(),
			},
		};
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "anthropic" });

		const [title, body] = showNotice.mock.calls[0] as [string, string[]];
		expect(title).toContain("Anthropic");
		expect(body.join(" ")).toContain("extra usage");
		expect(body.join(" ")).toContain("/settings");
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
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "anthropic" });

		expect(showNotice).toHaveBeenCalledTimes(1);
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
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "openai" });

		expect(showNotice).not.toHaveBeenCalled();
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
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "anthropic" });

		expect(showNotice).not.toHaveBeenCalled();
		expect(session.modelRegistry.authStorage.get).not.toHaveBeenCalled();
		expect(session.modelRegistry.getApiKeyForProvider).not.toHaveBeenCalled();
	});

	test("leaves the latch unclaimed for non-subscription auth", async () => {
		const session = {
			settingsManager: createSettingsManager(),
			modelRegistry: {
				authStorage: {
					get: vi.fn().mockReturnValue(undefined),
				},
				getApiKeyForProvider: vi.fn().mockResolvedValue("sk-ant-api03-test"),
			},
		};
		const showNotice = vi.fn();
		const controller: any = {
			anthropicSubscriptionWarningShown: false,
			session,
			deps: { showNotice },
		};

		await warn(controller, { provider: "anthropic" });

		expect(showNotice).not.toHaveBeenCalled();
		expect(controller.anthropicSubscriptionWarningShown).toBe(false);
	});
});
