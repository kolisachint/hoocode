//! A ratatui drawing closure as a [`Component`].

use hoocode_tui_render::Component;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

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
