//! `/login` and `/logout`: the provider and auth-type selectors, the login dialogs and their polls.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use hoocode_code_auth::provider_display_names::provider_auth_status;
use hoocode_code_auth::AuthCredential;
use hoocode_code_tui_selectors::login_dialog::{LoginDialogComponent, LoginDialogEvent};
use hoocode_code_tui_selectors::oauth_selector::{
    AuthSelectorProvider, AuthType, LoginMode, OAuthSelectorComponent, OAuthSelectorEvent,
};
use hoocode_tui_render::Component;

use crate::extension_selector::{ExtensionSelectorComponent, SelectorOutcome};
use crate::login_controller::{
    action_label, logged_out_message, login_provider_options, logout_provider_options,
    no_providers_message, open_url, post_login_model, run_oauth_login, LoginUpdate, OAuthBridge,
    PostLoginModel, API_KEY_LABEL, LOGIN_CANCELLED, NOTHING_TO_LOG_OUT, SUBSCRIPTION_LABEL,
};

use super::*;

/// Where a `/login` or `/logout` is (`LoginController`).
pub(super) enum LoginStep {
    /// "authentication method".
    AuthType(Rc<RefCell<ExtensionSelectorComponent>>, Outcomes),
    /// A provider pane.
    Provider {
        selector: Rc<RefCell<OAuthSelectorComponent>>,
        mode: LoginMode,
        options: Vec<AuthSelectorProvider>,
    },
    /// The API-key dialog.
    ApiKey {
        dialog: Rc<RefCell<LoginDialogComponent>>,
        provider: AuthSelectorProvider,
        had_model: bool,
    },
    /// An OAuth login running on the async runtime.
    OAuth(Box<OAuthLogin>),
}

/// The dialog of a running OAuth login and the answers it owes.
pub(super) struct OAuthLogin {
    dialog: Rc<RefCell<LoginDialogComponent>>,
    pub(super) provider: AuthSelectorProvider,
    had_model: bool,
    /// `onPrompt`'s answer.
    pub(super) prompt: Option<tokio::sync::oneshot::Sender<String>>,
    /// A pasted redirect URL (`onManualCodeInput`).
    manual: Option<tokio::sync::oneshot::Sender<String>>,
    /// An `onSelect` pane over the dialog.
    pub(super) select: Option<OAuthSelect>,
}

pub(super) struct OAuthSelect {
    outcomes: Outcomes,
    options: Vec<hoocode_ai_oauth::OAuthSelectOption>,
    reply: tokio::sync::oneshot::Sender<Option<String>>,
}

impl Mode {
    /// `showOAuthSelector`: `/login` asks how to sign in; `/logout` lists
    /// the stored credentials.
    pub(super) fn show_oauth_selector(&mut self, mode: LoginMode) {
        if mode == LoginMode::Login {
            self.show_login_auth_type_selector();
            return;
        }
        let options = logout_provider_options(&self.auth_storage);
        if options.is_empty() {
            self.show_status(NOTHING_TO_LOG_OUT);
            return;
        }
        let selector = handle(OAuthSelectorComponent::new(
            mode,
            self.auth_storage.clone(),
            options.clone(),
            None,
        ));
        self.show_in_editor_slot(as_component(&selector));
        self.login = Some(LoginStep::Provider {
            selector,
            mode,
            options,
        });
    }

    /// `showLoginAuthTypeSelector`.
    fn show_login_auth_type_selector(&mut self) {
        let outcomes: Outcomes = Rc::default();
        let sink = outcomes.clone();
        let selector = handle(ExtensionSelectorComponent::new(
            // Names the pane in its top border.
            "authentication method",
            vec![SUBSCRIPTION_LABEL.to_string(), API_KEY_LABEL.to_string()],
            None,
            Box::new(move |outcome| sink.borrow_mut().push(outcome)),
        ));
        self.show_in_editor_slot(as_component(&selector));
        self.login = Some(LoginStep::AuthType(selector, outcomes));
    }

    /// `showLoginProviderSelector`.
    fn show_login_provider_selector(&mut self, auth_type: AuthType) {
        let registry = self.session.model_registry().clone();
        let options = login_provider_options(&self.auth_storage, &registry, Some(auth_type));
        if options.is_empty() {
            self.show_status(no_providers_message(auth_type));
            return;
        }
        let auth = self.auth_storage.clone();
        let selector = handle(OAuthSelectorComponent::new(
            LoginMode::Login,
            self.auth_storage.clone(),
            options.clone(),
            Some(Box::new(move |id: &str| {
                provider_auth_status(&auth, &registry, id)
            })),
        ));
        self.show_in_editor_slot(as_component(&selector));
        self.login = Some(LoginStep::Provider {
            selector,
            mode: LoginMode::Login,
            options,
        });
    }

    /// The login panes' answers.
    pub(super) fn poll_login(&mut self) {
        let Some(step) = self.login.take() else {
            return;
        };
        match step {
            LoginStep::AuthType(selector, outcomes) => {
                let outcome = outcomes.borrow_mut().drain(..).next();
                match outcome {
                    None => self.login = Some(LoginStep::AuthType(selector, outcomes)),
                    Some(outcome) => {
                        self.restore_editor();
                        if let SelectorOutcome::Selected(option) = outcome {
                            self.show_login_provider_selector(if option == SUBSCRIPTION_LABEL {
                                AuthType::OAuth
                            } else {
                                AuthType::ApiKey
                            });
                        }
                    }
                }
            }
            LoginStep::Provider {
                selector,
                mode,
                options,
            } => {
                let event = selector.borrow_mut().take_events().into_iter().next();
                match event {
                    None => {
                        self.login = Some(LoginStep::Provider {
                            selector,
                            mode,
                            options,
                        })
                    }
                    Some(OAuthSelectorEvent::Cancel) => {
                        self.restore_editor();
                        if mode == LoginMode::Login {
                            self.show_login_auth_type_selector();
                        }
                    }
                    Some(OAuthSelectorEvent::Select(id)) => {
                        self.restore_editor();
                        let Some(provider) = options.into_iter().find(|p| p.id == id) else {
                            return;
                        };
                        match (mode, provider.auth_type) {
                            (LoginMode::Logout, _) => self.logout(&provider),
                            (LoginMode::Login, AuthType::OAuth) => self.show_login_dialog(provider),
                            (LoginMode::Login, AuthType::ApiKey) => {
                                self.show_api_key_login_dialog(provider)
                            }
                        }
                    }
                }
            }
            LoginStep::ApiKey {
                dialog,
                provider,
                had_model,
            } => {
                let event = dialog.borrow_mut().take_events().into_iter().next();
                match event {
                    None => {
                        self.login = Some(LoginStep::ApiKey {
                            dialog,
                            provider,
                            had_model,
                        })
                    }
                    Some(LoginDialogEvent::Cancelled) => self.restore_editor(),
                    Some(LoginDialogEvent::Submitted(value)) => {
                        self.restore_editor();
                        let key = value.trim();
                        if key.is_empty() {
                            self.show_error(&format!(
                                "Failed to save API key for {}: API key cannot be empty.",
                                provider.name
                            ));
                            return;
                        }
                        self.auth_storage.set(
                            &provider.id,
                            AuthCredential::ApiKey {
                                key: key.to_string(),
                            },
                        );
                        self.complete_provider_authentication(&provider, had_model);
                    }
                }
            }
            LoginStep::OAuth(mut login) => {
                if let Some(select) = &login.select {
                    let outcome = select.outcomes.borrow_mut().drain(..).next();
                    if let Some(outcome) = outcome {
                        let select = login.select.take().expect("checked above");
                        let id = match outcome {
                            SelectorOutcome::Selected(label) => select
                                .options
                                .iter()
                                .find(|o| o.label == label)
                                .map(|o| o.id.clone()),
                            SelectorOutcome::Cancelled => None,
                        };
                        let _ = select.reply.send(id);
                        self.show_in_editor_slot(as_component(&login.dialog));
                    }
                    self.login = Some(LoginStep::OAuth(login));
                    return;
                }
                let events = login.dialog.borrow_mut().take_events();
                for event in events {
                    match event {
                        LoginDialogEvent::Cancelled => {
                            // Dropping the owed answers fails the flow as
                            // cancelled; its end is not waited for.
                            self.restore_editor();
                            return;
                        }
                        LoginDialogEvent::Submitted(value) => {
                            if let Some(reply) = login.prompt.take() {
                                let _ = reply.send(value);
                            } else if !value.is_empty() {
                                if let Some(reply) = login.manual.take() {
                                    let _ = reply.send(value);
                                }
                            }
                        }
                    }
                }
                self.login = Some(LoginStep::OAuth(login));
            }
        }
        self.dirty.set(true);
    }

    /// `/logout`'s pick: forget the credential.
    pub(super) fn logout(&mut self, provider: &AuthSelectorProvider) {
        self.auth_storage.logout(&provider.id);
        if let Some(error) = self.auth_storage.drain_errors().into_iter().next() {
            self.show_error(&format!("Logout failed: {error}"));
            return;
        }
        self.update_available_provider_count();
        self.show_status(&logged_out_message(provider));
    }

    /// `showApiKeyLoginDialog`.
    fn show_api_key_login_dialog(&mut self, provider: AuthSelectorProvider) {
        let mut dialog = LoginDialogComponent::new(&provider.name, None);
        dialog.show_prompt("Enter API key:", None);
        let dialog = handle(dialog);
        self.show_in_editor_slot(as_component(&dialog));
        self.login = Some(LoginStep::ApiKey {
            dialog,
            provider,
            had_model: self.session.model().is_some(),
        });
    }

    /// `showLoginDialog`: the provider's OAuth flow, on the runtime.
    fn show_login_dialog(&mut self, provider: AuthSelectorProvider) {
        let uses_callback_server = self
            .auth_storage
            .get_oauth_providers()
            .iter()
            .find(|p| p.id() == provider.id)
            .is_some_and(|p| p.uses_callback_server());
        let dialog = handle(LoginDialogComponent::new(&provider.name, None));
        self.show_in_editor_slot(as_component(&dialog));
        let tx = self.tx.clone();
        let send = Mutex::new(tx.clone());
        let bridge = OAuthBridge::new(
            Box::new(move |update| {
                let _ = send
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(AppEvent::Login(update));
            }),
            dialog.borrow().signal(),
            uses_callback_server,
        );
        let auth = self.auth_storage.clone();
        let id = provider.id.clone();
        self.runtime.spawn(async move {
            let result = run_oauth_login(auth, id, bridge).await;
            let _ = tx.send(AppEvent::Login(LoginUpdate::Done(result)));
        });
        self.login = Some(LoginStep::OAuth(Box::new(OAuthLogin {
            dialog,
            provider,
            had_model: self.session.model().is_some(),
            prompt: None,
            manual: None,
            select: None,
        })));
    }

    /// A step of the running OAuth login. Steps of a login the user
    /// cancelled are dropped, which fails what they wait on.
    pub(super) fn handle_login_update(&mut self, update: LoginUpdate) {
        let Some(LoginStep::OAuth(login)) = &mut self.login else {
            return;
        };
        self.dirty.set(true);
        match update {
            LoginUpdate::Auth { info, manual } => {
                let mut dialog = login.dialog.borrow_mut();
                dialog.show_auth(&info.url, info.instructions.as_deref());
                open_url(&info.url);
                if let Some(manual) = manual {
                    dialog.show_manual_input(
                        "Paste redirect URL below, or complete login in browser:",
                    );
                    login.manual = Some(manual);
                } else if login.provider.id == "github-copilot" {
                    // Copilot polls after onAuth.
                    dialog.show_waiting("Waiting for browser authentication...");
                }
            }
            LoginUpdate::Prompt { prompt, reply } => {
                login
                    .dialog
                    .borrow_mut()
                    .show_prompt(&prompt.message, prompt.placeholder.as_deref());
                login.prompt = Some(reply);
            }
            LoginUpdate::Progress(message) => login.dialog.borrow_mut().show_progress(&message),
            LoginUpdate::Select { prompt, reply } => {
                let outcomes: Outcomes = Rc::default();
                let sink = outcomes.clone();
                let selector = handle(ExtensionSelectorComponent::new(
                    &prompt.message,
                    prompt.options.iter().map(|o| o.label.clone()).collect(),
                    None,
                    Box::new(move |outcome| sink.borrow_mut().push(outcome)),
                ));
                login.select = Some(OAuthSelect {
                    outcomes,
                    options: prompt.options,
                    reply,
                });
                self.show_in_editor_slot(as_component(&selector));
            }
            LoginUpdate::Done(result) => {
                let Some(LoginStep::OAuth(login)) = self.login.take() else {
                    return;
                };
                self.restore_editor();
                match result {
                    Ok(()) => {
                        self.complete_provider_authentication(&login.provider, login.had_model)
                    }
                    Err(error) if error == LOGIN_CANCELLED => {}
                    Err(error) => self.show_error(&format!(
                        "Failed to login to {}: {error}",
                        login.provider.name
                    )),
                }
            }
        }
    }

    /// `completeProviderAuthentication`: with no model yet, select the
    /// provider's default; record where the credential went.
    fn complete_provider_authentication(
        &mut self,
        provider: &AuthSelectorProvider,
        had_model: bool,
    ) {
        let label = action_label(&provider.name, provider.auth_type);
        let available = self.session.get_available_models();
        let (selected, error) = match post_login_model(had_model, &provider.id, &label, &available)
        {
            PostLoginModel::Keep => (None, None),
            PostLoginModel::Error(error) => (None, Some(error)),
            PostLoginModel::Select(model) => match self.session.set_model((*model).clone()) {
                Ok(()) => (Some(*model), None),
                Err(error) => (
                    None,
                    Some(format!(
                        "{label}, but selecting its default model failed: {error}. Use /model to select a model."
                    )),
                ),
            },
        };
        self.update_available_provider_count();
        self.footer.borrow_mut().invalidate();
        self.update_editor_border_color();
        let path = hoocode_code_paths::auth_path();
        match selected {
            Some(model) => {
                self.show_record(&format!(
                    "{label}. Selected {}. Credentials saved to {}",
                    model.id,
                    path.display()
                ));
                self.maybe_warn_about_anthropic_subscription_auth(Some(model));
            }
            None => {
                self.show_record(&format!("{label}. Credentials saved to {}", path.display()));
                match error {
                    Some(error) => self.show_error(&error),
                    None => self.maybe_warn_about_anthropic_subscription_auth(None),
                }
            }
        }
    }
}
