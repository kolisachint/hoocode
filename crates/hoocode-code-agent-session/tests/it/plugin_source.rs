#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! PL-4: a session's plugin tools follow its plugin source at once, with no `/reload`. The
//! source stands in for the CLI's (which builds `ext_` tools from the installed plugins).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_agent_session::PluginSource;
use hoocode_code_mcp::PluginServers;
use hoocode_code_tool_api::ToolDefinition;

use crate::common::{tool, Harness, HarnessOptions};

/// A plugin set the test installs and removes; each change moves the generation.
#[derive(Default)]
struct FakePlugins {
    tools: Mutex<Vec<ToolDefinition>>,
    mcp: Mutex<Vec<PluginServers>>,
    generation: AtomicU64,
    builds: AtomicUsize,
    hub_builds: AtomicUsize,
}

impl FakePlugins {
    fn set(&self, tools: Vec<ToolDefinition>) {
        *self.tools.lock().unwrap() = tools;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Changes the plugins' MCP servers (a server added, removed or changed).
    fn set_mcp(&self, servers: Vec<PluginServers>) {
        *self.mcp.lock().unwrap() = servers;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    fn hub_builds(&self) -> usize {
        self.hub_builds.load(Ordering::SeqCst)
    }
}

/// A plugin's MCP server entry; the set compares whole entries, so any change to one counts.
fn plugin_servers(id: &str) -> PluginServers {
    PluginServers {
        id: id.to_owned(),
        path: PathBuf::from(format!("/plugins/{id}")),
        marketplace_trusted: true,
        servers: Vec::new(),
    }
}

impl PluginSource for FakePlugins {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn tools(&self, taken: &[String]) -> (Vec<ToolDefinition>, Vec<String>) {
        self.builds.fetch_add(1, Ordering::SeqCst);
        let mut notices = Vec::new();
        let kept = self
            .tools
            .lock()
            .unwrap()
            .iter()
            .filter(|t| {
                if taken.contains(&t.name) {
                    notices.push(format!("tool {} not loaded: already a tool", t.name));
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        (kept, notices)
    }

    fn mcp_servers(&self) -> Option<Vec<PluginServers>> {
        Some(self.mcp.lock().unwrap().clone())
    }

    fn mcp_hub(&self) -> Option<Arc<McpHub>> {
        self.hub_builds.fetch_add(1, Ordering::SeqCst);
        Some(Arc::new(McpHub::for_folder(&std::env::temp_dir())))
    }
}

fn session() -> Harness {
    Harness::new(HarnessOptions {
        custom_tools: vec![tool("AskUserQuestion", |_| unimplemented!())],
        ..Default::default()
    })
}

fn active(h: &Harness) -> Vec<String> {
    h.session.get_active_tool_names()
}

fn all(h: &Harness) -> Vec<String> {
    h.session
        .get_all_tools()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

fn echo() -> ToolDefinition {
    tool("ext_echo_echo", |_| unimplemented!())
}

#[test]
fn installed_tools_are_active_at_the_next_refresh_and_removed_tools_leave() {
    let h = session();
    let plugins = Arc::new(FakePlugins::default());
    let notices = h.session.attach_plugins(plugins.clone());
    assert!(notices.is_empty(), "{notices:?}");
    assert!(!all(&h).contains(&"ext_echo_echo".to_owned()));

    // The same entry point `/plugin` and the model's plugin tools reach.
    plugins.set(vec![echo()]);
    h.session.refresh_plugins();
    assert!(all(&h).contains(&"ext_echo_echo".to_owned()));
    assert!(active(&h).contains(&"ext_echo_echo".to_owned()));
    // The session's own tools are kept.
    assert!(active(&h).contains(&"AskUserQuestion".to_owned()));

    plugins.set(vec![]);
    h.session.refresh_plugins();
    assert!(!all(&h).contains(&"ext_echo_echo".to_owned()));
    assert!(!active(&h).contains(&"ext_echo_echo".to_owned()));
    assert!(active(&h).contains(&"AskUserQuestion".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_picks_up_the_plugins_too() {
    let h = session();
    let plugins = Arc::new(FakePlugins::default());
    h.session.attach_plugins(plugins.clone());
    plugins.set(vec![echo()]);
    h.session.reload().await;
    assert!(active(&h).contains(&"ext_echo_echo".to_owned()));
}

#[test]
fn an_unchanged_plugin_set_is_not_built_again() {
    let h = session();
    let plugins = Arc::new(FakePlugins::default());
    h.session.attach_plugins(plugins.clone());
    assert_eq!(plugins.builds(), 1);
    h.session.refresh_plugins();
    h.session.refresh_plugins();
    assert_eq!(plugins.builds(), 1);
    plugins.set(vec![echo()]);
    h.session.refresh_plugins();
    assert_eq!(plugins.builds(), 2);
}

#[test]
fn a_plugin_tool_never_takes_a_name_the_session_has() {
    let h = session();
    let plugins = Arc::new(FakePlugins::default());
    h.session.attach_plugins(plugins.clone());
    plugins.set(vec![tool("AskUserQuestion", |_| unimplemented!()), echo()]);
    h.session.refresh_plugins();
    let names = all(&h);
    assert_eq!(
        names.iter().filter(|n| *n == "AskUserQuestion").count(),
        1,
        "{names:?}"
    );
    assert!(names.contains(&"ext_echo_echo".to_owned()));
    let notices = h.session.take_plugin_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("AskUserQuestion"));
    assert!(h.session.take_plugin_notices().is_empty());
}

/// A session with an MCP hub attached, and the plugins it was built from.
fn session_with_mcp() -> (Harness, Arc<FakePlugins>, Arc<McpHub>) {
    let h = session();
    let plugins = Arc::new(FakePlugins::default());
    plugins.set_mcp(vec![plugin_servers("demo")]);
    h.session.attach_plugins(plugins.clone());
    let hub = Arc::new(McpHub::for_folder(&std::env::temp_dir()));
    h.session.attach_mcp(hub.clone(), true);
    (h, plugins, hub)
}

#[test]
fn a_plugin_tool_change_keeps_the_mcp_hub() {
    let (h, plugins, hub) = session_with_mcp();
    plugins.set(vec![echo()]);
    h.session.refresh_plugins();
    assert!(all(&h).contains(&"ext_echo_echo".to_owned()));
    // The MCP servers did not change, so the running hub stays and is not built again.
    assert_eq!(plugins.hub_builds(), 0);
    assert!(Arc::ptr_eq(&h.session.mcp().unwrap(), &hub));
}

#[test]
fn an_mcp_server_change_builds_the_hub_again() {
    let (h, plugins, hub) = session_with_mcp();
    plugins.set_mcp(vec![plugin_servers("demo"), plugin_servers("other")]);
    h.session.refresh_plugins();
    assert_eq!(plugins.hub_builds(), 1);
    assert!(!Arc::ptr_eq(&h.session.mcp().unwrap(), &hub));
}

#[test]
fn a_source_that_does_not_say_builds_the_hub_on_every_change() {
    #[derive(Default)]
    struct Unsaid(FakePlugins);
    impl PluginSource for Unsaid {
        fn generation(&self) -> u64 {
            self.0.generation()
        }
        fn tools(&self, taken: &[String]) -> (Vec<ToolDefinition>, Vec<String>) {
            self.0.tools(taken)
        }
        fn mcp_hub(&self) -> Option<Arc<McpHub>> {
            self.0.mcp_hub()
        }
    }
    let h = session();
    let plugins = Arc::new(Unsaid::default());
    h.session.attach_plugins(plugins.clone());
    h.session
        .attach_mcp(Arc::new(McpHub::for_folder(&std::env::temp_dir())), true);
    plugins.0.set(vec![echo()]);
    h.session.refresh_plugins();
    assert_eq!(plugins.0.hub_builds(), 1);
}
