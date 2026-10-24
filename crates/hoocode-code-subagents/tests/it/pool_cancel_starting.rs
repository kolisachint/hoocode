#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Cancelling a task that is between admission and its child existing: the
//! runner's `spawn` is held open on a gate, so the task is deterministically
//! `starting` when `cancel` arrives.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::Duration;

use hoocode_code_subagents::pool::*;
use hoocode_code_subagents::runner::{BoxFuture, RunSpec, Runner, RunnerHandle};
use tokio::sync::mpsc;

/// Blocks the first spawn on a gate; later spawns return at once.
struct GatedRunner {
    entered: Mutex<Option<std_mpsc::Sender<()>>>,
    release: Mutex<Option<std_mpsc::Receiver<()>>>,
    spawned: Arc<AtomicUsize>,
    killed: Arc<AtomicUsize>,
}

impl Runner for GatedRunner {
    fn spawn(&self, _spec: RunSpec) -> io::Result<Box<dyn RunnerHandle>> {
        let entered = self.entered.lock().unwrap().take();
        let release = self.release.lock().unwrap().take();
        if let (Some(entered), Some(release)) = (entered, release) {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        self.spawned.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(IdleHandle {
            killed: self.killed.clone(),
        }))
    }
}

struct IdleHandle {
    killed: Arc<AtomicUsize>,
}

impl RunnerHandle for IdleHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        let (_, stdout) = mpsc::channel(1);
        let (_, stderr) = mpsc::channel(1);
        (stdout, stderr)
    }

    fn wait(&mut self) -> BoxFuture<'static, io::Result<Option<i32>>> {
        Box::pin(async { Ok(Some(0)) })
    }

    fn kill(&mut self) {
        self.killed.fetch_add(1, Ordering::SeqCst);
    }

    fn pid(&self) -> u32 {
        0
    }
}

fn task(id: &str) -> SubagentPoolTask {
    SubagentPoolTask {
        task_id: id.into(),
        agent_type: "explore".into(),
        task: id.into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_spawn_settles_once_and_frees_the_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (entered_tx, entered_rx) = std_mpsc::channel();
    let (release_tx, release_rx) = std_mpsc::channel();
    let spawned = Arc::new(AtomicUsize::new(0));
    let killed = Arc::new(AtomicUsize::new(0));
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: PathBuf::from("/nonexistent/hoocode"),
        max_concurrency: Some(1),
        cwd: Some(dir.path().to_path_buf()),
        runner: Some(Arc::new(GatedRunner {
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(Some(release_rx)),
            spawned: spawned.clone(),
            killed: killed.clone(),
        })),
        ..Default::default()
    });
    let cancelled = Arc::new(Mutex::new(Vec::new()));
    let sink = cancelled.clone();
    pool.on(move |event| {
        if event.name == "task_cancelled" {
            sink.lock().unwrap().push(event.data.clone());
        }
    });

    // `spawn` runs the runner on the calling thread, so it blocks on the gate.
    let spawner = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.spawn(task("starting")).unwrap())
    };
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(pool.get_status("starting"), TaskStatus::Running);
    assert_eq!(pool.running_count(), 1);

    assert!(pool.cancel("starting"));
    release_tx.send(()).unwrap();
    spawner.join().unwrap();

    let result = pool.wait_for("starting").await.unwrap();
    assert_eq!(result.status, Some(ResultStatus::Cancelled));
    assert_eq!(pool.get_status("starting"), TaskStatus::Cancelled);
    assert_eq!(cancelled.lock().unwrap().len(), 1);
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    assert_eq!(killed.load(Ordering::SeqCst), 1, "the child must be ended");
    assert_eq!(pool.running_count(), 0);
    assert!(!pool.cancel("starting"), "already settled");

    // Capacity is back: a new task is admitted and runs.
    pool.spawn(task("next")).unwrap();
    assert_eq!(pool.get_status("next"), TaskStatus::Running);
    pool.dispose();
}
