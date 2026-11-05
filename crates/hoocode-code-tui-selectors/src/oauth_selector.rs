//! The provider pickers of `/login` and `/logout`, hoocode
//! `components/oauth-selector.ts`, and `isApiKeyLoginProvider` from
//! `login-controller.ts`.
//!
//! The picker draws as the Log in / Log out pane: the title names the flow
//! and counts the providers, the filter row filters, and the hint row names
//! the keys. There is no QR facility in the repo and no new deps are
//! allowed, so the provider rows show here and the URL/code in the login
//! dialog; the dialog notes the QR gap.

use hoocode_code_tui_keybindings::{key_hint_text, key_pair_hint};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use hoocode_code_auth::provider_display_names::built_in_provider_display_name;
use hoocode_code_auth::{AuthCredential, AuthSource, AuthStatus, AuthStorage};
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{hints, query_row, Console};
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_fuzzy::fuzzy_filter;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

/// Whether a provider signs in with an API key rather than a subscription
/// (`isApiKeyLoginProvider`). `built_in_provider_ids` defaults to every
/// catalog provider.
pub fn is_api_key_login_provider(
    provider_id: &str,
    oauth_provider_ids: &HashSet<String>,
    built_in_provider_ids: Option<&HashSet<String>>,
) -> bool {
    if built_in_provider_display_name(provider_id).is_some() {
        return true;
    }
    let is_built_in = match built_in_provider_ids {
        Some(ids) => ids.contains(provider_id),
        None => hoocode_ai_models::get_providers().contains(&provider_id),
    };
    if is_built_in {
        return false;
    }
    !oauth_provider_ids.contains(provider_id)
}

/// How a provider signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    OAuth,
    ApiKey,
}

/// `AuthSelectorProvider`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSelectorProvider {
    pub id: String,
    pub name: String,
    pub auth_type: AuthType,
}

/// Which picker this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMode {
    Login,
    Logout,
}

/// What the picker asks its owner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthSelectorEvent {
    Select(String),
    Cancel,
}

/// A provider's auth status (`getAuthStatus`).
pub type AuthStatusFn = Box<dyn Fn(&str) -> AuthStatus>;

const MAX_VISIBLE: usize = 8;

struct State {
    all: Vec<AuthSelectorProvider>,
    filtered: Vec<AuthSelectorProvider>,
    cursor: PickerCursor,
    mode: LoginMode,
    auth: Arc<AuthStorage>,
    status: AuthStatusFn,
    /// Whether the picker has focus: the query caret draws while set.
    focused: bool,
}

impl State {
    /// The auth state after the provider name, as styled spans
    /// (`formatStatusIndicator`): plain words, `configured` in green.
    fn status_spans(&self, provider: &AuthSelectorProvider) -> Vec<Span<'static>> {
        let pal = Palette::current();
        let stored = self.auth.get(&provider.id).map(|c| match c {
            AuthCredential::OAuth(_) => AuthType::OAuth,
            AuthCredential::ApiKey { .. } => AuthType::ApiKey,
        });
        if stored == Some(provider.auth_type) {
            return vec![Span::styled("configured", pal.ok())];
        }
        if let Some(stored) = stored {
            let label = match stored {
                AuthType::OAuth => "subscription configured",
                AuthType::ApiKey => "API key configured",
            };
            return vec![Span::styled(label, pal.warn())];
        }
        let not_configured = || vec![Span::styled("not configured", pal.dim())];
        if provider.auth_type != AuthType::ApiKey {
            return not_configured();
        }
        let status = (self.status)(&provider.id);
        let source = match status.source {
            Some(AuthSource::Environment) => {
                format!("env {}", status.label.as_deref().unwrap_or("API key"))
            }
            Some(AuthSource::Runtime) => "runtime API key".to_string(),
            Some(AuthSource::Fallback) => "custom API key".to_string(),
            Some(AuthSource::ModelsJsonKey) => "key in models.json".to_string(),
            Some(AuthSource::ModelsJsonCommand) => "command in models.json".to_string(),
            _ => return not_configured(),
        };
        vec![
            Span::styled("configured", pal.ok()),
            Span::styled(format!("  {source}"), pal.dim()),
        ]
    }

    fn filter(&mut self, query: &str) {
        self.filtered = if query.is_empty() {
            self.all.clone()
        } else {
            fuzzy_filter(&self.all, query, |p| {
                let kind = match p.auth_type {
                    AuthType::OAuth => "oauth",
                    AuthType::ApiKey => "api_key",
                };
                format!("{} {} {kind}", p.name, p.id)
            })
        };
        self.cursor.set_query(self.filtered.len());
    }
}

/// One provider row (content only; the pane adds the ` › ` marker or
/// gutter and the selection band): the name padded to `name_width`, then
/// the auth state.
fn provider_row(
    provider: &AuthSelectorProvider,
    name_width: usize,
    status: Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = vec![
        Span::raw(format!("{:<name_width$}", provider.name)),
        Span::raw("  ".to_string()),
    ];
    spans.extend(status);
    Line::from(spans)
}

/// `1 provider`, `N providers`.
fn providers(count: usize) -> String {
    if count == 1 {
        "1 provider".to_string()
    } else {
        format!("{count} providers")
    }
}

/// `OAuthSelectorComponent`: a searchable provider list with each
/// provider's auth state, drawn as the Log in / Log out pane.
pub struct OAuthSelectorComponent {
    search: Rc<RefCell<Input>>,
    state: Rc<RefCell<State>>,
    events: Vec<OAuthSelectorEvent>,
    submitted: Rc<RefCell<bool>>,
}

impl OAuthSelectorComponent {
    /// `get_auth_status` defaults to the storage's own status.
    pub fn new(
        mode: LoginMode,
        auth: Arc<AuthStorage>,
        providers: Vec<AuthSelectorProvider>,
        get_auth_status: Option<AuthStatusFn>,
    ) -> Self {
        let status = get_auth_status.unwrap_or_else(|| {
            let auth = auth.clone();
            Box::new(move |id: &str| auth.get_auth_status(id))
        });
        let state = Rc::new(RefCell::new(State {
            filtered: providers.clone(),
            cursor: PickerCursor::new(providers.len(), MAX_VISIBLE),
            all: providers,
            mode,
            auth,
            status,
            focused: false,
        }));
        let mut input = Input::new();
        style_input(&mut input);
        let submitted: Rc<RefCell<bool>> = Rc::default();
        let flag = submitted.clone();
        input.on_submit = Some(Box::new(move |_: &str| *flag.borrow_mut() = true));
        let search = Rc::new(RefCell::new(input));
        Self {
            search,
            state,
            events: Vec::new(),
            submitted,
        }
    }

    fn select_current(&mut self) {
        let state = self.state.borrow();
        if let Some(provider) = state.filtered.get(state.cursor.selected()) {
            self.events
                .push(OAuthSelectorEvent::Select(provider.id.clone()));
        }
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<OAuthSelectorEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Component for OAuthSelectorComponent {
    /// The Log in / Log out pane: the providers with ` › ` on the selection
    /// band, the scroll position and empty-list rows, the filter row and the
    /// hint row. Keys and filtering are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let state = self.state.borrow();
        let search = self.search.borrow();
        let query = search.get_value().to_string();
        let total = state.all.len();
        let count = if query.is_empty() {
            providers(total)
        } else {
            format!("{} of {}", providers(state.filtered.len()), total)
        };
        let (name, action) = match state.mode {
            LoginMode::Login => ("Log in · /login", "log in"),
            LoginMode::Logout => ("Log out · /logout", "log out"),
        };
        let mut console = Console::new(name, count);
        console.query = Some(query_row(&query, state.focused));
        let selected = state.cursor.selected();
        let name_width = state
            .filtered
            .iter()
            .map(|p| visible_width(&p.name))
            .max()
            .unwrap_or(0);
        console.rows = state
            .filtered
            .iter()
            .map(|provider| provider_row(provider, name_width, state.status_spans(provider)))
            .collect();
        console.selected = (!state.filtered.is_empty()).then_some(selected);
        console.max_visible = Some(state.cursor.max_visible());
        if let Some(footer) = state.cursor.footer() {
            console.bottom.push(Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.dim(),
            )]));
        }
        if state.filtered.is_empty() {
            let message = if !state.all.is_empty() {
                "No matching providers"
            } else if state.mode == LoginMode::Login {
                "No providers available"
            } else {
                "No providers logged in. Use /login first."
            };
            console.bottom.push(Line::from(vec![Span::styled(
                format!("  {message}"),
                pal.dim(),
            )]));
        }
        console.hint = hints(&[
            (
                key_pair_hint("tui.select.up", "tui.select.down").as_str(),
                "move",
            ),
            (key_hint_text("tui.select.confirm").as_str(), action),
            (key_hint_text("tui.select.cancel").as_str(), "cancel"),
        ]);
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        let navigated = self.state.borrow_mut().cursor.handle_query_key(data, &kb);
        if navigated {
            return;
        }
        if kb.matches(data, "tui.select.confirm") {
            self.select_current();
        } else if kb.matches(data, "tui.select.cancel") {
            self.events.push(OAuthSelectorEvent::Cancel);
        } else {
            self.search.borrow_mut().handle_input_with(data, &kb);
            if std::mem::take(&mut *self.submitted.borrow_mut()) {
                self.select_current();
            }
            let query = self.search.borrow().get_value().to_string();
            self.state.borrow_mut().filter(&query);
        }
    }

    fn invalidate(&mut self) {
        self.search.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.state.borrow_mut().focused = focused;
        self.search.borrow_mut().set_focused(focused);
    }
}
