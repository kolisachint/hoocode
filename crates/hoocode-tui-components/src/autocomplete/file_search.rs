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
use std::thread;

use super::{FileFinder, FileMatch};

/// Matches asked for per query; the provider shows at most 20 of them.
const MAX_RESULTS: usize = 100;

/// `(base directory, query)`.
type Key = (PathBuf, String);

#[derive(Default)]
struct State {
    /// Bumped each time the wanted query changes.
    generation: u64,
    /// The query the current generation asked for.
    wanted: Option<Key>,
    /// Whether the walk for the current generation is still running.
    walking: bool,
    /// Matches for `wanted`, once its walk has finished.
    ready: Option<Vec<FileMatch>>,
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
        thread::Builder::new()
            .name("autocomplete-files".into())
            .spawn(move || run_worker(&finder, &inbox, &worker_state))
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
        if state.wanted.as_ref() == Some(&key) {
            if let Some(ready) = &state.ready {
                return ready.clone();
            }
        } else {
            state.generation += 1;
            state.wanted = Some(key.clone());
            state.ready = None;
            state.walking = true;
            let request = Request {
                generation: state.generation,
                key,
            };
            // The worker outlives the provider's requests; a send error only
            // means the worker is gone, and then there is nothing to wait for.
            let _ = self.requests.send(request);
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
        lock(&self.state).walking
    }

    /// Whether a walk for the current query has finished since the last call.
    pub fn take_completed(&self) -> bool {
        std::mem::take(&mut lock(&self.state).completed)
    }
}

fn run_worker(finder: &FileFinder, inbox: &Receiver<Request>, state: &Mutex<State>) {
    while let Ok(mut request) = inbox.recv() {
        // Only the newest query matters: skip the ones typed past meanwhile.
        while let Ok(newer) = inbox.try_recv() {
            request = newer;
        }
        let (base, query) = &request.key;
        let found = finder(base, query, MAX_RESULTS);
        let mut state = lock(state);
        if request.generation == state.generation {
            state.last = Some((base.clone(), found.clone()));
            state.ready = Some(found);
            state.walking = false;
            state.completed = true;
        }
    }
}
