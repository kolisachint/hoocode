//! Discovery: the effective servers with their trust, and the `/mcp` state the list shows.

use crate::config::{Diagnostic, McpConfig, ServerDef, Source, Transport};
use crate::trust::{TrustStatus, TrustStore};

/// The state a server shows in `/mcp`.
///
/// [`discover`] sets only `NotTrusted` and `Disabled`. The MCP client sets the others as it
/// connects: `Connected`, `AuthNeeded`, and `Failed` with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerState {
    Connected,
    NotTrusted,
    AuthNeeded,
    Failed(String),
    Disabled,
}

/// One effective server, with its source and trust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredServer {
    pub name: String,
    pub source: Source,
    pub transport: Transport,
    pub trust: TrustStatus,
    /// `None` when the server is trusted and enabled but not started yet; the client fills it in.
    pub state: Option<ServerState>,
}

/// A folder or plugin whose servers need a grant, or a new one, from the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustPrompt {
    /// The trust store key to grant (see [`crate::config::Source::trust_key`]).
    pub key: String,
    pub source: Source,
    /// [`TrustStatus::Untrusted`] for a new source, or [`TrustStatus::Changed`] with the diff.
    pub status: TrustStatus,
}

/// The output of [`discover`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Effective servers (one per name), in the order [`crate::config::load`] gives.
    pub servers: Vec<DiscoveredServer>,
    /// Sources to ask about. Empty when everything is trusted.
    pub prompts: Vec<TrustPrompt>,
    /// Load problems, as [`crate::config::load`] reported them.
    pub diagnostics: Vec<Diagnostic>,
}

/// Mark each effective server with its trust and state.
///
/// Servers from an untrusted or changed source are `NotTrusted` and do not start. Disabled
/// servers are `Disabled`, whatever their trust. Only sources with an effective server get a
/// prompt, so a file whose entries are all shadowed is not asked about.
pub fn discover(config: &McpConfig, trust: &TrustStore) -> Discovery {
    let statuses: Vec<(Source, TrustStatus)> = config
        .sources
        .iter()
        .map(|loaded| {
            (
                loaded.source.clone(),
                trust.status(&loaded.source, &loaded.declared),
            )
        })
        .collect();
    let status_of = |source: &Source| {
        statuses
            .iter()
            .find(|(s, _)| s == source)
            .map(|(_, status)| status.clone())
            .unwrap_or(TrustStatus::NotRequired)
    };

    let servers = config
        .servers
        .iter()
        .map(|def: &ServerDef| {
            let trust = status_of(&def.source);
            let state = if def.disabled {
                Some(ServerState::Disabled)
            } else {
                match trust {
                    TrustStatus::Untrusted | TrustStatus::Changed(_) => {
                        Some(ServerState::NotTrusted)
                    }
                    TrustStatus::NotRequired | TrustStatus::Trusted => None,
                }
            };
            DiscoveredServer {
                name: def.name.clone(),
                source: def.source.clone(),
                transport: def.transport.clone(),
                trust,
                state,
            }
        })
        .collect();

    let mut prompts: Vec<TrustPrompt> = Vec::new();
    for (source, status) in &statuses {
        let Some(key) = source.trust_key() else {
            continue;
        };
        if !matches!(status, TrustStatus::Untrusted | TrustStatus::Changed(_)) {
            continue;
        }
        let contributes = config.servers.iter().any(|def| &def.source == source);
        if !contributes || prompts.iter().any(|p| p.key == key) {
            continue;
        }
        prompts.push(TrustPrompt {
            key,
            source: source.clone(),
            status: status.clone(),
        });
    }

    Discovery {
        servers,
        prompts,
        diagnostics: config.diagnostics.clone(),
    }
}
