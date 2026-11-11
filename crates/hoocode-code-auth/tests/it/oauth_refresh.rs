#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Refresh on expiry through `AuthStorage::get_api_key` with the real provider
//! ids (`anthropic`, `openai-codex`, `google-gemini-cli`). The provider is the
//! real one, registered under its real id, with its token request sent through
//! the crate's `Fetch` seam (the stub the oauth crates' own tests use). That
//! seam lets the test check the request against the provider's real
//! `TOKEN_URL` and run without network access.

use base64::Engine;
use hoocode_ai_oauth::{register_oauth_provider, BoxFuture, Fetch, HttpRequest, HttpResponse};
use hoocode_ai_oauth_anthropic::AnthropicOAuthProvider;
use hoocode_ai_oauth_google::GeminiCliOAuthProvider;
use hoocode_ai_oauth_openai_codex::OpenAICodexOAuthProvider;
use hoocode_code_auth::AuthStorage;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Records every token request and answers each with `body`.
#[derive(Default)]
struct TokenEndpoint {
    requests: Mutex<Vec<HttpRequest>>,
    body: String,
}

/// The `Fetch` handed to a provider; shares the recording endpoint.
struct Shared(Arc<TokenEndpoint>);

impl Fetch for Shared {
    fn fetch(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, String>> {
        self.0.requests.lock().unwrap().push(request);
        let response = HttpResponse {
            status: 200,
            status_text: "OK".into(),
            body: self.0.body.clone(),
        };
        Box::pin(async move { Ok(response) })
    }
}

fn token_endpoint(body: Value) -> Arc<TokenEndpoint> {
    Arc::new(TokenEndpoint {
        requests: Mutex::default(),
        body: body.to_string(),
    })
}

/// An `auth.json` in a temp dir holding `entries`.
fn auth_file(dir: &Path, entries: Value) -> PathBuf {
    let path = dir.join("auth.json");
    std::fs::write(&path, serde_json::to_string_pretty(&entries).unwrap()).unwrap();
    path
}

fn read_entry(path: &Path, provider: &str) -> Value {
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    data[provider].clone()
}

/// An unsigned JWT whose payload carries the ChatGPT account claim.
fn jwt(account_id: &str) -> String {
    let payload = base64::engine::general_purpose::STANDARD.encode(
        json!({"https://api.openai.com/auth": {"chatgpt_account_id": account_id}}).to_string(),
    );
    format!("aaa.{payload}.bbb")
}

fn sole_request(endpoint: &TokenEndpoint, token_url: &str) -> HttpRequest {
    let requests = endpoint.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "one refresh request");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].url, token_url,
        "the provider's real token endpoint"
    );
    requests[0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn anthropic_expired_token_refreshes_through_get_api_key() {
    let endpoint = token_endpoint(json!({
        "access_token": "sk-ant-oat01-new",
        "refresh_token": "ref-new",
        "expires_in": 3600,
    }));
    register_oauth_provider(Arc::new(AnthropicOAuthProvider::with_fetch(Box::new(
        Shared(endpoint.clone()),
    ))));
    let dir = tempfile::tempdir().unwrap();
    let path = auth_file(
        dir.path(),
        json!({"anthropic": {"type": "oauth", "refresh": "ref-old", "access": "sk-ant-oat01-old", "expires": 0}}),
    );

    let storage = AuthStorage::create(Some(path.clone()));
    let key = storage.get_api_key("anthropic", false).await;

    assert_eq!(key.as_deref(), Some("sk-ant-oat01-new"));
    let request = sole_request(&endpoint, hoocode_ai_oauth_anthropic::TOKEN_URL);
    let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
    assert_eq!(body["grant_type"], "refresh_token");
    assert_eq!(body["refresh_token"], "ref-old");
    let stored = read_entry(&path, "anthropic");
    assert_eq!(stored["type"], "oauth");
    assert_eq!(stored["access"], "sk-ant-oat01-new");
    assert_eq!(stored["refresh"], "ref-new");
    assert!(stored["expires"].as_i64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn openai_codex_expired_token_refreshes_through_get_api_key() {
    let fresh = jwt("acc-new");
    let endpoint = token_endpoint(json!({
        "access_token": fresh,
        "refresh_token": "ref-new",
        "expires_in": 3600,
    }));
    register_oauth_provider(Arc::new(OpenAICodexOAuthProvider::with_fetch(Box::new(
        Shared(endpoint.clone()),
    ))));
    let dir = tempfile::tempdir().unwrap();
    let path = auth_file(
        dir.path(),
        json!({"openai-codex": {"type": "oauth", "refresh": "ref-old", "access": jwt("acc-old"), "expires": 0, "accountId": "acc-old"}}),
    );

    let storage = AuthStorage::create(Some(path.clone()));
    let key = storage.get_api_key("openai-codex", false).await;

    assert_eq!(key.as_deref(), Some(fresh.as_str()));
    let request = sole_request(&endpoint, hoocode_ai_oauth_openai_codex::TOKEN_URL);
    let form = request.body.as_deref().unwrap();
    assert!(form.contains("grant_type=refresh_token"), "{form}");
    assert!(form.contains("refresh_token=ref-old"), "{form}");
    let stored = read_entry(&path, "openai-codex");
    assert_eq!(stored["access"], fresh.as_str());
    assert_eq!(stored["refresh"], "ref-new");
    assert_eq!(stored["accountId"], "acc-new");
}

#[tokio::test(flavor = "multi_thread")]
async fn gemini_expired_token_refreshes_through_get_api_key_with_the_project() {
    // The refresh reads the Gemini CLI client from the environment.
    std::env::set_var("HOOCODE_GEMINI_CLI_CLIENT_ID", "client-id");
    std::env::set_var("HOOCODE_GEMINI_CLI_CLIENT_SECRET", "client-secret");
    // No refresh_token in the reply: the stored refresh token is kept.
    let endpoint = token_endpoint(json!({"access_token": "ya29.new", "expires_in": 3600}));
    register_oauth_provider(Arc::new(GeminiCliOAuthProvider::with_fetch(Box::new(
        Shared(endpoint.clone()),
    ))));
    let dir = tempfile::tempdir().unwrap();
    let path = auth_file(
        dir.path(),
        json!({"google-gemini-cli": {"type": "oauth", "refresh": "ref-old", "access": "ya29.old", "expires": 0, "projectId": "proj-1"}}),
    );

    let storage = AuthStorage::create(Some(path.clone()));
    let key = storage.get_api_key("google-gemini-cli", false).await;

    // The Gemini key is the JSON `{token, projectId}`.
    let key: Value = serde_json::from_str(&key.expect("refreshed key")).unwrap();
    assert_eq!(key, json!({"token": "ya29.new", "projectId": "proj-1"}));
    let request = sole_request(&endpoint, hoocode_ai_oauth_google::TOKEN_URL);
    let form = request.body.as_deref().unwrap();
    assert!(form.contains("refresh_token=ref-old"), "{form}");
    let stored = read_entry(&path, "google-gemini-cli");
    assert_eq!(stored["access"], "ya29.new");
    assert_eq!(stored["refresh"], "ref-old");
    assert_eq!(stored["projectId"], "proj-1");
    assert!(stored["expires"].as_i64().unwrap() > 0);
}
