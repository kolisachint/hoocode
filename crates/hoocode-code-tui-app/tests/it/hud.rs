//! The HUD row (M1c) as a component, and the app tree with full-screen on and
//! off. The HUD reads a stub session and a repo-less folder, so no paths from
//! the machine reach the golden.

use hoocode_code_tui_app::footer::{FooterModel, FooterSource};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_app::hud::{fullscreen_enabled, hud_component, tree_plan, TreePart};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

/// A fixed session: model, usage and context.
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

/// The inline tree, as it was before full-screen (`Mode::new`, today).
const INLINE_TREE: [TreePart; 13] = [
    TreePart::Header,
    TreePart::ScreenFill,
    TreePart::Attach,
    TreePart::Chat,
    TreePart::Pending,
    TreePart::Status,
    TreePart::WidgetAbove,
    TreePart::Tasks,
    TreePart::Notifications,
    TreePart::FooterTransient,
    TreePart::Editor,
    TreePart::WidgetsBelow,
    TreePart::Footer,
];

#[test]
fn hud_row_100_cols_golden() {
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    let mut component = hud_component(Box::new(Stub), data);
    assert_golden!("hud_row_100x1", render_golden(&mut component, 100));
}

#[test]
fn hud_row_shows_the_plan_chip_in_plan_mode() {
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("plan");
    let mut component = hud_component(Box::new(Stub), data);
    let rows = render_golden(&mut component, 100);
    assert!(rows.contains("◭ HOOCODE  PLAN  hoocode"), "{rows}");
}

#[test]
fn env_off_keeps_todays_inline_tree() {
    // Full-screen is on by default; HOOCODE_FULLSCREEN=0 takes it off.
    let fullscreen = fullscreen_enabled(Some("0"), true);
    assert!(!fullscreen);
    assert_eq!(tree_plan(fullscreen), INLINE_TREE);
}

#[test]
fn default_on_gives_hud_gap_body_bottom() {
    let fullscreen = fullscreen_enabled(None, true);
    assert!(fullscreen);
    let plan = tree_plan(fullscreen);
    assert_eq!(
        plan[..4],
        [
            TreePart::Hud,
            TreePart::Gap,
            TreePart::Attach,
            TreePart::Body
        ]
    );
    assert!(!plan.contains(&TreePart::Footer));
    assert!(!plan.contains(&TreePart::ScreenFill));
    assert!(!plan.contains(&TreePart::Header));
    assert!(!plan.contains(&TreePart::Chat));
}
