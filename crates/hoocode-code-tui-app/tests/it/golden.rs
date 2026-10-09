//! Component goldens (T0.2): the footer in the states the activity features
//! touch (model and token counts, no model, subagent counts, the session chip
//! on line 1), at 80 columns and 120 for the full state. The footer reads a
//! stub session, a repo-less directory and the task store (cleared first), so
//! no times or home-directory paths reach the output.

use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_app::footer::{FooterComponent, FooterModel, FooterSource};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

use crate::support::lock;

/// A fixed session: model, thinking level, usage and context numbers.
#[derive(Clone)]
struct Stub {
    name: String,
    model: Option<FooterModel>,
    usage: (u64, u64, u64, u64, f64),
    ctx: Option<(u64, Option<f64>)>,
    cwd: String,
}

impl FooterSource for Stub {
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64) {
        self.usage
    }
    fn context_usage(&self) -> Option<(u64, Option<f64>)> {
        self.ctx
    }
    fn model(&self) -> Option<FooterModel> {
        self.model.clone()
    }
    fn thinking_level(&self) -> String {
        "medium".into()
    }
    fn cwd(&self) -> String {
        self.cwd.clone()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn reserve_tokens(&self) -> u64 {
        16_384
    }
    fn is_using_oauth(&self) -> bool {
        false
    }
}

fn model() -> FooterModel {
    FooterModel {
        id: "claude-sonnet-5".into(),
        provider: "anthropic".into(),
        context_window: 200_000,
        reasoning: true,
    }
}

fn session(model: Option<FooterModel>) -> Stub {
    Stub {
        name: "amber-harbor".into(),
        model,
        usage: (48_200, 9_750, 120_000, 3_400, 0.42),
        ctx: Some((200_000, Some(31.0))),
        cwd: "/work/project".into(),
    }
}

/// A footer over a stub session in a directory with no repository.
fn footer(stub: Stub, dir: &tempfile::TempDir) -> FooterComponent {
    let data = FooterDataProvider::new(dir.path());
    data.set_available_provider_count(1);
    data.set_active_mode("build");
    FooterComponent::new(Box::new(stub), data)
}

fn reset_tasks() {
    task_store().clear();
}

fn subagent(title: &str, status: TaskStatus) {
    let id = task_store()
        .create(
            title,
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                subagent_mode: Some("explore".into()),
                ..Default::default()
            },
        )
        .id;
    if status != TaskStatus::Pending {
        task_store().update(
            id,
            TaskPatch {
                status: Some(status),
                ..Default::default()
            },
        );
    }
}

#[test]
fn footer_full_80() {
    let _g = lock(Some("dark"));
    reset_tasks();
    let dir = tempfile::tempdir().unwrap();
    let mut f = footer(session(Some(model())), &dir);
    assert_golden!("footer_full_80", render_golden(&mut f, 80));
}

#[test]
fn footer_full_120() {
    let _g = lock(Some("dark"));
    reset_tasks();
    let dir = tempfile::tempdir().unwrap();
    let mut f = footer(session(Some(model())), &dir);
    assert_golden!("footer_full_120", render_golden(&mut f, 120));
}

#[test]
fn footer_no_model_80() {
    let _g = lock(Some("dark"));
    reset_tasks();
    let dir = tempfile::tempdir().unwrap();
    let mut stub = session(None);
    stub.ctx = None;
    stub.usage = (0, 0, 0, 0, 0.0);
    let mut f = footer(stub, &dir);
    assert_golden!("footer_no_model_80", render_golden(&mut f, 80));
}

#[test]
fn footer_subagents_80() {
    let _g = lock(Some("dark"));
    reset_tasks();
    subagent("Find the footer code", TaskStatus::InProgress);
    subagent("Check the parity notes", TaskStatus::InProgress);
    subagent("Summarise the tests", TaskStatus::Pending);
    let dir = tempfile::tempdir().unwrap();
    let mut f = footer(session(Some(model())), &dir);
    assert_golden!("footer_subagents_80", render_golden(&mut f, 80));
    reset_tasks();
}

#[test]
fn footer_session_chip_80() {
    let _g = lock(Some("dark"));
    reset_tasks();
    let dir = tempfile::tempdir().unwrap();
    let mut f = footer(session(Some(model())), &dir);
    f.set_session_chip_shown(true);
    assert_golden!("footer_session_chip_80", render_golden(&mut f, 80));
}
