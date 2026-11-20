//! Shared test support: building the echo fixture, helper entries, a tiny HTTP
//! server for download tests, and reading echo results.
//!
//! The echo fixture is the `echo_helper` example of `hoocode-agent-mcp`. A
//! binary newer than the fixture source is reused as is; otherwise it is
//! built once with `cargo build -p hoocode-agent-mcp --example echo_helper`
//! (one builder at a time across test processes) and lands at
//! `<target>/debug/examples/echo_helper` (`$CARGO_TARGET_DIR` wins when
//! set). Later end-to-end tests in `hoocode-code-main` use the same recipe.
//!
//! Every test that starts a helper awaits `supervisor.shutdown()` at the
//! end, while its runtime still drives the rmcp transport tasks. `Drop`
//! only kills what is left synchronously and never blocks.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use hoocode_code_native::{Supervisor, SupervisorConfig};
use hoocode_code_plugins::native::{NativeBinary, NativeHelper, NativeLaunch, NativeSpec};
use hoocode_code_plugins::NativeHelperEntry;
use serde_json::Value;

/// A lock dir serializing the fixture build across test processes: parallel
/// `cargo build` invocations serialize on cargo's own package-cache lock and
/// slow every test down, so only one test process builds at a time.
fn build_lock_dir(target: &Path) -> PathBuf {
    target
        .join("debug")
        .join("examples")
        .join(".echo-helper-build.lock")
}

/// True when `bin` exists and is newer than the fixture source: no rebuild.
fn bin_is_fresh(bin: &Path, workspace: &Path) -> bool {
    let source = workspace
        .join("crates")
        .join("hoocode-agent-mcp")
        .join("examples")
        .join("echo_helper.rs");
    let (Ok(bin_meta), Ok(src_meta)) = (std::fs::metadata(bin), std::fs::metadata(&source)) else {
        return false;
    };
    let (Ok(bin_time), Ok(src_time)) = (bin_meta.modified(), src_meta.modified()) else {
        return false;
    };
    bin_time >= src_time
}

/// The built echo fixture binary, built once per test process.
///
/// A fresh binary (newer than the fixture source) is reused as is: rebuilding
/// from every test process serializes all tests on cargo's file locks.
/// Otherwise one process builds while the rest wait on a lock dir.
pub fn fixture_bin() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let workspace = manifest
                .parent()
                .and_then(|dir| dir.parent())
                .expect("crate lives in <workspace>/crates/<name>")
                .to_path_buf();
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace.join("target"));
            let bin = target
                .join("debug")
                .join("examples")
                .join(format!("echo_helper{}", std::env::consts::EXE_SUFFIX));
            if !bin_is_fresh(&bin, &workspace) {
                let lock = build_lock_dir(&target);
                // An atomic claim: exactly one process builds, the rest wait.
                // A stale lock (a crashed builder) is reaped after a minute.
                let mut waited = 0;
                while std::fs::create_dir(&lock).is_err() {
                    if waited > 60 {
                        let _ = std::fs::remove_dir_all(&lock);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    waited += 1;
                    if bin_is_fresh(&bin, &workspace) {
                        break;
                    }
                }
                if !bin_is_fresh(&bin, &workspace) {
                    let status = std::process::Command::new("cargo")
                        .arg("build")
                        .arg("-p")
                        .arg("hoocode-agent-mcp")
                        .arg("--example")
                        .arg("echo_helper")
                        .current_dir(&workspace)
                        .status()
                        .expect("cargo builds the echo fixture");
                    assert!(status.success(), "cargo build --example echo_helper");
                }
                let _ = std::fs::remove_dir_all(&lock);
            }
            assert!(
                bin.is_file(),
                "the echo fixture was built: {}",
                bin.display()
            );
            bin
        })
        .clone()
}

/// One `NativeHelperEntry` for the echo fixture: `plugin` starts
/// `exe` with no args or env.
pub fn echo_entry(dir: &Path, plugin: &str, exe: &Path) -> NativeHelperEntry {
    NativeHelperEntry {
        plugin: plugin.to_owned(),
        version: Some("test".to_owned()),
        root: dir.to_path_buf(),
        data_dir: dir.to_path_buf(),
        helper: NativeHelper {
            spec: NativeSpec {
                server: "echo".to_owned(),
                shared: true,
                binaries: BTreeMap::new(),
                tools: Vec::new(),
            },
            launch: NativeLaunch {
                command: exe.to_string_lossy().into_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: None,
            },
        },
    }
}

/// A helper entry that starts `/bin/sleep 30`: spawns fine, never serves.
pub fn sleep_entry(dir: &Path) -> NativeHelperEntry {
    let mut entry = echo_entry(dir, "sleep", Path::new("/bin/sleep"));
    entry.helper.launch.args = vec!["30".to_owned()];
    entry
}

/// Build a supervisor for `plugin` from parts, for tests that do not need the
/// full [`TestHelper`] (receipt dir, overrides).
pub fn start_supervisor(dir: &Path, config: SupervisorConfig) -> Supervisor {
    let entry = echo_entry(dir, "echo", &fixture_bin());
    Supervisor::new(dir.join("home"), vec![entry], HashMap::new(), config).expect("supervisor")
}

/// A supervisor under test: temp home, echo helper, receipt dir for the
/// `sessionEnding` marker. Shut down at the end of each test.
pub struct TestHelper {
    dir: tempfile::TempDir,
    supervisor: Supervisor,
    receipt_dir: PathBuf,
}

impl TestHelper {
    /// Start with the fast test config.
    pub fn start() -> Self {
        Self::start_with(SupervisorConfig::for_tests())
    }

    pub fn start_with(config: SupervisorConfig) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let receipt_dir = dir.path().join("receipts");
        std::fs::create_dir_all(&receipt_dir).expect("receipt dir");
        let mut entry = echo_entry(dir.path(), "echo", &fixture_bin());
        entry.helper.launch.env.insert(
            "HOOCODE_ECHO_RECEIPT_DIR".to_owned(),
            receipt_dir.to_string_lossy().into_owned(),
        );
        let supervisor =
            Supervisor::new(dir.path().join("home"), vec![entry], HashMap::new(), config)
                .expect("supervisor");
        Self {
            dir,
            supervisor,
            receipt_dir,
        }
    }

    /// An echo entry rooted at this helper's dir (for extra supervisors).
    pub fn entry(dir: &Path, exe: &Path) -> NativeHelperEntry {
        echo_entry(dir, "echo", exe)
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn receipt_dir(&self) -> &Path {
        &self.receipt_dir
    }
}

impl Drop for TestHelper {
    fn drop(&mut self) {
        // Synchronous only: dropping the supervisor kills the helper
        // children at once (kill_on_drop) without waiting. Graceful
        // shutdown is explicit: tests await `supervisor.shutdown()` while
        // their runtime still drives the rmcp transport tasks. Blocking
        // here on another runtime would wedge the notification send on the
        // call timeout.
    }
}

/// The parsed JSON of an echo result's text content.
pub fn echo_text(output: &hoocode_agent_mcp::ToolOutput) -> Value {
    assert!(!output.is_error);
    let [hoocode_agent_mcp::ToolContent::Text(text), ..] = &output.content[..] else {
        panic!("echo answers one text block: {:?}", output.content);
    };
    serde_json::from_str(text).expect("echo answers JSON")
}

/// One download binary entry for `plugin`, served from `url` with `sha256`.
pub fn download_entry(
    dir: &Path,
    plugin: &str,
    url: &str,
    sha256: &str,
    platform: &str,
) -> NativeHelperEntry {
    let mut entry = echo_entry(dir, plugin, Path::new("/nonexistent"));
    entry.helper.spec.binaries = BTreeMap::from([(
        platform.to_owned(),
        NativeBinary {
            url: url.to_owned(),
            sha256: sha256.to_owned(),
        },
    )]);
    entry
}

/// A tiny HTTP server that answers every request with `bytes` (200).
/// For download tests: plain HTTP, no TLS.
pub struct TestHttpServer {
    port: u16,
    _task: tokio::task::JoinHandle<()>,
}

impl TestHttpServer {
    pub async fn serve(bytes: Vec<u8>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("port").port();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let bytes = bytes.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut head = vec![0u8; 8192];
                    let _ = stream.read(&mut head).await;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&bytes).await;
                });
            }
        });
        Self { port, _task: task }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}
