//! Session CRUD, directory layout, and lifecycle for the hoocode coding agent.
//!
//! The JSONL session tree: sessions are stored as
//! append-only `.jsonl` files where each line is a typed entry. Entries form a
//! tree via `id`/`parent_id`, and the leaf pointer tracks the current branch.

pub mod context;
pub mod detached;
pub mod entry;
pub mod identity;
pub mod lock;
pub mod manager;

pub use context::{build_session_context, ModelRef, SessionContext};
pub use detached::{detached_status, DetachedStatus};
pub use entry::{CustomMessageContent, FileEntry, Header, CURRENT_SESSION_VERSION};
pub use lock::{lock_holder, lock_path_for, session_in_use_message, SessionLock, SessionLockError};
pub use manager::{
    append_custom_entry_to_file, build_session_info, create_session_id, default_session_dir,
    default_sessions_root, encode_cwd, find_most_recent_session, flush_session_writes, generate_id,
    list_all_sessions, list_sessions, list_subagent_sessions, load_entries_from_file,
    load_raw_entries, load_session_file, session_id_of_file, subagents_dir, LoadedSession,
    NameSource, NewSessionOptions, SessionError, SessionInfo, SessionListProgress, SessionManager,
    SessionTreeNode, SubagentRun, SubagentState, SESSION_FLUSH_DEADLINE, SUBAGENT_RUN_ENTRY,
};
