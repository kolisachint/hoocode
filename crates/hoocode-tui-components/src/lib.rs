//! UI components for the hoocode TUI.
//!
//! The autocomplete provider and the editor component interface live at the
//! crate root, tightly coupled to the `Editor` component.

mod autocomplete;
mod box_component;
mod color;
mod editor;
mod frame;
mod image;
mod input;
mod loader;
pub mod markdown;
mod picker_cursor;
mod select_list;
mod settings_list;
mod spacer;
mod text;
mod truncated_text;

pub use autocomplete::{
    ApplyCompletionResult, ArgumentCompletionsFn, AutocompleteItem, AutocompleteProvider,
    AutocompleteSuggestions, CombinedAutocompleteProvider, CommandEntry, FileFinder, FileMatch,
    SlashCommand,
};
pub use box_component::{BoxComponent, PaperFn, PaperSheet};
pub use color::{identity_color, ColorFn};
pub use editor::{
    find_paste_markers, identity_select_theme_color, is_paste_marker, len16, segment_with_markers,
    slice16, word_wrap_line, word_wrap_segments, Editor, EditorBorderStyle, EditorHost,
    EditorOptions, EditorTheme, EditorTopBorderLabel, JumpDirection, Segment, TextCallback,
    TextChunk,
};
pub use frame::{
    render_frame_edge, Frame, FrameBorderChars, FrameBorderCharsOverride, FrameBorderStyle,
    FrameEdge, FrameEdgeOptions, FrameLabel, FrameOptions, LABEL_RIGHT_INSET, MIN_LABEL_LEAD_IN,
};
pub use hoocode_tui_render::FlexSpacer;
pub use image::{Image, ImageOptions, ImageTheme};
pub use input::{Input, DEFAULT_INPUT_PROMPT};
pub use loader::{Loader, LoaderIndicatorOptions};
pub use markdown::{DefaultTextStyle, HeadingFn, HighlightCodeFn, Markdown, MarkdownTheme};
pub use picker_cursor::PickerCursor;
pub use select_list::{SelectItem, SelectList, SelectListLayoutOptions, SelectListTheme};
pub use settings_list::{
    SettingItem, SettingsList, SettingsListOptions, SettingsListTheme, SubmenuFactory,
    SubmenuOutcome,
};
pub use spacer::Spacer;
pub use text::Text;
pub use truncated_text::TruncatedText;
