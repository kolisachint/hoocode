//! Names for MCP tools as the model sees them: `mcp_<server>_<tool>`.

/// Longest tool name a model API accepts from us.
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// Length of the hash suffix on a shortened name: `_` plus 8 hex digits.
const HASH_SUFFIX_LEN: usize = 9;

/// The model-facing name of `tool` on `server`.
///
/// The result is `mcp_<server>_<tool>` with every character outside
/// `[A-Za-z0-9_-]` replaced by `_`. A name longer than
/// [`MAX_TOOL_NAME_LEN`] is cut to 55 characters and given `_` and an 8-digit
/// hex hash of the full (sanitized) name, so it stays 64 characters and two
/// long names with the same prefix still differ. The hash is FNV-1a, so the
/// result does not change between Rust versions.
pub fn tool_name(server: &str, tool: &str) -> String {
    let full: String = format!("mcp_{server}_{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
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

    #[test]
    fn short_names_are_prefixed_and_kept() {
        assert_eq!(tool_name("fs", "read_file"), "mcp_fs_read_file");
        assert_eq!(tool_name("my-server", "get-x"), "mcp_my-server_get-x");
    }

    #[test]
    fn unsafe_characters_become_underscores() {
        assert_eq!(
            tool_name("git hub", "repo.list/all"),
            "mcp_git_hub_repo_list_all"
        );
        assert_eq!(tool_name("srv", "tool:ü"), "mcp_srv_tool__");
    }

    #[test]
    fn long_names_are_cut_to_64_with_a_hash_suffix() {
        let tool = "t".repeat(100);
        let name = tool_name("server", &tool);
        assert_eq!(name.len(), MAX_TOOL_NAME_LEN);
        assert!(name.starts_with("mcp_server_ttt"));
        let suffix = &name[name.len() - 8..];
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(&name[name.len() - 9..name.len() - 8], "_");
    }

    #[test]
    fn long_names_with_the_same_prefix_differ() {
        let a = tool_name("server", &format!("{}a", "x".repeat(80)));
        let b = tool_name("server", &format!("{}b", "x".repeat(80)));
        assert_eq!(a.len(), MAX_TOOL_NAME_LEN);
        assert_eq!(b.len(), MAX_TOOL_NAME_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn names_at_the_limit_are_unchanged() {
        // 64 characters exactly: "mcp_s_" is 6 characters, so the tool has 58.
        let tool = "y".repeat(58);
        let name = tool_name("s", &tool);
        assert_eq!(name.len(), 64);
        assert_eq!(name, format!("mcp_s_{tool}"));
    }

    #[test]
    fn hash_is_stable() {
        // FNV-1a 64-bit reference vectors.
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        let name = tool_name("server", &"z".repeat(100));
        assert_eq!(name, tool_name("server", &"z".repeat(100)));
    }
}
