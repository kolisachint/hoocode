//! `core/slash-commands.ts`: slash command metadata and the built-in list.

use crate::source_info::SourceInfo;

/// `SlashCommandSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandSource {
    Extension,
    Prompt,
    Skill,
}

/// `SlashCommandInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandInfo {
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    pub source_info: SourceInfo,
}

/// `BuiltinSlashCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// `BUILTIN_SLASH_COMMANDS`, in hoocode's order (`APP_NAME` is hoocode).
pub const BUILTIN_SLASH_COMMANDS: &[BuiltinSlashCommand] = &[
    BuiltinSlashCommand {
        name: "settings",
        description: "Open settings menu",
    },
    BuiltinSlashCommand {
        name: "model",
        description: "Select model (opens selector UI)",
    },
    BuiltinSlashCommand {
        name: "scoped-models",
        description: "Enable/disable models for model cycling",
    },
    BuiltinSlashCommand {
        name: "export",
        description: "Export the session to a JSONL file: /export <file.jsonl>",
    },
    BuiltinSlashCommand {
        name: "import",
        description: "Import and resume a session from a JSONL file",
    },
    BuiltinSlashCommand {
        name: "copy",
        description: "Copy as markdown + formatted text: /copy (last reply), /copy all, /copy <turns>",
    },
    BuiltinSlashCommand {
        name: "name",
        description: "Set session display name",
    },
    BuiltinSlashCommand {
        name: "color",
        description: "Cycle the hull paint: /color [none|parity|cyan|violet|lime|red]",
    },
    BuiltinSlashCommand {
        name: "chrome",
        description: "Set how much screen the chrome gets: /chrome <full|compact>",
    },
    BuiltinSlashCommand {
        name: "session",
        description: "Show session info and stats",
    },
    BuiltinSlashCommand {
        name: "plugin",
        description: "Manage plugins: /plugin list, install, remove, update, trust, marketplace",
    },
    BuiltinSlashCommand {
        name: "changelog",
        description: "Show changelog entries",
    },
    BuiltinSlashCommand {
        name: "hotkeys",
        description: "Show all keyboard shortcuts",
    },
    BuiltinSlashCommand {
        name: "perf",
        description: "Show UI performance counters (threads, RSS, frame and keystroke timing)",
    },
    BuiltinSlashCommand {
        name: "fork",
        description: "Create a new fork from a previous user message",
    },
    BuiltinSlashCommand {
        name: "clone",
        description: "Duplicate the current session at the current position",
    },
    BuiltinSlashCommand {
        name: "tree",
        description: "Navigate session tree (switch branches)",
    },
    BuiltinSlashCommand {
        name: "login",
        description: "Configure provider authentication",
    },
    BuiltinSlashCommand {
        name: "logout",
        description: "Remove provider authentication",
    },
    BuiltinSlashCommand {
        name: "new",
        description: "Start a new session",
    },
    BuiltinSlashCommand {
        name: "compact",
        description: "Manually compact the session context",
    },
    BuiltinSlashCommand {
        name: "resume",
        description: "Resume a different session",
    },
    BuiltinSlashCommand {
        name: "cd",
        description: "Change working directory: /cd <path> (bare = home, - = previous). Starts a session there.",
    },
    BuiltinSlashCommand {
        name: "reload",
        description: "Reload keybindings, skills, prompts, and themes",
    },
    BuiltinSlashCommand {
        name: "quit",
        description: "Quit hoocode",
    },
    BuiltinSlashCommand {
        name: "subagent",
        description: "Spawn a subagent directly: /subagent <mode> <task>",
    },
    // Not in hoocode: orchestration of the dispatch ledger, ours.
    BuiltinSlashCommand {
        name: "subagent-cancel",
        description: "Cancel the newest running subagent: /subagent-cancel [task_id]",
    },
    BuiltinSlashCommand {
        name: "subagent-retry",
        description: "Re-dispatch the newest failed subagent: /subagent-retry [agent] [task]",
    },
    BuiltinSlashCommand {
        name: "subagent-stats",
        description: "Subagent reliability from the dispatch ledger: /subagent-stats [24h|7d|all]",
    },
    BuiltinSlashCommand {
        name: "agents",
        description: "Attach to a subagent run of this session (read-only)",
    },
    BuiltinSlashCommand {
        name: "loop",
        description: "Schedule a prompt (/loop 5m <prompt>) or run until done (/loop auto <task>)",
    },
];
