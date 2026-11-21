//! Executable resolution and downloads: overrides, file names, checksums and
//! the platform-binary install. The download tests serve bytes from a local
//! HTTP server (plain HTTP: https-only applies to manifests, and the client is
//! injected for tests).

use std::collections::HashMap;

use hoocode_code_native::{
    binary_path_override, binary_path_overrides, ensure_plugin_binary, file_name_from_url,
    plugin_bin_dir, resolve_executable, sha256_hex, socket_path_for, validate_path_component,
    verify_sha256, NativeError,
};
use hoocode_code_plugins::native::current_platform_key;
use serde_json::json;

use crate::common::{download_entry, TestHttpServer};

const HELLO: &[u8] = b"fake helper binary";

#[test]
fn absolute_override_wins() {
    let settings = json!({"nativeHelpers": {"hoo": {"binaryPath": "/opt/hoo"}}});
    assert_eq!(
        binary_path_override(&settings, "hoo"),
        Some(std::path::PathBuf::from("/opt/hoo"))
    );
}

#[test]
fn relative_override_is_ignored_with_a_warning() {
    let settings = json!({"nativeHelpers": {"hoo": {"binaryPath": "bin/hoo"}}});
    assert_eq!(binary_path_override(&settings, "hoo"), None);
    let (overrides, warnings) = binary_path_overrides(&settings);
    assert!(overrides.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("hoo"));
}

#[test]
fn file_name_rejects_empty_and_dot_segments() {
    assert_eq!(
        file_name_from_url("https://example.com/bin/hoo-browser").unwrap(),
        "hoo-browser"
    );
    assert_eq!(
        file_name_from_url("https://example.com/bin/hoo?sig=1").unwrap(),
        "hoo"
    );
    assert!(file_name_from_url("https://example.com/").is_err());
    assert!(file_name_from_url("https://example.com/../").is_err());
}

#[test]
fn sha256_verifies_case_insensitively() {
    let hex = sha256_hex(HELLO);
    assert_eq!(hex.len(), 64);
    verify_sha256("hoo", HELLO, &hex).expect("matches");
    verify_sha256("hoo", HELLO, &hex.to_ascii_uppercase()).expect("case-insensitive");
    let error = verify_sha256("hoo", b"other", &hex).expect_err("mismatch");
    assert!(
        matches!(error, hoocode_code_native::NativeError::Checksum { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn matching_download_installs_once() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let server = TestHttpServer::serve(HELLO.to_vec()).await;
    let url = server.url("/hoo-browser");
    let entry = download_entry(dir.path(), "hoo", &url, &sha256_hex(HELLO), platform);
    let client = reqwest::Client::new();
    let first = ensure_plugin_binary(&home, &entry, &client)
        .await
        .expect("installs")
        .expect("this platform");
    let version = entry.version.clone().unwrap();
    assert_eq!(
        first,
        plugin_bin_dir(&home, "hoo", &version).join("hoo-browser")
    );
    assert_eq!(std::fs::read(&first).expect("read"), HELLO);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&first)
                .expect("stat")
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
    }
    // A present file with a matching sha256 is reused, not re-fetched.
    let second = ensure_plugin_binary(&home, &entry, &client)
        .await
        .expect("reuses")
        .expect("this platform");
    assert_eq!(second, first);
}

#[tokio::test]
async fn sha256_mismatch_refuses_to_install() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let server = TestHttpServer::serve(b"tampered".to_vec()).await;
    let url = server.url("/hoo-browser");
    let entry = download_entry(dir.path(), "hoo", &url, &sha256_hex(HELLO), platform);
    let client = reqwest::Client::new();
    let error = ensure_plugin_binary(&home, &entry, &client)
        .await
        .expect_err("mismatch");
    assert!(
        matches!(error, hoocode_code_native::NativeError::Checksum { .. }),
        "{error:?}"
    );
    let version = entry.version.clone().unwrap();
    assert!(
        !plugin_bin_dir(&home, "hoo", &version)
            .join("hoo-browser")
            .exists(),
        "a bad file is never installed"
    );
}

#[tokio::test]
async fn archives_are_out_of_scope() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let server = TestHttpServer::serve(HELLO.to_vec()).await;
    let url = server.url("/hoo-browser.tar.gz");
    let entry = download_entry(dir.path(), "hoo", &url, &sha256_hex(HELLO), platform);
    let client = reqwest::Client::new();
    let error = ensure_plugin_binary(&home, &entry, &client)
        .await
        .expect_err("archive");
    assert!(
        matches!(error, hoocode_code_native::NativeError::Archive(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn no_platform_binary_falls_back_to_the_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    // No binaries map at all: resolution uses the mcp.json command.
    let entry =
        crate::common::echo_entry(dir.path(), "hoo", std::path::Path::new("/opt/hoo-browser"));
    let client = reqwest::Client::new();
    let path = resolve_executable(&home, &entry, &HashMap::new(), &client)
        .await
        .expect("command");
    assert_eq!(path, std::path::PathBuf::from("/opt/hoo-browser"));
}

#[tokio::test]
async fn in_plugin_relative_command_resolves_under_the_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let mut entry = crate::common::echo_entry(dir.path(), "hoo", std::path::Path::new("./bin/hoo"));
    entry.root = dir.path().join("plugin-root");
    let client = reqwest::Client::new();
    let path = resolve_executable(&home, &entry, &HashMap::new(), &client)
        .await
        .expect("relative command");
    assert_eq!(path, dir.path().join("plugin-root").join("bin/hoo"));
}

/// Unsafe plugin names never become a path: they would escape the folders
/// tier C builds from socket, log and plugin-bin paths.
#[test]
fn unsafe_plugin_names_are_rejected() {
    for bad in [
        "",
        ".",
        "..",
        "../evil",
        "a/b",
        "a\\b",
        "a..b",
        "evil\0name",
        "/absolute",
    ] {
        assert!(
            validate_path_component("plugin", bad).is_err(),
            "{bad:?} must be rejected"
        );
    }
    for good in ["hoo-browser", "v1", "a.b", "plugin_2", "UPPER"] {
        assert!(
            validate_path_component("plugin", good).is_ok(),
            "{good:?} must be accepted"
        );
    }
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(
        socket_path_for(dir.path(), "../evil").is_err(),
        "the socket path validates the plugin"
    );
    assert!(
        socket_path_for(dir.path(), "hoo").is_ok(),
        "a safe plugin passes"
    );
}

/// An unsafe manifest version never becomes a plugin-bin path component.
#[tokio::test]
async fn unsafe_versions_never_become_a_path() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    for version in ["../evil", "..", "", "a/b", "a\\b"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let server = TestHttpServer::serve(HELLO.to_vec()).await;
        let url = server.url("/hoo-browser");
        let mut entry = download_entry(dir.path(), "hoo", &url, &sha256_hex(HELLO), platform);
        entry.version = Some(version.to_owned());
        let client = reqwest::Client::new();
        let error = ensure_plugin_binary(&home, &entry, &client)
            .await
            .expect_err("unsafe version");
        assert!(
            matches!(error, NativeError::InvalidName(_)),
            "{version:?}: {error:?}"
        );
    }
}

/// An unsafe plugin name never becomes a plugin-bin path component.
#[tokio::test]
async fn unsafe_plugin_names_never_become_a_path() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let server = TestHttpServer::serve(HELLO.to_vec()).await;
    let url = server.url("/hoo-browser");
    let mut entry = download_entry(dir.path(), "hoo", &url, &sha256_hex(HELLO), platform);
    entry.plugin = "../evil".to_owned();
    let client = reqwest::Client::new();
    let error = ensure_plugin_binary(&home, &entry, &client)
        .await
        .expect_err("unsafe plugin");
    assert!(matches!(error, NativeError::InvalidName(_)), "{error:?}");
}

/// Parallel installers of the same binary agree on one file and leak no temp
/// files: each uses a unique temp name, and a concurrent winner is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_installs_agree_on_one_binary() {
    let Some(platform) = current_platform_key() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let server = TestHttpServer::serve([0xABu8; 64].to_vec()).await;
    let url = server.url("/hoo-browser");
    let entry = download_entry(
        dir.path(),
        "hoo",
        &url,
        &sha256_hex(&[0xABu8; 64]),
        platform,
    );
    let client = reqwest::Client::new();
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let task_home = home.clone();
        let task_entry = entry.clone();
        let task_client = client.clone();
        set.spawn(async move { ensure_plugin_binary(&task_home, &task_entry, &task_client).await });
    }
    let mut dest = None;
    while let Some(joined) = set.join_next().await {
        let path = joined
            .expect("task")
            .expect("install")
            .expect("binary for this platform");
        if let Some(first) = &dest {
            assert_eq!(first, &path);
        } else {
            dest = Some(path);
        }
    }
    let dest = dest.expect("an install");
    assert_eq!(std::fs::read(&dest).expect("read"), [0xABu8; 64]);
    let leftovers: Vec<std::path::PathBuf> = std::fs::read_dir(dest.parent().expect("parent"))
        .expect("read dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path != &dest)
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
