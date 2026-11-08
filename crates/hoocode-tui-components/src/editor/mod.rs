#[allow(clippy::module_inception)]
mod editor;
mod word_wrap;

pub use editor::{
    identity_select_theme_color, Editor, EditorBorderStyle, EditorHost, EditorOptions, EditorTheme,
    EditorTopBorderLabel, JumpDirection, TextCallback,
};
pub use word_wrap::{
    find_image_markers, find_paste_markers, is_image_marker, is_marker, is_paste_marker, len16,
    segment_with_all_markers, segment_with_markers, slice16, word_wrap_line, word_wrap_segments,
    Segment, TextChunk,
};
