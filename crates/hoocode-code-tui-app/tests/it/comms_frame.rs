//! The comms frame (M2) as a component: the 3-row heavy frame in READY and
//! IN FLIGHT, with a real borderless editor for the input row so the
//! placeholder shows. The theme lock pins the palette; the data provider
//! points at a scratch dir so no machine paths reach the goldens.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use hoocode_code_tui_app::comms_frame::{
    CommsFrame, CommsState, GlyphStyle, InputStyle, MissionOutcome, COMMS_LOST_PLACEHOLDER,
    IN_FLIGHT_PLACEHOLDER, READY_PLACEHOLDER,
};
use hoocode_code_tui_app::footer::{FooterModel, FooterSource, ToolOutputView};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_theme::get_editor_theme;
use hoocode_code_tui_widgets::radar::FlightMark;
use hoocode_tui_components::{Editor, EditorHost, EditorOptions, FrameBorderStyle};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::ComponentHandle;

use crate::support::{lock, plain};

/// A fixed session: model, usage and context (the HUD golden's stub).
struct Stub;

impl FooterSource for Stub {
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64) {
        (48_200, 9_750, 120_000, 3_400, 1.23)
    }
    fn context_usage(&self) -> Option<(u64, Option<f64>)> {
        Some((200_000, Some(42.0)))
    }
    fn model(&self) -> Option<FooterModel> {
        Some(FooterModel {
            id: "claude-haiku-5-5".into(),
            provider: "anthropic".into(),
            context_window: 200_000,
            reasoning: true,
        })
    }
    fn thinking_level(&self) -> String {
        "medium".into()
    }
    fn cwd(&self) -> String {
        "/work/hoocode".into()
    }
    fn display_name(&self) -> String {
        "amber-harbor".into()
    }
    fn reserve_tokens(&self) -> u64 {
        16_384
    }
    fn is_using_oauth(&self) -> bool {
        false
    }
}

/// The prompt editor as the app builds it: no border, `▸` prefix, the
/// palette's faint placeholder. The `set_input` closure styles it per state.
fn prompt_editor() -> Rc<RefCell<Editor>> {
    let mut editor = Editor::new(
        EditorHost::default(),
        get_editor_theme(),
        EditorOptions {
            border: Some(FrameBorderStyle::None),
            padding_x: Some(1),
            autocomplete_max_visible: None,
        },
    );
    editor.prompt_prefix = "▸".into();
    editor.placeholder_color = Box::new(|text: &str| {
        hoocode_tui_ratatui::paint(hoocode_tui_ratatui::Palette::current().faint(), text)
    });
    Rc::new(RefCell::new(editor))
}

fn frame(state: CommsState, view: ToolOutputView, mode: &str) -> CommsFrame {
    frame_painted(state, view, mode, None, (Vec::new(), Vec::new()))
}

fn frame_painted(
    state: CommsState,
    view: ToolOutputView,
    mode: &str,
    paint: Option<hoocode_code_tui_app::hull_paint::HullPaint>,
    queue: (Vec<String>, Vec<String>),
) -> CommsFrame {
    let editor = prompt_editor();
    let input = editor.clone();
    let handle: ComponentHandle = editor.clone();
    // A fixed path: no machine paths reach the goldens.
    let data = FooterDataProvider::new("/work/hoocode");
    data.set_active_mode(mode);
    data.set_hull_paint(paint);
    CommsFrame::new(
        handle,
        move || state.clone(),
        move || queue.clone(),
        move |style: InputStyle| {
            let mut editor = input.borrow_mut();
            editor.placeholder = style.placeholder;
            let glyph = style.glyph;
            editor.prompt_color =
                Box::new(move |text: &str| hoocode_tui_ratatui::paint(glyph.style(), text));
        },
        Box::new(Stub),
        data,
        view,
    )
}

fn ready() -> CommsFrame {
    frame(CommsState::Ready, ToolOutputView::Peek, "build")
}

fn in_flight() -> CommsFrame {
    frame(
        CommsState::InFlight {
            elapsed: Some(Duration::from_secs(4)),
            step: Some(1),
        },
        ToolOutputView::Peek,
        "build",
    )
}

/// The frame's plain rows (styles stripped) at `width` columns.
fn rows(frame: &mut CommsFrame, width: u16) -> Vec<String> {
    plain(&render_golden(frame, width))
        .lines()
        .take_while(|line| *line != "--- styles ---")
        .map(str::to_string)
        .collect()
}

#[test]
fn ready_frame_golden() {
    let _g = lock(Some("dark"));
    assert_golden!("comms_ready_100x3", render_golden(&mut ready(), 100));
}

#[test]
fn in_flight_frame_golden() {
    let _g = lock(Some("dark"));
    assert_golden!(
        "comms_in_flight_100x3",
        render_golden(&mut in_flight(), 100)
    );
}

#[test]
fn ready_shows_the_title_placeholder_and_foot() {
    let _g = lock(Some("dark"));
    let rows = rows(&mut ready(), 100);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows[0].starts_with("┏━ READY "), "row 0: {:?}", rows[0]);
    assert!(
        rows[0].contains(" amber-harbor "),
        "row 0 shows the session chip: {:?}",
        rows[0]
    );
    assert!(rows[1].contains(READY_PLACEHOLDER), "row 1: {:?}", rows[1]);
    assert!(rows[2].contains("peek"), "row 2: {:?}", rows[2]);
    assert!(
        rows[2].contains("↑48k ↓9.8k · 42% of 200k"),
        "row 2: {:?}",
        rows[2]
    );
}

#[test]
fn in_flight_shows_the_step_elapsed_time_and_queue_placeholder() {
    let _g = lock(Some("dark"));
    let rows = rows(&mut in_flight(), 100);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(
        rows[0].contains("◐ IN FLIGHT · step 1 · 4s · esc abort"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[1].contains(IN_FLIGHT_PLACEHOLDER),
        "row 1: {:?}",
        rows[1]
    );
}

#[test]
fn in_flight_without_elapsed_omits_the_time() {
    let _g = lock(Some("dark"));
    let mut frame = frame(
        CommsState::InFlight {
            elapsed: None,
            step: None,
        },
        ToolOutputView::Peek,
        "build",
    );
    let rows = rows(&mut frame, 100);
    assert!(
        rows[0].contains("◐ IN FLIGHT · esc abort"),
        "row 0: {:?}",
        rows[0]
    );
}

#[test]
fn settled_missions_read_landed_did_not_land_or_aborted_until_the_next() {
    let _g = lock(Some("dark"));
    let landed = MissionOutcome {
        mark: FlightMark::Landed,
        secs: 3.0,
        steps: 1,
    };
    let mut shown = frame(landed.comms_state(), ToolOutputView::Radar, "build");
    let row = rows(&mut shown, 100)[0].clone();
    assert!(row.contains("● LANDED · 3.0s · 1 step"), "row 0: {row:?}");

    let lost = MissionOutcome {
        mark: FlightMark::DidNotLand,
        secs: 2.5,
        steps: 4,
    };
    let mut shown = frame(lost.comms_state(), ToolOutputView::Radar, "build");
    let row = rows(&mut shown, 100)[0].clone();
    assert!(
        row.contains("⊗ DID NOT LAND · 2.5s · 4 steps"),
        "row 0: {row:?}"
    );

    let aborted = MissionOutcome {
        mark: FlightMark::Aborted,
        secs: 1.0,
        steps: 2,
    };
    let mut shown = frame(aborted.comms_state(), ToolOutputView::Radar, "build");
    let row = rows(&mut shown, 100)[0].clone();
    assert!(row.contains("⊘ ABORTED · READY"), "row 0: {row:?}");
}

#[test]
fn comms_lost_frame_golden() {
    let _g = lock(Some("dark"));
    assert_golden!(
        "comms_lost_100x3",
        render_golden(
            &mut frame(CommsState::CommsLost, ToolOutputView::Peek, "build"),
            100
        )
    );
}

#[test]
fn comms_lost_titles_the_hail_again_and_switch_ship() {
    let _g = lock(Some("dark"));
    let rows = rows(
        &mut frame(CommsState::CommsLost, ToolOutputView::Peek, "build"),
        100,
    );
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(
        rows[0].contains("⊗ COMMS LOST · enter hail again · alt+m switch ship"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[1].contains(COMMS_LOST_PLACEHOLDER),
        "row 1: {:?}",
        rows[1]
    );
}

#[test]
fn painted_frame_golden() {
    use hoocode_code_tui_app::hull_paint::HullPaint;
    let _g = lock(Some("dark"));
    let mut frame = frame_painted(
        CommsState::Ready,
        ToolOutputView::Peek,
        "build",
        Some(HullPaint::Parity),
        (Vec::new(), Vec::new()),
    );
    assert_golden!("comms_paint_100x3", render_golden(&mut frame, 100));
}

#[test]
fn painted_frame_names_the_paint_and_takes_its_colour() {
    use hoocode_code_tui_app::hull_paint::HullPaint;
    let _g = lock(Some("dark"));
    let mut frame = frame_painted(
        CommsState::Ready,
        ToolOutputView::Peek,
        "build",
        Some(HullPaint::Parity),
        (Vec::new(), Vec::new()),
    );
    let rows = rows(&mut frame, 100);
    assert!(rows[0].contains("READY"), "row 0: {:?}", rows[0]);
    assert!(rows[0].ends_with(" parity ━┓"), "row 0: {:?}", rows[0]);
    assert!(!rows[0].contains("app-edit"), "row 0: {:?}", rows[0]);
}

#[test]
fn queued_messages_draw_inside_the_frame_with_their_dispositions() {
    let _g = lock(Some("dark"));
    let mut frame = frame_painted(
        CommsState::InFlight {
            elapsed: Some(Duration::from_secs(6)),
            step: Some(1),
        },
        ToolOutputView::Peek,
        "build",
        None,
        (
            vec!["keep it short".to_string()],
            vec!["then list tests".to_string()],
        ),
    );
    let rows = rows(&mut frame, 100);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(
        rows[0].contains("◐ IN FLIGHT · step 1 · 6s · esc abort"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[1].contains("● keep it short queued · sends after this step"),
        "row 1: {:?}",
        rows[1]
    );
    assert!(
        rows[2].contains("○ then list tests queued · sends after landing"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(
        rows[4].contains("edit queue"),
        "foot names the way back: {:?}",
        rows[4]
    );
}

#[test]
fn foot_shows_the_radar_dial_and_plan_note() {
    let _g = lock(Some("dark"));
    let mut frame = frame(CommsState::Ready, ToolOutputView::Radar, "plan");
    let rows = rows(&mut frame, 100);
    assert!(
        rows[2].contains("radar · PLAN · nothing gets changed"),
        "row 2: {:?}",
        rows[2]
    );
}

#[test]
fn foot_names_each_layer_of_the_dial() {
    let _g = lock(Some("dark"));
    for (view, label) in [
        (ToolOutputView::Radar, "radar"),
        (ToolOutputView::Peek, "peek"),
        (ToolOutputView::Full, "full"),
    ] {
        let mut frame = frame(CommsState::Ready, view, "build");
        let rows = rows(&mut frame, 100);
        assert!(rows[2].contains(label), "{view:?}: {:?}", rows[2]);
    }
}

#[test]
fn stubbed_states_render_a_plausible_title() {
    let _g = lock(Some("dark"));
    for (state, title) in [
        (CommsState::CommsLost, "⊗ COMMS LOST"),
        (
            CommsState::Landed {
                elapsed: None,
                steps: None,
            },
            "● LANDED",
        ),
        (
            CommsState::DidNotLand {
                elapsed: None,
                steps: None,
            },
            "⊗ DID NOT LAND",
        ),
        (CommsState::Aborted, "⊘ ABORTED · READY"),
    ] {
        let mut frame = frame(state, ToolOutputView::Peek, "build");
        let rows = rows(&mut frame, 100);
        assert_eq!(rows.len(), 3, "{title}: {rows:?}");
        assert!(rows[0].contains(title), "{title}: {:?}", rows[0]);
    }
}

#[test]
fn input_styles_follow_the_state() {
    assert_eq!(
        CommsState::Ready.input_style(),
        InputStyle {
            placeholder: READY_PLACEHOLDER.to_string(),
            glyph: GlyphStyle::Ink,
        }
    );
    assert_eq!(
        CommsState::InFlight {
            elapsed: None,
            step: None,
        }
        .input_style(),
        InputStyle {
            placeholder: IN_FLIGHT_PLACEHOLDER.to_string(),
            glyph: GlyphStyle::Acc,
        }
    );
    assert_eq!(CommsState::CommsLost.input_style().glyph, GlyphStyle::Err);
}
