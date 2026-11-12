//! Port of `test/ask-options.test.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, look, strip, Look};
use hoocode_code_tools_optin::ask_options::{AskOption, AskQuestion};
use hoocode_code_tui_selectors::ask_options::{AskOptionsComponent, AskOptionsOptions};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

const UP: &str = "\x1b[A";
const DOWN: &str = "\x1b[B";
const RIGHT: &str = "\x1b[C";
const LEFT: &str = "\x1b[D";
const ENTER: &str = "\r";
const ESC: &str = "\x1b";

fn option(label: &str, description: &str) -> AskOption {
    AskOption {
        label: label.into(),
        description: Some(description.into()),
        recommended: false,
    }
}

fn questions() -> Vec<AskQuestion> {
    vec![
        AskQuestion {
            question: "where should retry logic live?".into(),
            short: Some("retry lives in".into()),
            detail: Some("the http client is shared by every provider adapter".into()),
            allow_custom: true,
            options: vec![
                option("hoocode-ai", "unified provider client"),
                option("agent-core", "runtime wrap each tool call"),
            ],
        },
        AskQuestion {
            question: "backoff strategy?".into(),
            short: Some("backoff".into()),
            detail: None,
            allow_custom: true,
            options: vec![
                option("exponential + jitter", "100ms x2"),
                option("fixed 1s", "simple"),
            ],
        },
    ]
}

type Submitted = Rc<RefCell<Option<Vec<String>>>>;

fn pane() -> (AskOptionsComponent, Submitted, Rc<RefCell<bool>>) {
    let submitted: Submitted = Rc::default();
    let cancelled = Rc::new(RefCell::new(false));
    let (s, c) = (submitted.clone(), cancelled.clone());
    let pane = AskOptionsComponent::new(
        questions(),
        Box::new(move |answers| *s.borrow_mut() = Some(answers)),
        Box::new(move || *c.borrow_mut() = true),
        AskOptionsOptions::default(),
    );
    (pane, submitted, cancelled)
}

fn out(c: &mut AskOptionsComponent) -> String {
    strip(&c.render(80))
}

fn answers(v: &[&str]) -> Option<Vec<String>> {
    Some(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn renders_header_question_options_and_custom_row() {
    let _g = lock();
    let (mut c, _, _) = pane();
    let out = out(&mut c);
    assert!(!out.contains("Question"), "{out}");
    assert!(
        out.contains("where should retry logic live?   1 of 2"),
        "{out}"
    );
    assert!(out.contains("the http client is shared by every provider adapter"));
    assert!(out.contains("1  hoocode-ai"));
    assert!(out.contains("2  agent-core"));
    assert!(out.contains("or type your own answer"));
    assert!(out.contains("1–2 pick"));
    assert!(out.contains("tab next question"));
    assert!(out.contains("esc skip"));
    assert!(!out.contains("pick 1"), "{out}");
}

#[test]
fn moves_the_cursor_and_clamps_at_both_ends() {
    let _g = lock();
    let (mut c, _, _) = pane();
    c.handle_input(DOWN);
    let o = out(&mut c);
    assert!(o.contains("› 2  agent-core"), "{o}");
    assert!(o.contains("1–2 pick"), "{o}");
    c.handle_input(UP);
    assert!(out(&mut c).contains("› 1  hoocode-ai"));
    c.handle_input(UP);
    assert!(
        out(&mut c).contains("› 1  hoocode-ai"),
        "up at the top stays put"
    );
}

#[test]
fn advances_with_right_and_submits_on_the_last_step() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    c.handle_input(RIGHT);
    let o = out(&mut c);
    assert!(o.contains("retry lives in"), "{o}");
    assert!(o.contains("hoocode-ai"));
    assert!(o.contains("backoff strategy?   2 of 2"));
    c.handle_input(DOWN);
    c.handle_input(RIGHT);
    assert_eq!(*submitted.borrow(), answers(&["hoocode-ai", "fixed 1s"]));
}

#[test]
fn quick_picks_with_number_keys() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    c.handle_input("2");
    c.handle_input("1");
    assert_eq!(
        *submitted.borrow(),
        answers(&["agent-core", "exponential + jitter"])
    );
}

#[test]
fn steps_back_with_left() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    c.handle_input("1");
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
    c.handle_input(LEFT);
    assert!(out(&mut c).contains("where should retry logic live?   1 of 2"));
    c.handle_input("2");
    c.handle_input("1");
    assert_eq!(
        *submitted.borrow(),
        answers(&["agent-core", "exponential + jitter"])
    );
}

fn type_custom(c: &mut AskOptionsComponent) {
    c.handle_input(DOWN);
    c.handle_input(DOWN);
    for ch in "per-provider".chars() {
        c.handle_input(&ch.to_string());
    }
}

#[test]
fn accepts_a_typed_custom_answer() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    type_custom(&mut c);
    assert!(out(&mut c).contains("per-provider"));
    c.handle_input(RIGHT);
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
    c.handle_input("2");
    assert_eq!(*submitted.borrow(), answers(&["per-provider", "fixed 1s"]));
}

#[test]
fn gives_the_arrows_to_the_text_once_typed() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    type_custom(&mut c);
    c.handle_input(LEFT);
    let o = out(&mut c);
    assert!(o.contains("where should retry logic live?   1 of 2"), "{o}");
    assert!(o.contains("per-provider"));
    c.handle_input(RIGHT);
    assert!(out(&mut c).contains("where should retry logic live?   1 of 2"));
    c.handle_input(RIGHT);
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
    c.handle_input("2");
    assert_eq!(*submitted.borrow(), answers(&["per-provider", "fixed 1s"]));
}

#[test]
fn commits_a_custom_answer_with_enter_anywhere() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    type_custom(&mut c);
    c.handle_input(LEFT);
    c.handle_input(LEFT);
    c.handle_input(ENTER);
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
    c.handle_input("2");
    assert_eq!(*submitted.borrow(), answers(&["per-provider", "fixed 1s"]));
}

#[test]
fn does_not_submit_an_empty_custom_answer() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    c.handle_input(DOWN);
    c.handle_input(DOWN);
    c.handle_input(RIGHT);
    assert!(out(&mut c).contains("where should retry logic live?   1 of 2"));
    assert_eq!(*submitted.borrow(), None);
}

#[test]
fn question_card_golden() {
    let _g = lock();
    let (mut c, _, _) = pane();
    assert_golden!("ask_card_100", render_golden(&mut c, 100));
}

#[test]
fn recommended_tag_shows() {
    let _g = lock();
    let submitted: Submitted = Rc::default();
    let mut pane = AskOptionsComponent::new(
        vec![AskQuestion {
            question: "strategy?".into(),
            short: None,
            detail: None,
            allow_custom: true,
            options: vec![
                AskOption {
                    label: "exponential".into(),
                    description: Some("doubling".into()),
                    recommended: true,
                },
                option("fixed", "same wait"),
            ],
        }],
        Box::new(move |answers| *submitted.borrow_mut() = Some(answers)),
        Box::new(|| {}),
        AskOptionsOptions::default(),
    );
    let out = strip(&pane.render(100));
    assert!(out.contains("1  exponential"), "{out}");
    assert!(out.contains("recommended"), "{out}");
    assert!(out.contains("or type your own answer"), "{out}");
}

#[test]
fn custom_row_number_moves_without_answering() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    // Row 3 is the answer row: its number only moves there.
    c.handle_input("3");
    assert_eq!(*submitted.borrow(), None);
    let rendered = out(&mut c);
    // On the custom row the input caret marks the answer row (the input
    // row carries no `›` marker since the P6 pane grammar).
    assert!(rendered.contains('▏'), "{rendered}");
    assert!(!rendered.contains("or type your own answer"), "{rendered}");
    // Typing then sends with enter on the next step.
    for key in ["m", "i", "n"] {
        c.handle_input(key);
    }
    assert!(out(&mut c).contains("min"));
}
#[test]
fn escape_skips_the_question() {
    let _g = lock();
    let (mut c, submitted, cancelled) = pane();
    c.handle_input(ESC);
    assert!(*cancelled.borrow());
    assert_eq!(*submitted.borrow(), None);
}

#[test]
fn a_letter_on_an_option_starts_the_answer() {
    let _g = lock();
    let (mut c, submitted, _) = pane();
    c.handle_input("z");
    assert!(out(&mut c).contains('z'));
    c.handle_input(ENTER);
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
    c.handle_input("2");
    assert_eq!(*submitted.borrow(), answers(&["z", "fixed 1s"]));
}

#[test]
fn tab_advances_like_right() {
    let _g = lock();
    let (mut c, _, _) = pane();
    c.handle_input("\t");
    assert!(out(&mut c).contains("backoff strategy?   2 of 2"));
}

#[test]
fn escape_skips_when_no_option_can_answer() {
    let _g = lock();
    let submitted: Submitted = Rc::default();
    let cancelled = Rc::new(RefCell::new(false));
    let (s, c) = (submitted.clone(), cancelled.clone());
    let mut pane = AskOptionsComponent::new(
        vec![AskQuestion {
            question: "free text?".into(),
            short: None,
            detail: None,
            allow_custom: false,
            options: vec![],
        }],
        Box::new(move |answers| *s.borrow_mut() = Some(answers)),
        Box::new(move || *c.borrow_mut() = true),
        AskOptionsOptions::default(),
    );
    pane.handle_input(ESC);
    assert!(*cancelled.borrow());
    assert_eq!(*submitted.borrow(), None);
}

#[test]
fn question_card_light_golden() {
    let _g = lock();
    look(Look::Light);
    let (mut c, _, _) = pane();
    assert_golden!("ask_card_light_100", render_golden(&mut c, 100));
}

#[test]
fn question_card_mono_golden() {
    let _g = lock();
    look(Look::Mono);
    let (mut c, _, _) = pane();
    assert_golden!("ask_card_mono_100", render_golden(&mut c, 100));
}
