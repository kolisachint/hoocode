//! The HUD row (M1c) as a component, and the app tree with full-screen on and
//! off. The HUD reads a stub session and a repo-less folder, so no paths from
//! the machine reach the golden.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tui_app::footer::{FooterModel, FooterSource};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_app::hud::{
    fullscreen_enabled, hud_component, hud_component_shared, read_hud, tree_plan, TreePart,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

use crate::support::lock;

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
fn hud_row_painted_golden() {
    use hoocode_code_tui_app::hull_paint::HullPaint;
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    data.set_hull_paint(Some(HullPaint::Parity));
    let mut component = hud_component(Box::new(Stub), data);
    assert_golden!("hud_row_paint_100x1", render_golden(&mut component, 100));
}

#[test]
fn hud_row_shows_the_paint_name_before_the_ship() {
    use hoocode_code_tui_app::hull_paint::HullPaint;
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    data.set_hull_paint(Some(HullPaint::Parity));
    let mut component = hud_component(Box::new(Stub), data);
    let rows = render_golden(&mut component, 100);
    assert!(
        rows.contains("hoocode") && rows.contains("parity  SHIP"),
        "{rows}"
    );
}

#[test]
fn hud_row_shows_the_plan_chip_in_plan_mode() {
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("plan");
    let mut component = hud_component(Box::new(Stub), data);
    let rows = render_golden(&mut component, 100);
    assert!(
        rows.contains("◭ HOOCODE") && rows.contains("PLAN") && rows.contains("/work/hoocode"),
        "{rows}"
    );
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

/// A stub session with a swappable model id: in-place mutation stands in for
/// `set_model` on the shared `AgentSession`, a box swap for a rebind.
#[derive(Clone)]
struct ModelStub {
    id: Rc<RefCell<String>>,
}

impl ModelStub {
    fn named(id: &str) -> Self {
        Self {
            id: Rc::new(RefCell::new(id.into())),
        }
    }

    fn set_model(&self, id: &str) {
        *self.id.borrow_mut() = id.into();
    }
}

impl FooterSource for ModelStub {
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64) {
        (48_200, 9_750, 120_000, 3_400, 1.23)
    }
    fn context_usage(&self) -> Option<(u64, Option<f64>)> {
        Some((200_000, Some(42.0)))
    }
    fn model(&self) -> Option<FooterModel> {
        Some(FooterModel {
            id: self.id.borrow().clone(),
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

#[test]
fn hud_reads_the_model_id_for_the_ship() {
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    let hud = read_hud(&ModelStub::named("claude-haiku-5-5"), &data);
    assert_eq!(hud.model, "claude-haiku-5-5");
}

#[test]
fn hud_follows_a_same_session_model_switch() {
    // `set_model` mutates the shared session in place: the HUD reads the new
    // id on the next render without any rebind.
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    let stub = ModelStub::named("model-a");
    let mut component = hud_component_shared(
        Rc::new(RefCell::new(Box::new(stub.clone()) as Box<dyn FooterSource>)),
        data,
    );
    let before = render_golden(&mut component, 100);
    assert!(before.contains("SHIP model-a"), "{before}");
    stub.set_model("model-b");
    let after = render_golden(&mut component, 100);
    assert!(after.contains("SHIP model-b"), "{after}");
}

#[test]
fn hud_follows_a_rebind_source_swap() {
    // `/new`, `/resume` and `/reload` rebind the session: swapping the shared
    // source must move SHIP to the new session's model.
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_active_mode("build");
    let shared: Rc<RefCell<Box<dyn FooterSource>>> =
        Rc::new(RefCell::new(Box::new(ModelStub::named("model-a"))));
    let mut component = hud_component_shared(shared.clone(), data);
    let before = render_golden(&mut component, 100);
    assert!(before.contains("SHIP model-a"), "{before}");
    *shared.borrow_mut() = Box::new(ModelStub::named("model-b"));
    let after = render_golden(&mut component, 100);
    assert!(after.contains("SHIP model-b"), "{after}");
    assert!(!after.contains("SHIP model-a"), "{after}");
}
