//! `core/auth-guidance.ts`: what to tell the user when no model or key is set.

const UNKNOWN_PROVIDER: &str = "unknown";

/// Login guidance that works for an installed binary: the docs are online
/// (the install ships no docs directory).
fn provider_login_help() -> String {
    [
        "Use /login to log into a provider (subscription or API key), or set a provider API key such as ANTHROPIC_API_KEY or OPENAI_API_KEY.",
        "Docs: https://kolisachint.github.io/hoocode/",
    ]
    .join("\n")
}

/// `formatNoModelsAvailableMessage`.
pub fn format_no_models_available_message() -> String {
    format!("No models available. {}", provider_login_help())
}

/// `formatNoModelSelectedMessage`.
pub fn format_no_model_selected_message() -> String {
    format!(
        "No model selected.\n\n{}\n\nThen use /model to select a model.",
        provider_login_help()
    )
}

/// The error when an OAuth provider's credential is missing at request time.
pub fn format_auth_failed_message(provider: &str) -> String {
    format!(
        "Authentication failed for \"{provider}\". Credentials may have expired or network is unavailable. Run '/login {provider}' to re-authenticate."
    )
}

/// `formatNoApiKeyFoundMessage`.
pub fn format_no_api_key_found_message(provider: &str) -> String {
    let provider_display = if provider == UNKNOWN_PROVIDER {
        "the selected model"
    } else {
        provider
    };
    format!(
        "No API key found for {provider_display}.\n\n{}",
        provider_login_help()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_help_points_at_login_env_vars_and_the_online_docs() {
        let help = format_no_models_available_message();
        assert!(help.contains("/login"), "{help}");
        assert!(help.contains("ANTHROPIC_API_KEY"), "{help}");
        assert!(
            help.contains("https://kolisachint.github.io/hoocode/"),
            "{help}"
        );
        assert!(!help.contains("providers.md"), "{help}");
    }

    #[test]
    fn auth_failed_names_the_provider_and_the_login_command() {
        assert_eq!(
            format_auth_failed_message("openai-codex"),
            "Authentication failed for \"openai-codex\". Credentials may have expired or network is unavailable. Run '/login openai-codex' to re-authenticate."
        );
    }
}
