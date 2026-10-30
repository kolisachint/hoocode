#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Killing a subagent's process tree safely (2026-10-30 safety review): only
//! a child of ours that has not been reaped is walked, only groups that a
//! descendant leads are killed, and never this process, its ancestors or
//! their groups.

use hoocode_code_subagents::lifeguard::{plan_tree_kill, ProcRow};

const SELF: u32 = 100;

/// A table: init (1) -> terminal (50, group 50) -> us (100, group 50) ->
/// our child 200 (group 200) -> its command 300 (group 300) -> 301.
fn rows() -> Vec<ProcRow> {
    vec![
        (1, 0, 1),
        (50, 1, 50),
        (100, 50, 50),
        (200, 100, 200),
        (300, 200, 300),
        (301, 300, 300),
    ]
}

#[test]
fn a_direct_child_is_walked_with_the_groups_its_descendants_lead() {
    let (pids, groups) = plan_tree_kill(&rows(), 200, SELF);
    assert_eq!(pids, [300, 301]);
    assert_eq!(groups, [300]);
}

/// A pid that is not our direct child (reused by another process, or a
/// grandchild) is never walked.
#[test]
fn a_pid_whose_parent_is_not_us_is_left_untouched() {
    assert_eq!(plan_tree_kill(&rows(), 300, SELF), (vec![], vec![]));
    // A reused pid now belongs to the user's terminal: never walked.
    assert_eq!(plan_tree_kill(&rows(), 50, SELF), (vec![], vec![]));
    assert_eq!(plan_tree_kill(&rows(), 0, SELF), (vec![], vec![]));
}

/// A descendant that joined a group it does not lead (the terminal's, or
/// group 1) does not take that group down, and we and our ancestors are never
/// in the kill set.
#[test]
fn foreign_groups_and_our_own_line_are_never_killed() {
    let rows = vec![
        (1, 0, 1),
        (50, 1, 50),
        (100, 50, 50),
        (200, 100, 200),
        // joined the terminal's group
        (310, 200, 50),
        // joined init's group
        (311, 200, 1),
        // joined another child's group it does not lead
        (312, 200, 300),
        // a pathological table that lists our ancestor below the child
        (50, 200, 50),
    ];
    let (pids, groups) = plan_tree_kill(&rows, 200, SELF);
    assert!(groups.is_empty(), "{groups:?}");
    assert!(!pids.contains(&SELF) && !pids.contains(&50) && !pids.contains(&1));
    assert!(pids.contains(&310) && pids.contains(&311) && pids.contains(&312));
}

#[cfg(target_os = "linux")]
mod live {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use hoocode_code_subagents::lifeguard::{
        child_exited, child_spawned, is_live_child, kill_child_trees, kill_process_tree,
        kill_trees, live_generation, plan_tree_kill, ChildPid, ProcTable, ProcessTable,
    };
    use hoocode_code_subagents::pool::{DispatchOptions, SubagentPool, SubagentPoolOptions};

    fn alive(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .and_then(|(_, r)| r.split_whitespace().next().map(String::from))
            })
            .is_some_and(|state| state != "Z" && state != "X")
    }

    fn kill(pid: u32) {
        // SAFETY: test cleanup of processes this test started.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }

    /// A grandchild (whose parent is not this process) is not walked: its
    /// own child survives `kill_trees`.
    #[test]
    fn a_grandchild_is_not_walked() {
        let mut parent = Command::new("sh")
            .arg("-c")
            .arg("sh -c 'sleep 300; :' & echo $!; wait")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(parent.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let grandchild: u32 = line.trim().parse().unwrap();
        let mut sleeper = None;
        for _ in 0..200 {
            sleeper = ProcTable
                .rows()
                .into_iter()
                .find(|&(_, ppid, _)| ppid == grandchild)
                .map(|(pid, _, _)| pid);
            if sleeper.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let sleeper = sleeper.expect("the grandchild's sleep runs");
        assert_eq!(
            plan_tree_kill(&ProcTable.rows(), grandchild, std::process::id()),
            (vec![], vec![])
        );
        kill_trees(&[grandchild], &ProcTable);
        std::thread::sleep(Duration::from_millis(50));
        assert!(alive(sleeper), "not our child: nothing below it is killed");
        assert!(alive(grandchild));
        // Our own direct child is walked (the positive control).
        let (pids, _) = plan_tree_kill(&ProcTable.rows(), parent.id(), std::process::id());
        assert!(
            pids.contains(&grandchild) && pids.contains(&sleeper),
            "{pids:?}"
        );
        kill(sleeper);
        kill(grandchild);
        let _ = parent.kill();
        let _ = parent.wait();
    }

    /// A stale (pid, generation) — a slot or monitor that still holds the pid
    /// of a reaped child — never kills the newer child of ours that reused
    /// the pid; the newer child's own generation does. A pid-only kill of a
    /// pid our child holds is refused too. Here a fresh process stands in for
    /// the newer child.
    #[test]
    fn a_stale_generation_never_kills_the_child_that_reused_its_pid() {
        let mut newer = Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = newer.id();
        let old = child_spawned(pid);
        child_exited(pid, old);
        assert!(!is_live_child(ChildPid {
            pid,
            generation: old
        }));
        let new = child_spawned(pid);
        assert_ne!(old, new);
        assert_eq!(live_generation(pid), new);

        kill_child_trees(&[ChildPid {
            pid,
            generation: old,
        }]);
        kill_process_tree(pid);
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            newer.try_wait().unwrap().is_none(),
            "the newer child was not killed by a stale kill"
        );

        kill_child_trees(&[ChildPid {
            pid,
            generation: new,
        }]);
        let status = newer.wait().unwrap();
        assert!(!status.success(), "its own generation kills it");
        child_exited(pid, new);
        assert_eq!(live_generation(pid), 0, "the entry goes with the child");
    }

    /// The pool marks its child exited before it reaps it: a cancel or
    /// dispose that comes after the exit finds the pid reaped and leaves it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_pool_marks_its_child_exited_before_the_reap() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("quick.sh");
        let pid_file = dir.path().join("child.pid");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho $$ > \"{}\"\nexit 1\n", pid_file.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let pool = SubagentPool::new(SubagentPoolOptions {
            executable: script,
            cwd: Some(dir.path().to_path_buf()),
            ..Default::default()
        });
        let _ = pool
            .dispatch(
                "quick",
                DispatchOptions {
                    force_agent: Some("explore".into()),
                    task_id: Some("quick-run".into()),
                    ..Default::default()
                },
            )
            .await;
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            live_generation(pid),
            0,
            "exited and reaped: no live generation, so a later kill is refused"
        );
        // A late cancel has nothing to do and signals nothing.
        assert!(!pool.cancel("quick-run"));
        pool.dispose();
    }
}
