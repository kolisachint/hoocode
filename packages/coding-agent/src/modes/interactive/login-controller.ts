/**
 * Auth / login flows for the interactive mode (/login, /logout).
 *
 * Owns the provider auth-type selector, the OAuth and API-key login dialogs,
 * the Bedrock setup notice, and post-login model selection. Extracted from
 * interactive-mode.ts behind a narrow LoginControllerDeps interface; the
 * `/login` and `/logout` commands call `showOAuthSelector`.
 */

import * as path from "node:path";
import { getProviders, type Model, type OAuthProviderId, type OAuthSelectPrompt } from "@kolisachint/hoocode-ai";
import type { Component, Container, TUI } from "@kolisachint/hoocode-tui";
import { getAuthPath, getDocsPath } from "../../config.js";
import type { AgentSession } from "../../core/agent-session.js";
import { defaultModelPerProvider } from "../../core/model-resolver.js";
import { BUILT_IN_PROVIDER_DISPLAY_NAMES } from "../../core/provider-display-names.js";
import { ExtensionSelectorComponent } from "./components/extension-selector.js";
import { LoginDialogComponent } from "./components/login-dialog.js";
import { type AuthSelectorProvider, OAuthSelectorComponent } from "./components/oauth-selector.js";
import { theme } from "./theme/theme.js";

const BEDROCK_PROVIDER_ID = "amazon-bedrock";

const BUILT_IN_MODEL_PROVIDERS = new Set<string>(getProviders());

export function isApiKeyLoginProvider(
	providerId: string,
	oauthProviderIds: ReadonlySet<string>,
	builtInProviderIds: ReadonlySet<string> = BUILT_IN_MODEL_PROVIDERS,
): boolean {
	if (BUILT_IN_PROVIDER_DISPLAY_NAMES[providerId]) {
		return true;
	}
	if (builtInProviderIds.has(providerId)) {
		return false;
	}
	return !oauthProviderIds.has(providerId);
}

function isUnknownModel(model: Model<any> | undefined): boolean {
	return !!model && model.provider === "unknown" && model.id === "unknown" && model.api === "unknown";
}

function hasDefaultModelProvider(providerId: string): providerId is keyof typeof defaultModelPerProvider {
	return providerId in defaultModelPerProvider;
}

/** The slice of the interactive mode the login flows need. */
export interface LoginControllerDeps {
	ui: TUI;
	/** The active session (read at call time; the session can be swapped). */
	get session(): AgentSession;
	/** The prompt editor (read at call time; the editor can be swapped). */
	getEditor(): Component;
	/** Container that hosts the prompt editor and login dialogs. */
	editorContainer: Container;
	showSelector(create: (done: () => void) => { component: Component; focus: Component }): void;
	showStatus(message: string): void;
	showError(errorMessage: string): void;
	/** Recompute the footer's available provider count. */
	updateAvailableProviderCount(): Promise<void>;
	updateEditorBorderColor(): void;
	invalidateFooter(): void;
	maybeWarnAboutAnthropicSubscriptionAuth(model?: Model<any>): Promise<void>;
}

export class LoginController {
	constructor(private readonly deps: LoginControllerDeps) {}

	private get session(): AgentSession {
		return this.deps.session;
	}

	private restoreEditor(): void {
		const { deps } = this;
		deps.editorContainer.clear();
		deps.editorContainer.addChild(deps.getEditor());
		deps.ui.setFocus(deps.getEditor());
		deps.ui.requestRender();
	}

	private getLoginProviderOptions(authType?: "oauth" | "api_key"): AuthSelectorProvider[] {
		const authStorage = this.session.modelRegistry.authStorage;
		const oauthProviders = authStorage.getOAuthProviders();
		const oauthProviderIds = new Set(oauthProviders.map((provider) => provider.id));
		const options: AuthSelectorProvider[] = oauthProviders.map((provider) => ({
			id: provider.id,
			name: provider.name,
			authType: "oauth",
		}));

		const modelProviders = new Set(this.session.modelRegistry.getAll().map((model) => model.provider));
		for (const providerId of modelProviders) {
			if (!isApiKeyLoginProvider(providerId, oauthProviderIds)) {
				continue;
			}
			options.push({
				id: providerId,
				name: this.session.modelRegistry.getProviderDisplayName(providerId),
				authType: "api_key",
			});
		}

		const filteredOptions = authType ? options.filter((option) => option.authType === authType) : options;
		return filteredOptions.sort((a, b) => a.name.localeCompare(b.name));
	}

	private getLogoutProviderOptions(): AuthSelectorProvider[] {
		const authStorage = this.session.modelRegistry.authStorage;
		const options: AuthSelectorProvider[] = [];

		for (const providerId of authStorage.list()) {
			const credential = authStorage.get(providerId);
			if (!credential) {
				continue;
			}
			options.push({
				id: providerId,
				name: this.session.modelRegistry.getProviderDisplayName(providerId),
				authType: credential.type,
			});
		}

		return options.sort((a, b) => a.name.localeCompare(b.name));
	}

	private showLoginAuthTypeSelector(): void {
		const subscriptionLabel = "Use a subscription";
		const apiKeyLabel = "Use an API key";
		this.deps.showSelector((done) => {
			const selector = new ExtensionSelectorComponent(
				"Select authentication method:",
				[subscriptionLabel, apiKeyLabel],
				(option) => {
					done();
					const authType = option === subscriptionLabel ? "oauth" : "api_key";
					this.showLoginProviderSelector(authType);
				},
				() => {
					done();
					this.deps.ui.requestRender();
				},
			);
			return { component: selector, focus: selector };
		});
	}

	private showLoginProviderSelector(authType: "oauth" | "api_key"): void {
		const providerOptions = this.getLoginProviderOptions(authType);
		if (providerOptions.length === 0) {
			this.deps.showStatus(
				authType === "oauth" ? "No subscription providers available." : "No API key providers available.",
			);
			return;
		}

		this.deps.showSelector((done) => {
			const selector = new OAuthSelectorComponent(
				"login",
				this.session.modelRegistry.authStorage,
				providerOptions,
				async (providerId: string) => {
					done();

					const providerOption = providerOptions.find((provider) => provider.id === providerId);
					if (!providerOption) {
						return;
					}

					if (providerOption.authType === "oauth") {
						await this.showLoginDialog(providerOption.id, providerOption.name);
					} else if (providerOption.id === BEDROCK_PROVIDER_ID) {
						this.showBedrockSetupDialog(providerOption.id, providerOption.name);
					} else {
						await this.showApiKeyLoginDialog(providerOption.id, providerOption.name);
					}
				},
				() => {
					done();
					this.showLoginAuthTypeSelector();
				},
				(providerId) => this.session.modelRegistry.getProviderAuthStatus(providerId),
			);
			return { component: selector, focus: selector };
		});
	}

	async showOAuthSelector(mode: "login" | "logout"): Promise<void> {
		if (mode === "login") {
			this.showLoginAuthTypeSelector();
			return;
		}

		const providerOptions = this.getLogoutProviderOptions();
		if (providerOptions.length === 0) {
			this.deps.showStatus(
				"No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.",
			);
			return;
		}

		this.deps.showSelector((done) => {
			const selector = new OAuthSelectorComponent(
				mode,
				this.session.modelRegistry.authStorage,
				providerOptions,
				async (providerId: string) => {
					done();

					const providerOption = providerOptions.find((provider) => provider.id === providerId);
					if (!providerOption) {
						return;
					}

					try {
						this.session.modelRegistry.authStorage.logout(providerOption.id);
						this.session.modelRegistry.refresh();
						await this.deps.updateAvailableProviderCount();
						const message =
							providerOption.authType === "oauth"
								? `Logged out of ${providerOption.name}`
								: `Removed stored API key for ${providerOption.name}. Environment variables and models.json config are unchanged.`;
						this.deps.showStatus(message);
					} catch (error: unknown) {
						this.deps.showError(`Logout failed: ${error instanceof Error ? error.message : String(error)}`);
					}
				},
				() => {
					done();
					this.deps.ui.requestRender();
				},
			);
			return { component: selector, focus: selector };
		});
	}

	private async completeProviderAuthentication(
		providerId: string,
		providerName: string,
		authType: "oauth" | "api_key",
		previousModel: Model<any> | undefined,
	): Promise<void> {
		this.session.modelRegistry.refresh();

		const actionLabel = authType === "oauth" ? `Logged in to ${providerName}` : `Saved API key for ${providerName}`;

		let selectedModel: Model<any> | undefined;
		let selectionError: string | undefined;
		if (isUnknownModel(previousModel)) {
			const availableModels = this.session.modelRegistry.getAvailable();
			const providerModels = availableModels.filter((model) => model.provider === providerId);
			if (!hasDefaultModelProvider(providerId)) {
				selectionError = `${actionLabel}, but no default model is configured for provider "${providerId}". Use /model to select a model.`;
			} else if (providerModels.length === 0) {
				selectionError = `${actionLabel}, but no models are available for that provider. Use /model to select a model.`;
			} else {
				const defaultModelId = defaultModelPerProvider[providerId];
				selectedModel = providerModels.find((model) => model.id === defaultModelId);
				if (!selectedModel) {
					selectionError = `${actionLabel}, but its default model "${defaultModelId}" is not available. Use /model to select a model.`;
				} else {
					try {
						await this.session.setModel(selectedModel);
					} catch (error: unknown) {
						selectedModel = undefined;
						const errorMessage = error instanceof Error ? error.message : String(error);
						selectionError = `${actionLabel}, but selecting its default model failed: ${errorMessage}. Use /model to select a model.`;
					}
				}
			}
		}

		await this.deps.updateAvailableProviderCount();
		this.deps.invalidateFooter();
		this.deps.updateEditorBorderColor();
		if (selectedModel) {
			this.deps.showStatus(`${actionLabel}. Selected ${selectedModel.id}. Credentials saved to ${getAuthPath()}`);
			void this.deps.maybeWarnAboutAnthropicSubscriptionAuth(selectedModel);
		} else {
			this.deps.showStatus(`${actionLabel}. Credentials saved to ${getAuthPath()}`);
			if (selectionError) {
				this.deps.showError(selectionError);
			} else {
				void this.deps.maybeWarnAboutAnthropicSubscriptionAuth();
			}
		}
	}

	private showBedrockSetupDialog(providerId: string, providerName: string): void {
		const dialog = new LoginDialogComponent(
			this.deps.ui,
			providerId,
			() => this.restoreEditor(),
			providerName,
			"Amazon Bedrock setup",
		);
		dialog.showInfo([
			theme.fg("text", "Amazon Bedrock uses AWS credentials instead of a single API key."),
			theme.fg("text", "Configure an AWS profile, IAM keys, bearer token, or role-based credentials."),
			theme.fg("muted", "See:"),
			theme.fg("accent", `  ${path.join(getDocsPath(), "providers.md")}`),
		]);

		this.deps.editorContainer.clear();
		this.deps.editorContainer.addChild(dialog);
		this.deps.ui.setFocus(dialog);
		this.deps.ui.requestRender();
	}

	private async showApiKeyLoginDialog(providerId: string, providerName: string): Promise<void> {
		const previousModel = this.session.model;

		const dialog = new LoginDialogComponent(
			this.deps.ui,
			providerId,
			(_success, _message) => {
				// Completion handled below
			},
			providerName,
		);

		this.deps.editorContainer.clear();
		this.deps.editorContainer.addChild(dialog);
		this.deps.ui.setFocus(dialog);
		this.deps.ui.requestRender();

		try {
			const apiKey = (await dialog.showPrompt("Enter API key:")).trim();
			if (!apiKey) {
				throw new Error("API key cannot be empty.");
			}

			this.session.modelRegistry.authStorage.set(providerId, { type: "api_key", key: apiKey });

			this.restoreEditor();
			await this.completeProviderAuthentication(providerId, providerName, "api_key", previousModel);
		} catch (error: unknown) {
			this.restoreEditor();
			const errorMsg = error instanceof Error ? error.message : String(error);
			if (errorMsg !== "Login cancelled") {
				this.deps.showError(`Failed to save API key for ${providerName}: ${errorMsg}`);
			}
		}
	}

	private showOAuthLoginSelect(dialog: LoginDialogComponent, prompt: OAuthSelectPrompt): Promise<string | undefined> {
		return new Promise((resolve) => {
			const restoreDialog = () => {
				this.deps.editorContainer.clear();
				this.deps.editorContainer.addChild(dialog);
				this.deps.ui.setFocus(dialog);
				this.deps.ui.requestRender();
			};
			const labels = prompt.options.map((option) => option.label);
			const selector = new ExtensionSelectorComponent(
				prompt.message,
				labels,
				(optionLabel) => {
					restoreDialog();
					resolve(prompt.options.find((option) => option.label === optionLabel)?.id);
				},
				() => {
					restoreDialog();
					resolve(undefined);
				},
			);
			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(selector);
			this.deps.ui.setFocus(selector);
			this.deps.ui.requestRender();
		});
	}

	private async showLoginDialog(providerId: string, providerName: string): Promise<void> {
		const providerInfo = this.session.modelRegistry.authStorage
			.getOAuthProviders()
			.find((provider) => provider.id === providerId);
		const previousModel = this.session.model;

		// Providers that use callback servers (can paste redirect URL)
		const usesCallbackServer = providerInfo?.usesCallbackServer ?? false;

		// Create login dialog component
		const dialog = new LoginDialogComponent(
			this.deps.ui,
			providerId,
			(_success, _message) => {
				// Completion handled below
			},
			providerName,
		);

		// Show dialog in editor container
		this.deps.editorContainer.clear();
		this.deps.editorContainer.addChild(dialog);
		this.deps.ui.setFocus(dialog);
		this.deps.ui.requestRender();

		// Promise for manual code input (racing with callback server)
		let manualCodeResolve: ((code: string) => void) | undefined;
		let manualCodeReject: ((err: Error) => void) | undefined;
		const manualCodePromise = new Promise<string>((resolve, reject) => {
			manualCodeResolve = resolve;
			manualCodeReject = reject;
		});

		try {
			await this.session.modelRegistry.authStorage.login(providerId as OAuthProviderId, {
				onAuth: (info: { url: string; instructions?: string }) => {
					dialog.showAuth(info.url, info.instructions);

					if (usesCallbackServer) {
						// Show input for manual paste, racing with callback
						dialog
							.showManualInput("Paste redirect URL below, or complete login in browser:")
							.then((value) => {
								if (value && manualCodeResolve) {
									manualCodeResolve(value);
									manualCodeResolve = undefined;
								}
							})
							.catch(() => {
								if (manualCodeReject) {
									manualCodeReject(new Error("Login cancelled"));
									manualCodeReject = undefined;
								}
							});
					} else if (providerId === "github-copilot") {
						// GitHub Copilot polls after onAuth
						dialog.showWaiting("Waiting for browser authentication...");
					}
					// For Anthropic: onPrompt is called immediately after
				},

				onPrompt: async (prompt: { message: string; placeholder?: string }) => {
					return dialog.showPrompt(prompt.message, prompt.placeholder);
				},

				onProgress: (message: string) => {
					dialog.showProgress(message);
				},

				onSelect: (prompt: OAuthSelectPrompt) => this.showOAuthLoginSelect(dialog, prompt),

				onManualCodeInput: () => manualCodePromise,

				signal: dialog.signal,
			});

			// Success
			this.restoreEditor();
			await this.completeProviderAuthentication(providerId, providerName, "oauth", previousModel);
		} catch (error: unknown) {
			this.restoreEditor();
			const errorMsg = error instanceof Error ? error.message : String(error);
			if (errorMsg !== "Login cancelled") {
				this.deps.showError(`Failed to login to ${providerName}: ${errorMsg}`);
			}
		}
	}
}
