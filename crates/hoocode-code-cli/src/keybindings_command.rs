//! `hoocode keybindings`: `--defaults` prints the default keymap, `check`
//! validates `keybindings.json`, and `--read` shows what the terminal sends for
//! each key (K-8, `docs/design/themes-keybindings.md`).

use std::io::{IsTerminal, Read, Write};
use std::path::Path;

use hoocode_code_paths::{agent_dir, VERSION};
use hoocode_code_tui_keybindings::{check_file, defaults_document};
use hoocode_tui_keys::parse_key;
use hoocode_tui_terminal::RawModeGuard;

const USAGE: &str = "usage: hoocode keybindings --defaults | check | --read";

/// Runs `hoocode keybindings <args>` and returns the exit code: 0 ok, 1 when
/// `check` finds errors or `--read` cannot run, 2 for a usage error.
pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--defaults"] => {
            let _ = write!(stdout, "{}", defaults_document(VERSION));
            0
        }
        ["check"] => check(&agent_dir(), stdout, stderr),
        ["--read"] => read_keys(stdout, stderr),
        _ => {
            let _ = writeln!(stderr, "{USAGE}");
            2
        }
    }
}

/// `check` for the agent dir `dir`: prints the file's errors (on stderr) and
/// cautions (on stderr too). Exits 1 when there is an error. A missing file is fine.
fn check(dir: &Path, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let report = check_file(dir);
    let path = report.path.display();
    if !report.exists {
        let _ = writeln!(stdout, "{path}: no file; the defaults apply");
        return 0;
    }
    for caution in &report.cautions {
        let _ = writeln!(stderr, "caution: {caution}");
    }
    for error in &report.errors {
        let _ = writeln!(stderr, "error: {error}");
    }
    if report.errors.is_empty() {
        let _ = writeln!(stdout, "{path}: ok");
        0
    } else {
        let count = report.errors.len();
        let _ = writeln!(stdout, "{path}: {count} error(s)");
        1
    }
}

/// Turns the raw bytes of one read into text: printable ASCII as it is, and the
/// rest as escapes (`\x1b`, `\r`, `\n`, `\t`).
pub fn describe_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        match byte {
            b'\r' => out.push_str("\\r"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(char::from(byte)),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out
}

/// Ends the read loop: `q`, or Ctrl+C twice in a row.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct QuitTracker {
    ctrl_c_pressed: bool,
}

impl QuitTracker {
    /// Whether the key `key` (as [`parse_key`] names it) ends the loop.
    pub fn press(&mut self, key: &str) -> bool {
        match key {
            "q" => true,
            "ctrl+c" => {
                let quit = self.ctrl_c_pressed;
                self.ctrl_c_pressed = true;
                quit
            }
            _ => {
                self.ctrl_c_pressed = false;
                false
            }
        }
    }
}

/// `--read`: prints the bytes each key press sends and the key id they parse to,
/// until `q` or Ctrl+C twice. Keys are read as legacy terminal input, so the
/// kitty protocol is not requested here.
fn read_keys(stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if !std::io::stdin().is_terminal() {
        let _ = writeln!(stderr, "hoocode keybindings --read needs a terminal");
        return 1;
    }
    let Ok(_raw) = RawModeGuard::enter() else {
        let _ = writeln!(stderr, "hoocode keybindings --read: cannot enter raw mode");
        return 1;
    };
    let _ = write!(stdout, "Press keys. q or Ctrl+C twice exits.\r\n");
    let _ = stdout.flush();
    let mut quit = QuitTracker::default();
    let mut buffer = [0u8; 64];
    loop {
        let read = match std::io::stdin().read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let bytes = &buffer[..read];
        let text = String::from_utf8_lossy(bytes);
        let key = parse_key(&text);
        let _ = write!(
            stdout,
            "{}  ->  {}\r\n",
            describe_bytes(bytes),
            key.as_deref().unwrap_or("(not a known key)")
        );
        let _ = stdout.flush();
        if quit.press(key.as_deref().unwrap_or_default()) {
            break;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    #[test]
    fn defaults_print_the_grouped_document() {
        let (code, out, _) = run_args(&["--defaults"]);
        assert_eq!(code, 0);
        assert!(out.contains("\"bindings\""), "{out}");
        assert!(out.contains("app.history.search"), "{out}");
    }

    #[test]
    fn an_unknown_argument_is_a_usage_error() {
        let (code, out, err) = run_args(&["--nope"]);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(err.contains("usage"), "{err}");
    }

    #[test]
    fn check_with_no_file_is_fine() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(check(dir.path(), &mut out, &mut err), 0);
        assert!(String::from_utf8_lossy(&out).contains("no file"));
    }

    #[test]
    fn check_exits_non_zero_on_an_error_and_zero_on_a_caution() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("keybindings.json");
        std::fs::write(&file, r#"{"app.model.cycleForward": "ctrl+c"}"#).expect("write");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(check(dir.path(), &mut out, &mut err), 1);
        assert!(String::from_utf8_lossy(&err).contains("reserved"));

        std::fs::write(&file, r#"{"app.model.cycleForward": "ctrl+s"}"#).expect("write");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(check(dir.path(), &mut out, &mut err), 0);
        assert!(String::from_utf8_lossy(&err).contains("caution"));
        assert!(String::from_utf8_lossy(&out).contains(": ok"));
    }

    #[test]
    fn describe_bytes_escapes_control_bytes() {
        assert_eq!(describe_bytes(b"\x1b[A"), "\\x1b[A");
        assert_eq!(describe_bytes(b"a\r\n\t"), "a\\r\\n\\t");
        assert_eq!(describe_bytes(&[0x7f]), "\\x7f");
    }

    #[test]
    fn q_or_ctrl_c_twice_quits() {
        let mut quit = QuitTracker::default();
        assert!(!quit.press("ctrl+c"));
        assert!(quit.press("ctrl+c"));
        let mut quit = QuitTracker::default();
        assert!(!quit.press("ctrl+c"));
        assert!(!quit.press("a"), "another key starts the count again");
        assert!(!quit.press("ctrl+c"));
        assert!(quit.press("q"));
    }
}
