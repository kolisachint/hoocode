#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! PL-4: a session's plugin tools follow its plugin source at once, with no `/reload`. The
//! source stands in for the CLI's (which builds `ext_` tools from the installed plugins).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_agent_session::PluginSource;
use hoocode_code_tool_api::ToolDefinition;

use crate::common::{tool, Harness, HarnessOptions};

/// A plugin set the test installs and removes; each change moves the generation.
#[derive(Default)]
struct FakePlugins {
    tools: Mutex<Vec<ToolDefinition>>,
    generation: AtomicU64,
    builds: AtomicUsize,
}

impl FakePlugins {
    fn set(&self, tools: Vec<ToolDefinition>) {
        *self.tools.lock().unwrap() = tools;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
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

    fn mcp_hub(&self) -> Option<Arc<McpHub>> {
        None
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
