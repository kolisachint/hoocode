//! The attached view of a shell job's output (T5.3): the job's log as plain
//! text, followed while the job runs. The log is read on the tools pool
//! (`attach.rs` `poll_shell_tail`); this file holds the view's state and what a
//! read does to it, with no I/O.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use hoocode_code_task_store::{Task, TaskStatus};
use hoocode_code_tool_bash::jobs::JobRead;
use hoocode_tui_components::Text;
use hoocode_tui_render::ComponentHandle;
use hoocode_tui_util::strip_vt_control_characters;

use super::{as_component, handle};

/// The most output kept on screen. Older text goes from the top.
const KEEP_BYTES: usize = 64 * 1024;
/// Shown until the first output arrives.
const EMPTY: &str = "No output yet.";
/// Put above the output when the view starts partway into the log.
const EARLIER: &str = "… earlier output not shown\n";

/// The state of one attached job's output.
pub(super) struct JobTail {
    /// The job's log file.
    pub(super) path: PathBuf,
    /// The text on screen, without escape sequences.
    text: String,
    /// False until the first read (the log's tail) has been asked for.
    started: bool,
    /// Where the next read starts: the byte offset after the text read so far.
    offset: u64,
    /// A read is on the tools pool.
    pub(super) reading: bool,
    /// The job ended and its whole log is read: nothing more to follow.
    pub(super) settled: bool,
    view: Rc<RefCell<Text>>,
}

impl JobTail {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            text: String::new(),
            started: false,
            offset: 0,
            reading: false,
            settled: false,
            view: handle(Text::new(EMPTY, 1, 0)),
        }
    }

    /// The component that shows the output, for the chat.
    pub(super) fn component(&self) -> ComponentHandle {
        as_component(&self.view)
    }

    /// The `since` of the next read: `None` for the first, which is the tail.
    pub(super) fn since(&self) -> Option<u64> {
        self.started.then_some(self.offset)
    }

    /// Whether a read that started at `since` still applies: it did not start
    /// from an earlier position than the view is at now.
    pub(super) fn accepts(&self, since: Option<u64>) -> bool {
        self.since() == since
    }

    /// Adds what a read returned. `finished` is whether the job has ended.
    pub(super) fn apply(&mut self, read: JobRead, finished: bool) {
        if !self.started && read.from > 0 {
            self.push(EARLIER);
        }
        self.started = true;
        self.offset = read.next;
        self.push(&plain(&read.text));
        self.settled = finished && read.next >= read.size;
        self.show();
    }

    /// The log could not be read. The view says why and stops following.
    pub(super) fn fail(&mut self, message: &str) {
        self.settled = true;
        self.text = plain(message);
        self.show();
    }

    fn push(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        if self.text.len() > KEEP_BYTES {
            let mut cut = self.text.len() - KEEP_BYTES;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }

    fn show(&mut self) {
        let shown = if self.text.is_empty() {
            EMPTY.to_owned()
        } else {
            self.text.clone()
        };
        self.view.borrow_mut().set_text(shown);
    }

    #[cfg(test)]
    fn shown(&self) -> String {
        self.view.borrow().text().to_owned()
    }
}

/// The output as the view shows it: escape sequences and carriage returns dropped.
fn plain(text: &str) -> String {
    strip_vt_control_characters(text).replace('\r', "")
}

/// The header's state word for a job's row: `running`, `done`, the failure
/// (`exit 101`), `killed`.
pub(super) fn state_word(task: &Task) -> String {
    match task.status {
        TaskStatus::Pending => "pending".into(),
        TaskStatus::InProgress => "running".into(),
        TaskStatus::Done => "done".into(),
        TaskStatus::Failed => task.note.clone().unwrap_or_else(|| "failed".into()),
        TaskStatus::Cancelled => "killed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, from: u64, size: u64) -> JobRead {
        JobRead {
            text: text.to_owned(),
            from,
            next: from + text.len() as u64,
            size,
            state: hoocode_code_tool_bash::jobs::JobState::Running,
        }
    }

    fn tail() -> JobTail {
        JobTail::new(PathBuf::from("/s/jobs/job-1.log"))
    }

    #[test]
    fn the_first_read_is_the_tail_and_later_reads_resume_where_it_stopped() {
        let mut view = tail();
        assert_eq!(view.since(), None, "the first read is the tail");
        assert_eq!(view.shown(), EMPTY);
        view.apply(read("one\n", 0, 4), false);
        assert_eq!(view.since(), Some(4));
        view.apply(read("two\n", 4, 8), false);
        assert_eq!(view.shown(), "one\ntwo\n");
        assert!(!view.settled, "a running job is followed");
    }

    #[test]
    fn a_read_from_an_earlier_position_no_longer_applies() {
        let mut view = tail();
        assert!(view.accepts(None), "the first read is the tail");
        view.apply(read("one\n", 0, 4), false);
        assert!(!view.accepts(None), "a tail read that was out is stale now");
        assert!(
            view.accepts(Some(4)),
            "the next read from where the view is"
        );
        assert!(
            !view.accepts(Some(0)),
            "a read from the start would repeat text"
        );
    }

    #[test]
    fn a_tail_that_starts_partway_says_so() {
        let mut view = tail();
        view.apply(read("late\n", 100, 105), false);
        assert!(view.shown().starts_with(EARLIER), "{:?}", view.shown());
    }

    #[test]
    fn the_view_settles_once_a_finished_job_is_read_to_its_end() {
        let mut view = tail();
        view.apply(read("ok\n", 0, 10), true);
        assert!(!view.settled, "the log is not read to its end yet");
        // The log is 8 bytes long, and this read reaches its end.
        view.apply(read("more\n", 3, 8), true);
        assert!(view.settled);
    }

    #[test]
    fn escape_sequences_and_carriage_returns_are_dropped() {
        let mut view = tail();
        view.apply(
            read(
                "\u{1b}[32mgreen\u{1b}[0m\r\nprogress 50%\rprogress 100%\n",
                0,
                40,
            ),
            false,
        );
        assert_eq!(view.shown(), "green\nprogress 50%progress 100%\n");
    }

    #[test]
    fn a_failed_read_says_why_and_stops_following() {
        let mut view = tail();
        view.fail("Cannot read the log: gone");
        assert!(view.settled);
        assert_eq!(view.shown(), "Cannot read the log: gone");
    }

    #[test]
    fn the_kept_text_is_the_tail_and_cut_on_a_character_boundary() {
        let mut view = tail();
        let big = "é".repeat(KEEP_BYTES);
        view.apply(read(&big, 0, big.len() as u64 * 2), false);
        let shown = view.shown();
        assert!(shown.len() <= KEEP_BYTES, "{}", shown.len());
        assert!(shown.chars().all(|c| c == 'é'), "no split character");
    }

    #[test]
    fn the_header_word_follows_the_row() {
        let row = |status: TaskStatus, note: Option<&str>| Task {
            id: 1,
            title: "cargo test".into(),
            status,
            source: None,
            subagent_mode: None,
            agent: None,
            parent_task_id: None,
            linked_task_id: None,
            note: note.map(str::to_owned),
            todo_content: None,
            created_at: 0,
            updated_at: 0,
            usage: None,
            key: None,
            next_at: None,
            action: None,
            log_path: None,
        };
        assert_eq!(state_word(&row(TaskStatus::InProgress, None)), "running");
        assert_eq!(state_word(&row(TaskStatus::Done, None)), "done");
        assert_eq!(
            state_word(&row(TaskStatus::Failed, Some("exit 101"))),
            "exit 101"
        );
        assert_eq!(state_word(&row(TaskStatus::Cancelled, None)), "killed");
    }
}
