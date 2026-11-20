//! Tier C agent identities: the root agent and the tokens minted per subagent.
//!
//! Every process that spawns a subagent mints the child's token (32 random
//! bytes, hex) and agent id (`<parent agent id>/<task id>`), and passes
//! `HOOCODE_AGENT_ID`, `HOOCODE_AGENT_TOKEN`, `HOOCODE_AGENT_PARENT` and
//! `HOOCODE_AGENT_DEPTH` in the child env (spec D5). The root agent's id is
//! `root`. Slice 4 passes these through the parent<-child relay; this crate
//! only mints them and tells helpers about them.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// The agent id in the child env (spec D5).
pub const AGENT_ID_ENV: &str = "HOOCODE_AGENT_ID";
/// The bearer token in the child env (spec D5). Never logged.
pub const AGENT_TOKEN_ENV: &str = "HOOCODE_AGENT_TOKEN";
/// The parent agent id in the child env (spec D5).
pub const AGENT_PARENT_ENV: &str = "HOOCODE_AGENT_PARENT";
/// The agent depth in the child env (spec D5).
pub const AGENT_DEPTH_ENV: &str = "HOOCODE_AGENT_DEPTH";

/// The root agent's id.
pub const ROOT_AGENT_ID: &str = "root";

/// The `_meta` key carrying the agent identity on every `tools/call`.
pub use hoocode_agent_mcp::AGENT_META_KEY;

/// One agent's identity: what it proves (the bearer token) and what it claims
/// (the `_meta` entry, checked against the token by the helper).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub parent_id: Option<String>,
    pub depth: u64,
    /// 32 random bytes, hex. Sent as `Authorization: Bearer <token>`, never to
    /// the model, never logged.
    pub token: String,
}

impl AgentIdentity {
    /// The root agent: id `root`, no parent, depth 0.
    pub fn root(token: impl Into<String>) -> Self {
        Self {
            agent_id: ROOT_AGENT_ID.to_owned(),
            parent_id: None,
            depth: 0,
            token: token.into(),
        }
    }

    /// The `_meta["dev.hoocode/agent"]` value carried on every `tools/call`.
    pub fn call_meta(&self) -> Map<String, Value> {
        let mut meta = Map::new();
        meta.insert(
            AGENT_META_KEY.to_owned(),
            serde_json::json!({
                "agentId": self.agent_id,
                "parentId": self.parent_id,
                "depth": self.depth,
            }),
        );
        meta
    }

    /// The `hoocode/agentStarted` params for this agent (plus its token).
    pub fn started_params(&self) -> Value {
        serde_json::json!({
            "agentId": self.agent_id,
            "parentId": self.parent_id,
            "depth": self.depth,
            "token": self.token,
        })
    }
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// A fresh bearer token: 32 random bytes from the OS, hex.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex_encode(&bytes)
}

/// Read one variable, ignoring empty values.
fn read_env(get: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    get(name).filter(|value| !value.is_empty())
}

/// The identity described by the four child env variables (spec D5): `None`
/// when the id or token is missing or empty (a root process, or a child
/// spawned without tier C). The depth falls back to 0 when it does not
/// parse; an empty parent means the root. Never logs: the token stays in
/// the returned value.
fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Option<AgentIdentity> {
    let agent_id = read_env(get, AGENT_ID_ENV)?;
    let token = read_env(get, AGENT_TOKEN_ENV)?;
    let parent_id = read_env(get, AGENT_PARENT_ENV);
    let depth = read_env(get, AGENT_DEPTH_ENV)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(0);
    Some(AgentIdentity {
        agent_id,
        parent_id,
        depth,
        token,
    })
}

impl AgentIdentity {
    /// This process's own identity from its environment ([`from_map`] over
    /// [`std::env`]): `Some` in a subagent child spawned from [`mint_agent`],
    /// `None` in a root process. Never logs.
    pub fn from_env() -> Option<Self> {
        from_vars(&|name| std::env::var(name).ok())
    }

    /// [`from_env`](Self::from_env) over an explicit map (tests, child env builders).
    pub fn from_map(env: &HashMap<String, String>) -> Option<Self> {
        from_vars(&|name| env.get(name).cloned())
    }

    /// This identity as the four child env variables (spec D5): id, token,
    /// parent (empty for the root) and depth. The caller passes them on
    /// explicitly (children start with `env_clear`). Never logs.
    pub fn child_env(&self) -> [(String, String); 4] {
        [
            (AGENT_ID_ENV.to_owned(), self.agent_id.clone()),
            (AGENT_TOKEN_ENV.to_owned(), self.token.clone()),
            (
                AGENT_PARENT_ENV.to_owned(),
                self.parent_id.clone().unwrap_or_default(),
            ),
            (AGENT_DEPTH_ENV.to_owned(), self.depth.to_string()),
        ]
    }
}

/// Mint a child agent of `parent` for `task_id`: id `<parent>/<task id>`,
/// depth one more, and a fresh token. Unique per session while task ids are.
pub fn mint_agent(parent: &AgentIdentity, task_id: &str) -> AgentIdentity {
    AgentIdentity {
        agent_id: format!("{}/{}", parent.agent_id, task_id),
        parent_id: Some(parent.agent_id.clone()),
        depth: parent.depth + 1,
        token: new_token(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_has_no_parent_and_depth_zero() {
        let root = AgentIdentity::root("token");
        assert_eq!(root.agent_id, "root");
        assert_eq!(root.parent_id, None);
        assert_eq!(root.depth, 0);
    }

    #[test]
    fn minted_children_nest_and_deepen() {
        let root = AgentIdentity::root("root-token");
        let child = mint_agent(&root, "task-1");
        assert_eq!(child.agent_id, "root/task-1");
        assert_eq!(child.parent_id, Some("root".to_owned()));
        assert_eq!(child.depth, 1);
        let grandchild = mint_agent(&child, "task-2");
        assert_eq!(grandchild.agent_id, "root/task-1/task-2");
        assert_eq!(grandchild.depth, 2);
    }

    #[test]
    fn tokens_are_64_hex_characters_and_unique() {
        let tokens: Vec<String> = (0..100).map(|_| new_token()).collect();
        for token in &tokens {
            assert_eq!(token.len(), 64, "{token}");
            assert!(token.bytes().all(|b| b.is_ascii_hexdigit()), "{token}");
        }
        let unique: std::collections::HashSet<&str> = tokens.iter().map(String::as_str).collect();
        assert_eq!(unique.len(), tokens.len());
    }
}
