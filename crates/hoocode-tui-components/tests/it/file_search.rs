//! `@file` suggestions run their walk off the calling thread (reliability 1.4):
//! typing never waits on the finder, and matches for a query the user has
//! typed past are dropped.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hoocode_tui_components::{
    AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider, FileMatch,
};

/// A provider whose finder answers `"<query>.txt"`, but only after the test
/// sends a release token for that walk. Each query it is asked for is recorded.
struct Gate {
    release: Sender<()>,
    asked: Arc<Mutex<Vec<String>>>,
}

fn gated_provider() -> (CombinedAutocompleteProvider, Gate) {
    let (release, inbox): (Sender<()>, Receiver<()>) = mpsc::channel();
    let inbox = Mutex::new(inbox);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&asked);
    let finder = Box::new(move |_base: &Path, query: &str, _max: usize| {
        recorded.lock().unwrap().push(query.to_string());
        inbox.lock().unwrap().recv().unwrap();
        vec![FileMatch {
            path: format!("{query}.txt"),
            is_directory: false,
        }]
    });
    let provider = CombinedAutocompleteProvider::new(vec![], std::env::temp_dir(), Some(finder));
    (provider, Gate { release, asked })
}

fn at(p: &CombinedAutocompleteProvider, text: &str) -> Option<AutocompleteSuggestions> {
    p.get_suggestions(&[text.to_string()], 0, text.chars().count(), false)
}

fn values(r: &Option<AutocompleteSuggestions>) -> Vec<String> {
    r.as_ref()
        .map(|r| r.items.iter().map(|i| i.value.clone()).collect())
        .unwrap_or_default()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn typing_does_not_wait_for_the_walk() {
    let (provider, gate) = gated_provider();

    // The finder is blocked on its token, yet the keystroke returns at once.
    let started = Instant::now();
    assert!(at(&provider, "@src").is_none());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "get_suggestions waited for the walk"
    );
    assert!(provider.file_walk_pending());
    assert!(!provider.take_ready());

    // Asking again for the same query does not start a second walk.
    wait_until("the walk to start", || {
        gate.asked.lock().unwrap().len() == 1
    });
    assert!(at(&provider, "@src").is_none());
    thread::sleep(Duration::from_millis(20));
    assert_eq!(*gate.asked.lock().unwrap(), ["src"]);

    gate.release.send(()).unwrap();
    wait_until("the walk to finish", || !provider.file_walk_pending());
    assert!(provider.take_ready(), "the finished walk is reported");
    assert!(!provider.take_ready(), "and reported only once");
    assert_eq!(values(&at(&provider, "@src")), ["@src.txt"]);
}

#[test]
fn matches_for_a_query_typed_past_are_dropped() {
    let (provider, gate) = gated_provider();

    // "a" starts its walk, then the user types on to "ab" while it runs.
    assert!(at(&provider, "@a").is_none());
    wait_until("the walk for a to start", || {
        gate.asked.lock().unwrap().len() == 1
    });
    assert!(at(&provider, "@ab").is_none());

    // The walk for "a" finishes now. Its matches must not show for "ab", and
    // it must not report completion: "ab" is the query still wanted.
    gate.release.send(()).unwrap();
    wait_until("the walk for ab to start", || {
        gate.asked.lock().unwrap().len() == 2
    });
    assert!(provider.file_walk_pending());
    assert!(
        at(&provider, "@ab").is_none(),
        "matches for a query typed past were shown"
    );
    assert!(!provider.take_ready(), "a stale walk reported completion");

    gate.release.send(()).unwrap();
    wait_until("the walk for ab to finish", || {
        !provider.file_walk_pending()
    });
    assert_eq!(values(&at(&provider, "@ab")), ["@ab.txt"]);
    assert_eq!(*gate.asked.lock().unwrap(), ["a", "ab"]);
}

#[test]
fn the_list_opens_when_the_walk_finishes_without_a_keystroke() {
    use crate::editor::{ed, flush_debounce, type_str};

    let (provider, gate) = gated_provider();
    let mut editor = ed();
    editor.set_autocomplete_provider(Box::new(provider));

    type_str(&mut editor, "@sr");
    flush_debounce(&mut editor);
    wait_until("the walk to start", || {
        gate.asked.lock().unwrap().len() == 1
    });
    assert!(
        !editor.is_showing_autocomplete(),
        "no list while the walk runs"
    );

    // Nothing else is typed: the editor must open the list on its own.
    gate.release.send(()).unwrap();
    wait_until("the list to open", || {
        editor.poll_autocomplete();
        editor.is_showing_autocomplete()
    });
}
