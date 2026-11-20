//! Tier C native helpers: manifest parsing and validation (PL-18 slice 2, parsing only).
//!
//! A `browser` fixture plugin lives in `tests/fixtures/native/valid`; the failure cases mutate
//! its manifest in a temp user-plugins folder, so the shared fixtures stay untouched.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hoocode_code_mcp::TrustStore;
use hoocode_code_plugins::native::{current_platform_key, NativePermission, NATIVE_PLATFORMS};
use hoocode_code_plugins::{load_plugins, LoadOptions, PluginDiagnostic, PluginSet, Severity};
use serde_json::{json, Value};

use crate::common::{copy_dir, fixtures};

struct Sandbox {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    user_plugins: PathBuf,
    data_root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let sandbox = Self {
            repo: base.join("repo"),
            user_plugins: base.join("plugins"),
            data_root: base.join("plugin-data"),
            _dir: dir,
        };
        std::fs::create_dir_all(&sandbox.repo).unwrap();
        std::fs::create_dir_all(&sandbox.user_plugins).unwrap();
        sandbox
    }

    /// Copy the valid `browser` fixture into the user plugins folder.
    fn install_fixture(&self) -> PathBuf {
        let to = self.user_plugins.join("browser");
        copy_dir(&fixtures().join("native/valid"), &to);
        to
    }

    /// Write `browser` from a manifest value and the fixture `mcp.json` plus skills.
    fn write_plugin(&self, manifest: &Value) -> PathBuf {
        let root = self.install_fixture();
        std::fs::write(
            root.join("plugin.json"),
            serde_json::to_string_pretty(manifest).unwrap(),
        )
        .unwrap();
        root
    }

    fn load(&self) -> PluginSet {
        let trust = TrustStore::empty(self.data_root.join("trust.json"));
        load_plugins(&LoadOptions {
            project_folder: self.repo.clone(),
            user_plugins_dir: self.user_plugins.clone(),
            data_root: self.data_root.clone(),
            trust: &trust,
            untrusted_installs: BTreeSet::new(),
            installs_unreadable: None,
        })
    }
}

fn manifest() -> Value {
    let text = std::fs::read_to_string(fixtures().join("native/valid/plugin.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn native(set: &PluginSet) -> &hoocode_code_plugins::native::NativeHelper {
    set.plugins
        .iter()
        .find(|p| p.name == "browser")
        .unwrap_or_else(|| panic!("plugin browser did not load: {:?}", set.diagnostics))
        .native
        .as_ref()
        .expect("native helper should have loaded")
}

fn native_errors(set: &PluginSet) -> Vec<&PluginDiagnostic> {
    set.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error && d.message.contains("native helpers are off"))
        .collect()
}

fn server_names(set: &PluginSet) -> Vec<String> {
    let mut names: Vec<String> = set
        .mcp_servers()
        .iter()
        .flat_map(|group| group.servers.iter().map(|s| s.name.clone()))
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn this_platform_has_a_known_binaries_key() {
    let key = current_platform_key().expect("tier C ships every tested platform");
    assert!(NATIVE_PLATFORMS.contains(&key), "{key}");
}

#[test]
fn a_valid_native_plugin_loads_with_its_launch() {
    let sb = Sandbox::new();
    let root = sb.install_fixture();
    let set = sb.load();
    assert!(native_errors(&set).is_empty(), "{:?}", set.diagnostics);

    let plugin = set.plugins.iter().find(|p| p.name == "browser").unwrap();
    let helper = plugin.native.as_ref().unwrap();
    assert_eq!(helper.spec.server, "hoo-browser");
    assert!(helper.spec.shared);
    assert_eq!(helper.spec.binaries.len(), 2);
    let linux = &helper.spec.binaries["linux-x86_64"];
    assert_eq!(linux.url, "https://example.com/hoo-browser-linux-x86_64");
    assert_eq!(
        linux.sha256,
        "abababababababababababababababababababababababababababababababab"
    );
    assert_eq!(helper.spec.tools.len(), 2);
    assert_eq!(helper.spec.tools[0].name, "browser_open");
    assert_eq!(helper.spec.tools[0].permission, NativePermission::Read);
    assert_eq!(helper.spec.tools[0].input_schema["type"], "object");
    // Missing permission defaults to act.
    assert_eq!(helper.spec.tools[1].name, "browser_snapshot");
    assert_eq!(helper.spec.tools[1].permission, NativePermission::Act);

    // The launch is the named stdio server, placeholders already expanded.
    let data = sb.data_root.join("browser");
    assert_eq!(
        helper.launch.command,
        root.join("bin/hoo-browser").to_string_lossy()
    );
    assert_eq!(
        helper.launch.args,
        vec!["--serve".to_owned(), format!("{}/state", data.display())]
    );
    assert_eq!(
        helper.launch.env["CONFIG"],
        format!("{}/config.json", root.display())
    );
    assert_eq!(helper.launch.env["PLUGIN_ROOT"], root.to_string_lossy());
    assert_eq!(helper.launch.env["PLUGIN_DATA"], data.to_string_lossy());
    assert_eq!(
        helper.launch.cwd.as_deref(),
        Some(root.to_string_lossy().as_ref())
    );
}

#[test]
fn the_named_server_is_not_a_plain_mcp_server_but_the_others_are() {
    let sb = Sandbox::new();
    sb.install_fixture();
    let set = sb.load();
    assert!(native_errors(&set).is_empty(), "{:?}", set.diagnostics);
    assert_eq!(server_names(&set), ["docs"]);
    // The plugin keeps its other server in its own list.
    let plugin = set.plugins.iter().find(|p| p.name == "browser").unwrap();
    assert_eq!(plugin.mcp_servers.len(), 1);
    assert_eq!(plugin.mcp_servers[0].name, "docs");
}

#[test]
fn native_helpers_lists_the_helper_with_plugin_name_version_and_dirs() {
    let sb = Sandbox::new();
    let root = sb.install_fixture();
    let set = sb.load();
    let helpers = set.native_helpers();
    assert_eq!(helpers.len(), 1);
    assert_eq!(helpers[0].plugin, "browser");
    assert_eq!(helpers[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(helpers[0].root, root);
    assert_eq!(helpers[0].data_dir, sb.data_root.join("browser"));
    assert_eq!(helpers[0].helper.spec.tools.len(), 2);
}

#[test]
fn no_native_block_is_fine_and_gives_no_diagnostics() {
    let sb = Sandbox::new();
    let root = sb.install_fixture();
    let mut manifest = manifest();
    manifest.as_object_mut().unwrap()["extensions"]
        .as_object_mut()
        .unwrap()
        .remove("dev.hoocode");
    std::fs::write(root.join("plugin.json"), manifest.to_string()).unwrap();
    let set = sb.load();
    let plugin = set.plugins.iter().find(|p| p.name == "browser").unwrap();
    assert!(plugin.native.is_none());
    assert!(set.diagnostics.is_empty(), "{:?}", set.diagnostics);
    // Untouched: both MCP servers load as plain servers.
    assert_eq!(server_names(&set), ["docs", "hoo-browser"]);
    assert!(set.native_helpers().is_empty());
}

#[test]
fn unknown_native_fields_are_warnings_and_the_block_still_loads() {
    let sb = Sandbox::new();
    let mut manifest = manifest();
    {
        let native = manifest.as_object_mut().unwrap()["extensions"]
            .as_object_mut()
            .unwrap()["dev.hoocode"]
            .as_object_mut()
            .unwrap()["native"]
            .as_object_mut()
            .unwrap();
        native.insert("future".to_owned(), json!({"x": 1}));
        native["tools"].as_array_mut().unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert("hint".to_owned(), json!("extra"));
        native["binaries"].as_object_mut().unwrap()["linux-x86_64"]
            .as_object_mut()
            .unwrap()
            .insert("size".to_owned(), json!(42));
    }
    sb.write_plugin(&manifest);
    let set = sb.load();
    assert!(native_errors(&set).is_empty(), "{:?}", set.diagnostics);
    assert!(native(&set).spec.tools.len() == 2);
    let warnings: Vec<&PluginDiagnostic> = set
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|d| d.plugin.as_deref() == Some("browser")),
        "{warnings:?}"
    );
}

/// Each mutation turns the native part off with an [`Error`], while the rest of the
/// plugin (skills, plain MCP servers including the named one) still loads.
#[test]
fn every_validation_failure_turns_native_off_and_keeps_the_rest() {
    let tool = || {
        json!({
            "name": "browser_open",
            "description": "Open a URL",
            "inputSchema": {"type": "object"},
            "permission": "read"
        })
    };
    type Mutate = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Mutate)> = vec![
        (
            "native not an object",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"] = json!(["server"]);
            }),
        ),
        (
            "missing server",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]
                    .as_object_mut()
                    .unwrap()
                    .remove("server");
            }),
        ),
        (
            "server not a string",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["server"] = json!(7);
            }),
        ),
        (
            "unknown server",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["server"] = json!("nope");
            }),
        ),
        (
            "server not stdio",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["server"] = json!("docs");
            }),
        ),
        (
            "shared not a boolean",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["shared"] = json!("yes");
            }),
        ),
        (
            "binaries not an object",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"] = json!(["x"]);
            }),
        ),
        (
            "unknown platform",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["ms-dos"] =
                    json!({"url": "https://example.com/x", "sha256": "ab"});
            }),
        ),
        (
            "binary not an object",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["linux-x86_64"] = json!("x");
            }),
        ),
        (
            "binary url not https",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["linux-x86_64"]["url"] =
                    json!("http://example.com/x");
            }),
        ),
        (
            "binary url missing",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["linux-x86_64"]
                    .as_object_mut()
                    .unwrap()
                    .remove("url");
            }),
        ),
        (
            "binary sha256 short",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["linux-x86_64"]["sha256"] =
                    json!("ab");
            }),
        ),
        (
            "binary sha256 not hex",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["binaries"]["linux-x86_64"]["sha256"] =
                    json!(&"xb".repeat(32));
            }),
        ),
        (
            "tools missing",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]
                    .as_object_mut()
                    .unwrap()
                    .remove("tools");
            }),
        ),
        (
            "tools empty",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"] = json!([]);
            }),
        ),
        (
            "tools not an array",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"] = json!({});
            }),
        ),
        (
            "tool not an object",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"] = json!(["browser_open"]);
            }),
        ),
        (
            "tool name bad charset",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["name"] = json!("has space");
            }),
        ),
        (
            "tool name leading digit",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["name"] = json!("1open");
            }),
        ),
        (
            "tool name mcp prefix",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["name"] = json!("MCP__open");
            }),
        ),
        (
            "tool names collide ignoring case",
            Box::new(move |m: &mut Value| {
                let mut dup = tool();
                dup["name"] = json!("BROWSER_OPEN");
                m["extensions"]["dev.hoocode"]["native"]["tools"]
                    .as_array_mut()
                    .unwrap()
                    .push(dup);
            }),
        ),
        (
            "tool missing description",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("description");
            }),
        ),
        (
            "tool inputSchema missing",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("inputSchema");
            }),
        ),
        (
            "tool inputSchema not an object",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["inputSchema"] =
                    json!(["object"]);
            }),
        ),
        (
            "tool inputSchema wrong type",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["inputSchema"] =
                    json!({"type": "array"});
            }),
        ),
        (
            "tool permission unknown",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["permission"] =
                    json!("sometimes");
            }),
        ),
        (
            "tool permission not a string",
            Box::new(|m: &mut Value| {
                m["extensions"]["dev.hoocode"]["native"]["tools"][0]["permission"] = json!(true);
            }),
        ),
    ];
    for (what, mutate) in cases {
        let sb = Sandbox::new();
        let mut manifest = manifest();
        mutate(&mut manifest);
        sb.write_plugin(&manifest);
        let set = sb.load();
        let plugin = set
            .plugins
            .iter()
            .find(|p| p.name == "browser")
            .unwrap_or_else(|| panic!("{what}: plugin did not load: {:?}", set.diagnostics));
        assert!(plugin.native.is_none(), "{what}: native should be off");
        assert!(set.native_helpers().is_empty(), "{what}");
        let errors = native_errors(&set);
        assert!(!errors.is_empty(), "{what}: no native Error diagnostic");
        assert!(
            errors
                .iter()
                .all(|d| d.plugin.as_deref() == Some("browser")),
            "{what}: {errors:?}"
        );
        // The rest of the plugin still loads: its skill and both MCP servers, including the
        // named one, which stays a plain server while native is off.
        assert!(!plugin.skill_dirs.is_empty(), "{what}: skills lost");
        let mut plain: Vec<&str> = plugin.mcp_servers.iter().map(|s| s.name.as_str()).collect();
        plain.sort_unstable();
        assert_eq!(plain, ["docs", "hoo-browser"], "{what}");
    }
}
