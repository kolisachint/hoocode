use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_ai_types::AbortSignal;
use hoocode_code_wasm::{CallEnv, Extension, ExtensionSpec, NetGate, WasmError, WasmHost};
use hoocode_runtime::block_on_entry;
use serde_json::json;

/// The echo fixture, built by scripts/dev/build_wasm_fixtures.sh.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo-ext.wasm")
}

struct DenyAll;

impl NetGate for DenyAll {
    fn check(&self, _url: &str) -> Result<(), String> {
        Err("network is closed in tests".to_owned())
    }
}

fn env_with(config: &[(&str, &str)], abort: Option<AbortSignal>) -> CallEnv {
    let map: HashMap<String, String> = config
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    CallEnv {
        config: Arc::new(move |key: &str| map.get(key).cloned()),
        net: Arc::new(DenyAll),
        abort,
    }
}

fn spec(dir: &Path) -> ExtensionSpec {
    ExtensionSpec {
        plugin: "echo".to_owned(),
        wasm_path: fixture(),
        data_dir: dir.join("data"),
    }
}

fn host(dir: &Path) -> WasmHost {
    WasmHost::new(dir.join("cache")).expect("host")
}

fn load(dir: &Path) -> Extension {
    host(dir).load(spec(dir)).expect("load fixture")
}

fn cwasm_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("cache"))
        .expect("cache dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "cwasm"))
        .collect();
    files.sort();
    files
}

#[test]
fn loads_and_lists_tools() {
    let dir = tempfile::tempdir().unwrap();
    let ext = load(dir.path());
    assert_eq!(ext.plugin(), "echo");
    let names: Vec<&str> = ext.tools().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "echo",
            "config",
            "spin",
            "alloc",
            "fail",
            "write-data",
            "fetch",
            "chatter"
        ]
    );
    let echo = &ext.tools()[0];
    assert!(echo.input_schema.is_object());
    assert!(!echo.description.is_empty());
}

#[test]
fn echo_round_trip() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let args = json!({"a": 1, "b": ["x"]});
        let out = ext
            .call("echo", args.clone(), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, args);
    });
}

#[test]
fn config_get_reads_plugin_setting() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let env = env_with(&[("api_key", "secret-1")], None);
        let out = ext
            .call("config", json!({"key": "api_key"}), env)
            .await
            .unwrap();
        assert_eq!(out, json!({"value": "secret-1"}));

        let out = ext
            .call("config", json!({"key": "unset"}), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, json!({"value": null}));
    });
}

#[test]
fn bad_args_are_invalid_args() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let err = ext
            .call("config", json!({}), env_with(&[], None))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::InvalidArgs(_)), "{err:?}");
    });
}

#[test]
fn fail_maps_to_failed() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let err = ext
            .call("fail", json!({}), env_with(&[], None))
            .await
            .unwrap_err();
        match err {
            WasmError::Failed(msg) => assert!(msg.contains("fails on purpose"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    });
}

#[test]
fn unknown_tool_is_refused_before_the_call() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let err = ext
            .call("nope", json!({}), env_with(&[], None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, WasmError::UnknownTool(ref n) if n == "nope"),
            "{err:?}"
        );
    });
}

/// Guest stdout and debug-level logs stay off stderr unless debugging is on;
/// guest stderr (a panic message, say) always reaches the host log.
#[test]
fn guest_stdout_and_debug_stay_off_stderr_but_guest_stderr_shows() {
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = captured.clone();
    hoocode_runtime::diagnostics::set_stderr_sink(Some(Box::new(move |text| {
        sink.lock().unwrap().push_str(text);
    })));
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        ext.call("chatter", json!({}), env_with(&[], None))
            .await
            .unwrap();
    });
    hoocode_runtime::diagnostics::set_stderr_sink(None);
    let seen = captured.lock().unwrap().clone();
    assert!(!seen.contains("CHATTER-STDOUT"), "{seen}");
    assert!(!seen.contains("CHATTER-DEBUG"), "{seen}");
    assert!(seen.contains("CHATTER-STDERR"), "{seen}");
}

#[test]
fn spin_times_out() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path()).with_call_timeout(Duration::from_secs(1));
        let ext = host.load(spec(dir.path())).expect("load");
        let started = Instant::now();
        let err = ext
            .call("spin", json!({}), env_with(&[], None))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Timeout), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    });
}

#[test]
fn abort_cancels_a_running_call() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let signal = AbortSignal::new();
        let trigger = signal.clone();
        // The guest never yields, so the abort comes from another thread.
        hoocode_runtime::spawn_thread("abort-trigger", move || {
            std::thread::sleep(Duration::from_millis(200));
            trigger.abort();
        });
        let started = Instant::now();
        let err = ext
            .call("spin", json!({}), env_with(&[], Some(signal)))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Cancelled), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
    });
}

#[test]
fn memory_limit_stops_a_big_allocation() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let err = ext
            .call("alloc", json!({}), env_with(&[], None))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Trap(_)), "{err:?}");
    });
}

#[test]
fn write_data_lands_in_data_dir() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path());
        let out = ext
            .call("write-data", json!({}), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, json!({"read": "hello from the fixture"}));
        let on_disk = std::fs::read_to_string(dir.path().join("data/x.txt")).unwrap();
        assert_eq!(on_disk, "hello from the fixture");
    });
}

#[test]
fn second_load_reuses_the_cached_component() {
    let dir = tempfile::tempdir().unwrap();
    let _first = load(dir.path());
    let files = cwasm_files(dir.path());
    assert_eq!(files.len(), 1, "one precompiled entry: {files:?}");
    let before = std::fs::metadata(&files[0]).unwrap().modified().unwrap();

    let _second = load(dir.path());
    let after_files = cwasm_files(dir.path());
    assert_eq!(after_files, files);
    let after = std::fs::metadata(&files[0]).unwrap().modified().unwrap();
    assert_eq!(before, after, "the cached file was rewritten");
}

#[test]
fn corrupt_cache_entry_is_recompiled() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let _first = load(dir.path());
        let file = cwasm_files(dir.path()).remove(0);
        let good = std::fs::read(&file).unwrap();
        // Replace the file rather than truncating it: the first component still maps it.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, b"not a component").unwrap();

        let ext = load(dir.path());
        let out = ext
            .call("echo", json!({"ok": true}), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, json!({"ok": true}));
        let repaired = std::fs::read(&file).unwrap();
        assert_eq!(repaired, good, "the entry was recompiled and written back");
    });
}

#[cfg(unix)]
#[test]
fn cache_dir_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let _host = host(dir.path());
    let mode = std::fs::metadata(dir.path().join("cache"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[cfg(unix)]
#[test]
fn a_world_writable_cache_dir_is_not_used() {
    use std::os::unix::fs::PermissionsExt;
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o777)).unwrap();

        let ext = load(dir.path());
        assert!(
            cwasm_files(dir.path()).is_empty(),
            "nothing is cached in an open folder"
        );
        let out = ext
            .call("echo", json!({"ok": true}), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, json!({"ok": true}));
    });
}

#[test]
fn a_failed_cache_write_still_loads_and_leaves_no_temp_file() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let _first = load(dir.path());
        let file = cwasm_files(dir.path()).remove(0);
        std::fs::remove_file(&file).unwrap();
        // The rename onto the entry's name now fails: a directory is in the way.
        std::fs::create_dir(&file).unwrap();

        let ext = load(dir.path());
        let out = ext
            .call("echo", json!({"ok": true}), env_with(&[], None))
            .await
            .unwrap();
        assert_eq!(out, json!({"ok": true}));
        let leftovers: Vec<String> = std::fs::read_dir(dir.path().join("cache"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    });
}
