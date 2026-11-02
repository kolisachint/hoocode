//! The built-in read-only Shell allowlist (`safe_shell` in `hoo-config.json`):
//! commands that only read run in every mode without a prompt.

use crate::{has_code_expansion, is_assignment, redirects_are_harmless, split_segments};

/// Programs that only read, whatever their (non-listed) arguments.
const READ_ONLY_PROGRAMS: [&str; 33] = [
    "ls", "pwd", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "which", "whoami",
    "uname", "basename", "dirname", "realpath", "readlink", "stat", "du", "df", "nl", "cut", "tr",
    "diff", "cmp", "comm", "true", "false", "echo", "printf", "tree", "file", "sort",
];

/// Arguments that make an otherwise read-only program write or run code.
fn has_unsafe_arg(program: &str, args: &[String]) -> bool {
    let long = |prefixes: &[&str]| {
        args.iter()
            .any(|a| prefixes.iter().any(|p| a.starts_with(p)))
    };
    // A short flag, alone or in a cluster (`-o`, `-ro`).
    let short = |flag: char| {
        args.iter().any(|a| {
            a.len() > 1 && a.starts_with('-') && !a.starts_with("--") && a[1..].contains(flag)
        })
    };
    match program {
        // `sort -o file`, `tree -o file`: write a file.
        // `sort --compress-program=prog` runs a program.
        "sort" => short('o') || long(&["--output", "--compress-program"]),
        "tree" => short('o') || long(&["--output"]),
        // `rg --pre cmd` runs a command on every file.
        "rg" => long(&["--pre"]),
        // `file -C` compiles a magic file.
        "file" => short('C') || long(&["--compile"]),
        _ => false,
    }
}

/// `git` subcommands that only read.
const GIT_READ_ONLY: [&str; 9] = [
    "status",
    "diff",
    "log",
    "show",
    "blame",
    "rev-parse",
    "ls-files",
    "grep",
    "branch",
];

/// `git branch` flags that only list.
const GIT_BRANCH_LIST_FLAGS: [&str; 8] = [
    "-a",
    "-r",
    "-v",
    "-vv",
    "--all",
    "--remotes",
    "--list",
    "--show-current",
];

fn git_is_read_only(args: &[String]) -> bool {
    // The subcommand must come first: `git -c ...` / `git --exec-path` can run code.
    let Some((sub, rest)) = args.split_first() else {
        return false;
    };
    if !GIT_READ_ONLY.contains(&sub.as_str()) {
        return false;
    }
    // `--output` writes a file; `-O` / `--open-files-in-pager` and
    // `--ext-diff` / `--textconv` run other programs.
    let unsafe_flag = rest.iter().any(|a| {
        a.starts_with("--output")
            || a.starts_with("--open-files-in-pager")
            || a.starts_with("-O")
            || a == "--ext-diff"
            || a == "--textconv"
    });
    if unsafe_flag {
        return false;
    }
    if sub == "branch" {
        return rest
            .iter()
            .all(|a| GIT_BRANCH_LIST_FLAGS.contains(&a.as_str()));
    }
    true
}

/// Whether one pipe/list segment (program plus arguments) only reads.
fn segment_is_read_only(words: &[String]) -> bool {
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    if program.contains('/') || is_assignment(program) {
        return false;
    }
    match program.as_str() {
        // `cd` only moves the command's own shell.
        "cd" => args.len() <= 1,
        "git" => git_is_read_only(args),
        p => READ_ONLY_PROGRAMS.contains(&p) && !has_unsafe_arg(p, args),
    }
}

/// Whether `command` only reads: every segment (split on `|`, `&&`, `||`,
/// `;`, `&`) is a listed read-only program with safe arguments, there is
/// no code expansion (`$(…)`, backticks, here-docs, line breaks), and no
/// file redirect (only `/dev/null` and descriptor dups). Anything else is
/// not "safe" and goes through the normal approval.
pub fn is_safe_shell_command(command: &str) -> bool {
    if command.trim().is_empty() || command.len() > 32 * 1024 {
        return false;
    }
    if has_code_expansion(command) {
        return false;
    }
    let (segments, redirects, _) = split_segments(command);
    if segments.is_empty() {
        return false;
    }
    if !redirects_are_harmless(command, &redirects) {
        return false;
    }
    segments.iter().all(|words| segment_is_read_only(words))
}
