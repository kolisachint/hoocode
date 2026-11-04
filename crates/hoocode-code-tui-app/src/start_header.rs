//! The top of the screen (declutter spec v2, v2.1 amendment 5). On the start
//! screen (nothing said yet) two lines: ` hoocode v0.2.31` in bold and
//! ` /work/hoocode · 1 skill · 5 agents` dim. Once the transcript has a turn,
//! one blank row, so the screen never opens with a surface band at the edge.
//! No logo, no starfield.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_paths::APP_NAME;
use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::truncate_to_width;

/// Whether the transcript holds a turn yet (the start screen ends with it).
pub type HasTurns = Rc<dyn Fn() -> bool>;

/// What the start screen's second line counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartFacts {
    /// The working directory as shown (`~` for home).
    pub cwd: String,
    pub skills: usize,
    pub agents: usize,
}

/// `1 skill`, `5 agents`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The two start-screen lines, plain (no styling): name and version, then the
/// folder and what loaded. Zero counts are left out.
pub fn start_lines(version: &str, facts: &StartFacts) -> (String, String) {
    let mut parts = vec![facts.cwd.clone()];
    if facts.skills > 0 {
        parts.push(count(facts.skills, "skill"));
    }
    if facts.agents > 0 {
        parts.push(count(facts.agents, "agent"));
    }
    (
        format!(" {APP_NAME} v{version}"),
        format!(" {}", parts.join(" · ")),
    )
}

/// The top rows: the start lines, or one blank row once there is a turn.
pub struct StartHeader {
    version: String,
    facts: Rc<RefCell<StartFacts>>,
    has_turns: HasTurns,
}

impl StartHeader {
    pub fn new(version: &str, facts: Rc<RefCell<StartFacts>>, has_turns: HasTurns) -> Self {
        Self {
            version: version.to_string(),
            facts,
            has_turns,
        }
    }
}

impl Component for StartHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        if (self.has_turns)() {
            return vec![String::new()];
        }
        let width = width as usize;
        let (name, place) = start_lines(&self.version, &self.facts.borrow());
        let t = theme();
        vec![
            t.bold(&truncate_to_width(&name, width, "…", false)),
            t.fg("dim", &truncate_to_width(&place, width, "…", false)),
        ]
    }
}
