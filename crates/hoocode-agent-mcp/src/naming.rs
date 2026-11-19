//! Model-facing names for MCP tools: `mcp__<server>__<tool>` (`docs/design/plugins.md`,
//! Naming and identity).
//!
//! Plugin WASM tools are not named here: they keep the name their plugin declares. An MCP
//! server is not written for hoocode, so its tools keep a host prefix.

/// Longest tool name a model API accepts from us.
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// Length of the hash suffix on a shortened name: `_` plus 8 hex digits.
const HASH_SUFFIX_LEN: usize = 9;

/// The model-facing name of `tool` on `server`: `mcp__<server>__<tool>`.
///
/// Each part is sanitised (a character outside `[A-Za-z0-9_-]` becomes `_`, and a run of `_`
/// becomes one `_`), so `__` only ever separates the parts. `None` when a part is empty: the
/// tool is then not loaded, and the caller says so.
pub fn tool_name(server: &str, tool: &str) -> Option<String> {
    let server = sanitize_part(server);
    let tool = sanitize_part(tool);
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some(capped(format!("mcp__{server}__{tool}")))
}

/// One part of a name: unsafe characters become `_`, every run of `_` becomes one `_`, and the
/// leading and trailing `_` are dropped. So a part never starts or ends with `_`, and the `__`
/// separator cannot run into a neighbouring `_` (`a_` + `b` and `a` + `_b` both give `a`, `b`).
fn sanitize_part(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for c in part.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            c
        } else {
            '_'
        };
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    out.trim_matches('_').to_owned()
}

/// A name longer than [`MAX_TOOL_NAME_LEN`] is cut to 55 characters and given `_` and an
/// 8-digit hex hash of the full name, so it stays 64 characters and two long names with the
/// same prefix still differ. The hash is FNV-1a, so the result does not change between Rust
/// versions.
fn capped(full: String) -> String {
    if full.len() <= MAX_TOOL_NAME_LEN {
        return full;
    }
    let hash = fnv1a_64(full.as_bytes()) as u32;
    let keep = MAX_TOOL_NAME_LEN - HASH_SUFFIX_LEN;
    // Sanitized names are ASCII, so byte indexing is safe.
    format!("{}_{hash:08x}", &full[..keep])
}

/// 64-bit FNV-1a hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes
        .iter()
        .fold(OFFSET, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(server: &str, tool: &str) -> String {
        tool_name(server, tool).expect("both parts are non-empty")
    }

    #[test]
    fn short_names_are_double_underscored_and_kept() {
        assert_eq!(name("fs", "read_file"), "mcp__fs__read_file");
        assert_eq!(name("my-server", "get-x"), "mcp__my-server__get-x");
    }

    #[test]
    fn unsafe_characters_become_underscores_and_runs_collapse() {
        assert_eq!(
            name("git hub", "repo.list/all"),
            "mcp__git_hub__repo_list_all"
        );
        assert_eq!(name("srv", "tool:ü"), "mcp__srv__tool");
        // A `__` inside a part collapses, so it cannot be mistaken for the separator.
        assert_eq!(name("a__b", "c___d"), "mcp__a_b__c_d");
    }

    #[test]
    fn an_empty_part_is_refused() {
        assert_eq!(tool_name("", "x"), None);
        assert_eq!(tool_name("s", ""), None);
        // A part of only `_` is empty once its underscores are dropped.
        assert_eq!(tool_name("s", "___"), None);
    }

    #[test]
    fn a_part_has_no_leading_or_trailing_underscore() {
        // The two splits of the same characters give the same name, so the collision check
        // (not the naming) decides which tool keeps it.
        assert_eq!(name("a_", "b"), "mcp__a__b");
        assert_eq!(name("a_", "b"), name("a", "_b"));
        for (server, tool) in [("a_", "_b__"), ("__x", "y_"), ("q", "_")] {
            if let Some(full) = tool_name(server, tool) {
                assert!(!full.contains("___"), "{full}");
                assert!(!full.contains("____"), "{full}");
            }
        }
    }

    #[test]
    fn long_names_are_cut_to_64_with_a_hash_suffix() {
        let tool = "t".repeat(100);
        let full = name("server", &tool);
        assert_eq!(full.len(), MAX_TOOL_NAME_LEN);
        assert!(full.starts_with("mcp__server__ttt"));
        let suffix = &full[full.len() - 8..];
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(&full[full.len() - 9..full.len() - 8], "_");
    }

    #[test]
    fn long_names_with_the_same_prefix_differ() {
        let a = name("server", &format!("{}a", "x".repeat(80)));
        let b = name("server", &format!("{}b", "x".repeat(80)));
        assert_eq!(a.len(), MAX_TOOL_NAME_LEN);
        assert_eq!(b.len(), MAX_TOOL_NAME_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn names_at_the_limit_are_unchanged() {
        // 64 characters exactly: "mcp__s__" is 8 characters, so the tool has 56.
        let tool = "y".repeat(56);
        let full = name("s", &tool);
        assert_eq!(full.len(), 64);
        assert_eq!(full, format!("mcp__s__{tool}"));
    }

    #[test]
    fn hash_is_stable() {
        // FNV-1a 64-bit reference vectors.
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            name("server", &"z".repeat(100)),
            name("server", &"z".repeat(100))
        );
    }
}
