//! Diagnostic text that would go to stderr: a warning once, the output of a
//! child process, a note about thread priority.
//!
//! An interactive session draws on the terminal. A stray write there scrolls
//! the alternate screen and desyncs the frame. The interactive mode installs a
//! sink (the debug log), so everything in this module stays off the terminal.
//! Print and RPC modes have no sink and write to stderr as before.
//!
//! Every stderr write on an interactive path goes through here: a direct
//! `eprintln!` there would reach the terminal under the frame. The old
//! after-the-fact repaint check is gone, since with a sink installed nothing
//! could set it.

use std::sync::Mutex;

/// Receives diagnostic text in place of stderr.
pub type StderrSink = Box<dyn Fn(&str) + Send + Sync>;

static SINK: Mutex<Option<StderrSink>> = Mutex::new(None);

/// Routes diagnostic text to `sink`; `None` puts it back on stderr.
pub fn set_stderr_sink(sink: Option<StderrSink>) {
    *SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = sink;
}

/// Writes diagnostic text, verbatim, to the sink or else to stderr.
pub fn write_stderr(text: &str) {
    let sink = SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match sink.as_ref() {
        Some(sink) => sink(text),
        None => {
            drop(sink);
            eprint!("{text}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;

    #[test]
    fn sink_takes_the_text_and_stderr_is_not_marked() {
        let seen = Arc::new(StdMutex::new(String::new()));
        let captured = seen.clone();
        set_stderr_sink(Some(Box::new(move |text| {
            captured.lock().unwrap().push_str(text);
        })));
        write_stderr("warn\n");
        write_stderr("more");
        set_stderr_sink(None);
        assert_eq!(*seen.lock().unwrap(), "warn\nmore");
    }
}
