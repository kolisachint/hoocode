//! The built-in read-only Shell allowlist (`safe_shell` in `hoo-config.json`):
//! commands that only read run in every mode without a prompt.

use crate::{has_code_expansion, is_assignment, redirects_are_harmless, split_words, Word};

/// Programs that only read, whatever their (non-listed) arguments.
const READ_ONLY_PROGRAMS: [&str; 33] = [
    "ls", "pwd", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "which", "whoami",
    "uname", "basename", "dirname", "realpath", "readlink", "stat", "du", "df", "nl", "cut", "tr",
    "diff", "cmp", "comm", "true", "false", "echo", "printf", "tree", "file", "sort",
];

/// Whether `arg` is a long option naming `name` (`--name`, `--name=x`,
/// `--name-more`), or a GNU-style abbreviation of it (`--out` for
/// `--output`: getopt accepts any unambiguous prefix).
fn long_flag(arg: &str, name: &str) -> bool {
    let Some(given) = arg.strip_prefix("--") else {
        return false;
    };
    let given = given.split('=').next().unwrap_or("");
    !given.is_empty() && (name.starts_with(given) || given.starts_with(name))
}

/// Whether a single-dash argument (alone or in a cluster: `-o`, `-ro`)
/// holds `flag`.
fn short_flag(arg: &str, flag: char) -> bool {
    arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains(flag)
}

/// Programs with arguments that write or run code (see [`has_unsafe_arg`]).
/// For these, a word the shell could expand into such an argument (an
/// unquoted `*`, `?`, `[` or `{` before any `/`) is not safe either.
const RISKY_ARG_PROGRAMS: [&str; 6] = ["sort", "tree", "rg", "file", "tail", "git"];

/// Arguments that make an otherwise read-only program write, run code or
/// never end.
fn has_unsafe_arg(program: &str, args: &[String]) -> bool {
    let long = |names: &[&str]| args.iter().any(|a| names.iter().any(|n| long_flag(a, n)));
    let short = |flag: char| args.iter().any(|a| short_flag(a, flag));
    match program {
        // `sort -o file`, `tree -o file`: write a file.
        // `sort --compress-program=prog` runs a program.
        "sort" => short('o') || long(&["output", "compress-program"]),
        // `tree -R` writes `00Tree.html` into every directory.
        "tree" => short('o') || short('R') || long(&["output"]),
        // `rg --pre cmd` runs a command on every file; `--hostname-bin`
        // runs one for hyperlinks.
        "rg" => long(&["pre", "hostname-bin"]),
        // `file -C` compiles a magic file.
        "file" => short('C') || long(&["compile"]),
        // `tail -f` never ends.
        "tail" => short('f') || short('F') || long(&["follow"]),
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
    // `--ext-diff` / `--textconv` run other programs. Git takes long
    // option prefixes and short clusters (`-nO`) too.
    let unsafe_flag = rest.iter().any(|a| {
        short_flag(a, 'O')
            || ["output", "open-files-in-pager", "ext-diff", "textconv"]
                .iter()
                .any(|n| long_flag(a, n))
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
fn segment_is_read_only(words: &[Word]) -> bool {
    let Some((program, rest)) = words.split_first() else {
        return false;
    };
    let program = &program.text;
    if RISKY_ARG_PROGRAMS.contains(&program.as_str()) && rest.iter().any(|w| w.expands_to_flag) {
        return false;
    }
    let args: Vec<String> = rest.iter().map(|w| w.text.clone()).collect();
    let args = args.as_slice();
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

/// Whether `command` has a `$` the shell expands (outside single quotes,
/// not escaped): `$VAR`, `$1`, `$'\x2d-pre'` and `$"..."` all turn into
/// text the word checks cannot see.
fn has_parameter_expansion(command: &str) -> bool {
    let mut single = false;
    let mut escaped = false;
    for c in command.chars() {
        if single {
            single = c != '\'';
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '\'' => single = true,
            '$' => return true,
            _ => {}
        }
    }
    false
}

/// Whether `command` only reads: every segment (split on `|`, `&&`, `||`,
/// `;`, `&`) is a listed read-only program with safe arguments, there is
/// no expansion (`$(…)`, `$VAR`, backticks, here-docs, line breaks, and for
/// programs with risky flags no glob or brace before a `/`), and no
/// file redirect (only `/dev/null` and descriptor dups). Anything else is
/// not "safe" and goes through the normal approval.
pub fn is_safe_shell_command(command: &str) -> bool {
    if command.trim().is_empty() || command.len() > 32 * 1024 {
        return false;
    }
    if has_code_expansion(command) || has_parameter_expansion(command) {
        return false;
    }
    let (segments, redirects, _) = split_words(command);
    if segments.is_empty() {
        return false;
    }
    if !redirects_are_harmless(command, &redirects) {
        return false;
    }
    segments.iter().all(|words| segment_is_read_only(words))
}
