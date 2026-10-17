//! Diagnostic text that would go to stderr: a warning once, the output of a
//! child process, a note about thread priority.
//!
//! An interactive session draws on the terminal. A stray write there scrolls
//! the alternate screen and desyncs the frame. The interactive mode installs a
//! sink (the debug log), and checks [`take_stderr_written`] to repaint the
//! screen if anything reached the terminal anyway. Print and RPC modes have no
//! sink and write to stderr as before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Receives diagnostic text in place of stderr.
pub type StderrSink = Box<dyn Fn(&str) + Send + Sync>;

static SINK: Mutex<Option<StderrSink>> = Mutex::new(None);
static WROTE_TERMINAL: AtomicBool = AtomicBool::new(false);

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
            WROTE_TERMINAL.store(true, Ordering::SeqCst);
        }
    }
}

/// Whether text reached the terminal since the last call. Clears the flag.
pub fn take_stderr_written() -> bool {
    WROTE_TERMINAL.swap(false, Ordering::SeqCst)
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
        take_stderr_written();
        write_stderr("warn\n");
        write_stderr("more");
        set_stderr_sink(None);
        assert_eq!(*seen.lock().unwrap(), "warn\nmore");
        assert!(!take_stderr_written(), "the sink kept it off the terminal");
    }
}
