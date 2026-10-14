//! Deleting a session takes its subagent transcripts (`<session>/subagents/`)
//! with it. The session file itself is deleted by the picker, not here.

use hoocode_code_tui_selectors::session_selector::delete_subagent_transcripts;

fn session_with_transcripts(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let session = dir.join("2026-10-14T00-00-00-000Z_abc.jsonl");
    std::fs::write(&session, "{}\n").unwrap();
    let transcripts = hoocode_code_session::subagents_dir(&session);
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join("run-1.jsonl"), "{}\n").unwrap();
    (session, transcripts)
}

#[test]
fn deleting_a_session_takes_its_subagent_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let (session, transcripts) = session_with_transcripts(dir.path());
    delete_subagent_transcripts(&session);
    assert!(!transcripts.exists(), "the transcripts go with the session");
    assert!(
        session.exists(),
        "the session file is the picker's to delete"
    );
}

#[test]
fn a_session_without_transcripts_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("plain.jsonl");
    std::fs::write(&session, "{}\n").unwrap();
    delete_subagent_transcripts(&session);
    assert!(session.exists());
}
