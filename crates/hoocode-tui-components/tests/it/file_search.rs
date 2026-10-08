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

/// Like `gated_provider`, but the finder always lists the same two files.
fn listed_provider() -> (CombinedAutocompleteProvider, Gate) {
    let (release, inbox): (Sender<()>, Receiver<()>) = mpsc::channel();
    let inbox = Mutex::new(inbox);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&asked);
    let finder = Box::new(move |_base: &Path, query: &str, _max: usize| {
        recorded.lock().unwrap().push(query.to_string());
        inbox.lock().unwrap().recv().unwrap();
        ["apple.txt", "banana.txt"]
            .into_iter()
            .map(|path| FileMatch {
                path: path.to_string(),
                is_directory: false,
            })
            .collect()
    });
    let provider = CombinedAutocompleteProvider::new(vec![], std::env::temp_dir(), Some(finder));
    (provider, Gate { release, asked })
}

fn sorted(r: &Option<AutocompleteSuggestions>) -> Vec<String> {
    let mut v = values(r);
    v.sort();
    v
}

#[test]
fn typing_past_a_query_keeps_the_previous_list_until_the_walk_finishes() {
    let (provider, gate) = listed_provider();
    assert!(
        at(&provider, "@a").is_none(),
        "nothing to show before the first walk"
    );
    wait_until("the walk for a", || gate.asked.lock().unwrap().len() == 1);
    gate.release.send(()).unwrap();
    wait_until("the walk for a to finish", || !provider.file_walk_pending());
    assert_eq!(sorted(&at(&provider, "@a")), ["@apple.txt", "@banana.txt"]);

    // "ap" starts a walk that blocks. The previous list stays, narrowed by "ap".
    assert_eq!(sorted(&at(&provider, "@ap")), ["@apple.txt"]);
    assert!(provider.file_walk_pending());
    wait_until("the walk for ap", || gate.asked.lock().unwrap().len() == 2);
    assert_eq!(sorted(&at(&provider, "@ap")), ["@apple.txt"]);

    gate.release.send(()).unwrap();
    wait_until("the walk for ap to finish", || {
        !provider.file_walk_pending()
    });
    assert_eq!(sorted(&at(&provider, "@ap")), ["@apple.txt"]);
}

/// An editor with an `@sr` walk blocked, dismissed with Escape, and then the
/// walk released. Returns the editor and the gate.
fn dismissed_during_walk() -> (hoocode_tui_components::Editor, Gate) {
    use crate::editor::{ed, flush_debounce, type_str};
    use hoocode_tui_render::Component;

    let (provider, gate) = gated_provider();
    let mut editor = ed();
    editor.set_autocomplete_provider(Box::new(provider));
    type_str(&mut editor, "@sr");
    flush_debounce(&mut editor);
    wait_until("the walk to start", || {
        gate.asked.lock().unwrap().len() == 1
    });

    editor.handle_input("\x1b"); // Escape, while the list is closed
    gate.release.send(()).unwrap();
    // Give the finished walk time to report; the list must stay closed.
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        editor.poll_autocomplete();
        assert!(
            !editor.is_showing_autocomplete(),
            "a dismissed list reopened"
        );
        thread::sleep(Duration::from_millis(5));
    }
    (editor, gate)
}

#[test]
fn escape_during_a_walk_keeps_the_list_closed_when_the_walk_finishes() {
    let _ = dismissed_during_walk();
}

#[test]
fn typing_a_new_query_after_a_dismissal_opens_the_list_again() {
    use crate::editor::{flush_debounce, type_str};

    let (mut editor, gate) = dismissed_during_walk();
    type_str(&mut editor, "c"); // "@src": a new query re-arms the list
    flush_debounce(&mut editor);
    wait_until("the walk for src", || gate.asked.lock().unwrap().len() == 2);
    gate.release.send(()).unwrap();
    wait_until("the list to open", || {
        editor.poll_autocomplete();
        editor.is_showing_autocomplete()
    });
}
