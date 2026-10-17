//! A handler that fails is reported on the diagnostic channel. An interactive
//! session draws on the terminal, so a direct stderr write would land under
//! the frame; the sink takes it instead.

use std::sync::{Arc, Mutex};

use hoocode_code_agent_session::event_bus::EventBus;
use hoocode_runtime::diagnostics::set_stderr_sink;

#[test]
fn a_failing_handler_reports_through_the_diagnostic_sink() {
    let seen = Arc::new(Mutex::new(String::new()));
    let captured = seen.clone();
    set_stderr_sink(Some(Box::new(move |text| {
        captured.lock().unwrap().push_str(text);
    })));
    let bus = EventBus::new();
    let _subscription = bus.on("ch", |_| Err("boom".to_string()));
    bus.emit("ch", &serde_json::json!({}));
    set_stderr_sink(None);
    assert_eq!(*seen.lock().unwrap(), "Event handler error (ch): boom\n");
}
