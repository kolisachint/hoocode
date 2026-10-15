//! T3.3: the attached view's header row and the prompt's stand-in. The header
//! names the run, its state and how to go back; the hint says the view is
//! read-only and takes Esc.

use hoocode_code_tui_widgets::attach_view::{AttachHeader, AttachPrompt};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};

use crate::support::{lock, strip_all};

#[test]
fn the_header_names_the_run_its_state_and_how_to_go_back() {
    let _g = lock();
    let mut header = AttachHeader::new("explore", "Find the footer code", "running");
    let shown = strip_all(&header.render(80));
    assert_eq!(
        shown.trim_end(),
        "◂ explore · Find the footer code · running · Esc back"
    );
    assert_golden!("attach_header_80", render_golden(&mut header, 80));
}

#[test]
fn a_long_description_gives_way_before_the_state_or_the_way_back() {
    let _g = lock();
    let mut header = AttachHeader::new(
        "general-purpose",
        "Read every file under crates/hoocode-code-tui-widgets and summarise each one",
        "complete",
    );
    let shown = strip_all(&header.render(60));
    assert!(shown.contains("complete · Esc back"), "{shown}");
    assert!(shown.contains('…'), "{shown}");
    assert!(shown.trim_end().chars().count() <= 60, "{shown}");
    assert_golden!("attach_header_60_long", render_golden(&mut header, 60));
}

#[test]
fn a_header_narrower_than_its_fixed_parts_still_fits_the_row() {
    let _g = lock();
    let mut header = AttachHeader::new("general-purpose", "Compute a product", "complete");
    for width in [4_u16, 12, 30] {
        let shown = strip_all(&header.render(width));
        assert!(
            shown.trim_end().chars().count() <= width as usize,
            "width {width}: {shown:?}"
        );
    }
}

#[test]
fn a_state_change_shows_in_the_header() {
    let _g = lock();
    let mut header = AttachHeader::new("explore", "Find the footer code", "running");
    header.set_state("interrupted");
    let shown = strip_all(&header.render(80));
    assert!(shown.contains("· interrupted · Esc back"), "{shown}");
}

#[test]
fn the_prompt_hint_says_the_view_is_read_only_and_names_the_way_back() {
    let _g = lock();
    let mut prompt = AttachPrompt::new();
    let shown = strip_all(&prompt.render(80));
    assert!(shown.contains("Read-only"), "{shown}");
    assert!(shown.contains("goes back"), "{shown}");
    assert_golden!("attach_prompt_80", render_golden(&mut prompt, 80));
}

#[test]
fn escape_takes_the_view_back_once_and_typing_does_not() {
    let _g = lock();
    let mut prompt = AttachPrompt::new();
    prompt.handle_input("a");
    assert!(!prompt.take_detach(), "typed text is not a way back");
    prompt.handle_input("\x1b");
    assert!(prompt.take_detach());
    assert!(!prompt.take_detach(), "one press, one answer");
}
