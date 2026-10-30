//! Runs the `@file` finder off the UI thread.
//!
//! A [`FileFinder`] can take a long time on a big tree, so it runs on one
//! worker thread. [`FileSearch::results`] never waits. It returns the matches
//! for the query asked for once their walk has finished. While a newer walk
//! runs, it returns the matches of the last finished walk for the same base,
//! so the list does not flicker as the user types; the caller narrows them by
//! the new query. Each new query bumps a generation counter, and a walk whose
//! generation is no longer current drops its matches, so a query the user has
//! typed past never shows its results. When the walk for the current query
//! finishes, [`FileSearch::take_completed`] reports it once, and the editor
//! asks the provider again.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{FileFinder, FileMatch};

/// Matches asked for per query; the provider shows at most 20 of them.
const MAX_RESULTS: usize = 100;

/// `(base directory, query)`.
type Key = (PathBuf, String);

/// The query the current generation asked for. Its walk is running until `ready` is set.
struct Wanted {
    key: Key,
    /// Matches for `key`, once its walk has finished.
    ready: Option<Vec<FileMatch>>,
}

#[derive(Default)]
struct State {
    /// Bumped each time the wanted query changes.
    generation: u64,
    wanted: Option<Wanted>,
    /// Matches of the last finished walk, with its base. Shown while a newer
    /// walk runs; cleared by [`FileSearch::forget`].
    last: Option<(PathBuf, Vec<FileMatch>)>,
    /// Set when `ready` is stored, cleared by [`FileSearch::take_completed`].
    completed: bool,
}

struct Request {
    generation: u64,
    key: Key,
}

/// Handle to the worker thread that owns the finder.
pub struct FileSearch {
    state: Arc<Mutex<State>>,
    requests: Sender<Request>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FileSearch {
    /// Starts the worker. Returns `None` if the thread cannot be started, in
    /// which case `@` completion offers nothing.
    pub fn new(finder: FileFinder) -> Option<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let (requests, inbox) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        hoocode_runtime::spawn_named_thread("autocomplete-files", move || {
            run_worker(&finder, &inbox, &worker_state)
        })
        .ok()?;
        Some(Self { state, requests })
    }

    /// The matches for `query` under `base`. Once that query's walk has
    /// finished, these are its matches. Until then, they are the last finished
    /// walk's matches for `base`, or nothing. A query that is not the wanted
    /// one starts its walk at once; this never waits.
    pub fn results(&self, base: &Path, query: &str) -> Vec<FileMatch> {
        let key = (base.to_path_buf(), query.to_string());
        let mut state = lock(&self.state);
        if state.wanted.as_ref().is_some_and(|w| w.key == key) {
            if let Some(ready) = state.wanted.as_ref().and_then(|w| w.ready.as_ref()) {
                return ready.clone();
            }
        } else {
            state.generation += 1;
            state.wanted = Some(Wanted {
                key: key.clone(),
                ready: None,
            });
            let request = Request {
                generation: state.generation,
                key,
            };
            // A send error means the worker is gone: no walk will settle this
            // query, so it reads as no matches now, not pending forever.
            if self.requests.send(request).is_err() {
                if let Some(wanted) = state.wanted.as_mut() {
                    wanted.ready = Some(Vec::new());
                }
                state.completed = true;
            }
        }
        match &state.last {
            Some((last_base, matches)) if last_base == base => matches.clone(),
            _ => Vec::new(),
        }
    }

    /// Drops the last finished walk's matches. Called when `@` completion ends,
    /// so a later `@` does not show matches from an earlier one.
    pub fn forget(&self) {
        lock(&self.state).last = None;
    }

    /// Whether a walk for the current query is still running.
    pub fn walk_pending(&self) -> bool {
        lock(&self.state)
            .wanted
            .as_ref()
            .is_some_and(|w| w.ready.is_none())
    }

    /// Whether a walk for the current query has finished since the last call.
    pub fn take_completed(&self) -> bool {
        std::mem::take(&mut lock(&self.state).completed)
    }
}

/// Settles the wanted walk when the worker stops inside it (a panicking finder
/// unwinds through here). The worker is dying, so no walk will settle the
/// wanted query: it is settled whatever its generation, which may already be
/// a newer query typed past the walk. The query then reads as no matches, so
/// `walk_pending` cannot stay true and the editor cannot poll forever.
struct WalkGuard<'a> {
    state: &'a Mutex<State>,
    finished: bool,
}

impl Drop for WalkGuard<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut state = lock(self.state);
        if let Some(wanted) = &mut state.wanted {
            wanted.ready.get_or_insert_with(Vec::new);
            state.completed = true;
        }
    }
}

fn run_worker(finder: &FileFinder, inbox: &Receiver<Request>, state: &Mutex<State>) {
    while let Ok(mut request) = inbox.recv() {
        // Only the newest query matters: skip the ones typed past meanwhile.
        while let Ok(newer) = inbox.try_recv() {
            request = newer;
        }
        let (base, query) = &request.key;
        let mut guard = WalkGuard {
            state,
            finished: false,
        };
        let found = finder(base, query, MAX_RESULTS);
        guard.finished = true;
        let mut state = lock(state);
        if request.generation == state.generation {
            state.last = Some((base.clone(), found.clone()));
            if let Some(wanted) = &mut state.wanted {
                wanted.ready = Some(found);
            }
            state.completed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn search_with(state: Arc<Mutex<State>>, requests: Sender<Request>) -> FileSearch {
        FileSearch { state, requests }
    }

    #[test]
    fn a_worker_that_dies_mid_walk_settles_its_query() {
        let finder: FileFinder = Box::new(|_, _, _| -> Vec<FileMatch> { panic!("walk died") });
        let (requests, inbox) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::default()));
        let search = search_with(Arc::clone(&state), requests);
        assert!(search.results(Path::new("base"), "q").is_empty());
        assert!(search.walk_pending());

        let died = catch_unwind(AssertUnwindSafe(|| run_worker(&finder, &inbox, &state)));
        assert!(died.is_err());
        // The walk never finished, but its query now reads settled: no matches.
        assert!(!search.walk_pending());
        assert!(search.take_completed());
        assert!(search.results(Path::new("base"), "q").is_empty());
    }

    #[test]
    fn a_newer_query_sent_during_a_panicking_walk_settles() {
        let (requests, inbox) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::default()));
        // The finder for "old" is still running when the user types "new", then it dies.
        let typed_past = search_with(Arc::clone(&state), requests.clone());
        let finder: FileFinder = Box::new(move |_, query: &str, _| {
            if query == "old" {
                assert!(typed_past.results(Path::new("base"), "new").is_empty());
                panic!("walk died");
            }
            Vec::new()
        });
        let search = search_with(Arc::clone(&state), requests);
        assert!(search.results(Path::new("base"), "old").is_empty());

        let died = catch_unwind(AssertUnwindSafe(|| run_worker(&finder, &inbox, &state)));
        assert!(died.is_err());
        // "new" was never walked, but the dying worker settles it: no pending forever.
        assert!(!search.walk_pending());
        assert!(search.take_completed());
        assert!(search.results(Path::new("base"), "new").is_empty());
        assert!(!search.walk_pending());
    }

    #[test]
    fn a_query_sent_to_a_dead_worker_does_not_wait_forever() {
        let (requests, inbox) = mpsc::channel::<Request>();
        drop(inbox);
        let search = search_with(Arc::new(Mutex::new(State::default())), requests);
        assert!(search.results(Path::new("base"), "q").is_empty());
        assert!(!search.walk_pending());
    }

    #[test]
    fn a_finished_walk_keeps_its_own_matches() {
        let finder: FileFinder = Box::new(|_, _, _| {
            vec![FileMatch {
                path: "src/lib.rs".to_string(),
                is_directory: false,
            }]
        });
        let (requests, inbox) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::default()));
        let search = search_with(Arc::clone(&state), requests);
        assert!(search.results(Path::new("base"), "lib").is_empty());
        // One request, then the channel closes and the worker's loop ends.
        let request = inbox.recv().expect("the query was sent");
        run_worker_once(&finder, request, &state);
        let guard = lock(&state);
        let wanted = guard.wanted.as_ref().expect("a wanted query");
        assert_eq!(wanted.ready.as_ref().map(Vec::len), Some(1));
    }

    /// One request through the worker's own loop: the channel holds only it.
    fn run_worker_once(finder: &FileFinder, request: Request, state: &Mutex<State>) {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(request);
        drop(tx);
        run_worker(finder, &rx, state);
    }
}
