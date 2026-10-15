//! Startup version check and the `hoocode update` command (extras.md, item 1).
//!
//! At most once a day the interactive mode asks the npm registry for the latest
//! `@kolisachint/hoocode`. The request runs on the IO runtime and its result
//! reaches the TUI over a channel, so startup never waits for the network. The
//! last check time and the latest version seen are kept in
//! `<agent dir>/version-check.json`: a newer release is still announced on later
//! starts without another request.
//!
//! Print, json and rpc modes never check. `hoocode update` only prints the command
//! for how hoocode was installed; it never replaces the binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The npm registry's `latest` document for the package.
pub const REGISTRY_LATEST_URL: &str = "https://registry.npmjs.org/@kolisachint/hoocode/latest";
/// The curl installer (`docs/design/distribution.md`, channel 4).
pub const CURL_UPDATE_COMMAND: &str =
    "curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh";
pub const NPM_UPDATE_COMMAND: &str = "npm install -g @kolisachint/hoocode@latest";
pub const BUN_UPDATE_COMMAND: &str = "bun add -g @kolisachint/hoocode@latest";
/// One check a day.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// The whole request, body included. Errors are silent, so this only bounds the wait.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const STATE_FILE: &str = "version-check.json";

/// `major.minor.patch` from a version string. A leading `v` and any pre-release or
/// build suffix are dropped; anything else that is not three numbers is `None`.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let core = text.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Whether `text` is a pre-release (`1.2.3-beta.1`). Build metadata (`+...`) does not count.
fn is_prerelease(text: &str) -> bool {
    let core = text.trim().trim_start_matches('v');
    core.split('+')
        .next()
        .is_some_and(|core| core.contains('-'))
}

/// Whether `latest` is a newer release than `running`. Unparseable input is never newer.
/// On the same `major.minor.patch`, a release is newer than its own pre-release
/// (`1.0.0` after `1.0.0-beta.1`). Two pre-releases of one version are not ordered.
pub fn is_newer(latest: &str, running: &str) -> bool {
    match (parse_version(latest), parse_version(running)) {
        (Some(l), Some(r)) if l == r => !is_prerelease(latest) && is_prerelease(running),
        (Some(latest), Some(running)) => latest > running,
        _ => false,
    }
}

/// The startup check runs only in interactive mode, when offline mode is off and
/// the `checkForUpdates` setting is on.
pub fn check_enabled(interactive: bool, offline: bool, setting_enabled: bool) -> bool {
    interactive && !offline && setting_enabled
}

/// Whether a check is due at `now` (unix seconds). A last-check time in the future
/// (a clock moved back) counts as due.
pub fn check_due(last_checked: Option<u64>, now: u64) -> bool {
    match last_checked {
        None => true,
        Some(last) if last > now => true,
        Some(last) => now - last >= CHECK_INTERVAL_SECS,
    }
}

/// What is kept between starts.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckState {
    /// Unix seconds of the last attempt (recorded before the request).
    pub checked_at: Option<u64>,
    /// The latest version the registry reported on the last successful check.
    pub latest: Option<String>,
}

/// Reads the state text; anything unreadable is an empty state.
pub fn parse_state(text: &str) -> CheckState {
    serde_json::from_str(text).unwrap_or_default()
}

pub fn render_state(state: &CheckState) -> String {
    serde_json::to_string(state).unwrap_or_default()
}

/// The `version` of a registry `latest` document, when it is a version.
pub fn parse_registry_latest(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let version = value.get("version")?.as_str()?.trim();
    parse_version(version).map(|_| version.to_string())
}

/// How this binary was installed, from its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Under the curl installer's `bin` directory.
    Curl,
    /// Inside a `node_modules` tree installed by npm.
    Npm,
    /// Inside a `node_modules` tree under bun's global store.
    Bun,
    /// Anywhere else: no command is offered.
    Other,
}

/// The install method for an executable at `exe`. `curl_bin_dir` is the curl
/// installer's `bin` directory.
pub fn install_method(exe: &Path, curl_bin_dir: &Path) -> InstallMethod {
    if exe.starts_with(curl_bin_dir) {
        return InstallMethod::Curl;
    }
    let parts: Vec<&str> = exe.iter().filter_map(|part| part.to_str()).collect();
    if !parts.contains(&"node_modules") {
        return InstallMethod::Other;
    }
    if parts.contains(&".bun") {
        InstallMethod::Bun
    } else {
        InstallMethod::Npm
    }
}

/// The update command for an install method; `None` when there is no known one.
pub fn update_command(method: InstallMethod) -> Option<&'static str> {
    match method {
        InstallMethod::Curl => Some(CURL_UPDATE_COMMAND),
        InstallMethod::Npm => Some(NPM_UPDATE_COMMAND),
        InstallMethod::Bun => Some(BUN_UPDATE_COMMAND),
        InstallMethod::Other => None,
    }
}

/// `bin` under the curl installer's root: `HOOCODE_INSTALL_DIR` or `~/.hoocode`, as
/// `scripts/install.sh` reads it.
pub fn curl_bin_dir() -> PathBuf {
    let root = hoocode_code_paths::env_override("INSTALL_DIR")
        .map(|dir| hoocode_code_paths::expand_tilde_path(&dir))
        .unwrap_or_else(|| {
            hoocode_code_paths::home_dir().join(hoocode_code_paths::CONFIG_DIR_NAME)
        });
    root.join("bin")
}

/// [`install_method`] after resolving symlinks on both sides. Some platforms report
/// the path the binary was launched by (npm's and bun's bin links), not its real
/// location under `node_modules`. A path that cannot be resolved is used as it is.
pub fn install_method_resolved(exe: &Path, curl_bin_dir: &Path) -> InstallMethod {
    let resolve = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    install_method(&resolve(exe), &resolve(curl_bin_dir))
}

/// The update command for the running binary, if its install method has one.
pub fn current_update_command() -> Option<&'static str> {
    let exe = std::env::current_exe().ok()?;
    update_command(install_method_resolved(&exe, &curl_bin_dir()))
}

/// A newer release than the running one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    pub running: String,
    pub latest: String,
    pub command: Option<&'static str>,
}

/// The notice for `latest` when it is newer than `running`.
pub fn available_update(
    latest: Option<&str>,
    running: &str,
    command: Option<&'static str>,
) -> Option<UpdateAvailable> {
    let latest = latest?;
    is_newer(latest, running).then(|| UpdateAvailable {
        running: running.to_string(),
        latest: latest.to_string(),
        command,
    })
}

/// Title and body rows of the startup notice.
pub fn notice_text(update: &UpdateAvailable) -> (String, Vec<String>) {
    let title = format!(
        "hoocode {} is available (you have {})",
        update.latest, update.running
    );
    let body = match update.command {
        Some(command) => vec![format!("Update: {command}")],
        None => vec!["Update hoocode with the tool you installed it with.".to_string()],
    };
    (title, body)
}

/// The output of `hoocode update`.
pub fn update_message(command: Option<&str>) -> String {
    match command {
        Some(command) => format!("To update hoocode, run:\n  {command}\n"),
        None => "hoocode cannot tell how it was installed, so it has no update command.\n\
                 Update it with the tool you installed it with.\n"
            .to_string(),
    }
}

/// `hoocode update`: prints the command and exits 0. Nothing is downloaded or replaced.
pub fn run_update_command(out: &mut dyn Write) -> i32 {
    let _ = out.write_all(update_message(current_update_command()).as_bytes());
    0
}

/// `<agent dir>/version-check.json`.
pub fn state_path() -> PathBuf {
    hoocode_code_paths::agent_dir().join(STATE_FILE)
}

/// The state at `path`; a missing or unreadable file is an empty state.
pub fn read_state(path: &Path) -> CheckState {
    std::fs::read_to_string(path)
        .map(|text| parse_state(&text))
        .unwrap_or_default()
}

/// Writes the state at `path`. Failures are ignored: the check is best effort.
pub fn write_state(path: &Path, state: &CheckState) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, render_state(state));
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The registry's latest version, or `None` on any error or timeout.
async fn fetch_latest() -> Option<String> {
    let client = hoocode_ai_util::tls::http_client_builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .ok()?;
    let response = client.get(REGISTRY_LATEST_URL).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_registry_latest(&response.text().await.ok()?)
}

/// Starts the startup check on `handle` when `enabled` (see [`check_enabled`]).
/// Returns the receiver the TUI polls; it gets one message only when a newer release
/// is known. Nothing blocks: the request runs as a task on `handle`.
pub fn start_startup_check(
    handle: &tokio::runtime::Handle,
    running: &str,
    enabled: bool,
) -> Option<Receiver<UpdateAvailable>> {
    if !enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let running = running.to_string();
    handle.spawn(async move {
        let path = state_path();
        let mut state = read_state(&path);
        let now = now_secs();
        if check_due(state.checked_at, now) {
            // Recorded before the request, so a slow or failed check still counts for today.
            state.checked_at = Some(now);
            write_state(&path, &state);
            if let Some(latest) = fetch_latest().await {
                state.latest = Some(latest);
                write_state(&path, &state);
            }
        }
        let update = available_update(state.latest.as_deref(), &running, current_update_command());
        if let Some(update) = update {
            let _ = tx.send(update);
        }
    });
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_prefixed_and_suffixed_versions() {
        assert_eq!(parse_version("0.1.17"), Some([0, 1, 17]));
        assert_eq!(parse_version(" v1.2.3\n"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2.3-beta.1"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2.3+build7"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn compares_numerically_not_as_text() {
        assert!(is_newer("0.1.18", "0.1.17"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.17", "0.1.17"));
        assert!(!is_newer("0.1.16", "0.1.17"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("garbage", "0.1.17"));
        assert!(!is_newer("0.1.18", "garbage"));
    }

    #[test]
    fn a_release_is_newer_than_its_own_prerelease() {
        assert!(is_newer("1.0.0", "1.0.0-beta.1"), "running a pre-release");
        assert!(is_newer("v1.0.0", "1.0.0-rc.2"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0"), "a pre-release is older");
        assert!(
            !is_newer("1.0.0", "1.0.0+build.5"),
            "build metadata is not a pre-release"
        );
        assert!(is_newer("1.0.1-rc.1", "1.0.0"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_npm_bin_resolves_to_its_package() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir
            .path()
            .join("lib/node_modules/@kolisachint/hoocode-linux-x64/bin");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("hoocode"), "").unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(real.join("hoocode"), bin.join("hoocode")).unwrap();
        let curl = dir.path().join(".hoocode/bin");

        assert_eq!(
            install_method(&bin.join("hoocode"), &curl),
            InstallMethod::Other,
            "the unresolved link is not under node_modules"
        );
        assert_eq!(
            install_method_resolved(&bin.join("hoocode"), &curl),
            InstallMethod::Npm
        );
        assert_eq!(
            install_method_resolved(Path::new("/no/such/hoocode"), &curl),
            InstallMethod::Other,
            "an unresolvable path falls back to the plain check"
        );
    }

    #[test]
    fn gate_needs_interactive_online_and_setting() {
        assert!(check_enabled(true, false, true));
        assert!(!check_enabled(false, false, true), "print, json and rpc");
        assert!(!check_enabled(true, true, true), "offline");
        assert!(!check_enabled(true, false, false), "checkForUpdates: false");
    }

    #[test]
    fn check_is_due_once_a_day() {
        let now = 1_000_000;
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - 60), now));
        assert!(!check_due(Some(now - CHECK_INTERVAL_SECS + 1), now));
        assert!(check_due(Some(now - CHECK_INTERVAL_SECS), now));
        assert!(
            check_due(Some(now + 5_000), now),
            "a future time counts as due"
        );
    }

    #[test]
    fn state_round_trips_and_tolerates_garbage() {
        let state = CheckState {
            checked_at: Some(42),
            latest: Some("0.1.18".into()),
        };
        assert_eq!(parse_state(&render_state(&state)), state);
        assert_eq!(parse_state("not json"), CheckState::default());
        assert_eq!(parse_state("{}"), CheckState::default());
    }

    #[test]
    fn registry_document_gives_its_version() {
        assert_eq!(
            parse_registry_latest(r#"{"name":"@kolisachint/hoocode","version":"0.1.18"}"#),
            Some("0.1.18".into())
        );
        assert_eq!(parse_registry_latest(r#"{"version":"soon"}"#), None);
        assert_eq!(parse_registry_latest(r#"{"error":"Not found"}"#), None);
        assert_eq!(parse_registry_latest("<html>"), None);
    }

    #[test]
    fn install_method_comes_from_the_path() {
        let curl = Path::new("/home/u/.hoocode/bin");
        assert_eq!(
            install_method(Path::new("/home/u/.hoocode/bin/hoocode"), curl),
            InstallMethod::Curl
        );
        assert_eq!(
            install_method(
                Path::new("/usr/lib/node_modules/@kolisachint/hoocode-linux-x64/bin/hoocode"),
                curl
            ),
            InstallMethod::Npm
        );
        assert_eq!(
            install_method(
                Path::new(
                    "/home/u/.bun/install/global/node_modules/@kolisachint/hoocode/bin/hoocode"
                ),
                curl
            ),
            InstallMethod::Bun
        );
        assert_eq!(
            install_method(Path::new("/usr/local/bin/hoocode"), curl),
            InstallMethod::Other
        );
    }

    #[test]
    fn each_install_method_has_its_command() {
        assert_eq!(
            update_command(InstallMethod::Curl),
            Some(CURL_UPDATE_COMMAND)
        );
        assert_eq!(update_command(InstallMethod::Npm), Some(NPM_UPDATE_COMMAND));
        assert_eq!(update_command(InstallMethod::Bun), Some(BUN_UPDATE_COMMAND));
        assert_eq!(update_command(InstallMethod::Other), None);
        assert!(CURL_UPDATE_COMMAND.ends_with("kolisachint.github.io/hoocode/install.sh | sh"));
    }

    #[test]
    fn update_message_prints_the_command_or_says_none_is_known() {
        assert_eq!(
            update_message(Some(NPM_UPDATE_COMMAND)),
            format!("To update hoocode, run:\n  {NPM_UPDATE_COMMAND}\n")
        );
        assert!(update_message(None).contains("tool you installed it with"));
    }

    #[test]
    fn notice_only_for_a_newer_release() {
        let cmd = Some(CURL_UPDATE_COMMAND);
        assert_eq!(
            available_update(Some("0.1.18"), "0.1.17", cmd),
            Some(UpdateAvailable {
                running: "0.1.17".into(),
                latest: "0.1.18".into(),
                command: cmd,
            })
        );
        assert_eq!(available_update(Some("0.1.17"), "0.1.17", cmd), None);
        assert_eq!(available_update(None, "0.1.17", cmd), None);
    }

    #[test]
    fn notice_text_names_versions_and_command() {
        let update = UpdateAvailable {
            running: "0.1.17".into(),
            latest: "0.1.18".into(),
            command: Some(NPM_UPDATE_COMMAND),
        };
        let (title, body) = notice_text(&update);
        assert_eq!(title, "hoocode 0.1.18 is available (you have 0.1.17)");
        assert_eq!(body, vec![format!("Update: {NPM_UPDATE_COMMAND}")]);
        let (_, body) = notice_text(&UpdateAvailable {
            command: None,
            ..update
        });
        assert_eq!(body.len(), 1);
        assert!(body[0].contains("tool you installed it with"));
    }

    #[test]
    fn state_file_is_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(STATE_FILE);
        assert_eq!(read_state(&path), CheckState::default());
        let state = CheckState {
            checked_at: Some(7),
            latest: None,
        };
        write_state(&path, &state);
        assert_eq!(read_state(&path), state);
    }
}
