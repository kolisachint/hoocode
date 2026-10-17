//! Golden: a ratatui widget tree rendered through the Component model. This is the
//! Mission Control comms frame sketch (M0): a heavy bordered block with a title and
//! a prompt line inside.

use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::widgets::Widget;
use hoocode_tui_ratatui::widgets::block::Block;
use hoocode_tui_ratatui::widgets::borders::BorderType;
use hoocode_tui_ratatui::widgets::paragraph::Paragraph;
use hoocode_tui_ratatui::RatatuiComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

fn comms_frame(area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .title("READY");
    let inner = block.inner(area);
    block.render(area, buf);
    Paragraph::new("▸  name your mission").render(inner, buf);
}

#[test]
fn comms_frame_40x3() {
    let mut component = RatatuiComponent::new(3, comms_frame);
    assert_golden!("comms_frame_40x3", render_golden(&mut component, 40));
}
