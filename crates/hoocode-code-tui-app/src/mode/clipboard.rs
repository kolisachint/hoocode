//! Copy (`/copy`) and clipboard image paste.

use hoocode_code_agent_session::TranscriptSelection;
use hoocode_code_media::clipboard::{NativeWriter, SystemClipboardHost};
use hoocode_code_media::clipboard_image::{
    extension_for_image_mime_type, read_clipboard_image, rgba_to_png, ClipboardImage,
    SystemClipboardImageHost,
};
use hoocode_code_media::markdown_to_html::markdown_to_html;
use hoocode_code_media::rich_clipboard::{copy_rich_to_clipboard, CopyFlavour, RichPayload};
use hoocode_code_paths::{APP_NAME, APP_TITLE};

use super::*;

/// The native clipboard write (`clipboard-native.ts`), only where a display
/// is available. `copy_to_clipboard` never uses it on Linux, where the
/// platform tools keep selection ownership and the native library does not.
fn native_clipboard_writer() -> Option<NativeWriter> {
    SystemClipboardHost::has_native_display().then(|| {
        Box::new(|text: &str| {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
                .map_err(|e| e.to_string())
        }) as NativeWriter
    })
}

/// The native clipboard's image as PNG (`clipboard.getImageBinary`).
fn native_clipboard_image_reader() -> Option<Box<dyn Fn() -> Option<Vec<u8>>>> {
    SystemClipboardHost::has_native_display().then(|| {
        Box::new(|| {
            let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
            rgba_to_png(
                image.width as u32,
                image.height as u32,
                image.bytes.into_owned(),
            )
        }) as Box<dyn Fn() -> Option<Vec<u8>>>
    })
}

impl Mode {
    /// `handleCopy`: the last agent message, the last n turns, or the whole
    /// session, as markdown plus (where the platform can carry it) HTML. The
    /// write runs off the UI thread; [`Self::finish_copy`] reports it.
    pub(super) fn handle_copy_command(&mut self, text: &str) {
        let argument = text
            .strip_prefix("/copy")
            .unwrap_or(text)
            .trim()
            .to_lowercase();
        // `/copy 0` is a typo, not a request for nothing.
        let turns = (!argument.is_empty() && argument.bytes().all(|b| b.is_ascii_digit()))
            .then(|| argument.parse::<usize>().unwrap_or(usize::MAX).max(1));
        let whole = argument == "all" || argument == "session";
        if !argument.is_empty() && !whole && turns.is_none() {
            self.show_warning("Usage: /copy [all|<number of turns>]");
            return;
        }

        let markdown = if whole || turns.is_some() {
            Some(self.session.get_transcript_markdown(&TranscriptSelection {
                turns,
                user_label: None,
                agent_label: Some(APP_TITLE.to_string()),
            }))
        } else {
            self.session.get_last_assistant_text()
        };
        let Some(markdown) = markdown.filter(|m| !m.is_empty()) else {
            self.show_error(if whole || turns.is_some() {
                "Nothing in this session to copy yet."
            } else {
                "No agent messages to copy yet."
            });
            return;
        };

        let subject = if whole {
            "session transcript".to_string()
        } else if let Some(turns) = turns {
            format!("last {turns} turn{}", if turns > 1 { "s" } else { "" })
        } else {
            "last agent message".to_string()
        };
        let tx = self.tx.clone();
        hoocode_runtime::spawn_thread("hoocode-ui-task", move || {
            let host = SystemClipboardHost {
                native: native_clipboard_writer(),
            };
            let payload = RichPayload {
                html: markdown_to_html(&markdown),
                text: markdown,
            };
            let result = copy_rich_to_clipboard(&host, &payload);
            let _ = tx.send(AppEvent::CopyDone(subject, result));
        });
    }

    /// The end of a `/copy`: name the flavour that landed, since "copied"
    /// meaning markdown on one machine and formatted text on another is how
    /// this gets reported as broken.
    pub(super) fn finish_copy(&mut self, subject: &str, result: Result<CopyFlavour, String>) {
        match result {
            Ok(flavour) => {
                let as_ = match flavour {
                    CopyFlavour::Rich => "markdown + formatted text",
                    CopyFlavour::Text => "markdown",
                };
                self.show_status(&format!("Copied {subject} as {as_}"));
            }
            Err(reason) => self.show_error(&format!(
                "Could not copy the {subject}: {reason}. /export writes it to a file instead."
            )),
        }
    }

    /// `handleClipboardImagePaste`: read the clipboard's image off the UI
    /// thread; [`Self::insert_pasted_image`] puts its path in the prompt.
    pub(super) fn handle_clipboard_image_paste(&mut self) {
        let tx = self.tx.clone();
        hoocode_runtime::spawn_thread("hoocode-ui-task", move || {
            let host = SystemClipboardImageHost {
                native: native_clipboard_image_reader(),
            };
            let _ = tx.send(AppEvent::PastedImage(read_clipboard_image(&host)));
        });
    }

    /// Write the pasted image to a temp file and insert its path at the
    /// cursor. Failures are silent (no clipboard access, no image).
    pub(super) fn insert_pasted_image(&mut self, image: Option<ClipboardImage>) {
        let Some(image) = image else {
            return;
        };
        let ext = extension_for_image_mime_type(&image.mime_type).unwrap_or("png");
        let file_name = format!("{APP_NAME}-clipboard-{}.{ext}", uuid::Uuid::new_v4());
        let path = std::env::temp_dir().join(file_name);
        if std::fs::write(&path, &image.bytes).is_err() {
            return;
        }
        self.editor
            .borrow_mut()
            .editor
            .insert_text_at_cursor(&path.to_string_lossy());
        self.dirty.set(true);
    }
}
