//! Herdr support: inside a Herdr pane (<https://herdr.dev>), the interactive
//! mode reports its state (`working`, `idle`, `blocked`), the command that
//! resumes its session, and its release on quit, through the Herdr CLI
//! (`"$HERDR_BIN_PATH" pane report-agent …`). Outside Herdr it does nothing.
//!
//! The core is pure: [`HerdrEnv::from_vars`] reads an injected environment,
//! [`report_argv`] / [`release_argv`] / [`resume_argv`] build the argv, and
//! [`Reporter`] keeps the sequence number rising and drops repeated states.
//! Reports go out on one background thread ([`Transport`]); a report that
//! queues behind one in flight replaces any older one still waiting. The
//! release on quit goes through the same thread, after (and cancelling) any
//! report in flight, and the UI thread waits for it only briefly.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `--source` and `--agent`: the integration and the name Herdr shows.
pub const SOURCE: &str = "hoocode";
/// The command the resume argv starts with (a plain name on `PATH`).
pub const RESUME_COMMAND: &str = "hoocode";
/// A background report that runs longer than this is killed.
pub const REPORT_TIMEOUT: Duration = Duration::from_secs(2);
/// The release on quit waits at most this long.
pub const RELEASE_TIMEOUT: Duration = Duration::from_millis(500);
/// Herdr's limits on the resume argv.
const RESUME_MAX_ARGS: usize = 64;
const RESUME_MAX_BYTES: usize = 8 * 1024;

/// The state reported to Herdr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// A turn is running.
    Working,
    /// Ready for input.
    Idle,
    /// Waiting for the user to decide (a permission card, a question).
    Blocked,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Idle => "idle",
            AgentState::Blocked => "blocked",
        }
    }
}

/// The pane this process runs in, from `HERDR_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrEnv {
    pub bin: PathBuf,
    pub pane_id: String,
}

impl HerdrEnv {
    /// `Some` only when `HERDR_ENV=1` and `HERDR_PANE_ID`, `HERDR_BIN_PATH`
    /// and `HERDR_SOCKET_PATH` are all set and non-empty.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if get("HERDR_ENV").as_deref() != Some("1") {
            return None;
        }
        let set = |name: &str| get(name).filter(|v| !v.is_empty());
        let pane_id = set("HERDR_PANE_ID")?;
        let bin = set("HERDR_BIN_PATH")?;
        set("HERDR_SOCKET_PATH")?;
        Some(Self {
            bin: PathBuf::from(bin),
            pane_id,
        })
    }

    /// [`Self::from_vars`] on this process's environment.
    pub fn from_process_env() -> Option<Self> {
        Self::from_vars(|name| std::env::var(name).ok())
    }
}

/// The session the pane holds: what the report names and how to resume it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReport {
    pub id: String,
    pub path: Option<String>,
    /// The resume command, `hoocode --session …`; `None` when the session
    /// cannot be resumed (not saved yet, or an argument breaks Herdr's rules).
    pub resume: Option<Vec<String>>,
}

/// Whether `arg` may go in a resume argv: no apostrophe, no control character.
pub fn resume_arg_ok(arg: &str) -> bool {
    !arg.chars().any(|c| c == '\'' || c.is_control())
}

/// Whether `argv` follows Herdr's resume rules: a plain command name first,
/// no bad characters, at most 64 arguments and 8 KiB.
pub fn resume_argv_ok(argv: &[String]) -> bool {
    let Some(first) = argv.first() else {
        return false;
    };
    !first.is_empty()
        && !first.contains(['/', '\\'])
        && argv.len() <= RESUME_MAX_ARGS
        && argv.iter().map(|a| a.len() + 1).sum::<usize>() <= RESUME_MAX_BYTES
        && argv.iter().all(|a| resume_arg_ok(a))
}

/// The command that reopens the saved session `id` at `file`. Herdr runs it in
/// the pane's directory, where hoocode started: `--session <id>` looks the id
/// up in that directory's default session dir, and asks to fork a session it
/// finds in another project. So the id form is for a file in `launch_dir`
/// (the default session dir of the launch cwd); any other file (a session
/// resumed from another project, a custom session dir) goes by absolute path,
/// which `--session` opens as it is, with no question. `None` when neither
/// follows Herdr's rules.
pub fn resume_argv(id: &str, file: &Path, launch_dir: Option<&Path>) -> Option<Vec<String>> {
    let file = std::path::absolute(file).ok()?;
    let target = if !id.is_empty() && launch_dir.is_some() && file.parent() == launch_dir {
        id.to_string()
    } else {
        file.to_str()?.to_string()
    };
    let argv = vec![RESUME_COMMAND.to_string(), "--session".to_string(), target];
    resume_argv_ok(&argv).then_some(argv)
}

/// `pane report-agent` argv (without the binary).
pub fn report_argv(
    pane_id: &str,
    state: AgentState,
    message: Option<&str>,
    seq: u64,
    session: Option<&SessionReport>,
) -> Vec<String> {
    let mut argv: Vec<String> = [
        "pane",
        "report-agent",
        pane_id,
        "--source",
        SOURCE,
        "--agent",
        SOURCE,
        "--state",
        state.as_str(),
        "--seq",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    argv.push(seq.to_string());
    if let Some(message) = message {
        argv.push("--message".into());
        argv.push(message.to_string());
    }
    if let Some(session) = session {
        argv.push("--agent-session-id".into());
        argv.push(session.id.clone());
        if let Some(path) = &session.path {
            argv.push("--agent-session-path".into());
            argv.push(path.clone());
        }
        if let Some(resume) = &session.resume {
            argv.push("--".into());
            argv.extend(resume.iter().cloned());
        }
    }
    argv
}

/// `pane release-agent` argv (without the binary).
pub fn release_argv(pane_id: &str, seq: u64) -> Vec<String> {
    [
        "pane",
        "release-agent",
        pane_id,
        "--source",
        SOURCE,
        "--agent",
        SOURCE,
        "--seq",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(std::iter::once(seq.to_string()))
    .collect()
}

/// The `--seq` source: unix milliseconds, so it keeps rising across restarts,
/// and never less than one past the last number handed out.
#[derive(Debug, Default)]
pub struct SeqClock {
    last: u64,
}

impl SeqClock {
    pub fn next(&mut self, now_ms: u64) -> u64 {
        self.last = now_ms.max(self.last + 1);
        self.last
    }
}

/// Unix time in milliseconds.
pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Where the argv goes: the CLI on a background thread, or a test's log.
pub trait Transport {
    /// Queues a report; never blocks.
    fn send(&self, argv: Vec<String>);
    /// Sends the release, after any report in flight, and waits for it
    /// (bounded). Reports still queued are dropped.
    fn release(&self, argv: Vec<String>);
}

/// What the last report said.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Last {
    state: AgentState,
    message: Option<String>,
    session_id: String,
}

/// Turns the mode's state into reports: one per change, never a repeat.
pub struct Reporter {
    pane_id: String,
    transport: Box<dyn Transport>,
    clock: Box<dyn FnMut() -> u64>,
    exists: Box<dyn Fn(&Path) -> bool>,
    /// The default session dir of the cwd hoocode started in (see [`resume_argv`]).
    launch_dir: Option<PathBuf>,
    seq: SeqClock,
    last: Option<Last>,
    /// The session id whose file was seen on disk (so it is not checked again).
    saved: Option<String>,
    released: bool,
}

impl Reporter {
    pub fn new(
        pane_id: String,
        transport: Box<dyn Transport>,
        clock: Box<dyn FnMut() -> u64>,
        exists: Box<dyn Fn(&Path) -> bool>,
        launch_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            pane_id,
            transport,
            clock,
            exists,
            launch_dir,
            seq: SeqClock::default(),
            last: None,
            saved: None,
            released: false,
        }
    }

    /// The reporter for this process: `None` outside Herdr. The launch cwd is
    /// read once, here.
    pub fn from_process_env() -> Option<Self> {
        let env = HerdrEnv::from_process_env()?;
        let launch_dir = std::env::current_dir()
            .ok()
            .map(|cwd| hoocode_code_session::default_session_dir(&cwd.to_string_lossy()));
        Some(Self::new(
            env.pane_id.clone(),
            Box::new(CliTransport::start(env.bin)),
            Box::new(unix_millis),
            Box::new(|path: &Path| path.is_file()),
            launch_dir,
        ))
    }

    /// The mode's state now. `file` is the session file (`None` when not
    /// persisted). Called every loop pass, so an unchanged state costs only
    /// comparisons. Reports only when something Herdr shows has changed. The
    /// file is checked, and the resume command built, only then.
    pub fn update(
        &mut self,
        state: AgentState,
        message: Option<&str>,
        session_id: &str,
        file: Option<&Path>,
    ) {
        if self.released {
            return;
        }
        let message = if state == AgentState::Blocked {
            message
        } else {
            None
        };
        if self.last.as_ref().is_some_and(|last| {
            last.state == state
                && last.message.as_deref() == message
                && last.session_id == session_id
        }) {
            return;
        }
        let saved = self.saved.as_deref() == Some(session_id);
        let resume = match file {
            Some(file) if saved || (self.exists)(file) => {
                self.saved = Some(session_id.to_string());
                resume_argv(session_id, file, self.launch_dir.as_deref())
            }
            _ => None,
        };
        let session = (!session_id.is_empty()).then(|| SessionReport {
            id: session_id.to_string(),
            path: file.and_then(Path::to_str).map(str::to_string),
            resume,
        });
        let seq = self.seq.next((self.clock)());
        self.transport.send(report_argv(
            &self.pane_id,
            state,
            message,
            seq,
            session.as_ref(),
        ));
        self.last = Some(Last {
            state,
            message: message.map(str::to_string),
            session_id: session_id.to_string(),
        });
    }

    /// The loop ended. Only a quit the user asked for releases the pane (not a
    /// switch of sessions, and not a fatal exit, where Herdr keeps the resume
    /// command and its own check clears the pane).
    pub fn finish(&mut self, user_quit: bool) {
        if user_quit {
            self.release();
        }
    }

    /// The user quits: release the pane (once).
    pub fn release(&mut self) {
        if std::mem::replace(&mut self.released, true) {
            return;
        }
        let seq = self.seq.next((self.clock)());
        self.transport.release(release_argv(&self.pane_id, seq));
    }
}

/// What the worker thread is asked to run.
enum Job {
    Report(Vec<String>),
    /// The release, and where to say it is done.
    Release(Vec<String>, mpsc::Sender<()>),
}

/// Runs the Herdr CLI: reports on one background thread, latest first.
pub struct CliTransport {
    tx: mpsc::Sender<Job>,
    /// Set by the release: a report in flight is cut short, none starts.
    releasing: Arc<AtomicBool>,
    release_timeout: Duration,
}

impl CliTransport {
    pub fn start(bin: PathBuf) -> Self {
        Self::start_with(bin, REPORT_TIMEOUT, RELEASE_TIMEOUT)
    }

    /// [`Self::start`] with the two timeouts given (tests use longer ones).
    pub fn start_with(bin: PathBuf, report_timeout: Duration, release_timeout: Duration) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let releasing = Arc::new(AtomicBool::new(false));
        let worker_releasing = releasing.clone();
        let _ = hoocode_runtime::spawn_named_thread("hoocode-herdr", move || {
            while let Ok(mut job) = rx.recv() {
                // Only the latest matters: skip the ones behind it. The
                // release is always last.
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                match job {
                    Job::Report(argv) => {
                        if worker_releasing.load(Ordering::SeqCst) {
                            continue;
                        }
                        run_cli(&bin, &argv, report_timeout, Some(&worker_releasing));
                    }
                    Job::Release(argv, done) => {
                        run_cli(&bin, &argv, release_timeout, None);
                        let _ = done.send(());
                        break;
                    }
                }
            }
        });
        Self {
            tx,
            releasing,
            release_timeout,
        }
    }
}

impl Transport for CliTransport {
    fn send(&self, argv: Vec<String>) {
        let _ = self.tx.send(Job::Report(argv));
    }

    fn release(&self, argv: Vec<String>) {
        self.releasing.store(true, Ordering::SeqCst);
        let (done, wait) = mpsc::channel();
        if self.tx.send(Job::Release(argv, done)).is_ok() {
            let _ = wait.recv_timeout(self.release_timeout);
        }
    }
}

/// Runs `bin argv` with no terminal attached, in a session of its own (so a
/// kill reaches whatever it started), killed after `timeout` or as soon as
/// `cancel` is set. Failures are ignored: Herdr is never in hoocode's way.
pub fn run_cli(bin: &Path, argv: &[String], timeout: Duration, cancel: Option<&AtomicBool>) {
    let mut command = Command::new(bin);
    command
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hoocode_runtime::spawn_in_new_session(&mut command);
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None)
                if Instant::now() >= deadline
                    || cancel.is_some_and(|c| c.load(Ordering::SeqCst)) =>
            {
                kill_tree(&mut child);
                let _ = child.wait();
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Kills the child and its process group (the child leads one: it was started
/// in a new session).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: kill(2) with a group id of our own child; no memory is touched.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn env_of(map: &HashMap<String, String>) -> Option<HerdrEnv> {
        HerdrEnv::from_vars(|name| map.get(name).cloned())
    }

    const FULL: [(&str, &str); 4] = [
        ("HERDR_ENV", "1"),
        ("HERDR_PANE_ID", "w1:p1"),
        ("HERDR_BIN_PATH", "/opt/herdr"),
        ("HERDR_SOCKET_PATH", "/tmp/x"),
    ];

    #[test]
    fn env_needs_every_variable_set() {
        assert_eq!(
            env_of(&vars(&FULL)),
            Some(HerdrEnv {
                bin: PathBuf::from("/opt/herdr"),
                pane_id: "w1:p1".into()
            })
        );
        assert_eq!(env_of(&HashMap::new()), None);
        let mut off = vars(&FULL);
        off.insert("HERDR_ENV".into(), "0".into());
        assert_eq!(env_of(&off), None);
        for name in ["HERDR_PANE_ID", "HERDR_BIN_PATH", "HERDR_SOCKET_PATH"] {
            let mut missing = vars(&FULL);
            missing.remove(name);
            assert_eq!(env_of(&missing), None, "{name} missing");
            let mut empty = vars(&FULL);
            empty.insert(name.into(), String::new());
            assert_eq!(env_of(&empty), None, "{name} empty");
        }
    }

    #[test]
    fn seq_rises_with_the_clock_and_past_it() {
        let mut seq = SeqClock::default();
        assert_eq!(seq.next(100), 100);
        assert_eq!(seq.next(100), 101);
        assert_eq!(seq.next(50), 102);
        assert_eq!(seq.next(500), 500);
    }

    #[test]
    fn resume_by_id_only_in_the_launch_dirs_sessions_else_by_path() {
        let dir = Path::new("/s/--w--");
        let by = |id: &str, file: &str, launch: Option<&Path>| {
            resume_argv(id, Path::new(file), launch).map(|v| v[1..].join(" "))
        };
        assert_eq!(
            by("abc-123", "/s/--w--/x_abc-123.jsonl", Some(dir)).as_deref(),
            Some("--session abc-123")
        );
        // A session resumed from another project: `--session <id>` would not
        // find it from the pane's directory, and would ask to fork it.
        assert_eq!(
            by("abc-123", "/s/--other--/x_abc-123.jsonl", Some(dir)).as_deref(),
            Some("--session /s/--other--/x_abc-123.jsonl")
        );
        assert_eq!(
            by("abc-123", "/custom/x.jsonl", Some(dir)).as_deref(),
            Some("--session /custom/x.jsonl")
        );
        assert_eq!(
            by("abc-123", "/s/--w--/x.jsonl", None).as_deref(),
            Some("--session /s/--w--/x.jsonl"),
            "no launch dir known: by path"
        );
        let relative = resume_argv("abc", Path::new("x.jsonl"), None).unwrap();
        assert!(Path::new(&relative[2]).is_absolute(), "{relative:?}");
        assert_eq!(by("abc", "/it's/x.jsonl", Some(dir)), None);
        assert_eq!(by("a\nb", "/s/--w--/x.jsonl", Some(dir)), None);
        assert_eq!(by("abc", "/other/x\ny.jsonl", Some(dir)), None);
    }

    #[test]
    fn resume_rules() {
        let ok = |v: &[&str]| resume_argv_ok(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(ok(&["hoocode", "--session", "id"]));
        assert!(!ok(&[]));
        assert!(!ok(&["/usr/bin/hoocode"]));
        assert!(!ok(&["hoocode", "a\tb"]));
        assert!(!ok(&["hoocode"; 65]));
        let long = "x".repeat(9000);
        assert!(!ok(&["hoocode", &long]));
    }

    #[test]
    fn argv_shapes() {
        let session = SessionReport {
            id: "abc".into(),
            path: Some("/s/abc.jsonl".into()),
            resume: Some(vec!["hoocode".into(), "--session".into(), "abc".into()]),
        };
        assert_eq!(
            report_argv("w1:p1", AgentState::Blocked, Some("why"), 7, Some(&session)).join(" "),
            "pane report-agent w1:p1 --source hoocode --agent hoocode --state blocked --seq 7 \
             --message why --agent-session-id abc --agent-session-path /s/abc.jsonl \
             -- hoocode --session abc"
        );
        assert_eq!(
            report_argv("w1:p1", AgentState::Idle, None, 8, None).join(" "),
            "pane report-agent w1:p1 --source hoocode --agent hoocode --state idle --seq 8"
        );
        assert_eq!(
            release_argv("w1:p1", 9).join(" "),
            "pane release-agent w1:p1 --source hoocode --agent hoocode --seq 9"
        );
    }

    #[derive(Clone, Default)]
    struct Log(Rc<RefCell<Vec<String>>>);

    impl Transport for Log {
        fn send(&self, argv: Vec<String>) {
            self.0.borrow_mut().push(argv.join(" "));
        }
        fn release(&self, argv: Vec<String>) {
            self.0.borrow_mut().push(argv.join(" "));
        }
    }

    fn reporter(log: &Log, on_disk: Rc<Cell<bool>>, checks: Rc<Cell<u32>>) -> Reporter {
        let mut now = 1000;
        Reporter::new(
            "p".into(),
            Box::new(log.clone()),
            Box::new(move || {
                now += 1;
                now
            }),
            Box::new(move |_| {
                checks.set(checks.get() + 1);
                on_disk.get()
            }),
            Some(PathBuf::from("/s")),
        )
    }

    #[test]
    fn reports_each_change_once_and_resume_once_saved() {
        let log = Log::default();
        let on_disk = Rc::new(Cell::new(false));
        let checks = Rc::new(Cell::new(0));
        let mut r = reporter(&log, on_disk.clone(), checks.clone());
        let file = Path::new("/s/a.jsonl");
        r.update(AgentState::Idle, None, "a", Some(file));
        r.update(AgentState::Idle, None, "a", Some(file));
        assert_eq!(checks.get(), 1, "the file is checked only on a change");
        r.update(AgentState::Working, Some("ignored"), "a", Some(file));
        on_disk.set(true);
        r.update(AgentState::Working, None, "a", Some(file));
        r.update(AgentState::Blocked, Some("Permission"), "a", Some(file));
        r.update(AgentState::Blocked, Some("Permission"), "a", Some(file));
        r.update(AgentState::Idle, None, "a", Some(file));
        r.update(AgentState::Idle, None, "b", Some(Path::new("/s/b.jsonl")));
        r.finish(true);
        r.release();
        r.update(AgentState::Working, None, "b", None);
        let base = "pane report-agent p --source hoocode --agent hoocode --state";
        assert_eq!(
            *log.0.borrow(),
            vec![
                format!("{base} idle --seq 1001 --agent-session-id a --agent-session-path /s/a.jsonl"),
                format!("{base} working --seq 1002 --agent-session-id a --agent-session-path /s/a.jsonl"),
                format!(
                    "{base} blocked --seq 1003 --message Permission --agent-session-id a \
                     --agent-session-path /s/a.jsonl -- hoocode --session a"
                ),
                format!(
                    "{base} idle --seq 1004 --agent-session-id a --agent-session-path /s/a.jsonl \
                     -- hoocode --session a"
                ),
                format!(
                    "{base} idle --seq 1005 --agent-session-id b --agent-session-path /s/b.jsonl \
                     -- hoocode --session b"
                ),
                "pane release-agent p --source hoocode --agent hoocode --seq 1006".to_string(),
            ]
        );
    }

    #[test]
    fn a_rejected_resume_is_not_rebuilt_every_pass() {
        let log = Log::default();
        let checks = Rc::new(Cell::new(0));
        let mut r = reporter(&log, Rc::new(Cell::new(true)), checks.clone());
        let file = Path::new("/it's/a.jsonl");
        for _ in 0..5 {
            r.update(AgentState::Idle, None, "a", Some(file));
        }
        assert_eq!(log.0.borrow().len(), 1);
        assert!(!log.0.borrow()[0].contains(" -- "), "no resume part");
        assert_eq!(checks.get(), 1);
    }

    #[test]
    fn a_session_from_another_project_resumes_by_path() {
        let log = Log::default();
        let mut r = reporter(&log, Rc::new(Cell::new(true)), Rc::new(Cell::new(0)));
        r.update(AgentState::Idle, None, "z", Some(Path::new("/s2/z.jsonl")));
        assert!(
            log.0.borrow()[0].ends_with("-- hoocode --session /s2/z.jsonl"),
            "{:?}",
            log.0.borrow()
        );
    }

    #[test]
    fn only_a_user_quit_releases() {
        let log = Log::default();
        let mut r = reporter(&log, Rc::new(Cell::new(true)), Rc::new(Cell::new(0)));
        r.update(AgentState::Idle, None, "a", None);
        r.finish(false);
        assert_eq!(log.0.borrow().len(), 1, "a fatal exit sends no release");
        r.finish(true);
        assert_eq!(log.0.borrow().len(), 2);
        assert!(log.0.borrow()[1].starts_with("pane release-agent"));
    }
}
