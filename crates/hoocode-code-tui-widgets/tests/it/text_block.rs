//! Component goldens for the shared text-block helper (U22 P6(d)): the S22
//! MCP status pane and the S23 text output blocks render through one helper,
//! [`TextBlockComponent`]. Widths mirror the mock (`--cols:100` with a
//! `data-w="40"` narrow variant).

use hoocode_code_tui_widgets::text_block::{DetailTone, StatusRow, StatusTone, TextBlockComponent};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use hoocode_tui_util::visible_width;

use crate::support::{lock, strip};

/// The S23 block: `/session`-style bold heading, dim labels, ink values, no
/// blank rows inside. Content follows the mock's S23 wide frame.
fn session_info() -> TextBlockComponent {
    let mut block = TextBlockComponent::new("Session info");
    block.add_field("Name", "amber-harbor");
    block.add_field("File", "~/.hoocode/sessions/amber-harbor.jsonl");
    block.add_field("Messages", "4 user · 4 assistant · 9 tool calls");
    block.add_field("Tokens", "23k in · 4.1k out · $1.23");
    block
}

/// The S22 pane: `MCP servers` with its dim config paths, then one glyph row
/// per server. Content follows the mock's S22 wide frame.
fn mcp_status() -> TextBlockComponent {
    let mut block = TextBlockComponent::new("MCP servers");
    block.set_heading_detail("~/.agents/mcp.json · ./.agents/mcp.json");
    block.add_status(StatusRow::new(
        StatusTone::Ready,
        "docs",
        "project",
        "4 tools",
    ));
    block.add_status(
        StatusRow::new(StatusTone::Working, "github", "user", "auth needed")
            .with_hint("/mcp login github"),
    );
    block.add_status(
        StatusRow::new(StatusTone::Failed, "search", "plugin", "timeout after 30s")
            .with_state_tone(DetailTone::Dim),
    );
    block
}

#[test]
fn text_block_session_100() {
    let _g = lock();
    let mut block = session_info();
    assert_golden!("text_block_session_100", render_golden(&mut block, 100));
}

#[test]
fn text_block_session_40() {
    let _g = lock();
    let mut block = session_info();
    assert_golden!("text_block_session_40", render_golden(&mut block, 40));
}

#[test]
fn text_block_mcp_100() {
    let _g = lock();
    let mut block = mcp_status();
    assert_golden!("text_block_mcp_100", render_golden(&mut block, 100));
}

#[test]
fn text_block_mcp_40() {
    let _g = lock();
    let mut block = mcp_status();
    assert_golden!("text_block_mcp_40", render_golden(&mut block, 40));
}

#[test]
fn narrow_panes_hide_the_source_column_and_wrap_the_hint() {
    let _g = lock();
    let lines: Vec<String> = mcp_status().render(40).iter().map(|l| strip(l)).collect();
    // The heading detail drops at 40 (the S22 narrow frame shows the
    // heading alone); the source column drops with it.
    assert!(lines.iter().any(|l| l == " MCP servers"), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("project")), "{lines:#?}");
    // The login hint wraps to its own indent-5 line.
    assert!(
        lines.iter().any(|l| l == "     /mcp login github"),
        "{lines:#?}"
    );
    // The blocking state stays on the row, and no row is blank.
    assert!(
        lines.iter().any(|l| l.contains("auth needed")),
        "{lines:#?}"
    );
    assert!(lines.iter().all(|l| !l.trim().is_empty()), "{lines:#?}");
}

#[test]
fn every_width_fits_without_blank_rows() {
    let _g = lock();
    for width in [20u16, 40, 60, 80, 100, 120] {
        for mut block in [session_info(), mcp_status()] {
            let lines = block.render(width);
            assert!(!lines.is_empty(), "width {width}");
            for line in &lines {
                assert!(
                    visible_width(line) <= usize::from(width),
                    "width {width}: {line:?}"
                );
                assert!(!strip(line).trim().is_empty(), "width {width}");
            }
        }
    }
}
