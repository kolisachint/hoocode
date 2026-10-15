#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Admission: a cold pool never runs more than `max_concurrency` children at
//! once, even when many dispatches arrive at the same moment.
//!
//! The runner here takes time to spawn, which is the window in which two pulls
//! used to read the same free count and both start a child.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hoocode_code_subagents::pool::*;
use hoocode_code_subagents::runner::{BoxFuture, RunSpec, Runner, RunnerHandle};
use serde_json::json;
use tokio::sync::mpsc;

/// Children alive at once, and the most that were ever alive.
#[derive(Default)]
struct Gauge {
    live: AtomicUsize,
    peak: AtomicUsize,
}

/// A runner whose children take 50 ms to spawn and 50 ms to finish.
struct CountingRunner {
    gauge: Arc<Gauge>,
    cwd: PathBuf,
}

impl Runner for CountingRunner {
    fn spawn(&self, spec: RunSpec) -> io::Result<Box<dyn RunnerHandle>> {
        std::thread::sleep(Duration::from_millis(50));
        let live = self.gauge.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.gauge.peak.fetch_max(live, Ordering::SeqCst);
        let dir = hoocode_code_paths::dispatch_task_dir(&self.cwd, &spec.task_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("result.json"),
            json!({"summary": "done", "files_changed": [], "confidence": 0.9, "status": "complete"})
                .to_string(),
        )?;
        Ok(Box::new(CountingHandle {
            gauge: Some(self.gauge.clone()),
        }))
    }
}

struct CountingHandle {
    /// Taken by whichever of `wait` or `drop` releases the child first.
    gauge: Option<Arc<Gauge>>,
}

impl Drop for CountingHandle {
    fn drop(&mut self) {
        if let Some(gauge) = self.gauge.take() {
            gauge.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl RunnerHandle for CountingHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        // The senders drop here, so both streams read as closed.
        let (_, stdout) = mpsc::channel(1);
        let (_, stderr) = mpsc::channel(1);
        (stdout, stderr)
    }

    fn wait(&mut self) -> BoxFuture<'static, io::Result<Option<i32>>> {
        let gauge = self.gauge.take();
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(gauge) = gauge {
                gauge.live.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(Some(0))
        })
    }

    fn kill(&mut self) {}

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

/// Twelve dispatches from four worker threads at once, against a cap of three.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_dispatches_never_exceed_max_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let gauge = Arc::new(Gauge::default());
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: PathBuf::from("/nonexistent/hoocode"),
        max_concurrency: Some(3),
        max_queued: Some(32),
        cwd: Some(dir.path().to_path_buf()),
        runner: Some(Arc::new(CountingRunner {
            gauge: gauge.clone(),
            cwd: dir.path().to_path_buf(),
        })),
        ..Default::default()
    });

    let ids: Vec<String> = (1..=12).map(|n| format!("adm{n}")).collect();
    let spawns: Vec<_> = ids
        .iter()
        .map(|id| {
            let pool = pool.clone();
            let task = task(id);
            tokio::spawn(async move { pool.spawn(task) })
        })
        .collect();
    for spawn in spawns {
        spawn.await.unwrap().unwrap();
    }

    for id in &ids {
        let result = pool.wait_for(id).await.unwrap();
        assert_eq!(&result.task_id, id);
    }

    let peak = gauge.peak.load(Ordering::SeqCst);
    assert!(
        peak <= 3,
        "peak live children was {peak}, above max_concurrency 3"
    );
    assert_eq!(pool.running_count(), 0);
    assert_eq!(pool.queued_count(), 0);
    pool.dispose();
}
