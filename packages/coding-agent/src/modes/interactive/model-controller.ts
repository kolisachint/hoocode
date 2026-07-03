/**
 * Model selection for the interactive mode (/model, /models, cycle keys).
 *
 * Owns the single-model picker, the scoped-models (enable set) picker, model
 * cycling, exact-match lookup for slash-command arguments, the footer's
 * available-provider count, and the Anthropic subscription-auth warning.
 * Extracted from interactive-mode.ts behind a narrow ModelControllerDeps
 * interface.
 */

import type { Model } from "@kolisachint/hoocode-ai";
import type { Component, TUI } from "@kolisachint/hoocode-tui";
import type { AgentSession } from "../../core/agent-session.js";
import { findExactModelReferenceMatch, resolveModelScope } from "../../core/model-resolver.js";
import { ModelSelectorComponent } from "./components/model-selector.js";
import { ScopedModelsSelectorComponent } from "./components/scoped-models-selector.js";

const ANTHROPIC_SUBSCRIPTION_AUTH_WARNING =
	"Anthropic subscription auth: billed per token as extra usage, not plan limits.";

function isAnthropicSubscriptionAuthKey(apiKey: string | undefined): boolean {
	return typeof apiKey === "string" && apiKey.startsWith("sk-ant-oat");
}

/** The slice of the interactive mode the model flows need. */
export interface ModelControllerDeps {
	ui: TUI;
	/** The active session (read at call time; the session can be swapped). */
	get session(): AgentSession;
	showSelector(create: (done: () => void) => { component: Component; focus: Component }): void;
	showStatus(message: string): void;
	showError(errorMessage: string): void;
	showWarning(warningMessage: string): void;
	updateEditorBorderColor(): void;
	invalidateFooter(): void;
	setAvailableProviderCount(count: number): void;
}

export class ModelController {
	private anthropicSubscriptionWarningShown = false;

	constructor(private readonly deps: ModelControllerDeps) {}

	private get session(): AgentSession {
		return this.deps.session;
	}

	async findExactModelMatch(searchTerm: string): Promise<Model<any> | undefined> {
		const models = await this.getModelCandidates();
		return findExactModelReferenceMatch(searchTerm, models);
	}

	private async getModelCandidates(): Promise<Model<any>[]> {
		if (this.session.scopedModels.length > 0) {
			return this.session.scopedModels.map((scoped) => scoped.model);
		}

		this.session.modelRegistry.refresh();
		try {
			return await this.session.modelRegistry.getAvailable();
		} catch {
			return [];
		}
	}

	/** Update the footer's available provider count from current model candidates */
	async updateAvailableProviderCount(): Promise<void> {
		const models = await this.getModelCandidates();
		const uniqueProviders = new Set(models.map((m) => m.provider));
		this.deps.setAvailableProviderCount(uniqueProviders.size);
	}

	async maybeWarnAboutAnthropicSubscriptionAuth(model: Model<any> | undefined = this.session.model): Promise<void> {
		if (this.session.settingsManager.getWarnings().anthropicExtraUsage === false) {
			return;
		}
		if (this.anthropicSubscriptionWarningShown) {
			return;
		}
		if (!model || model.provider !== "anthropic") {
			return;
		}

		const storedCredential = this.session.modelRegistry.authStorage.get("anthropic");
		if (storedCredential?.type === "oauth") {
			this.anthropicSubscriptionWarningShown = true;
			this.deps.showWarning(ANTHROPIC_SUBSCRIPTION_AUTH_WARNING);
			return;
		}

		try {
			const apiKey = await this.session.modelRegistry.getApiKeyForProvider(model.provider);
			if (!isAnthropicSubscriptionAuthKey(apiKey)) {
				return;
			}
			this.anthropicSubscriptionWarningShown = true;
			this.deps.showWarning(ANTHROPIC_SUBSCRIPTION_AUTH_WARNING);
		} catch {
			// Ignore auth lookup failures for warning-only checks.
		}
	}

	async cycleModel(direction: "forward" | "backward"): Promise<void> {
		try {
			const result = await this.session.cycleModel(direction);
			if (result === undefined) {
				const msg = this.session.scopedModels.length > 0 ? "Only one model in scope" : "Only one model available";
				this.deps.showStatus(msg);
			} else {
				this.deps.invalidateFooter();
				this.deps.updateEditorBorderColor();
				const thinkingStr =
					result.model.reasoning && result.thinkingLevel !== "off" ? ` (thinking: ${result.thinkingLevel})` : "";
				this.deps.showStatus(`Switched to ${result.model.name || result.model.id}${thinkingStr}`);
				void this.maybeWarnAboutAnthropicSubscriptionAuth(result.model);
			}
		} catch (error) {
			this.deps.showError(error instanceof Error ? error.message : String(error));
		}
	}

	showModelSelector(initialSearchInput?: string): void {
		this.deps.showSelector((done) => {
			const selector = new ModelSelectorComponent(
				this.deps.ui,
				this.session.model,
				this.session.settingsManager,
				this.session.modelRegistry,
				this.session.scopedModels,
				async (model) => {
					try {
						await this.session.setModel(model);
						this.deps.invalidateFooter();
						this.deps.updateEditorBorderColor();
						done();
						this.deps.showStatus(`Model: ${model.id}`);
						void this.maybeWarnAboutAnthropicSubscriptionAuth(model);
					} catch (error) {
						done();
						this.deps.showError(error instanceof Error ? error.message : String(error));
					}
				},
				() => {
					done();
					this.deps.ui.requestRender();
				},
				initialSearchInput,
			);
			return { component: selector, focus: selector };
		});
	}

	async showModelsSelector(): Promise<void> {
		// Get all available models
		this.session.modelRegistry.refresh();
		const allModels = this.session.modelRegistry.getAvailable();

		if (allModels.length === 0) {
			this.deps.showStatus("No models available");
			return;
		}

		// Check if session has scoped models (from previous session-only changes or CLI --models)
		const sessionScopedModels = this.session.scopedModels;
		const hasSessionScope = sessionScopedModels.length > 0;

		// Build enabled model IDs from session state or settings
		let currentEnabledIds: string[] | null = null;

		if (hasSessionScope) {
			// Use current session's scoped models
			currentEnabledIds = sessionScopedModels.map((scoped) => `${scoped.model.provider}/${scoped.model.id}`);
		} else {
			// Fall back to settings
			const patterns = this.session.settingsManager.getEnabledModels();
			if (patterns !== undefined && patterns.length > 0) {
				const scopedModels = await resolveModelScope(patterns, this.session.modelRegistry);
				currentEnabledIds = scopedModels.map((scoped) => `${scoped.model.provider}/${scoped.model.id}`);
			}
		}

		// Helper to update session's scoped models (session-only, no persist)
		const updateSessionModels = async (enabledIds: string[] | null) => {
			currentEnabledIds = enabledIds === null ? null : [...enabledIds];
			if (enabledIds && enabledIds.length > 0 && enabledIds.length < allModels.length) {
				const newScopedModels = await resolveModelScope(enabledIds, this.session.modelRegistry);
				this.session.setScopedModels(
					newScopedModels.map((sm) => ({
						model: sm.model,
						thinkingLevel: sm.thinkingLevel,
					})),
				);
			} else {
				// All enabled or none enabled = no filter
				this.session.setScopedModels([]);
			}
			await this.updateAvailableProviderCount();
			this.deps.ui.requestRender();
		};

		this.deps.showSelector((done) => {
			const selector = new ScopedModelsSelectorComponent(
				{
					allModels,
					enabledModelIds: currentEnabledIds,
				},
				{
					onChange: async (enabledIds) => {
						await updateSessionModels(enabledIds);
					},
					onPersist: (enabledIds) => {
						// Persist to settings
						const newPatterns =
							enabledIds === null || enabledIds.length === allModels.length
								? undefined // All enabled = clear filter
								: enabledIds;
						this.session.settingsManager.setEnabledModels(newPatterns ? [...newPatterns] : undefined);
						this.deps.showStatus("Model selection saved to settings");
					},
					onCancel: () => {
						done();
						this.deps.ui.requestRender();
					},
				},
			);
			return { component: selector, focus: selector };
		});
	}
}
