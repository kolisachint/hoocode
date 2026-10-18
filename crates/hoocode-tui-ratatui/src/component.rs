//! A ratatui drawing closure as a [`Component`].

use hoocode_tui_render::Component;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::text::{Line, Span};

use crate::ansi::truncate_spans;
use crate::lines::buffer_to_lines;

/// A component that draws with ratatui into a fixed-height buffer.
///
/// Each render allocates a `width x height` buffer, calls the draw closure with its
/// full area, and returns the buffer as lines. Width 0 or height 0 renders nothing.
pub struct RatatuiComponent<F> {
    height: u16,
    draw: F,
}

impl<F> RatatuiComponent<F>
where
    F: FnMut(Rect, &mut Buffer),
{
    /// A component `height` rows tall that draws with `draw`.
    pub fn new(height: u16, draw: F) -> Self {
        Self { height, draw }
    }
}

impl<F> Component for RatatuiComponent<F>
where
    F: FnMut(Rect, &mut Buffer),
{
    fn render(&mut self, width: u16) -> Vec<String> {
        if width == 0 || self.height == 0 {
            return Vec::new();
        }
        let area = Rect::new(0, 0, width, self.height);
        let mut buf = Buffer::empty(area);
        (self.draw)(area, &mut buf);
        buffer_to_lines(&buf)
    }
}

/// One row of `spans` as a line `width` cells wide: spans are cut with `…`
/// first when they do not fit, and the 1-line buffer only clips. The shared
/// row path (trajectory, tool chains, radar rows); rows laid out elsewhere
/// draw their own buffer, and rows that must never truncate use `set_line`
/// directly.
pub fn row(spans: Vec<Span<'static>>, width: usize) -> String {
    let width = u16::try_from(width).unwrap_or(u16::MAX).max(1);
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        // The closure may draw again after an invalidate: rebuild the row
        // from the captured spans every time.
        let spans = truncate_spans(spans.clone(), usize::from(area.width), "…");
        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    })
    .render(width)
    .pop()
    .unwrap_or_default()
}
